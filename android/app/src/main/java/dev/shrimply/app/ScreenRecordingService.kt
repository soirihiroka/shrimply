package dev.shrimply.app

import android.media.MediaCodecInfo

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

internal object NativeScreenRecorder {
    external fun open(config: String): Surface
    external fun poll(): Boolean
    external fun finish(elapsedUs: Long): Long
    external fun close()
}

/** Android's screen grant feeds a hardware encoder Surface without CPU pixel copies. */
class ScreenRecordingService : Service() {
    companion object {
        private const val Channel = "screen-recording"
        private const val NotificationId = 1
        private const val Stop = "dev.shrimply.app.STOP_SCREEN_RECORDING"
        private const val PollIntervalMs = 10L
        private const val ImageTimeoutMs = 10_000L
        private class Delivery(val owner: String, @Volatile var event: (JSONObject) -> Unit) {
            @Volatile var terminal: JSONObject? = null
            @Volatile var image: Bitmap? = null
            @Volatile var abandoned = false
            val stopping = AtomicBoolean(false)
        }
        private val deliveries = ConcurrentHashMap<String, Delivery>()

        internal fun busy(): Boolean = deliveries.values.any { it.terminal == null }

        private fun deliver(id: String, value: JSONObject, image: Bitmap? = null) {
            value.put("id", id)
            deliveries[id]?.let { delivery ->
                val terminal = value.getString("event") != "ready"
                if (terminal) { delivery.image = image; delivery.terminal = value }
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

        internal fun acknowledge(id: String): Bitmap? = deliveries[id]?.takeIf { it.terminal != null && deliveries.remove(id, it) }?.image

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
        var pixels: Bitmap? = null
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
                        try { pixels = checkNotNull(hardware.copy(Bitmap.Config.ARGB_8888, false)) }
                        finally { hardware.recycle() }
                    }
                }
                if (stopping.get()) break
                terminal = JSONObject().put("event", "finished")
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
            deliver(id, terminal, pixels.takeIf { terminal.getString("event") == "finished" })
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        }
    }

    private fun capture(config: JSONObject, resultCode: Int, grant: Intent?) {
        if (config.optString("mode") == "image") { captureImage(config, resultCode, grant); return }
        val id = config.getString("id")
        var surface: Surface? = null
        var projection: MediaProjection? = null
        var display: VirtualDisplay? = null
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
            val settings = videoEncoders(mime, hardware = true).firstNotNullOfOrNull { (codec, caps) ->
                if (MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface !in caps.colorFormats) return@firstNotNullOfOrNull null
                val video = caps.videoCapabilities ?: return@firstNotNullOfOrNull null
                fun align(size: Int, multiple: Int) = ((size + multiple - 1) / multiple) * multiple
                val width = align(screenWidth, video.widthAlignment)
                val height = align(screenHeight, video.heightAlignment)
                if (!video.areSizeAndRateSupported(width, height, fps)) return@firstNotNullOfOrNull null
                config.put("width", width).put("height", height).put("codec_name", codec.name)
                    .put("bitrate", video.bitrateRange.clamp(config.getInt("bitrate")))
            } ?: error("No hardware encoder supports the captured screen size and frame rate")
            val width = settings.getInt("width")
            val height = settings.getInt("height")
            surface = NativeScreenRecorder.open(settings.toString())
            val activeProjection = projection(resultCode, grant)
            projection = activeProjection
            display = checkNotNull(activeProjection.createVirtualDisplay(config.getString("notification_title"), width, height,
                config.getInt("density_dpi"), DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, surface, null, null)) { "Could not create the recording display" }
            val started = System.nanoTime()
            deliver(id, JSONObject().put("event", "ready").put("width", width).put("height", height))
            while (!stopping.get()) {
                if (!NativeScreenRecorder.poll()) Thread.sleep(PollIntervalMs)
            }
            val elapsedUs = TimeUnit.NANOSECONDS.toMicros(System.nanoTime() - started)
            display.release()
            display = null
            activeProjection.stop()
            projection = null
            val duration = NativeScreenRecorder.finish(elapsedUs)
            terminal = JSONObject().put("event", "finished").put("width", width).put("height", height)
                .put("duration_us", duration)
            successful = true
        } catch (error: Exception) {
            terminal = JSONObject().put("event", "failed").put("error", error.message ?: error.javaClass.simpleName)
        } finally {
            var cleanupFailure: Exception? = null
            fun release(action: () -> Unit) { try { action() } catch (error: Exception) { if (cleanupFailure == null) cleanupFailure = error } }
            release { display?.release() }
            release { projection?.stop() }
            release { NativeScreenRecorder.close() }
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
