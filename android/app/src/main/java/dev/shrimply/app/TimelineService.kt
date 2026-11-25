package dev.shrimply.app

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.json.JSONObject

/** Render Rust's service document using the inspector's existing native controls. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun TimelineService(timeline: TimelineSession, files: ProjectFiles) {
    val event by timeline.snapshot
    val service = event.optJSONObject("timeline_service") ?: return
    val project = event.getJSONObject("status").getJSONObject("project")
    val scope = rememberCoroutineScope()
    var pendingFile by rememberSaveable { mutableStateOf<String?>(null) }
    var pendingOutput by rememberSaveable { mutableStateOf<String?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    fun send(command: JSONObject) { scope.launch {
        busy = true
        try { timeline.request("timeline_service_action", "project_path" to project.getString("path"),
            *command.keys().asSequence().map { it to command.get(it) }.toList().toTypedArray()); failure = null }
        catch (error: Exception) { failure = error.message }
        finally { busy = false }
    } }
    val chooseFile = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        val pending = pendingFile?.let(::JSONObject)
        pendingFile = null
        if (pending != null && uri != null) scope.launch {
            busy = true
            try { files.importFile(uri) { name, path, descriptor ->
                timeline.request("timeline_service_file", "project_path" to pending.getString("project_path"),
                    "command" to pending.getJSONObject("command"), "name" to name, "path" to path, descriptor = descriptor)
            } } catch (error: Exception) { failure = error.message }
            finally { busy = false }
        }
    }
    val save = rememberExportPicker { uri ->
        val pending = pendingOutput?.let(::JSONObject)
        pendingOutput = null
        if (pending != null && uri != null) scope.launch {
            busy = true
            try { timeline.request(pending.put("destination", uri.toString())); failure = null }
            catch (error: Exception) { failure = error.message }
            finally { busy = false }
        }
    }
    val close = service.getJSONObject("close")
    if (!service.getBoolean("visible")) return
    ModalBottomSheet(onDismissRequest = { send(service.getJSONObject("dismiss_command")) }) {
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 24.dp).padding(bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(service.getString("title"), style = MaterialTheme.typography.headlineSmall)
            service.getJSONArray("controls").objects().forEach { control -> key(control.getString("id")) {
                InspectorControl(timeline, control, !busy, ::send, { file ->
                    pendingFile = JSONObject().put("project_path", project.getString("path")).put("command", file.getJSONObject("command")).toString()
                    chooseFile.launch(arrayOf(file.getString("mime")))
                }, settings = true)
            } }
            failure?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                TextButton(enabled = !busy && close.getBoolean("enabled"), onClick = { send(close.getJSONObject("command")) }) { Text(close.getString("label")) }
                service.optJSONObject("submit")?.let { submit ->
                    Button(enabled = !busy && pendingOutput == null && submit.getBoolean("enabled"), onClick = {
                        val command = submit.getJSONObject("command")
                        val destination = submit.optJSONObject("destination")
                        if (destination == null) send(command) else {
                            pendingOutput = JSONObject(command.toString()).put("project_path", project.getString("path")).toString()
                            try { save(destination) }
                            catch (error: Exception) { pendingOutput = null; failure = error.message }
                        }
                    }) { Text(submit.getString("label")) }
                }
            }
        }
    }
}
