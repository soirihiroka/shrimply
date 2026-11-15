package dev.shrimply.app

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.DpOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import kotlinx.coroutines.launch
import org.json.JSONObject

/** Rust supplies all graph state, commands, labels and gesture semantics. */
@Composable
internal fun FrameGraphEditor(timeline: TimelineSession) {
    val graph = timeline.snapshot.value.optJSONObject("framegraph") ?: return
    val scope = rememberCoroutineScope()
    var error by remember(graph.getLong("serial")) { mutableStateOf<String?>(null) }
    val send: (JSONObject) -> Unit = { command -> scope.launch {
        try { timeline.request(command) } catch (failure: Exception) { error = failure.message }
    }; Unit }
    val dismiss = { send(graph.getJSONObject("dismiss")) }
    Dialog(onDismissRequest = dismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxWidth().fillMaxHeight(0.85f).padding(12.dp), shape = MaterialTheme.shapes.large) {
            Column {
                Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(graph.getString("title"), Modifier.weight(1f), style = MaterialTheme.typography.titleMedium)
                    TextButton(onClick = dismiss) { Text(graph.getString("close_label")) }
                }
                Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState())) {
                    graph.getJSONArray("actions").objects().forEach { action ->
                        IconButton(enabled = action.getBoolean("enabled"), onClick = { send(action.getJSONObject("command")) }) {
                            Icon(painterResource(inspectorIcon(action.getString("icon"))), action.getString("label"), Modifier.size(16.dp))
                        }
                    }
                }
                Box(Modifier.weight(1f).fillMaxWidth()) {
                    NativeSurface(timeline, Modifier.fillMaxSize(), role = 2)
                    graph.optJSONObject("menu")?.let { menu ->
                        AdaptiveMenu(expanded = true, bottomSheet = menu.getBoolean("bottom_sheet"),
                            dismiss = { send(menu.getJSONObject("dismiss")) },
                            offset = DpOffset(menu.getDouble("x").dp, menu.getDouble("y").dp), atPoint = true) {
                            menu.getJSONArray("items").objects().forEach { item ->
                                DropdownMenuItem(text = { Text(item.getString("label")) },
                                    trailingIcon = { Checkbox(checked = item.getBoolean("selected"), onCheckedChange = null) },
                                    onClick = { send(item.getJSONObject("command")) })
                            }
                        }
                    }
                }
                error?.let { Text(it, Modifier.padding(12.dp), color = MaterialTheme.colorScheme.error) }
            }
        }
    }
}
