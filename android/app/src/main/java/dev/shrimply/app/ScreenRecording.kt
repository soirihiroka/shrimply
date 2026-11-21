package dev.shrimply.app

import android.app.Activity
import android.media.projection.MediaProjectionManager
import android.os.Handler
import android.os.Looper
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
internal fun ScreenRecordingAction(timeline: TimelineSession, onError: (String) -> Unit) {
    val event by timeline.snapshot
    val recording = event.optJSONObject("video_recording")
    val context = LocalContext.current
    val activity = checkNotNull(LocalActivity.current)
    val scope = rememberCoroutineScope()
    val main = remember { Handler(Looper.getMainLooper()) }
    val currentError by rememberUpdatedState(onError)
    var requested by rememberSaveable { mutableStateOf<String?>(null) }
    var pending by rememberSaveable { mutableStateOf<String?>(null) }
    val receive: (JSONObject) -> Unit = { value -> main.post { scope.launch {
        try {
            timeline.request("video_recording_event", *value.keys().asSequence().map { it to value.get(it) }.toList().toTypedArray())
            if (value.getString("event") != "ready") ScreenRecordingService.acknowledge(value.getString("id"))
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (error: Exception) {
            if (value.getString("event") != "ready") ScreenRecordingService.acknowledge(value.getString("id"))
            currentError(error.message ?: error.javaClass.simpleName)
        }
    } } }
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
            if (!ScreenRecordingService.stop(value.getString("id"))) {
                pending = null
                receive(JSONObject().put("id", value.getString("id")).put("event", "cancelled"))
            }
        } else if (requested != value.getString("id")) {
            requested = value.getString("id")
            pending = value.toString()
            consent.launch(context.getSystemService(MediaProjectionManager::class.java).createScreenCaptureIntent())
        }
    }
}
