package dev.shrimply.app

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import kotlinx.coroutines.launch
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import org.json.JSONArray
import org.json.JSONObject

@Composable
internal fun VideoExportSettings(timeline: TimelineSession, projectPath: String, dismiss: () -> Unit, export: (Array<out Pair<String, Any>>) -> Unit) {
    var schema by remember { mutableStateOf<JSONObject?>(null) }
    var values by rememberSaveable(projectPath) { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val input = LocalMenuInputSource.current
    val bottomSheet = remember { NativeLayout.menuUsesBottomSheet(input.value) }
    LaunchedEffect(projectPath) {
        try {
            val result = timeline.request("export_schema", "project_path" to projectPath, "values" to (values?.let(::JSONObject) ?: JSONObject()))
            schema = result
            if (values == null) values = result.getJSONObject("values").toString()
        } catch (failure: Exception) { error = failure.message }
    }
    val fields = values?.let(::JSONObject)
    fun set(id: String, value: Any) {
        val next = JSONObject(checkNotNull(values)).put(id, value).toString()
        values = next
        scope.launch {
            try {
                val result = timeline.request("export_schema", "project_path" to projectPath, "values" to JSONObject(next))
                if (values == next) { schema = result; error = null }
            } catch (failure: Exception) { error = failure.message }
        }
    }
    AdaptiveMenu(expanded = true, bottomSheet = bottomSheet, dismiss = dismiss) {
        Column(Modifier.fillMaxWidth().padding(24.dp).imePadding(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            schema?.let { description ->
                Text(description.getString("title"), style = MaterialTheme.typography.headlineSmall)
                fields?.let { fields -> description.getJSONArray("groups").objects().filter { it.getBoolean("visible") }.forEach { group ->
                    Text(group.getString("label"), style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary)
                    description.getJSONArray("fields").objects().filter { it.getString("group") == group.getString("label") && it.getBoolean("visible") }.forEach { field -> key(field.getString("id")) {
                        val id = field.getString("id")
                        val enabled = !busy && field.getBoolean("enabled")
                        val label = field.getString("label")
                        when (field.getString("widget")) {
                            "choice" -> {
                                val choices = field.getJSONArray("choices").objects()
                                val selected = choices.first { it.getString("value") == fields.get(id).toString() }
                                ChoiceInput(label, selected.getString("label"), choices.map { it.getString("label") }, enabled) { set(id, choices[it].getString("value")) }
                            }
                            "number" -> OutlinedTextField(fields.get(id).toString(), { set(id, it) }, enabled = enabled,
                                label = { Text(label) }, singleLine = true, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number))
                            "toggle" -> Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                                Text(label)
                                Switch(checked = fields.getBoolean(id), enabled = enabled, onCheckedChange = { set(id, it) })
                            }
                            else -> error("Unknown export widget")
                        }
                        if (field.optString("subtitle").isNotEmpty()) Text(field.getString("subtitle"), style = MaterialTheme.typography.bodySmall)
                    } }
                } }
            }
            if (schema == null && error == null) LinearProgressIndicator(Modifier.fillMaxWidth())
            error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            schema?.let { description -> Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                TextButton(enabled = !busy, onClick = dismiss) { Text(description.getString("cancel_label")) }
                Button(enabled = !busy && fields != null, onClick = { scope.launch {
                    busy = true
                    try {
                        val fields = checkNotNull(fields)
                        timeline.request("export_validate", "project_path" to projectPath, "values" to fields)
                        export(arrayOf("values" to fields))
                    } catch (failure: Exception) { error = failure.message }
                    finally { busy = false }
                } }) { Text(description.getString("submit_label")) }
            } }
        }
    }
}

@Composable
internal fun ExportChoice(label: String, value: String, choices: JSONArray, select: (String) -> Unit) {
    val options = (0 until choices.length()).map { choices.getJSONObject(it) }
    val selected = options.firstOrNull { it.getString("value") == value } ?: error("Unknown export choice: $value")
    ChoiceInput(tr(label), tr(selected.getString("label")), options.map { tr(it.getString("label")) }) { select(options[it].getString("value")) }
}

@Composable
internal fun ExportProgress(export: JSONObject, enabled: Boolean, command: (String) -> Unit, save: () -> Unit) {
    val model = export.getJSONObject("presentation")
    val progress = export.optJSONObject("progress")
    val confirm = model.getJSONObject("confirm")
    AlertDialog(onDismissRequest = { command(model.getString("dismiss")) }, title = { Text(model.getString("title")) }, text = {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            if (model.getBoolean("running")) {
                if (!model.isNull("cancelling")) Text(model.getString("cancelling"))
                else if (progress != null) Text(progress.getString("label"))
                if (progress == null || progress.isNull("fraction")) LinearProgressIndicator(Modifier.fillMaxWidth())
                else LinearProgressIndicator(progress = { progress.getDouble("fraction").toFloat() }, modifier = Modifier.fillMaxWidth())
                if (progress != null) Text(progress.getString("message"))
            } else if (model.getBoolean("failed")) Text(export.getString("error"), color = MaterialTheme.colorScheme.error)
            else if (model.getBoolean("complete")) {
                Text(export.getJSONArray("files").objects().joinToString("\n") { it.getString("filename") })
            }
        }
    }, confirmButton = {
        TextButton(onClick = { if (confirm.getString("action") == "save") save() else command(confirm.getString("action")) },
            enabled = enabled && confirm.getBoolean("enabled")) { Text(confirm.getString("label")) }
    }, dismissButton = {
        model.optJSONObject("secondary")?.let { secondary ->
            TextButton(onClick = { command(secondary.getString("action")) }, enabled = enabled) { Text(secondary.getString("label")) }
        }
    })
}
