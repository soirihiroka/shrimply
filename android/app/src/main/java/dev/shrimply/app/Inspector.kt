package dev.shrimply.app

import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.selection.toggleable
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
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject

internal fun JSONArray.objects(): List<JSONObject> = (0 until length()).map { getJSONObject(it) }

/** Render localized Rust widget descriptions and forward their opaque commands. */
@Composable
internal fun Inspector(timeline: TimelineSession, document: JSONObject?, enabled: Boolean, command: (Array<out Pair<String, Any>>) -> Unit, modifier: Modifier, selectFile: (JSONObject) -> Unit) {
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
            category?.getJSONArray("items")?.objects()?.forEach { item -> key(item.getString("id")) { InspectorItem(timeline, item, enabled, send, selectFile) } }
        }
    }
}

internal fun inspectorIcon(icon: String): Int = when (icon) {
    "padlock2-symbolic" -> R.drawable.padlock2
    "padlock2-open-symbolic" -> R.drawable.padlock2_open
    "rotation.svg", "arrow3-up-symbolic" -> R.drawable.arrow3_up
    "closed-captioning-symbolic" -> R.drawable.closed_captioning
    "network-server-symbolic" -> R.drawable.network_server
    "ruler-angled-symbolic" -> R.drawable.ruler_angled
    "folder-symbolic" -> R.drawable.folder
    "folder-open-symbolic" -> R.drawable.folder_open
    "sliders-horizontal-symbolic" -> R.drawable.sliders_horizontal
    "rich-text-symbolic" -> R.drawable.rich_text
    "image-symbolic" -> R.drawable.image
    "audio-volume-high-symbolic" -> R.drawable.audio_volume_high
    "media-playback-start-symbolic" -> R.drawable.media_playback_start
    "info-outline-symbolic" -> R.drawable.info_outline
    "speedometer-symbolic" -> R.drawable.speedometer
    "select-symbolic" -> R.drawable.select
    "edit-copy-symbolic" -> R.drawable.edit_copy
    "edit-paste-symbolic" -> R.drawable.edit_paste
    "zoom-in-symbolic" -> R.drawable.zoom_in
    "zoom-out-symbolic" -> R.drawable.zoom_out
    "list-add-symbolic" -> R.drawable.list_add
    "media-seek-backward-symbolic" -> R.drawable.media_seek_backward
    "media-seek-forward-symbolic" -> R.drawable.media_seek_forward
    "edit-clear-symbolic" -> R.drawable.edit_clear
    "go-up-symbolic" -> R.drawable.go_up
    "go-down-symbolic" -> R.drawable.go_down
    "user-trash-symbolic" -> R.drawable.user_trash
    "view-refresh-symbolic" -> R.drawable.view_refresh
    else -> error("Unknown native icon: $icon")
}

@Composable
private fun InspectorItem(timeline: TimelineSession, item: JSONObject, enabled: Boolean, send: (JSONObject) -> Unit, selectFile: (JSONObject) -> Unit) {
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
            if (item.getJSONArray("actions").length() > 0) AdaptiveMenuButton(enabled,
                icon = { Icon(painterResource(R.drawable.view_more), item.getString("actions_label"), Modifier.size(16.dp)) }) { dismiss ->
                item.getJSONArray("actions").objects().forEach { action ->
                    DropdownMenuItem(text = { Text(action.getString("label")) }, enabled = enabled && action.getBoolean("enabled"),
                        leadingIcon = { Icon(painterResource(inspectorIcon(action.getString("icon"))), null, Modifier.size(16.dp)) },
                        onClick = { dismiss(); send(action.getJSONObject("command")) })
                }
            }
        }
        item.getJSONArray("controls").objects().forEach { control -> key(control.getString("id")) { InspectorControl(timeline, control, enabled, send, selectFile) } }
    }
}

@Composable
internal fun InspectorControl(timeline: TimelineSession, control: JSONObject, enabled: Boolean, send: (JSONObject) -> Unit, selectFile: (JSONObject) -> Unit, settings: Boolean = false) {
    val label = control.getString("label")
    val summary = control.getString("subtitle")
    val active = enabled && control.getBoolean("enabled")
    val editable = active && control.getBoolean("editable")
    var editor by remember { mutableStateOf<JSONObject?>(null) }
    Column(Modifier.fillMaxWidth()) {
        when (val widget = control.getString("widget")) {
            "group" -> {
                if (label.isNotEmpty()) Text(label, Modifier.padding(if (settings) PaddingValues(start = 24.dp, top = 24.dp, end = 16.dp, bottom = 8.dp) else PaddingValues()), color = if (settings) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.labelLarge)
                control.getJSONArray("controls").objects().forEach { child ->
                    key(child.getString("id")) { InspectorControl(timeline, child, active, send, selectFile, settings) }
                }
            }
            "file" -> {
                Text(label, style = MaterialTheme.typography.labelMedium)
                if (control.getString("display").isNotEmpty()) Text(control.getString("display"))
                Row {
                    TextButton(onClick = { selectFile(control) }, enabled = editable) { Text(control.getString("choose_label")) }
                    control.optJSONObject("clear")?.let { clear ->
                        TextButton(onClick = { send(clear.getJSONObject("command")) }, enabled = editable) { Text(clear.getString("label")) }
                    }
                }
            }
            "toggle" -> if (settings) {
                ListItem(headlineContent = { Text(label) },
                    supportingContent = if (summary.isEmpty()) null else { { Text(summary) } },
                    trailingContent = { Switch(checked = control.getBoolean("selected"), enabled = editable, onCheckedChange = null) },
                    colors = ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.surface),
                    modifier = Modifier.fillMaxWidth().toggleable(value = control.getBoolean("selected"), enabled = editable,
                        role = Role.Switch, onValueChange = { send(control.getJSONObject("command")) }).padding(start = 8.dp).heightIn(min = 72.dp))
            } else Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Text(label, Modifier.weight(1f))
                Switch(checked = control.getBoolean("selected"), enabled = editable, onCheckedChange = { send(control.getJSONObject("command")) })
            }
            "choice" -> {
                val choices = control.getJSONArray("choices").objects()
                ChoiceInput(label, control.getString("display"), choices.map { it.getString("label") }, editable,
                    control.getString("choice_style") == "button", summary = if (settings) summary else "", settings = settings) { send(choices[it].getJSONObject("command")) }
            }
            "button" -> if (settings) ListItem(headlineContent = { Text(label, color = MaterialTheme.colorScheme.primary) },
                colors = ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.surface),
                modifier = Modifier.fillMaxWidth().clickable(enabled = active) { send(control.getJSONObject("command")) }.padding(start = 8.dp).heightIn(min = 72.dp))
            else TextButton(onClick = { send(control.getJSONObject("command")) }, enabled = active) { Text(label) }
            "live_rows" -> LiveRows(timeline, control.getJSONObject("resource"))
            "text", "loading" -> {
                if (settings) ListItem(headlineContent = { Text(label) },
                    supportingContent = if (control.getString("display").isEmpty()) null else { { Text(control.getString("display")) } },
                    colors = ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.surface),
                    modifier = Modifier.fillMaxWidth().padding(start = 8.dp).heightIn(min = 72.dp))
                else {
                    if (label.isNotEmpty()) Text(label, style = MaterialTheme.typography.labelMedium)
                    if (control.getString("display").isNotEmpty()) Text(control.getString("display"), color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                if (widget == "loading") LinearProgressIndicator(Modifier.fillMaxWidth())
            }
            "number" -> NumberInput(timeline, control, editable)
            "editor" -> ListItem(headlineContent = { Text(label) },
                supportingContent = { Text(listOf(control.getString("display"), if (settings) summary else "").filter { it.isNotEmpty() }.joinToString("\n")) },
                colors = ListItemDefaults.colors(containerColor = if (settings) MaterialTheme.colorScheme.surface else MaterialTheme.colorScheme.surfaceContainerLow),
                modifier = Modifier.fillMaxWidth().clickable(enabled = editable) { editor = control.getJSONObject("editor") }
                    .then(if (settings) Modifier.padding(start = 8.dp).heightIn(min = 72.dp) else Modifier))
            else -> error("Unknown native widget: $widget")
        }
        if (summary.isNotEmpty() && !settings) Text(summary, style = MaterialTheme.typography.bodySmall)
        control.optJSONArray("toggles")?.let { toggles -> Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            toggles.objects().forEach { toggle -> FilterChip(selected = toggle.getBoolean("selected"), enabled = editable,
                onClick = { send(toggle.getJSONObject("command")) }, label = { Text(toggle.getString("label")) }) }
        } }
        control.optJSONObject("extra_editor")?.let { extra -> TextButton(onClick = { editor = extra.getJSONObject("editor") }, enabled = editable) { Text(extra.getString("label")) } }
        control.optJSONObject("graph")?.let { graph ->
            val scope = rememberCoroutineScope()
            var failure by remember { mutableStateOf<String?>(null) }
            TextButton(enabled = editable, onClick = { scope.launch {
                try { timeline.request(graph.getJSONObject("command")) } catch (error: Exception) { failure = error.message }
            } }) { Text(graph.getString("label")) }
            failure?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        }
    }
    editor?.let { description -> InspectorEdit(description, { editor = null }, send) }
}

@Composable
private fun InspectorEdit(editor: JSONObject, dismiss: () -> Unit, send: (JSONObject) -> Unit) {
    val fields = editor.getJSONArray("fields").objects()
    val values = remember(editor) { fields.map { mutableStateOf(it.getString("value")) } }
    AlertDialog(modifier = Modifier.imePadding(), onDismissRequest = dismiss, title = { Text(editor.getString("title")) }, text = {
        Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            fields.forEachIndexed { index, field ->
                val choices = field.optJSONArray("choices")
                if (choices != null) {
                    val options = choices.objects()
                    ChoiceInput(if (field.isNull("label")) "" else field.getString("label"), options.first { it.getString("value") == values[index].value }.getString("label"), options.map { it.getString("label") }) {
                        values[index].value = options[it].getString("value")
                    }
                } else OutlinedTextField(value = values[index].value, onValueChange = { values[index].value = it },
                label = if (!field.isNull("label") && field.getString("label").isNotEmpty()) { { Text(field.getString("label")) } } else null,
                singleLine = !field.getBoolean("multiline"),
                keyboardOptions = KeyboardOptions(keyboardType = if (field.getString("keyboard") == "decimal") KeyboardType.Decimal else KeyboardType.Text)) }
        }
    }, confirmButton = { TextButton(onClick = {
        val payload = JSONObject(editor.getJSONObject("command").toString()).put("values", JSONArray(values.map { it.value }))
        send(payload); dismiss()
    }) { Text(editor.getString("confirm_label")) } }, dismissButton = { TextButton(onClick = dismiss) { Text(editor.getString("cancel_label")) } })
}
