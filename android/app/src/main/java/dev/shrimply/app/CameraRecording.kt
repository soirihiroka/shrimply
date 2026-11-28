package dev.shrimply.app

import android.annotation.SuppressLint
import android.content.Context
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.params.OutputConfiguration
import android.hardware.camera2.params.SessionConfiguration
import android.media.MediaMetadataRetriever
import android.media.MediaRecorder
import android.os.Handler
import android.os.HandlerThread
import android.util.Size
import org.json.JSONObject

internal class CameraRecording(
    private val context: Context,
    private val config: JSONObject,
    private val deviceRotation: Int,
    private val deliver: (JSONObject) -> Unit,
) {
    val id: String = config.getString("id")
    private val thread = HandlerThread("camera-recording").apply { start() }
    private val handler = Handler(thread.looper)
    private var recorder: MediaRecorder? = null
    private var camera: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    private var opening = false
    private var started = false
    private var finished = false
    private lateinit var size: Size
    private var orientation = 0

    init { handler.post { attempt(::open) } }

    fun stop() { handler.post { finish(null) } }

    @SuppressLint("MissingPermission")
    private fun open() {
        val manager = context.getSystemService(CameraManager::class.java)
        val cameraId = config.getString("camera_id")
        val characteristics = manager.getCameraCharacteristics(cameraId)
        val sizes = characteristics.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
            ?.getOutputSizes(MediaRecorder::class.java).orEmpty()
        val byArea: (Size) -> Int = { it.width * it.height }
        size = sizes.filter { it.width <= 1920 && it.height <= 1080 }.maxByOrNull(byArea)
            ?: sizes.minByOrNull(byArea) ?: error("The camera has no video recording formats")
        val requestedRate = (config.getDouble("fps_numerator") / config.getDouble("fps_denominator"))
            .toInt().coerceAtLeast(1)
        val ranges = characteristics.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES).orEmpty()
        val range = ranges.minWithOrNull(compareBy({ kotlin.math.abs(it.upper - requestedRate) }, { it.upper - it.lower }))
        val video = MediaRecorder(context)
        recorder = video
        video.setOnErrorListener { _, what, extra ->
            finish("Camera recording failed ($what, $extra)")
        }
        video.setVideoSource(MediaRecorder.VideoSource.SURFACE)
        video.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4)
        video.setVideoEncoder(MediaRecorder.VideoEncoder.H264)
        video.setVideoSize(size.width, size.height)
        video.setVideoFrameRate(range?.upper ?: requestedRate)
        video.setVideoEncodingBitRate(config.getInt("bitrate"))
        val sensorRotation = characteristics.get(CameraCharacteristics.SENSOR_ORIENTATION) ?: 0
        val front = characteristics.get(CameraCharacteristics.LENS_FACING) == CameraCharacteristics.LENS_FACING_FRONT
        orientation = (sensorRotation + (if (front) -deviceRotation else deviceRotation) + 360) % 360
        video.setOrientationHint(orientation)
        video.setOutputFile(config.getString("path"))
        video.prepare()
        val surface = video.surface
        manager.openCamera(cameraId, object : CameraDevice.StateCallback() {
            override fun onOpened(device: CameraDevice) {
                opening = false
                if (finished) { device.close(); return }
                camera = device
                attempt {
                    val request = device.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
                        addTarget(surface)
                        if (range != null) set(android.hardware.camera2.CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, range)
                    }.build()
                    device.createCaptureSession(SessionConfiguration(
                        SessionConfiguration.SESSION_REGULAR, listOf(OutputConfiguration(surface)),
                        { task -> handler.post(task) }, object : CameraCaptureSession.StateCallback() {
                            override fun onConfigured(capture: CameraCaptureSession) {
                                if (finished) { capture.close(); return }
                                session = capture
                                attempt {
                                    capture.setRepeatingRequest(request, null, handler)
                                    video.start()
                                    started = true
                                    deliver(event("ready"))
                                }
                            }
                            override fun onConfigureFailed(capture: CameraCaptureSession) {
                                capture.close()
                                finish("Could not configure the camera")
                            }
                        },
                    ))
                }
            }
            override fun onDisconnected(device: CameraDevice) {
                opening = false
                device.close()
                finish("The camera was disconnected")
            }
            override fun onError(device: CameraDevice, error: Int) {
                opening = false
                device.close()
                finish("Could not record video with the selected camera ($error)")
            }
            override fun onClosed(device: CameraDevice) {
                if (finished) thread.quitSafely()
            }
        }, handler)
        opening = true
    }

    private fun attempt(operation: () -> Unit) {
        try { operation() }
        catch (error: Exception) { finish(error.message ?: error.javaClass.simpleName) }
    }

    private fun event(state: String): JSONObject {
        val event = JSONObject().put("id", id).put("event", state)
        if (::size.isInitialized) event.put("width", size.width).put("height", size.height)
        return event
    }

    private fun finish(error: String?) {
        if (finished) return
        finished = true
        val result = runCatching {
            if (error != null) return@runCatching event("failed").put("error", error)
            if (!started) return@runCatching event("cancelled")
            recorder?.stop()
            val duration = MediaMetadataRetriever().use {
                it.setDataSource(config.getString("path"))
                it.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLongOrNull()
            } ?: kotlin.error("The camera recording has no duration")
            event("finished").put("duration_us", duration * 1000).put("rotation", orientation)
        }.getOrElse { event("failed").put("error", it.message ?: it.javaClass.simpleName) }
        session?.close()
        camera?.close()
        recorder?.release()
        if (!opening) thread.quitSafely()
        deliver(result)
    }
}
