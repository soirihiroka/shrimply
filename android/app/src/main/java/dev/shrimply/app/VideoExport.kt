package dev.shrimply.app

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import org.json.JSONArray
import org.json.JSONObject

@Composable
internal fun VideoExportSettings(timeline: TimelineSession, projectPath: String, dismiss: () -> Unit, export: (Array<out Pair<String, Any>>) -> Unit) {
    var schema by remember { mutableStateOf<JSONObject?>(null) }
    var values by rememberSaveable { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(projectPath) {
        try {
            val result = timeline.request("export_schema", "project_path" to projectPath)
            schema = result
            if (values == null) values = result.getJSONObject("defaults").toString()
        } catch (failure: Exception) { error = failure.message }
    }
    val fields = values?.let(::JSONObject)
    fun set(name: String, value: String) { values = checkNotNull(fields).put(name, value).toString(); error = null }
    AlertDialog(modifier = Modifier.imePadding(), onDismissRequest = dismiss, title = { Text(tr("Export Video")) }, text = {
        Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            if (schema == null && error == null) LinearProgressIndicator(Modifier.fillMaxWidth())
            schema?.let { schema -> fields?.let { fields ->
                ExportChoice("Video Codec", fields.getString("video_codec"), schema.getJSONArray("video_codecs")) { set("video_codec", it) }
                val gif = fields.getString("video_codec") == "gif"
                if (!gif) ExportChoice("Container", fields.getString("container"), schema.getJSONArray("containers")) { set("container", it) }
                ExportChoice("Frame Rate", fields.getString("fps"), schema.getJSONArray("frame_rates")) { set("fps", it) }
                if (!gif) ExportChoice("Rate control", fields.getString("rate_control"), schema.getJSONArray("rate_controls")) { set("rate_control", it) }
                val numeric = if (gif) listOf("background_alpha" to "Background Alpha") else
                    listOf((if (fields.getString("rate_control") == "constant_qp") "constant_qp" to "Constant QP" else "bitrate_kbps" to "Video Bitrate"), "keyframe_interval_seconds" to "Keyframe Interval")
                numeric.forEach { (key, label) -> OutlinedTextField(fields.optString(key), { set(key, it) }, label = { Text(tr(label)) }, singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number)) }
                if (fields.getString("video_codec") != "gif") {
                    HorizontalDivider()
                    ExportChoice("Audio Encoder", fields.getString("audio_encoder"), schema.getJSONArray("audio_encoders")) { set("audio_encoder", it) }
                    listOf("audio_sample_rate" to "Audio Sample Rate", "audio_bitrate_kbps" to "Audio Bitrate").forEach { (key, label) ->
                        OutlinedTextField(fields.optString(key), { set(key, it) }, label = { Text(tr(label)) }, singleLine = true,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number))
                    }
                }
            } }
            error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        }
    }, confirmButton = { TextButton(enabled = schema != null && fields != null, onClick = {
        try {
            val fields = checkNotNull(fields)
            val arguments = fields.keys().asSequence().map { key ->
                val value: Any = if (key in setOf("video_codec", "container", "fps", "audio_encoder", "rate_control")) fields.getString(key)
                    else fields.optString(key).toLongOrNull() ?: throw IllegalArgumentException("${key.replace('_', ' ')} must be a whole number.")
                key to value
            }.toList().toTypedArray()
            export(arguments)
        } catch (failure: Exception) { error = failure.message }
    }) { Text(tr("Export")) } }, dismissButton = { TextButton(onClick = dismiss) { Text(tr("Cancel")) } })
}

@Composable
internal fun ExportChoice(label: String, value: String, choices: JSONArray, select: (String) -> Unit) {
    var expanded by remember { mutableStateOf(false) }
    val options = (0 until choices.length()).map { choices.getJSONObject(it) }
    val selected = options.firstOrNull { it.getString("value") == value } ?: error("Unknown export choice: $value")
    Box {
        OutlinedButton(onClick = { expanded = true }) { Text("${tr(label)}: ${tr(selected.getString("label"))}") }
        DropdownMenu(expanded, { expanded = false }) { options.forEach { option ->
            DropdownMenuItem(text = { Text(tr(option.getString("label"))) }, onClick = { expanded = false; select(option.getString("value")) })
        } }
    }
}

@Composable
internal fun ExportProgress(export: JSONObject, enabled: Boolean, command: (String) -> Unit, save: () -> Unit) {
    val status = export.getString("status")
    val running = status == "running"
    val cancelling = export.getBoolean("cancelling")
    val progress = export.optJSONObject("progress")
    val captions = export.getString("kind") == "captions"
    val title = when (status) {
        "running" -> if (captions) "Export Captions" else "Export Video"
        "complete" -> if (captions) "Captions exported" else "Video exported"
        "cancelled" -> "Export cancelled"
        "failed" -> if (captions) "Could not export captions" else "Could not export video"
        else -> error("Unknown export state: $status")
    }
    AlertDialog(onDismissRequest = { command(if (running) "export_cancel" else "export_dismiss") }, title = { Text(tr(title)) }, text = {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            if (running) {
                if (cancelling) Text(tr("Cancelling…"))
                else if (progress != null) Text(tr(progress.getString("label")))
                if (progress == null || progress.isNull("fraction")) LinearProgressIndicator(Modifier.fillMaxWidth())
                else LinearProgressIndicator(progress = { progress.getDouble("fraction").toFloat() }, modifier = Modifier.fillMaxWidth())
                if (progress != null) {
                    val arguments = progress.getJSONObject("arguments")
                    val values = arguments.keys().asSequence().map { it to arguments.getString(it) as Any }.toList().toTypedArray()
                    Text(tr(progress.getString("message"), *values))
                }
            } else if (status == "failed") Text(export.getString("error"), color = MaterialTheme.colorScheme.error)
            else if (status == "complete") {
                val files = export.getJSONArray("files")
                Text((0 until files.length()).joinToString("\n") { files.getJSONObject(it).getString("filename") })
            }
        }
    }, confirmButton = {
        if (status == "complete") TextButton(onClick = save, enabled = enabled) { Text(tr("Save")) }
        else TextButton(onClick = { command(if (running) "export_cancel" else "export_dismiss") }, enabled = enabled && (!running || !cancelling)) { Text(tr(if (running) "Cancel" else "Close")) }
    }, dismissButton = {
        if (status == "complete") TextButton(onClick = { command("export_dismiss") }, enabled = enabled) { Text(tr("Close")) }
    })
}
