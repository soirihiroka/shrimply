package dev.shrimply.app

import android.content.ClipboardManager
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.DpOffset
import androidx.compose.ui.unit.dp
import org.json.JSONObject

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun TimelineContextMenu(menu: JSONObject, enabled: Boolean, command: (String, Array<out Pair<String, Any>>) -> Unit) {
    val context = LocalContext.current
    val dismiss = { command("context_dismiss", emptyArray()) }
    val sections = menu.getJSONArray("sections")
    val content: @Composable () -> Unit = {
        for (sectionIndex in 0 until sections.length()) {
            if (sectionIndex > 0) HorizontalDivider()
            val section = sections.getJSONArray(sectionIndex)
            for (index in 0 until section.length()) {
                val entry = section.getJSONObject(index)
                if (entry.has("action")) DropdownMenuItem(text = { Text(tr(entry.getString("label"))) }, enabled = enabled && entry.getBoolean("enabled"), onClick = {
                    val action = entry.getString("action")
                    val arguments = mutableListOf<Pair<String, Any>>("action" to action)
                    if (action == "paste") {
                        val clipboard = context.getSystemService(ClipboardManager::class.java)
                        arguments += "clipboard_marker" to (clipboard.primaryClip?.let { it.itemCount > 0 && it.getItemAt(0).text?.toString() == menu.getString("clipboard_marker") } == true)
                    }
                    command("context_action", arguments.toTypedArray())
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
    else if (menu.getBoolean("touch")) ModalBottomSheet(onDismissRequest = dismiss) {
        Column(Modifier.fillMaxWidth().heightIn(max = 560.dp).verticalScroll(rememberScrollState()).navigationBarsPadding()) { content() }
    } else DropdownMenu(expanded = true, onDismissRequest = dismiss, offset = DpOffset(menu.getDouble("x").dp, menu.getDouble("y").dp)) { content() }
}
