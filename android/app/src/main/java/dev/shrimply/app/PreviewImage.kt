package dev.shrimply.app

import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.FileProvider
import kotlinx.coroutines.launch
import java.io.File

/** Deliver Rust's completed PNG through Android's clipboard and document provider. */
@Composable
internal fun PreviewImageAction(timeline: TimelineSession) {
    val snapshot by timeline.snapshot
    val image = snapshot.optJSONObject("preview_image")
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var delivered by rememberSaveable { mutableStateOf<String?>(null) }
    var pendingId by rememberSaveable { mutableStateOf<String?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    suspend fun dismiss(id: String) { timeline.request("preview_image_dismiss", "id" to id) }
    val save = rememberExportPicker { uri ->
        val id = pendingId
        pendingId = null
        if (id != null) scope.launch {
            try {
                if (uri == null) dismiss(id)
                else timeline.request("preview_image_save", "id" to id, "destination" to uri.toString())
            } catch (error: Exception) { failure = error.message }
        }
    }
    LaunchedEffect(image?.optString("id"), image?.optString("status")) {
        val value = image ?: return@LaunchedEffect
        val status = value.getString("status")
        val id = value.getString("id")
        if ((status != "ready" && status != "choose") || delivered == id) return@LaunchedEffect
        delivered = id
        try {
            if (status == "choose") {
                pendingId = id
                save(value)
            } else {
                val uri = FileProvider.getUriForFile(context, context.packageName + ".preview-images", File(value.getString("path")))
                context.getSystemService(ClipboardManager::class.java).setPrimaryClip(
                    ClipData.newUri(context.contentResolver, value.getString("label"), uri))
                dismiss(id)
            }
        } catch (error: Exception) { failure = error.message }
    }
    if (image != null && (image.getString("status") == "failed" || failure != null)) {
        val close: () -> Unit = { failure = null; scope.launch { dismiss(image.getString("id")) } }
        AlertDialog(onDismissRequest = close,
            title = { Text(image.getString("error_title")) },
            text = { Text(failure ?: image.getString("error")) },
            confirmButton = { TextButton(onClick = close) { Text(image.getString("close_label")) } })
    }
}
