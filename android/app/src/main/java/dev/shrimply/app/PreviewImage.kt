package dev.shrimply.app

import android.content.ClipData
import android.content.ClipboardManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
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
internal fun PreviewImageAction(timeline: TimelineSession, files: ProjectFiles) {
    val snapshot by timeline.snapshot
    val image = snapshot.optJSONObject("preview_image")
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var delivered by rememberSaveable { mutableStateOf<String?>(null) }
    var pending by rememberSaveable { mutableStateOf<String?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    suspend fun dismiss(value: JSONObject) { timeline.request("preview_image_dismiss", "id" to value.getString("id")) }
    val save = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("image/png")) { uri ->
        val target = pending?.let(::JSONObject)
        pending = null
        if (target != null) scope.launch {
            try {
                if (uri != null) files.exportCompletedFile(target.getString("path"), uri)
                dismiss(target)
            } catch (error: Exception) { failure = error.message }
        }
    }
    LaunchedEffect(image?.optString("id"), image?.optString("status")) {
        val value = image ?: return@LaunchedEffect
        if (value.getString("status") != "ready" || delivered == value.getString("id")) return@LaunchedEffect
        delivered = value.getString("id")
        try {
            when (value.getString("action")) {
                "copy" -> {
                    val uri = FileProvider.getUriForFile(context, context.packageName + ".preview-images", File(value.getString("path")))
                    context.getSystemService(ClipboardManager::class.java).setPrimaryClip(
                        ClipData.newUri(context.contentResolver, value.getString("label"), uri))
                    dismiss(value)
                }
                "save" -> { pending = value.toString(); save.launch(value.getString("filename")) }
                else -> error("Unknown native image delivery action")
            }
        } catch (error: Exception) { failure = error.message }
    }
    if (image != null && (image.optString("status") == "failed" || failure != null)) {
        AlertDialog(onDismissRequest = { failure = null; scope.launch { dismiss(image) } },
            title = { Text(image.getString("error_title")) },
            text = { Text(failure ?: image.getString("error")) },
            confirmButton = { TextButton(onClick = { failure = null; scope.launch { dismiss(image) } }) { Text(image.getString("close_label")) } })
    }
}
