package dev.shrimply.app

import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import org.json.JSONArray
import org.json.JSONObject

private fun JSONArray.objects(): List<JSONObject> = (0 until length()).map { getJSONObject(it) }

/** Render localized Rust widget descriptions and forward their opaque commands. */
@Composable
internal fun Inspector(timeline: TimelineSession, document: JSONObject?, enabled: Boolean, command: (Array<out Pair<String, Any>>) -> Unit, modifier: Modifier) {
    val categories = document?.optJSONArray("categories")?.objects().orEmpty()
    var categoryKey by rememberSaveable { mutableStateOf<String?>(null) }
    val category = categories.firstOrNull { it.getString("key") == categoryKey } ?: categories.firstOrNull()
    val send: (JSONObject) -> Unit = { payload -> command(payload.keys().asSequence().map { it to payload.get(it) }.toList().toTypedArray()) }
    Column(modifier) {
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), verticalAlignment = Alignment.CenterVertically) {
            categories.forEach { candidate ->
                IconToggleButton(checked = candidate == category, onCheckedChange = { categoryKey = candidate.getString("key") }) {
                    Icon(painterResource(inspectorIcon(candidate.getString("icon"))), candidate.getString("label"), Modifier.size(16.dp))
                }
            }
        }
        HorizontalDivider()
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(12.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            category?.getJSONArray("items")?.objects()?.forEach { item -> key(item.getString("id")) { InspectorItem(timeline, item, enabled, send) } }
        }
    }
}

internal fun inspectorIcon(icon: String): Int = when (icon) {
    "folder-symbolic" -> R.drawable.folder
    "sliders-horizontal-symbolic" -> R.drawable.sliders_horizontal
    "rich-text-symbolic" -> R.drawable.rich_text
    "image-symbolic" -> R.drawable.image
    "audio-volume-high-symbolic" -> R.drawable.audio_volume_high
    "media-playback-start-symbolic" -> R.drawable.media_playback_start
    "info-outline-symbolic" -> R.drawable.info_outline
    "speedometer-symbolic" -> R.drawable.speedometer
    "select-symbolic" -> R.drawable.select
    "edit-copy-symbolic" -> R.drawable.edit_copy
    "edit-clear-symbolic" -> R.drawable.edit_clear
    "go-up-symbolic" -> R.drawable.go_up
    "go-down-symbolic" -> R.drawable.go_down
    "user-trash-symbolic" -> R.drawable.user_trash
    "view-refresh-symbolic" -> R.drawable.view_refresh
    else -> error("Unknown native icon: $icon")
}

@Composable
private fun InspectorItem(timeline: TimelineSession, item: JSONObject, enabled: Boolean, send: (JSONObject) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        val title = item.getString("title")
        if (title.isNotEmpty()) Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(title, Modifier.weight(1f), style = MaterialTheme.typography.titleSmall)
            item.optJSONObject("toggle")?.let { toggle -> Switch(checked = toggle.getBoolean("selected"), enabled = enabled,
                onCheckedChange = { send(toggle.getJSONObject("command")) }) }
            item.optJSONObject("button_toggle")?.let { toggle ->
                IconToggleButton(checked = toggle.getBoolean("selected"), enabled = enabled, onCheckedChange = { send(toggle.getJSONObject("command")) }) {
                    Icon(painterResource(inspectorIcon(toggle.getString("icon"))), toggle.getString("label"), Modifier.size(16.dp))
                }
            }
            var menu by remember { mutableStateOf(false) }
            if (item.getJSONArray("actions").length() > 0) Box {
                IconButton(onClick = { menu = true }, enabled = enabled) { Icon(painterResource(R.drawable.view_more), item.getString("actions_label"), Modifier.size(16.dp)) }
                DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                    item.getJSONArray("actions").objects().forEach { action ->
                        DropdownMenuItem(text = { Text(action.getString("label")) }, enabled = enabled && action.getBoolean("enabled"),
                            leadingIcon = { Icon(painterResource(inspectorIcon(action.getString("icon"))), null, Modifier.size(16.dp)) },
                            onClick = { menu = false; send(action.getJSONObject("command")) })
                    }
                }
            }
        }
        item.getJSONArray("controls").objects().forEach { control -> key(control.getString("id")) { InspectorControl(timeline, control, enabled, send) } }
    }
}

@Composable
private fun InspectorControl(timeline: TimelineSession, control: JSONObject, enabled: Boolean, send: (JSONObject) -> Unit) {
    val label = control.getString("label")
    val active = enabled && control.getBoolean("enabled")
    val editable = active && control.getBoolean("editable")
    var editor by remember { mutableStateOf<JSONObject?>(null) }
    Column(Modifier.fillMaxWidth()) {
        when (val widget = control.getString("widget")) {
            "toggle" -> Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Text(label, Modifier.weight(1f))
                Switch(checked = control.getBoolean("selected"), enabled = editable, onCheckedChange = { send(control.getJSONObject("command")) })
            }
            "choice" -> {
                var menu by remember { mutableStateOf(false) }
                Box {
                    TextButton(onClick = { menu = true }, enabled = editable) { Text(label) }
                    DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                        control.getJSONArray("choices").objects().forEach { choice -> DropdownMenuItem(text = { Text(choice.getString("label")) },
                            onClick = { menu = false; send(choice.getJSONObject("command")) }) }
                    }
                }
            }
            "button" -> TextButton(onClick = { send(control.getJSONObject("command")) }, enabled = active) { Text(label) }
            "live_rows" -> LiveRows(timeline, control.getJSONObject("resource"))
            "text", "loading" -> {
                if (label.isNotEmpty()) Text(label, style = MaterialTheme.typography.labelMedium)
                if (control.getString("display").isNotEmpty()) Text(control.getString("display"), color = MaterialTheme.colorScheme.onSurfaceVariant)
                if (widget == "loading") LinearProgressIndicator(Modifier.fillMaxWidth())
            }
            "editor" -> ListItem(headlineContent = { Text(label) }, supportingContent = { Text(control.getString("display")) },
                modifier = Modifier.clickable(enabled = editable) { editor = control.getJSONObject("editor") })
            else -> error("Unknown native widget: $widget")
        }
        if (control.getString("subtitle").isNotEmpty()) Text(control.getString("subtitle"), style = MaterialTheme.typography.bodySmall)
        control.optJSONArray("toggles")?.let { toggles -> Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            toggles.objects().forEach { toggle -> FilterChip(selected = toggle.getBoolean("selected"), enabled = editable,
                onClick = { send(toggle.getJSONObject("command")) }, label = { Text(toggle.getString("label")) }) }
        } }
        control.optJSONObject("extra_editor")?.let { extra -> TextButton(onClick = { editor = extra.getJSONObject("editor") }, enabled = editable) { Text(extra.getString("label")) } }
    }
    editor?.let { description -> InspectorEdit(description, { editor = null }, send) }
}

@Composable
private fun InspectorEdit(editor: JSONObject, dismiss: () -> Unit, send: (JSONObject) -> Unit) {
    val fields = editor.getJSONArray("fields").objects()
    val values = remember(editor) { fields.map { mutableStateOf(it.getString("value")) } }
    AlertDialog(modifier = Modifier.imePadding(), onDismissRequest = dismiss, title = { Text(editor.getString("title")) }, text = {
        Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            fields.forEachIndexed { index, field -> OutlinedTextField(value = values[index].value, onValueChange = { values[index].value = it },
                label = if (!field.isNull("label") && field.getString("label").isNotEmpty()) { { Text(field.getString("label")) } } else null,
                singleLine = !field.getBoolean("multiline"),
                keyboardOptions = KeyboardOptions(keyboardType = if (field.getString("keyboard") == "decimal") KeyboardType.Decimal else KeyboardType.Text)) }
        }
    }, confirmButton = { TextButton(onClick = {
        val payload = JSONObject(editor.getJSONObject("command").toString()).put("values", JSONArray(values.map { it.value }))
        send(payload); dismiss()
    }) { Text(editor.getString("confirm_label")) } }, dismissButton = { TextButton(onClick = dismiss) { Text(editor.getString("cancel_label")) } })
}
