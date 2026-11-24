package dev.shrimply.app

import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.json.JSONObject

/** Native lifecycle and clipboard adapter for a Rust-described live resource. */
@Composable
internal fun LiveRows(timeline: TimelineSession, resource: JSONObject, enabled: Boolean) {
    var expanded by rememberSaveable { mutableStateOf(false) }
    var snapshot by remember { mutableStateOf<JSONObject?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val lifecycle = (context as LifecycleOwner).lifecycle
    var resumed by remember { mutableStateOf(lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) }
    DisposableEffect(lifecycle) {
        val observer = LifecycleEventObserver { _, _ -> resumed = lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED) }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
    suspend fun request(payload: JSONObject) {
        try {
            val result = timeline.request(payload)
            snapshot = result; error = null
            if (result.has("clipboard")) context.getSystemService(ClipboardManager::class.java)
                .setPrimaryClip(ClipData.newPlainText(result.getString("clipboard_label"), result.getString("clipboard")))
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (failure: Exception) {
            error = failure.message
        }
    }
    LaunchedEffect(expanded, resumed) {
        while (expanded && resumed) { request(resource.getJSONObject("request")); delay(resource.getLong("interval_ms")) }
    }
    Card(Modifier.fillMaxWidth(), colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainer)) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            InspectorDisclosure(expanded, tr(if (expanded) "Collapse" else "Expand"), enabled) {
                expanded = !expanded
                if (!expanded) { snapshot = null; error = null }
            }
            Text(resource.getString("label"), Modifier.weight(1f), style = MaterialTheme.typography.titleSmall)
            resource.getJSONArray("actions").objects().forEach { action ->
                IconButton(enabled = enabled, onClick = { scope.launch { request(action.getJSONObject("request")) } }) {
                    Icon(iconVector(action.getString("icon")), action.getString("label"), Modifier.size(16.dp))
                }
            }
        }
        if (expanded) {
            error?.let { Text(it, Modifier.padding(horizontal = 12.dp), color = MaterialTheme.colorScheme.error) }
            snapshot?.getJSONArray("rows")?.objects()?.takeIf { it.isNotEmpty() }?.let { rows ->
                Column(Modifier.fillMaxWidth().padding(start = 12.dp, end = 12.dp, bottom = 12.dp).clip(MaterialTheme.shapes.small),
                    verticalArrangement = Arrangement.spacedBy(1.dp)) {
                    rows.forEach { row -> SettingsRow(row.getString("title"), row.getString("subtitle")) }
                }
            }
        }
    }
}
