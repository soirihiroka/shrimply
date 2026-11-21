package dev.shrimply.app

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.DpOffset
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.json.JSONObject

/** Rust supplies all graph state, commands, labels and gesture semantics. */
@Composable
internal fun FrameGraphEditor(timeline: TimelineSession, graph: JSONObject) {
    val scope = rememberCoroutineScope()
    var error by remember(graph.getLong("serial")) { mutableStateOf<String?>(null) }
    val send: (JSONObject) -> Unit = { command -> scope.launch {
        try { timeline.request(command); error = null }
        catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
        catch (failure: Exception) { error = failure.message }
    } }
    Column(Modifier.fillMaxWidth()) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End, verticalAlignment = Alignment.CenterVertically) {
            graph.getJSONArray("actions").objects().forEach { action ->
                IconButton(modifier = Modifier.size(32.dp), enabled = action.getBoolean("enabled"), onClick = { send(action.getJSONObject("command")) }) {
                    Icon(painterResource(inspectorIcon(action.getString("icon"))), action.getString("label"), Modifier.size(16.dp))
                }
            }
        }
        Box(Modifier.fillMaxWidth().height(graph.getInt("height").dp)) {
            key(graph.getInt("surface")) { NativeSurface(timeline, Modifier.fillMaxSize(), role = graph.getInt("surface"), enabled = graph.getBoolean("enabled")) }
            graph.optJSONObject("menu")?.let { menu ->
                AdaptiveMenu(expanded = true, bottomSheet = menu.getBoolean("bottom_sheet"),
                    dismiss = { send(menu.getJSONObject("dismiss")) },
                    offset = DpOffset(menu.getDouble("x").dp, menu.getDouble("y").dp), atPoint = true) {
                    menu.getJSONArray("items").objects().forEach { item ->
                        DropdownMenuItem(text = { Text(item.getString("label")) },
                            enabled = item.optBoolean("enabled", true),
                            leadingIcon = item.optString("icon").takeIf { it.isNotEmpty() }?.let { icon ->
                                { Icon(painterResource(inspectorIcon(icon)), null, Modifier.size(16.dp)) }
                            },
                            trailingIcon = if (item.has("selected")) { { Checkbox(checked = item.getBoolean("selected"), onCheckedChange = null) } } else null,
                            onClick = { send(item.getJSONObject("command")) })
                    }
                }
            }
        }
        error?.let { Text(it, Modifier.padding(12.dp), color = MaterialTheme.colorScheme.error) }
    }
}
