package dev.shrimply.app

import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.FileProvider
import kotlinx.coroutines.launch
import org.json.JSONObject
import java.io.File

/** Deliver Rust's completed PNG through Android's clipboard and document provider. */
@Composable
internal fun PreviewImageAction(timeline: TimelineSession) {
    val snapshot by timeline.snapshot
    val image = snapshot.optJSONObject("preview_image")
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var delivered by rememberSaveable { mutableStateOf<String?>(null) }
    var pending by rememberSaveable { mutableStateOf<String?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    suspend fun dismiss(value: JSONObject) { timeline.request("preview_image_dismiss", "id" to value.getString("id")) }
    val save = rememberExportPicker { uri ->
        val target = pending?.let(::JSONObject)
        pending = null
        if (target != null) scope.launch {
            try {
                if (uri == null) dismiss(target)
                else timeline.request("preview_image_save", "id" to target.getString("id"), "destination" to uri.toString())
            } catch (error: Exception) { failure = error.message }
        }
    }
    LaunchedEffect(image?.optString("id"), image?.optString("status")) {
        val value = image ?: return@LaunchedEffect
        if (value.getString("status") !in listOf("ready", "choose") || delivered == value.getString("id")) return@LaunchedEffect
        delivered = value.getString("id")
        try {
            when (value.getString("action")) {
                "copy" -> {
                    val uri = FileProvider.getUriForFile(context, context.packageName + ".preview-images", File(value.getString("path")))
                    context.getSystemService(ClipboardManager::class.java).setPrimaryClip(
                        ClipData.newUri(context.contentResolver, value.getString("label"), uri))
                    dismiss(value)
                }
                "save" -> { pending = value.toString(); save(value) }
                else -> error("Unknown native image delivery action")
            }
        } catch (error: Exception) { failure = error.message }
    }
    if (image != null && (image.optString("status") in listOf("complete", "failed") || failure != null)) {
        val failed = failure != null || image.getString("status") == "failed"
        val close: () -> Unit = { failure = null; scope.launch { dismiss(image) } }
        AlertDialog(onDismissRequest = close,
            title = { Text(image.getString(if (failed) "error_title" else "saved_title")) },
            text = if (failed) ({ Text(failure ?: image.getString("error")) }) else null,
            confirmButton = { TextButton(onClick = close) { Text(image.getString("close_label")) } })
    }
}
