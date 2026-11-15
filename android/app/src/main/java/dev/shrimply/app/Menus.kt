package dev.shrimply.app

import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.PointerType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.DpOffset
import androidx.compose.ui.unit.dp
import org.json.JSONObject
import org.json.JSONArray

/** SettingsLib Spa's preference layout: 24/16dp edges and 16dp text padding. */
@Composable
internal fun SettingsRow(title: String, summary: String = "", enabled: Boolean = true,
    modifier: Modifier = Modifier, leading: (@Composable () -> Unit)? = null,
    trailing: (@Composable () -> Unit)? = null) {
    Row(modifier.fillMaxWidth().padding(end = 16.dp), verticalAlignment = Alignment.CenterVertically) {
        if (leading == null) Spacer(Modifier.width(24.dp))
        else Box(Modifier.size(72.dp).alpha(if (enabled) 1f else 0.38f), contentAlignment = Alignment.Center) { leading() }
        Column(Modifier.weight(1f).padding(vertical = 16.dp).alpha(if (enabled) 1f else 0.38f)) {
            Text(title, Modifier.padding(vertical = 2.dp), style = MaterialTheme.typography.titleMedium)
            if (summary.isNotEmpty()) Text(summary, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        if (trailing != null) { Spacer(Modifier.width(16.dp)); trailing() }
    }
}

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
        Column(Modifier.fillMaxWidth().heightIn(max = 560.dp).verticalScroll(rememberScrollState()).navigationBarsPadding()) { content() }
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

/** Native settings rows share the same searched, input-aware choice menu. */
@Composable
internal fun ChoiceInput(label: String, selected: String, labels: List<String>, enabled: Boolean = true,
    button: Boolean = false, summary: String = "", settings: Boolean = false, description: String = "", select: (Int) -> Unit) {
    AdaptiveMenuAnchor(anchor = { open ->
        if (button) TextButton(onClick = open, enabled = enabled) {
            Text(label)
            Icon(painterResource(R.drawable.go_down), null, Modifier.padding(start = 4.dp).size(16.dp))
        } else if (settings) SettingsRow(label, listOf(selected, summary).filter { it.isNotEmpty() }.joinToString("\n"), enabled,
            modifier = Modifier.clickable(enabled = enabled, onClick = open),
            trailing = { Icon(painterResource(R.drawable.go_down), null, Modifier.size(20.dp).alpha(if (enabled) 1f else 0.38f)) })
        else ListItem(headlineContent = { Text(label) },
            supportingContent = if (selected.isEmpty() && summary.isEmpty()) null else { { Text(listOf(selected, summary).filter { it.isNotEmpty() }.joinToString("\n")) } },
            trailingContent = { Icon(painterResource(R.drawable.go_down), null, Modifier.size(16.dp)) },
            colors = ListItemDefaults.colors(containerColor = if (settings) MaterialTheme.colorScheme.surface else MaterialTheme.colorScheme.surfaceContainerLow),
            modifier = Modifier.fillMaxWidth().clickable(enabled = enabled, onClick = open)
                .then(if (settings) Modifier.padding(start = 8.dp).heightIn(min = 72.dp) else Modifier))
    }) { dismiss ->
        if (description.isNotEmpty()) Text(description, Modifier.padding(horizontal = 24.dp, vertical = 16.dp), style = MaterialTheme.typography.bodyMedium)
        ChoiceMenuItems(labels, selected = labels.indexOf(selected)) { index -> dismiss(); select(index) }
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
internal fun ChoiceMenuItems(labels: List<String>, selected: Int = -1, select: (Int) -> Unit) {
    var query by remember { mutableStateOf("") }
    val encoded = remember(labels) { JSONArray(labels).toString() }
    val matches = remember(encoded, query) { JSONArray(NativeLayout.matchingChoices(encoded, query)) }
    if (NativeLayout.choiceSearchable(labels.size)) OutlinedTextField(
        value = query, onValueChange = { query = it }, singleLine = true,
        label = { Text(tr("Search")) }, modifier = Modifier.padding(12.dp).widthIn(min = 240.dp))
    for (index in 0 until matches.length()) {
        val choice = matches.getInt(index)
        DropdownMenuItem(text = { Text(labels[choice]) },
            trailingIcon = if (selected < 0) null else { { RadioButton(selected = choice == selected, onClick = null) } },
            onClick = { select(choice) })
    }
}
