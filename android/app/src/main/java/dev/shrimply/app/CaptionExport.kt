package dev.shrimply.app

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import org.json.JSONObject

@Composable
internal fun CaptionExportSettings(timeline: TimelineSession, projectPath: String, dismiss: () -> Unit, export: (Array<out Pair<String, Any>>) -> Unit) {
    var schema by remember { mutableStateOf<JSONObject?>(null) }
    var format by rememberSaveable(projectPath) { mutableStateOf("ytt") }
    var mode by rememberSaveable(projectPath) { mutableStateOf("merge") }
    var error by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(projectPath) {
        try { schema = timeline.request("caption_export_schema", "project_path" to projectPath) }
        catch (failure: Exception) { error = failure.message }
    }
    AlertDialog(onDismissRequest = dismiss, title = { Text(tr("Export Captions")) }, text = {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            if (schema == null && error == null) LinearProgressIndicator(Modifier.fillMaxWidth())
            schema?.let {
                ExportChoice("Format", format, it.getJSONArray("formats")) { format = it }
                ExportChoice("Tracks", mode, it.getJSONArray("modes")) { mode = it }
            }
            error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        }
    }, confirmButton = { TextButton(enabled = schema != null, onClick = { export(arrayOf("format" to format, "mode" to mode)) }) { Text(tr("Export")) } },
        dismissButton = { TextButton(onClick = dismiss) { Text(tr("Cancel")) } })
}
