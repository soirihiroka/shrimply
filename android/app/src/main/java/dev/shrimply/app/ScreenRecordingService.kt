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
import android.graphics.Bitmap
import android.graphics.ColorSpace
import android.graphics.PixelFormat
import android.hardware.HardwareBuffer
import android.media.ImageReader
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
import org.json.JSONObject
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Semaphore
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

/** Android's screen grant feeds a hardware encoder Surface without CPU pixel copies. */
class ScreenRecordingService : Service() {
    companion object {
        private const val Channel = "screen-recording"
        private const val NotificationId = 1
        private const val Stop = "dev.shrimply.app.STOP_SCREEN_RECORDING"
        private const val PollTimeoutUs = 10_000L
        private const val StopTimeoutMs = 5_000L
        private const val ImageTimeoutMs = 10_000L
        private class Delivery(val owner: String, @Volatile var event: (JSONObject) -> Unit) {
            @Volatile var terminal: JSONObject? = null
            @Volatile var abandoned = false
            val stopping = AtomicBoolean(false)
        }
        private val deliveries = ConcurrentHashMap<String, Delivery>()

        private fun deliver(id: String, value: JSONObject) {
            value.put("id", id)
            deliveries[id]?.let { delivery ->
                val terminal = value.getString("event") != "ready"
                if (terminal) delivery.terminal = value
                delivery.event(value)
                if (terminal && delivery.abandoned) deliveries.remove(id, delivery)
            }
        }

        internal fun start(context: Context, owner: String, config: JSONObject, resultCode: Int,
            grant: Intent, event: (JSONObject) -> Unit) {
            val id = config.getString("id")
            if (deliveries.putIfAbsent(id, Delivery(owner, event)) != null) return
            try {
                context.startForegroundService(Intent(context, ScreenRecordingService::class.java)
                    .putExtra("config", config.toString()).putExtra("result", resultCode).putExtra("grant", grant))
            } catch (error: Exception) {
                val failure = JSONObject().put("id", id).put("event", "failed").put("error", error.message)
                deliver(id, failure)
            }
        }

        internal fun observe(owner: String, id: String, event: (JSONObject) -> Unit) {
            deliveries[id]?.takeIf { it.owner == owner }?.let { delivery ->
                delivery.event = event
                delivery.terminal?.let(event)
            }
        }

        internal fun acknowledge(id: String) { deliveries[id]?.takeIf { it.terminal != null }?.let { deliveries.remove(id, it) } }

        internal fun stop(id: String): Boolean {
            val delivery = deliveries[id] ?: return false
            delivery.stopping.set(true)
            return true
        }

        internal fun stopOwner(owner: String) {
            deliveries.entries.filter { it.value.owner == owner }.forEach { (id, delivery) ->
                delivery.stopping.set(true)
                delivery.abandoned = true
                delivery.event = {}
                if (delivery.terminal != null) deliveries.remove(id, delivery)
            }
        }
    }

    private var stopping = AtomicBoolean(false)
    private var recordingId: String? = null
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == Stop) {
            if (intent.getStringExtra("id") == recordingId) stopping.set(true)
            if (recordingId == null) stopSelf(startId)
            return START_NOT_STICKY
        }
        val config = intent?.getStringExtra("config")?.let(::JSONObject) ?: run { stopSelf(); return START_NOT_STICKY }
        val id = config.getString("id")
        if (recordingId != null) {
            if (recordingId != id) deliver(id, JSONObject().put("event", "failed").put("error", "A screen capture is already active"))
            return START_NOT_STICKY
        }
        recordingId = id
        stopping = deliveries[id]?.stopping ?: AtomicBoolean(true)
        try {
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
        } catch (error: Exception) {
            val failure = JSONObject().put("id", id).put("event", "failed").put("error", error.message ?: error.javaClass.simpleName)
            deliver(id, failure)
            stopSelf()
        }
        return START_NOT_STICKY
    }

    override fun onDestroy() { stopping.set(true); super.onDestroy() }

    private fun projection(resultCode: Int, grant: Intent?): MediaProjection {
        check(grant != null) { "Screen capture permission was not granted" }
        return checkNotNull(getSystemService(MediaProjectionManager::class.java).getMediaProjection(resultCode, grant)).also {
            it.registerCallback(object : MediaProjection.Callback() {
                override fun onStop() { stopping.set(true) }
            }, Handler(Looper.getMainLooper()))
        }
    }

    /** One consented frame for native pixel sampling, using the existing capture service. */
    private fun captureImage(config: JSONObject, resultCode: Int, grant: Intent?) {
        val id = config.getString("id")
        val path = File(config.getString("path"))
        var reader: ImageReader? = null
        var projection: MediaProjection? = null
        var display: VirtualDisplay? = null
        var terminal = JSONObject().put("event", "cancelled")
        try {
            val width = config.getInt("screen_width")
            val height = config.getInt("screen_height")
            val frames = Semaphore(0)
            val images = ImageReader.newInstance(width, height, PixelFormat.RGBA_8888, 2,
                HardwareBuffer.USAGE_GPU_SAMPLED_IMAGE or HardwareBuffer.USAGE_CPU_READ_OFTEN)
            reader = images
            images.setOnImageAvailableListener({ frames.release() }, Handler(Looper.getMainLooper()))
            val capture = projection(resultCode, grant)
            projection = capture
            display = capture.createVirtualDisplay(config.getString("notification_title"), width, height,
                config.getInt("density_dpi"), DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, images.surface, null, null)
            val deadline = android.os.SystemClock.elapsedRealtime() + ImageTimeoutMs
            while (!stopping.get()) {
                check(android.os.SystemClock.elapsedRealtime() < deadline) { "Screen capture returned no image" }
                if (!frames.tryAcquire(100, TimeUnit.MILLISECONDS)) continue
                val image = images.acquireLatestImage() ?: continue
                image.use {
                    checkNotNull(it.hardwareBuffer).use { buffer ->
                        val hardware = checkNotNull(Bitmap.wrapHardwareBuffer(buffer, ColorSpace.get(ColorSpace.Named.SRGB)))
                        val pixels = checkNotNull(hardware.copy(Bitmap.Config.ARGB_8888, false))
                        try {
                            path.outputStream().use { stream -> check(pixels.compress(Bitmap.CompressFormat.PNG, 100, stream)) }
                        } finally { pixels.recycle(); hardware.recycle() }
                    }
                }
                if (stopping.get()) break
                terminal = JSONObject().put("event", "finished").put("path", path.absolutePath)
                break
            }
        } catch (error: Exception) {
            terminal = JSONObject().put("event", "failed").put("error", error.message ?: error.javaClass.simpleName)
        } finally {
            var cleanupFailure: Exception? = null
            for (release in listOf<() -> Unit>({ display?.release() }, { projection?.stop() }, { reader?.close() })) {
                try { release() } catch (error: Exception) { if (cleanupFailure == null) cleanupFailure = error }
            }
            if (cleanupFailure != null) terminal = JSONObject().put("event", "failed").put("error", cleanupFailure.message)
            if (terminal.getString("event") != "finished" && path.exists() && !path.delete()) {
                terminal = JSONObject().put("event", "failed").put("error", "capture_cleanup_failed")
            }
            deliver(id, terminal)
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        }
    }

    private fun capture(config: JSONObject, resultCode: Int, grant: Intent?) {
        if (config.optString("mode") == "image") { captureImage(config, resultCode, grant); return }
        val id = config.getString("id")
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
            val screenWidth = config.getInt("screen_width")
            val screenHeight = config.getInt("screen_height")
            val mime = config.getString("mime")
            val fps = config.getLong("fps_numerator").toDouble() / config.getLong("fps_denominator")
            val candidates = MediaCodecList(MediaCodecList.ALL_CODECS).codecInfos.filter {
                it.isEncoder && it.isHardwareAccelerated && it.supportedTypes.any { type -> type.equals(mime, ignoreCase = true) }
            }
            val encoder = candidates.firstOrNull { info ->
                val caps = info.getCapabilitiesForType(mime)
                val video = caps.videoCapabilities ?: return@firstOrNull false
                val width = ((screenWidth + video.widthAlignment - 1) / video.widthAlignment) * video.widthAlignment
                val height = ((screenHeight + video.heightAlignment - 1) / video.heightAlignment) * video.heightAlignment
                MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface in caps.colorFormats && video.areSizeAndRateSupported(width, height, fps)
            } ?: error("No hardware encoder supports the captured screen size and frame rate")
            val capabilities = checkNotNull(encoder.getCapabilitiesForType(mime).videoCapabilities)
            val width = ((screenWidth + capabilities.widthAlignment - 1) / capabilities.widthAlignment) * capabilities.widthAlignment
            val height = ((screenHeight + capabilities.heightAlignment - 1) / capabilities.heightAlignment) * capabilities.heightAlignment
            val format = MediaFormat.createVideoFormat(mime, width, height).apply {
                setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
                setInteger(MediaFormat.KEY_BIT_RATE, capabilities.bitrateRange.clamp(config.getInt("bitrate")))
                setFloat(MediaFormat.KEY_FRAME_RATE, fps.toFloat())
                setFloat(MediaFormat.KEY_MAX_FPS_TO_ENCODER, fps.toFloat())
                setLong(MediaFormat.KEY_REPEAT_PREVIOUS_FRAME_AFTER, config.getLong("repeat_frame_after_us"))
                setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, config.getInt("keyframe_seconds"))
                setInteger(MediaFormat.KEY_MAX_B_FRAMES, config.getInt("b_frames"))
            }
            val activeCodec = MediaCodec.createByCodecName(encoder.name)
            codec = activeCodec
            activeCodec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            surface = activeCodec.createInputSurface()
            val activeMuxer = MediaMuxer(path.absolutePath, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)
            muxer = activeMuxer
            activeCodec.start()
            val activeProjection = projection(resultCode, grant)
            projection = activeProjection
            display = activeProjection.createVirtualDisplay(config.getString("notification_title"), width, height,
                config.getInt("density_dpi"), DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, surface, null, null)
            deliver(id, JSONObject().put("event", "ready").put("width", width).put("height", height))
            val info = MediaCodec.BufferInfo()
            var track = -1
            var firstTimestamp: Long? = null
            var lastTimestamp = -1L
            var stopAt: Long? = null
            var eos = false
            while (!eos) {
                if (stopping.get() && stopAt == null) {
                    display?.release(); display = null
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
                                bytes.position(info.offset); bytes.limit(info.offset + info.size)
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
            var cleanupFailure: Exception? = null
            fun release(action: () -> Unit) { try { action() } catch (error: Exception) { if (cleanupFailure == null) cleanupFailure = error } }
            release { display?.release() }
            release { projection?.stop() }
            release { if (muxing) muxer?.stop() }
            release { muxer?.release() }
            release { codec?.stop() }
            release { codec?.release() }
            release { surface?.release() }
            if (successful && cleanupFailure != null) {
                successful = false
                terminal = JSONObject().put("event", "failed").put("error", cleanupFailure.message)
            }
            if (!successful && path.exists() && !path.delete()) terminal = JSONObject().put("event", "failed").put("error", "recording_cleanup_failed")
            deliver(id, terminal)
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        }
    }
}
