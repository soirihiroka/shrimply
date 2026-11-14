package dev.shrimply.app

import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.json.JSONObject

/** Native lifecycle and clipboard adapter for a Rust-described live resource. */
@Composable
internal fun LiveRows(timeline: TimelineSession, resource: JSONObject) {
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
            val arguments = payload.keys().asSequence().filter { it != "op" }.map { it to payload.get(it) }.toList().toTypedArray()
            val result = timeline.request(payload.getString("op"), *arguments)
            snapshot = result; error = null
            if (result.has("clipboard")) context.getSystemService(ClipboardManager::class.java)
                .setPrimaryClip(ClipData.newPlainText(result.getString("clipboard_label"), result.getString("clipboard")))
        } catch (failure: Exception) { error = failure.message }
    }
    LaunchedEffect(expanded, resumed) {
        while (expanded && resumed) { request(resource.getJSONObject("request")); delay(resource.getLong("interval_ms")) }
    }
    Column {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            TextButton(onClick = { expanded = !expanded }, modifier = Modifier.weight(1f)) { Text(resource.getString("label")) }
            val actions = resource.getJSONArray("actions")
            for (index in 0 until actions.length()) {
                val action = actions.getJSONObject(index)
                IconButton(onClick = { scope.launch { request(action.getJSONObject("request")) } }) {
                    Icon(painterResource(inspectorIcon(action.getString("icon"))), action.getString("label"), Modifier.size(16.dp))
                }
            }
        }
        error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        if (expanded) snapshot?.getJSONArray("rows")?.let { rows ->
            for (index in 0 until rows.length()) {
                val row = rows.getJSONObject(index)
                ListItem(headlineContent = { Text(row.getString("title")) }, supportingContent = { Text(row.getString("subtitle")) })
            }
        }
    }
}
