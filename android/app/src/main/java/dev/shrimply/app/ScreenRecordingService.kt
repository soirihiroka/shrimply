package dev.shrimply.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.media.MediaMuxer
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.view.Surface
import android.view.WindowManager
import org.json.JSONObject
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean

/** Android's screen grant feeds a hardware encoder Surface without CPU pixel copies. */
class ScreenRecordingService : Service() {
    companion object {
        private const val Channel = "screen-recording"
        private const val NotificationId = 1
        private const val Stop = "dev.shrimply.app.STOP_SCREEN_RECORDING"
        private const val PollTimeoutUs = 10_000L
        private const val StopTimeoutMs = 5_000L
        private data class Delivery(val owner: String, val event: (JSONObject) -> Unit)
        private val deliveries = ConcurrentHashMap<String, Delivery>()

        internal fun start(context: Context, owner: String, config: JSONObject, resultCode: Int,
            grant: Intent, event: (JSONObject) -> Unit) {
            val id = config.getString("id")
            if (deliveries.putIfAbsent(id, Delivery(owner, event)) != null) return
            try {
                context.startForegroundService(Intent(context, ScreenRecordingService::class.java)
                    .putExtra("config", config.toString()).putExtra("result", resultCode).putExtra("grant", grant))
            } catch (error: Exception) {
                deliveries.remove(id)?.event?.invoke(JSONObject().put("id", id).put("event", "failed").put("error", error.message))
            }
        }

        internal fun stop(context: Context, id: String): Boolean {
            if (!deliveries.containsKey(id)) return false
            context.startService(Intent(context, ScreenRecordingService::class.java).setAction(Stop).putExtra("id", id))
            return true
        }

        internal fun stopOwner(context: Context, owner: String) {
            deliveries.entries.filter { it.value.owner == owner }.forEach { stop(context, it.key) }
        }
    }

    private val stopping = AtomicBoolean(false)
    private var recordingId: String? = null
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == Stop) {
            if (intent.getStringExtra("id") == recordingId) stopping.set(true)
            return START_NOT_STICKY
        }
        val config = intent?.getStringExtra("config")?.let(::JSONObject) ?: run { stopSelf(); return START_NOT_STICKY }
        val id = config.getString("id")
        if (recordingId != null) {
            deliveries.remove(id)?.event?.invoke(JSONObject().put("id", id).put("event", "failed")
                .put("error", "A screen capture is already active"))
            return START_NOT_STICKY
        }
        recordingId = id
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(Channel, config.getString("notification_title"), NotificationManager.IMPORTANCE_LOW))
        val stop = PendingIntent.getService(this, NotificationId,
            Intent(this, ScreenRecordingService::class.java).setAction(Stop).putExtra("id", id),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val notification = Notification.Builder(this, Channel).setSmallIcon(R.drawable.shrimply_monochrome)
            .setContentTitle(config.getString("notification_title")).setContentText(config.getString("notification_text"))
            .setOngoing(true).addAction(Notification.Action.Builder(null, config.getString("stop_label"), stop).build()).build()
        startForeground(NotificationId, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)
        val grant = intent.getParcelableExtra("grant", Intent::class.java)
        val result = intent.getIntExtra("result", 0)
        Thread({ capture(config, result, grant) }, "Shrimply screen capture").start()
        return START_NOT_STICKY
    }

    override fun onDestroy() { stopping.set(true); super.onDestroy() }

    private fun capture(config: JSONObject, resultCode: Int, grant: Intent?) {
        val id = config.getString("id")
        fun event(value: JSONObject) { value.put("id", id); deliveries[id]?.event?.invoke(value) }
        var codec: MediaCodec? = null
        var surface: Surface? = null
        var projection: MediaProjection? = null
        var display: VirtualDisplay? = null
        var muxer: MediaMuxer? = null
        var muxing = false
        var successful = false
        val path = File(config.getString("path"))
        var terminal = JSONObject().put("event", "cancelled")
        try {
            check(grant != null) { "Screen capture permission was not granted" }
            if (stopping.get()) return
            val bounds = getSystemService(WindowManager::class.java).maximumWindowMetrics.bounds
            val mime = config.getString("mime")
            val fps = config.getLong("fps_numerator").toDouble() / config.getLong("fps_denominator")
            val candidates = MediaCodecList(MediaCodecList.ALL_CODECS).codecInfos.filter {
                it.isEncoder && it.isHardwareAccelerated && it.supportedTypes.any { type -> type.equals(mime, ignoreCase = true) }
            }
            val encoder = candidates.firstOrNull { info ->
                val caps = info.getCapabilitiesForType(mime)
                val video = caps.videoCapabilities
                val width = ((bounds.width() + video.widthAlignment - 1) / video.widthAlignment) * video.widthAlignment
                val height = ((bounds.height() + video.heightAlignment - 1) / video.heightAlignment) * video.heightAlignment
                MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface in caps.colorFormats && video.areSizeAndRateSupported(width, height, fps)
            } ?: error("No hardware encoder supports the captured screen size and frame rate")
            val capabilities = encoder.getCapabilitiesForType(mime).videoCapabilities
            val width = ((bounds.width() + capabilities.widthAlignment - 1) / capabilities.widthAlignment) * capabilities.widthAlignment
            val height = ((bounds.height() + capabilities.heightAlignment - 1) / capabilities.heightAlignment) * capabilities.heightAlignment
            val format = MediaFormat.createVideoFormat(mime, width, height).apply {
                setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
                setInteger(MediaFormat.KEY_BIT_RATE, capabilities.bitrateRange.clamp(config.getInt("bitrate")))
                setFloat(MediaFormat.KEY_FRAME_RATE, fps.toFloat())
                setFloat(MediaFormat.KEY_MAX_FPS_TO_ENCODER, fps.toFloat())
                setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, config.getInt("keyframe_seconds"))
                setInteger(MediaFormat.KEY_MAX_B_FRAMES, 0)
            }
            val activeCodec = MediaCodec.createByCodecName(encoder.name)
            codec = activeCodec
            activeCodec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            surface = activeCodec.createInputSurface()
            muxer = MediaMuxer(path.absolutePath, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)
            val activeMuxer = muxer
            activeCodec.start()
            val activeProjection = getSystemService(MediaProjectionManager::class.java).getMediaProjection(resultCode, grant)
            projection = activeProjection
            activeProjection.registerCallback(object : MediaProjection.Callback() {
                override fun onStop() { stopping.set(true) }
            }, Handler(Looper.getMainLooper()))
            display = activeProjection.createVirtualDisplay(config.getString("notification_title"), width, height,
                resources.configuration.densityDpi, DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, surface, null, null)
            event(JSONObject().put("event", "ready").put("width", width).put("height", height))
            val info = MediaCodec.BufferInfo()
            var track = -1
            var firstTimestamp: Long? = null
            var lastTimestamp = -1L
            var stopAt: Long? = null
            var eos = false
            while (!eos) {
                if (stopping.get() && stopAt == null) {
                    display.release(); display = null
                    activeProjection.stop()
                    activeCodec.signalEndOfInputStream()
                    stopAt = android.os.SystemClock.elapsedRealtime()
                }
                check(stopAt == null || android.os.SystemClock.elapsedRealtime() - stopAt < StopTimeoutMs) { "Hardware encoder did not finish the screen recording" }
                when (val index = activeCodec.dequeueOutputBuffer(info, PollTimeoutUs)) {
                    MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                        check(track == -1) { "Hardware encoder changed its output format during recording" }
                        track = activeMuxer.addTrack(activeCodec.outputFormat)
                        activeMuxer.start(); muxing = true
                    }
                    MediaCodec.INFO_TRY_AGAIN_LATER -> Unit
                    else -> if (index >= 0) {
                        try {
                            if (info.size > 0 && info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG == 0) {
                                check(muxing) { "Hardware encoder returned data before its output format" }
                                val bytes = checkNotNull(activeCodec.getOutputBuffer(index))
                                if (firstTimestamp == null) firstTimestamp = info.presentationTimeUs
                                info.presentationTimeUs -= checkNotNull(firstTimestamp)
                                check(info.presentationTimeUs >= lastTimestamp) { "Screen recording timestamps went backwards" }
                                lastTimestamp = info.presentationTimeUs
                                activeMuxer.writeSampleData(track, bytes, info)
                            }
                            eos = info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0
                        } finally { activeCodec.releaseOutputBuffer(index, false) }
                    }
                }
            }
            check(lastTimestamp >= 0L) { "Screen recording captured no video frames" }
            activeMuxer.stop(); muxing = false
            terminal = JSONObject().put("event", "finished").put("width", width).put("height", height)
                .put("last_timestamp_us", lastTimestamp)
            successful = true
        } catch (error: Exception) {
            terminal = JSONObject().put("event", "failed").put("error", error.message ?: error.javaClass.simpleName)
        } finally {
            display?.release()
            projection?.stop()
            if (muxing) runCatching { muxer?.stop() }
            muxer?.release()
            runCatching { codec?.stop() }
            codec?.release()
            surface?.release()
            if (!successful) path.delete()
            event(terminal)
            deliveries.remove(id)
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        }
    }
}
