package dev.shrimply.app

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraManager
import android.media.projection.MediaProjectionManager
import android.os.Handler
import android.os.Looper
import android.view.Surface
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.LocalActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.platform.LocalContext
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import org.json.JSONObject

/** Consent and native surfaces forward lifecycle events to Rust's recording job. */
@Composable
internal fun VideoRecordingAction(timeline: TimelineSession, onError: (String) -> Unit) {
    val event by timeline.snapshot
    val recording = event.optJSONObject("video_recording")
    val context = LocalContext.current
    val activity = checkNotNull(LocalActivity.current)
    val scope = rememberCoroutineScope()
    val main = remember { Handler(Looper.getMainLooper()) }
    val currentError by rememberUpdatedState(onError)
    var requested by rememberSaveable { mutableStateOf<String?>(null) }
    var pending by rememberSaveable { mutableStateOf<String?>(null) }
    var camera by remember(timeline.identity) { mutableStateOf<CameraRecording?>(null) }
    val cameraManager = remember(context) { context.getSystemService(CameraManager::class.java) }
    val frontLabel = tr("Front Camera")
    val backLabel = tr("Back Camera")
    val externalLabel = tr("External Camera")
    val receive: (JSONObject) -> Unit = { value -> main.post { scope.launch {
        try {
            timeline.request("video_recording_event", *value.keys().asSequence().map { it to value.get(it) }.toList().toTypedArray())
            if (value.getString("event") != "ready") {
                if (camera?.id == value.getString("id")) camera = null
                ScreenRecordingService.acknowledge(value.getString("id"))
            }
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (error: Exception) {
            if (value.getString("event") != "ready") ScreenRecordingService.acknowledge(value.getString("id"))
            currentError(error.message ?: error.javaClass.simpleName)
        }
    } } }
    fun startCamera(config: JSONObject) {
        val rotation = when (activity.display?.rotation) {
            Surface.ROTATION_90 -> 90
            Surface.ROTATION_180 -> 180
            Surface.ROTATION_270 -> 270
            else -> 0
        }
        camera = CameraRecording(context.applicationContext, config, rotation, receive)
    }
    DisposableEffect(timeline.identity, cameraManager) {
        fun refreshDevices() {
            scope.launch {
                try {
                    val devices = cameraManager.cameraIdList.mapNotNull { id ->
                        val info = cameraManager.getCameraCharacteristics(id)
                        if (info.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
                                ?.getOutputSizes(android.media.MediaRecorder::class.java).isNullOrEmpty()) return@mapNotNull null
                        id to when (info.get(CameraCharacteristics.LENS_FACING)) {
                            CameraCharacteristics.LENS_FACING_FRONT -> frontLabel
                            CameraCharacteristics.LENS_FACING_BACK -> backLabel
                            else -> externalLabel
                        }
                    }
                    val counts = devices.groupingBy { it.second }.eachCount()
                    val sources = org.json.JSONArray()
                    for ((id, label) in devices)
                        sources.put(org.json.JSONArray(listOf(id, if (counts[label] == 1) label else "$label ($id)")))
                    timeline.request("camera_sources", "sources" to sources)
                } catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { /* A disconnected camera will trigger another availability event. */ }
            }
        }
        val availability = object : CameraManager.AvailabilityCallback() {
            override fun onCameraAvailable(cameraId: String) { refreshDevices() }
            override fun onCameraUnavailable(cameraId: String) { refreshDevices() }
        }
        refreshDevices()
        cameraManager.registerAvailabilityCallback(availability, main)
        onDispose {
            cameraManager.unregisterAvailabilityCallback(availability)
            camera?.stop()
        }
    }
    val cameraPermissionMessage = tr("Camera permission is required to record video.")
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        val config = pending?.let(::JSONObject)
        pending = null
        if (config != null) {
            val id = config.getString("id")
            val current = timeline.snapshot.value.optJSONObject("video_recording")
            if (current?.optString("id") == id && current.optString("state") == "start") {
                if (granted) startCamera(config)
                else receive(JSONObject().put("id", id).put("event", "failed")
                    .put("error", cameraPermissionMessage))
            }
        }
    }
    val consent = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        val config = pending?.let(::JSONObject)
        pending = null
        if (config != null) {
            val id = config.getString("id")
            val current = timeline.snapshot.value.optJSONObject("video_recording")
            if (current?.optString("id") == id && current.optString("state") == "start" && result.resultCode == Activity.RESULT_OK && result.data != null) {
                val bounds = activity.windowManager.maximumWindowMetrics.bounds
                val native = JSONObject(config.toString()).put("screen_width", bounds.width()).put("screen_height", bounds.height())
                    .put("density_dpi", activity.resources.configuration.densityDpi)
                ScreenRecordingService.start(context, timeline.identity, native, result.resultCode, result.data!!, receive)
            } else receive(JSONObject().put("id", id).put("event", "cancelled"))
        }
    }
    val id = recording?.optString("id")
    DisposableEffect(timeline.identity, id) {
        if (id != null) ScreenRecordingService.observe(timeline.identity, id, receive)
        onDispose {}
    }
    LaunchedEffect(id, recording?.optString("state")) {
        val value = recording ?: return@LaunchedEffect
        if (value.getString("state") == "stop") {
            val activeCamera = camera?.takeIf { it.id == value.getString("id") }
            if (activeCamera != null) activeCamera.stop()
            else if (!ScreenRecordingService.stop(value.getString("id"))) {
                pending = null
                receive(JSONObject().put("id", value.getString("id")).put("event", "cancelled"))
            }
        } else if (requested != value.getString("id")) {
            requested = value.getString("id")
            if (!value.isNull("camera_id")) {
                if (context.checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED)
                    startCamera(value)
                else {
                    pending = value.toString()
                    permission.launch(Manifest.permission.CAMERA)
                }
            } else {
                pending = value.toString()
                consent.launch(context.getSystemService(MediaProjectionManager::class.java).createScreenCaptureIntent())
            }
        }
    }
}
