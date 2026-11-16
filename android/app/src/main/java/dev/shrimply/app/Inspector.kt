package dev.shrimply.app

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.SizeTransform
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.Image
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
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

internal fun JSONArray.objects(): List<JSONObject> = (0 until length()).map { getJSONObject(it) }

/** Render localized Rust widget descriptions and forward their opaque commands. */
@OptIn(ExperimentalMaterial3Api::class)
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
    document?.optJSONObject("font_picker")?.let { picker ->
        val close = { send(picker.getJSONObject("close")) }
        ModalBottomSheet(onDismissRequest = close) {
            Column(Modifier.fillMaxWidth().fillMaxHeight(0.85f)) {
                Row(Modifier.fillMaxWidth().padding(horizontal = 24.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(picker.getString("title"), Modifier.weight(1f), style = MaterialTheme.typography.titleMedium)
                    TextButton(onClick = close) { Text(picker.getString("close_label")) }
                }
                FontPreferences(picker, enabled && !picker.getBoolean("busy"), send)
            }
        }
    }
}

internal fun inspectorIcon(icon: String): Int = when (icon) {
    "document-edit-symbolic" -> R.drawable.document_edit
    "fill-tool-symbolic" -> R.drawable.fill_tool
    "eraser-symbolic" -> R.drawable.eraser
    "function-exponential-symbolic" -> R.drawable.function_exponential
    "stopwatch-symbolic" -> R.drawable.stopwatch
    "code-symbolic" -> R.drawable.code
    "move-tool-symbolic" -> R.drawable.move_tool
    "appearance-symbolic" -> R.drawable.appearance
    "application-x-executable-symbolic" -> R.drawable.application_x_executable
    "security-high-symbolic" -> R.drawable.security_high
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
    "blend-tool-symbolic" -> R.drawable.blend_tool
    "playback-speed-symbolic" -> R.drawable.playback_speed
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
    "window-close-symbolic" -> R.drawable.window_close
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
                onCheckedChange = { send(toggle.getJSONObject("command")) }, modifier = Modifier.semantics { contentDescription = title }) }
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
    val title = control.getString("label")
    val label = if (control.optBoolean("wide")) "" else title
    val summary = control.getString("subtitle")
    val active = enabled && control.getBoolean("enabled")
    val editable = active && control.getBoolean("editable")
    var editor by remember { mutableStateOf<JSONObject?>(null) }
    var confirmation by remember { mutableStateOf<JSONObject?>(null) }
    val toggles = control.optJSONArray("toggles")?.objects().orEmpty()
    val wide = control.optBoolean("wide")
    val toggleButtons: @Composable () -> Unit = {
        toggles.forEach { toggle ->
            IconToggleButton(checked = toggle.getBoolean("selected"), enabled = editable,
                onCheckedChange = { send(toggle.getJSONObject("command")) }, modifier = Modifier.size(40.dp)) {
                Icon(painterResource(inspectorIcon(toggle.getString("icon"))), toggle.getString("label"), Modifier.size(16.dp))
            }
        }
    }
    Column(Modifier.fillMaxWidth()) {
        if (wide) Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(title, Modifier.weight(1f), style = MaterialTheme.typography.labelMedium)
            toggleButtons()
        }
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                when (val widget = control.getString("widget")) {
                    "tags" -> Tags(control, editable, send)
                    "group" -> {
                        if (label.isNotEmpty()) Text(label, Modifier.padding(if (settings) PaddingValues(start = 24.dp, top = 20.dp, end = 16.dp, bottom = 8.dp) else PaddingValues()), color = if (settings) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant, style = if (settings) MaterialTheme.typography.labelMedium else MaterialTheme.typography.labelLarge)
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
                    "actions" -> Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                        control.getJSONArray("actions").objects().forEach { action ->
                            TextButton(enabled = editable, onClick = {
                                if (action.has("confirmation")) confirmation = action else send(action.getJSONObject("command"))
                            }) { Text(action.getString("label")) }
                        }
                    }
                    "action_list" -> {
                        Text(label, style = MaterialTheme.typography.labelMedium)
                        val rows = control.getJSONArray("rows").objects()
                        if (rows.isEmpty()) Text(control.getString("empty"), color = MaterialTheme.colorScheme.onSurfaceVariant)
                        rows.forEach { row ->
                            ListItem(headlineContent = { Text(row.getString("label")) },
                                supportingContent = { Text(row.getString("subtitle")) },
                                trailingContent = { Row {
                                    row.getJSONArray("actions").objects().forEach { action ->
                                        IconButton(enabled = editable && action.getBoolean("enabled"), onClick = { send(action.getJSONObject("command")) }) {
                                            Icon(painterResource(inspectorIcon(action.getString("icon"))), action.getString("label"), Modifier.size(16.dp))
                                        }
                                    }
                                } }, modifier = Modifier.fillMaxWidth().clickable(enabled = editable) { send(row.getJSONObject("command")) })
                        }
                        control.getJSONObject("add").let { add ->
                            TextButton(enabled = editable, onClick = { send(add.getJSONObject("command")) }) { Text(add.getString("label")) }
                        }
                    }
                    "file_location" -> {
                        val reveal = control.getJSONObject("reveal")
                        ListItem(headlineContent = { Text(label) },
                            supportingContent = { androidx.compose.foundation.text.selection.SelectionContainer { Text(control.getString("display")) } },
                            trailingContent = { IconButton(enabled = active, onClick = { send(reveal.getJSONObject("command")) }) {
                                Icon(painterResource(inspectorIcon(reveal.getString("icon"))), reveal.getString("label"), Modifier.size(16.dp))
                            } },
                            modifier = Modifier.fillMaxWidth().clickable(enabled = active) { send(reveal.getJSONObject("command")) })
                    }
                    "image" -> InspectorImage(timeline, control)
                    "toggle" -> if (settings) {
                        val checked = control.getBoolean("selected")
                        SettingsRow(label, summary, editable,
                            modifier = Modifier.toggleable(value = checked, enabled = editable,
                                role = Role.Switch, onValueChange = { send(control.getJSONObject("command")) }),
                            trailing = { Switch(checked = checked, enabled = editable, onCheckedChange = null,
                                thumbContent = if (checked) { { Icon(painterResource(R.drawable.check_plain), null, Modifier.size(SwitchDefaults.IconSize)) } } else null) })
                    } else Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        Text(label, Modifier.weight(1f))
                        if (control.optBoolean("busy")) CircularProgressIndicator(Modifier.size(20.dp))
                        Switch(checked = control.getBoolean("selected"), enabled = editable, onCheckedChange = { send(control.getJSONObject("command")) },
                            modifier = Modifier.semantics { contentDescription = label })
                    }
                    "choice" -> {
                        val choices = control.getJSONArray("choices").objects()
                        ChoiceInput(label, control.getString("display"), choices.map { it.getString("label") }, editable,
                            control.getString("choice_style") == "button", summary = if (settings) summary else "", settings = settings,
                            description = control.optString("choice_description")) { send(choices[it].getJSONObject("command")) }
                    }
                    "progress" -> Column(Modifier.fillMaxWidth()) {
                        TextButton(onClick = { send(control.getJSONObject("command")) }, enabled = active) { Text(control.optString("cancel_label").ifEmpty { label }) }
                        control.getString("tooltip").takeIf { it.isNotEmpty() }?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
                        if (!control.isNull("progress")) {
                            val progress = control.getDouble("progress").toFloat()
                            if (progress < 0f) LinearProgressIndicator(Modifier.fillMaxWidth())
                            else LinearProgressIndicator(progress = { progress }, modifier = Modifier.fillMaxWidth())
                        }
                    }
                    "button" -> if (settings) SettingsRow(label, summary, active,
                        modifier = Modifier.clickable(enabled = active) { send(control.getJSONObject("command")) })
                    else TextButton(onClick = { send(control.getJSONObject("command")) }, enabled = active) { Text(label) }
                    "live_rows" -> LiveRows(timeline, control.getJSONObject("resource"))
                    "text", "loading" -> {
                        if (settings) SettingsRow(label, control.getString("display"), active)
                        else {
                            if (label.isNotEmpty()) Text(label, style = MaterialTheme.typography.labelMedium)
                            if (control.getString("display").isNotEmpty()) Text(control.getString("display"), color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                        if (widget == "loading") LinearProgressIndicator(Modifier.fillMaxWidth())
                    }
                    "number" -> NumberInput(timeline, control, editable)
                    "font" -> SettingsRow(label, control.getString("display"), editable,
                        modifier = Modifier.clickable(enabled = editable) { send(control.getJSONObject("command")) },
                        trailing = { Icon(painterResource(R.drawable.go_next), null, Modifier.size(20.dp)) })
                    "editor" -> if (settings) SettingsRow(label,
                        listOf(control.getString("display"), summary).filter { it.isNotEmpty() }.joinToString("\n"), editable,
                        modifier = Modifier.clickable(enabled = editable) { editor = control.getJSONObject("editor") })
                    else ListItem(headlineContent = { Text(if (label.isEmpty()) control.getString("display") else label) },
                        supportingContent = if (label.isNotEmpty()) { { Text(control.getString("display")) } } else null,
                        colors = ListItemDefaults.colors(containerColor = if (settings) MaterialTheme.colorScheme.surface else MaterialTheme.colorScheme.surfaceContainerLow),
                        modifier = Modifier.fillMaxWidth().clickable(enabled = editable) { editor = control.getJSONObject("editor") }
                            .then(if (settings) Modifier.padding(start = 8.dp).heightIn(min = 72.dp) else Modifier))
                    else -> error("Unknown native widget: $widget")
                }
            }
            if (!wide && toggles.isNotEmpty()) Row(Modifier.align(Alignment.Bottom)) { toggleButtons() }
        }
        if (summary.isNotEmpty() && !settings) Text(summary, style = MaterialTheme.typography.bodySmall)
        if (toggles.isNotEmpty()) {
            val duration = control.getInt("reveal_duration_ms")
            InlineSection(control.optJSONObject("graph"), duration) { graph -> FrameGraphEditor(timeline, graph) }
            control.optString("graph_error").takeUnless { it.isEmpty() || it == "null" }?.let {
                Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
            }
            InlineSection(control.optJSONObject("expression_editor"), duration) { expression ->
                InspectorExpression(timeline, expression, editable)
            }
        }
    }
    editor?.let { description -> InspectorEdit(description, { editor = null }, send) }
    confirmation?.let { action ->
        val dialog = action.getJSONObject("confirmation")
        AlertDialog(onDismissRequest = { confirmation = null },
            title = { Text(dialog.getString("title")) }, text = { Text(dialog.getString("message")) },
            confirmButton = { TextButton(onClick = { send(action.getJSONObject("command")); confirmation = null }) { Text(dialog.getString("confirm_label")) } },
            dismissButton = { TextButton(onClick = { confirmation = null }) { Text(dialog.getString("cancel_label")) } })
    }
}

/** Preserve the outgoing child during collapse; descriptor updates do not restart the animation. */
@Composable
private fun InlineSection(description: JSONObject?, duration: Int, content: @Composable (JSONObject) -> Unit) {
    AnimatedContent(targetState = description, contentKey = { it != null }, label = "Inspector section",
        transitionSpec = { (fadeIn(tween(duration)) togetherWith fadeOut(tween(duration)))
            .using(SizeTransform(sizeAnimationSpec = { _, _ -> tween(duration) })) }) { current ->
        current?.let { Column(Modifier.fillMaxWidth().padding(top = 6.dp)) { content(it) } }
    }
}

@Composable
private fun InspectorExpression(timeline: TimelineSession, editor: JSONObject, enabled: Boolean) {
    val owner = editor.getJSONObject("command").toString()
    val field = editor.getJSONArray("fields").getJSONObject(0)
    val source = field.getString("value")
    val events = remember(timeline.identity, owner) { Channel<JSONObject>(Channel.UNLIMITED) }
    var value by remember(owner) { mutableStateOf(source) }
    var focused by remember(owner) { mutableStateOf(false) }
    var failure by remember(owner) { mutableStateOf<String?>(null) }
    val finish = remember(owner) { editor.getJSONObject("finish") }
    LaunchedEffect(source, focused) { if (!focused) value = source }
    LaunchedEffect(timeline.identity, owner) {
        try {
            for (command in events) {
                try { timeline.request(command); failure = null }
                catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
                catch (error: Exception) { failure = error.message }
            }
        } finally {
            withContext(NonCancellable) {
                // Preserve the last edit before the native field is disposed.
                // A changed selection rejects stale opaque commands in Rust.
                try {
                    while (true) timeline.request(events.tryReceive().getOrNull() ?: break)
                    timeline.request(finish)
                } catch (_: Exception) { /* The owning selection may already be closed. */ }
            }
        }
    }
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        OutlinedTextField(value = value, onValueChange = {
            value = it
            events.trySend(JSONObject(editor.getJSONObject("command").toString()).put("values", JSONArray(listOf(it))))
        }, enabled = enabled, singleLine = false,
            label = if (!field.isNull("label") && field.getString("label").isNotEmpty()) { { Text(field.getString("label")) } } else null,
            textStyle = MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace),
            keyboardOptions = KeyboardOptions(autoCorrectEnabled = false, keyboardType = KeyboardType.Text),
            modifier = Modifier.fillMaxWidth().height(editor.getInt("height").dp).onFocusChanged {
                if (focused && !it.isFocused) events.trySend(finish)
                focused = it.isFocused
            })
        editor.getJSONObject("output").let { output ->
            val error = failure ?: output.optString("error").takeUnless { it.isEmpty() || it == "null" }
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(output.getString("label"), color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.bodySmall)
                androidx.compose.foundation.text.selection.SelectionContainer(Modifier.weight(1f)) {
                    Text(output.getString("value"), Modifier.fillMaxWidth(), maxLines = 1, overflow = TextOverflow.Ellipsis,
                        textAlign = androidx.compose.ui.text.style.TextAlign.End,
                        color = if (error == null) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.error,
                        style = MaterialTheme.typography.bodySmall)
                }
            }
            error?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
        }
    }
}

/** Native image decoding and drawing; Rust owns the resource and presentation. */
@Composable
private fun InspectorImage(timeline: TimelineSession, control: JSONObject) {
    val resource = control.getJSONObject("resource")
    val key = resource.toString()
    var image by remember(key) { mutableStateOf<ImageBitmap?>(null) }
    var failure by remember(key) { mutableStateOf<String?>(null) }
    LaunchedEffect(key) {
        try {
            val result = timeline.request(resource)
            image = withContext(Dispatchers.IO) {
                val bytes = android.util.Base64.decode(result.getString("image"), android.util.Base64.DEFAULT)
                android.graphics.ImageDecoder.decodeBitmap(android.graphics.ImageDecoder.createSource(java.nio.ByteBuffer.wrap(bytes))).asImageBitmap()
            }
        } catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
        catch (error: Exception) { failure = error.message ?: control.getString("error") }
    }
    Text(control.getString("label"), style = MaterialTheme.typography.labelMedium)
    image?.let { Image(it, contentDescription = control.getString("label"), modifier = Modifier.fillMaxWidth().height(control.getInt("height").dp)) }
    if (image == null && failure == null) LinearProgressIndicator(Modifier.fillMaxWidth())
    failure?.let { Text(it, color = MaterialTheme.colorScheme.error) }
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
