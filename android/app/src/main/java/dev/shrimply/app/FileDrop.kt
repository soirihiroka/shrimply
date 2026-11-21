package dev.shrimply.app

import androidx.activity.compose.LocalActivity
import androidx.compose.foundation.draganddrop.dragAndDropTarget
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draganddrop.DragAndDropEvent
import androidx.compose.ui.draganddrop.DragAndDropTarget
import androidx.compose.ui.draganddrop.toAndroidDragEvent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.findRootCoordinates
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

/** Only URI leases, coordinates, and native presentation live here. Rust owns the import. */
@Composable
internal fun Modifier.fileDropTarget(
    timeline: TimelineSession,
    files: ProjectFiles,
    projectPath: String?,
    atTimeline: Boolean,
    onError: (String) -> Unit,
): Modifier {
    val activity = checkNotNull(LocalActivity.current)
    val density = LocalDensity.current.density
    val scope = rememberCoroutineScope()
    val currentPath by rememberUpdatedState(projectPath)
    val report by rememberUpdatedState(onError)
    val coordinates = remember { arrayOfNulls<LayoutCoordinates>(1) }
    var pending by remember(timeline, projectPath) { mutableStateOf<String?>(null) }
    val active = remember(timeline, projectPath) { booleanArrayOf(true) }
    DisposableEffect(timeline, projectPath) {
        onDispose {
            active[0] = false
            pending?.let { encoded ->
                val session = JSONObject(encoded)
                scope.launch(NonCancellable, start = CoroutineStart.UNDISPATCHED) {
                    runCatching { timeline.request("file_drop_cancel", "project_path" to session.getString("project_path"),
                        "id" to session.getString("id")) }
                }
            }
        }
    }
    val target = remember(timeline, files, activity, density, atTimeline, projectPath) {
        object : DragAndDropTarget {
            override fun onDrop(event: DragAndDropEvent): Boolean {
                val project = currentPath ?: return false
                val androidEvent = event.toAndroidDragEvent()
                val clip = androidEvent.clipData ?: return false
                val uris = (0 until clip.itemCount).mapNotNull { clip.getItemAt(it).uri }.distinct()
                if (uris.isEmpty()) return false
                val point = if (atTimeline) {
                    val local = coordinates[0]?.takeIf { it.isAttached } ?: return false
                    local.localPositionOf(local.findRootCoordinates(), Offset(androidEvent.x, androidEvent.y)) / density
                } else null
                val permission = activity.requestDragAndDropPermissions(androidEvent)
                // Enter the lease before returning to Android, even if the target is disposed.
                scope.launch(start = CoroutineStart.UNDISPATCHED) {
                    var id: String? = null
                    var handedOff = false
                    try {
                        val begin = JSONObject().put("op", "file_drop_begin").put("project_path", project)
                        point?.let { begin.put("point", JSONObject().put("x", it.x).put("y", it.y)) }
                        val sessionId = withContext(NonCancellable) {
                            timeline.request(begin).getString("id").also { id = it }
                        }
                        for (uri in uris) files.importFile(uri) { name, path, descriptor ->
                            timeline.request("file_drop_file", "project_path" to project, "id" to sessionId,
                                "name" to name, "path" to path, descriptor = descriptor)
                        }
                        val result = withContext(NonCancellable) {
                            timeline.request("file_drop_commit", "project_path" to project, "id" to sessionId).also {
                                handedOff = !it.has("confirmation")
                            }
                        }
                        currentCoroutineContext().ensureActive()
                        if (!active[0]) throw kotlinx.coroutines.CancellationException("File drop target closed")
                        result.optJSONObject("confirmation")?.let { prompt ->
                            pending = JSONObject().put("id", id).put("project_path", project).put("prompt", prompt).toString()
                        }
                        handedOff = true
                    } catch (failure: Exception) {
                        if (failure !is kotlinx.coroutines.CancellationException) report(failure.message ?: failure.toString())
                    } finally {
                        try {
                            val abandoned = id
                            if (!handedOff && abandoned != null) withContext(NonCancellable) {
                                runCatching { timeline.request("file_drop_cancel", "project_path" to project, "id" to abandoned) }
                            }
                        } finally { permission?.release() }
                    }
                }
                return true
            }
        }
    }
    pending?.let { encoded ->
        val session = JSONObject(encoded)
        val prompt = session.getJSONObject("prompt")
        fun finish(accepted: Boolean) {
            pending = null
            scope.launch(NonCancellable, start = CoroutineStart.UNDISPATCHED) {
                try { timeline.request("file_drop_remux", "project_path" to session.getString("project_path"),
                    "id" to session.getString("id"), "accepted" to accepted) }
                catch (failure: Exception) { report(failure.message ?: failure.toString()) }
            }
        }
        RemuxConfirmation(prompt, ::finish)
    }
    return onGloballyPositioned { coordinates[0] = it }
        .dragAndDropTarget(shouldStartDragAndDrop = { currentPath != null }, target = target)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun RemuxConfirmation(prompt: JSONObject, finish: (Boolean) -> Unit) {
    ModalBottomSheet(onDismissRequest = { finish(false) }) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp).padding(bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text(prompt.getString("title"), style = MaterialTheme.typography.headlineSmall)
            Text(prompt.getString("body"))
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                TextButton(onClick = { finish(false) }) { Text(prompt.getString("cancel_label")) }
                Button(onClick = { finish(true) }) { Text(prompt.getString("confirm_label")) }
            }
        }
    }
}
