package dev.shrimply.app

import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.PointerType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.DpOffset
import androidx.compose.ui.unit.dp
import org.json.JSONObject
import org.json.JSONArray

@Composable
internal fun SettingsRow(title: String, summary: String = "", enabled: Boolean = true,
    modifier: Modifier = Modifier, leading: (@Composable () -> Unit)? = null,
    trailing: (@Composable () -> Unit)? = null) {
    Row(Modifier.fillMaxWidth().clip(MaterialTheme.shapes.extraSmall)
        .background(MaterialTheme.colorScheme.surfaceContainerLow).then(modifier).padding(end = 16.dp), verticalAlignment = Alignment.CenterVertically) {
        if (leading == null) Spacer(Modifier.width(16.dp))
        else Box(Modifier.size(56.dp).alpha(if (enabled) 1f else 0.38f), contentAlignment = Alignment.Center) { leading() }
        Column(Modifier.weight(1f).padding(vertical = 16.dp).alpha(if (enabled) 1f else 0.38f)) {
            Text(title, Modifier.padding(vertical = 2.dp), style = MaterialTheme.typography.titleMedium)
            if (summary.isNotEmpty()) Text(summary, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        if (trailing != null) { Spacer(Modifier.width(16.dp)); trailing() }
    }
}

private val LocalMenuBottomSheet = staticCompositionLocalOf { false }

internal val LocalMenuInputSource = staticCompositionLocalOf<MutableState<String>> { error("Native menus need an input source") }

private fun Modifier.trackMenuInput(inputSource: MutableState<String>): Modifier = pointerInput(inputSource) {
    awaitPointerEventScope {
        while (true) {
            val event = awaitPointerEvent(PointerEventPass.Initial)
            event.changes.firstOrNull()?.let { change ->
                inputSource.value = when (change.type) {
                    PointerType.Touch -> "touch"
                    PointerType.Stylus, PointerType.Eraser -> "pen"
                    else -> "pointer"
                }
            }
        }
    }
}.onPreviewKeyEvent { inputSource.value = "keyboard"; false }

/** Native dialogs have separate input roots, so anchors use this observer too. */
@Composable
internal fun MenuInputScope(inputSource: MutableState<String>, content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalMenuInputSource provides inputSource) {
        Box(Modifier.fillMaxSize().trackMenuInput(inputSource)) { content() }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun AdaptiveMenu(expanded: Boolean, bottomSheet: Boolean, dismiss: () -> Unit,
    offset: DpOffset = DpOffset.Zero, atPoint: Boolean = false, content: @Composable () -> Unit) {
    if (!expanded) return
    if (bottomSheet) ModalBottomSheet(onDismissRequest = dismiss) {
        CompositionLocalProvider(LocalMenuBottomSheet provides true) {
            Column(Modifier.fillMaxWidth().heightIn(max = 560.dp).verticalScroll(rememberScrollState()).navigationBarsPadding()) { content() }
        }
    } else if (atPoint) Box(Modifier.offset(offset.x, offset.y)) {
        DropdownMenu(expanded = true, onDismissRequest = dismiss) { content() }
    } else DropdownMenu(expanded = true, onDismissRequest = dismiss, offset = offset) { content() }
}

@Composable
internal fun AdaptiveMenuAnchor(anchor: @Composable (open: () -> Unit) -> Unit,
    content: @Composable (dismiss: () -> Unit) -> Unit) {
    val inputSource = LocalMenuInputSource.current
    var bottomSheet by remember { mutableStateOf<Boolean?>(null) }
    val dismiss = { bottomSheet = null }
    Box(Modifier.trackMenuInput(inputSource)) {
        anchor { bottomSheet = NativeLayout.menuUsesBottomSheet(inputSource.value) }
        AdaptiveMenu(expanded = bottomSheet != null, bottomSheet = bottomSheet == true, dismiss = dismiss) { content(dismiss) }
    }
}

@Composable
internal fun AdaptiveMenuButton(enabled: Boolean, modifier: Modifier = Modifier, icon: @Composable () -> Unit,
    content: @Composable (dismiss: () -> Unit) -> Unit) {
    AdaptiveMenuAnchor(anchor = { open -> IconButton(onClick = open, enabled = enabled, modifier = modifier) { icon() } }, content = content)
}

@Composable
internal fun InspectorInput(
    modifier: Modifier = Modifier, enabled: Boolean = true, content: @Composable RowScope.() -> Unit,
) {
    Surface(modifier, shape = MaterialTheme.shapes.small, color = MaterialTheme.colorScheme.surfaceContainerHigh) {
        Row(Modifier.heightIn(min = 40.dp).padding(horizontal = 8.dp).alpha(if (enabled) 1f else 0.38f),
            horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically,
            content = content)
    }
}

@Composable
internal fun InspectorField(label: String, field: @Composable (Modifier) -> Unit) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically) {
        if (label.isNotEmpty()) Text(label, Modifier.weight(1f), style = MaterialTheme.typography.labelMedium)
        field(Modifier.weight(1.5f))
    }
}

@Composable
internal fun ChoiceInput(label: String, selected: String, labels: List<String>, enabled: Boolean = true,
    button: Boolean = false, buttonIcon: String? = null, select: (Int) -> Unit) {
    val active = enabled && labels.isNotEmpty()
    val anchor: @Composable (open: () -> Unit) -> Unit = { open ->
        if (button) Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
            TextButton(onClick = open, enabled = active) {
                buttonIcon?.let { Icon(iconVector(it), null, Modifier.padding(end = 6.dp).size(16.dp)) }
                Text(label)
            }
        } else InspectorField(label) { modifier ->
            InspectorInput(modifier.clickable(enabled = active, role = Role.Button, onClick = open), active) {
                Text(selected, Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis,
                    style = MaterialTheme.typography.bodyMedium)
                Icon(iconVector("go-down-symbolic"), null, Modifier.size(16.dp))
            }
        }
    }
    AdaptiveMenuAnchor(anchor) { dismiss ->
        ChoiceMenuItems(labels, selected = labels.indexOf(selected)) { index -> dismiss(); select(index) }
    }
}

@Composable
internal fun SettingsChoice(control: JSONObject, enabled: Boolean, send: (JSONObject) -> Unit) {
    var open by rememberSaveable { mutableStateOf(false) }
    val title = control.getString("label")
    val choices = control.getJSONArray("choices").objects()
    val labels = choices.map { it.getString("label") }
    val selected = control.getString("display")
    val summary = listOf(selected, control.getString("subtitle")).filter { it.isNotEmpty() }.joinToString("\n")
    val active = enabled && choices.isNotEmpty()
    SettingsRow(title, summary, active, Modifier.clickable(enabled = active, role = Role.Button) { open = true },
        trailing = { Icon(iconVector("go-down-symbolic"), null, Modifier.size(20.dp).alpha(if (active) 1f else 0.38f)) })
    if (open && active) AlertDialog(onDismissRequest = { open = false }, title = { Text(title) },
        text = {
            val index = labels.indexOf(selected)
            Column(Modifier.verticalScroll(rememberScrollState()).selectableGroup()) {
                control.optString("choice_description").takeIf { it.isNotEmpty() }?.let {
                    Text(it, Modifier.padding(bottom = 12.dp))
                }
                ChoiceMenuItems(labels, index, radioRows = true) { choice ->
                    open = false
                    send(choices[choice].getJSONObject("command"))
                }
            }
        }, confirmButton = { TextButton(onClick = { open = false }) { Text(tr("Cancel")) } })
}

@Composable
internal fun ProjectMenuItems(menu: JSONObject, dismiss: () -> Unit, activate: (String) -> Unit) {
    var path by remember { mutableStateOf(emptyList<String>()) }
    val active = path.fold(menu) { parent, id ->
        parent.getJSONArray("entries").objects().firstOrNull { it.optString("id") == id } ?: parent
    }
    if (path.isNotEmpty()) {
        DropdownMenuItem(text = { Text(active.getString("label")) },
            leadingIcon = { Icon(iconVector("go-previous-symbolic"), tr("Back"), Modifier.size(16.dp)) },
            onClick = { path = path.dropLast(1) })
        HorizontalDivider()
    }
    active.getJSONArray("entries").objects().forEach { entry ->
        when (entry.getString("kind")) {
            "separator" -> HorizontalDivider()
            "submenu" -> DropdownMenuItem(text = { Text(entry.getString("label")) },
                trailingIcon = { Icon(iconVector("go-next-symbolic"), null, Modifier.size(16.dp)) },
                onClick = { path = path + entry.getString("id") })
            "action" -> DropdownMenuItem(text = { Text(entry.getString("label")) },
                enabled = entry.getBoolean("enabled"), onClick = { dismiss(); activate(entry.getString("action")) })
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun TimelineContextMenu(menu: JSONObject, enabled: Boolean, origin: DpOffset, command: (String, Array<out Pair<String, Any>>) -> Unit) {
    val context = LocalContext.current
    val dismiss = { command("context_dismiss", emptyArray()) }
    val clipboardRequest = menu.optJSONObject("clipboard")
    LaunchedEffect(menu.getLong("serial"), clipboardRequest?.toString()) {
        val request = clipboardRequest ?: return@LaunchedEffect
        val clipboard = context.getSystemService(ClipboardManager::class.java)
        val marker = request.getString("marker")
        val arguments = if (request.getBoolean("write")) {
            clipboard.setPrimaryClip(ClipData.newPlainText("", marker))
            emptyArray()
        } else arrayOf<Pair<String, Any>>("clipboard_marker" to
            (clipboard.primaryClip?.let { it.itemCount > 0 && it.getItemAt(0).text?.toString() == marker } == true))
        command(request.getJSONObject("command").getString("op"), arguments)
    }
    if (clipboardRequest != null) return
    val bottomSheet = remember(menu.getLong("serial")) { menu.getBoolean("bottom_sheet") }
    val sections = menu.getJSONArray("sections")
    val content: @Composable () -> Unit = {
        for (sectionIndex in 0 until sections.length()) {
            if (sectionIndex > 0) HorizontalDivider()
            val section = sections.getJSONArray(sectionIndex)
            for (index in 0 until section.length()) {
                val entry = section.getJSONObject(index)
                if (entry.has("action")) DropdownMenuItem(text = { Text(tr(entry.getString("label"))) },
                    trailingIcon = if (entry.isNull("checked")) null else { { Checkbox(checked = entry.getBoolean("checked"), onCheckedChange = null) } },
                    enabled = enabled && entry.getBoolean("enabled"), onClick = {
                    command("context_action", arrayOf("action" to entry.getString("action")))
                }) else {
                    var value by remember(menu.getLong("serial"), sectionIndex, index, entry.getDouble("value")) { mutableFloatStateOf(entry.getDouble("value").toFloat()) }
                    Column(Modifier.widthIn(min = 240.dp).padding(horizontal = 16.dp)) {
                        Text(tr(entry.getString("label")))
                        Slider(value, { value = it }, enabled = enabled,
                            valueRange = entry.getDouble("minimum").toFloat()..entry.getDouble("maximum").toFloat(),
                            onValueChangeFinished = { command("context_control", arrayOf("section" to sectionIndex, "control" to entry.getInt("control"), "value" to value)) })
                    }
                }
            }
        }
    }
    if (!menu.isNull("delete_count")) AlertDialog(onDismissRequest = dismiss, title = { Text(tr("Delete Tracks?")) },
        text = { Text(tr("%{count} clips are about to be deleted, are you sure?", "count" to menu.getInt("delete_count"))) },
        confirmButton = { TextButton(enabled = enabled, onClick = { command("context_delete", emptyArray()) }) { Text(tr("Delete")) } },
        dismissButton = { TextButton(enabled = enabled, onClick = dismiss) { Text(tr("Cancel")) } })
    else AdaptiveMenu(expanded = true, bottomSheet = bottomSheet, dismiss = dismiss,
        offset = origin + DpOffset(menu.getDouble("x").dp, menu.getDouble("y").dp), atPoint = true, content = content)
}

/** Search eligibility and matching are the same Rust selector contract used by GTK. */
@Composable
internal fun ChoiceMenuItems(labels: List<String>, selected: Int = -1, radioRows: Boolean = false, select: (Int) -> Unit) {
    var query by remember { mutableStateOf("") }
    val encoded = remember(labels) { JSONArray(labels).toString() }
    val matches = remember(encoded, query) { JSONArray(NativeLayout.matchingChoices(encoded, query)) }
    if (NativeLayout.choiceSearchable(labels.size)) {
        val width = if (LocalMenuBottomSheet.current || radioRows) Modifier.fillMaxWidth() else Modifier.widthIn(min = 240.dp)
        OutlinedTextField(value = query, onValueChange = { query = it }, singleLine = true,
            label = { Text(tr("Search")) }, modifier = Modifier.padding(12.dp).then(width))
    }
    for (index in 0 until matches.length()) {
        val choice = matches.getInt(index)
        if (radioRows || selected >= 0) Row(
            Modifier.fillMaxWidth().selectable(selected = choice == selected, role = Role.RadioButton,
                onClick = { select(choice) }).heightIn(min = 48.dp).padding(horizontal = 12.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically,
        ) {
            RadioButton(selected = choice == selected, onClick = null)
            Text(labels[choice])
        } else DropdownMenuItem(text = { Text(labels[choice]) }, onClick = { select(choice) })
    }
}
