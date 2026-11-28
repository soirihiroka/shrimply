package dev.shrimply.app

import android.content.Intent
import androidx.activity.compose.LocalActivity
import android.view.WindowInsets
import androidx.compose.foundation.layout.WindowInsets as ComposeInsets
import android.view.WindowInsetsController
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.zIndex
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.DpOffset
import androidx.core.graphics.drawable.toBitmap
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

private val ExpandedWidth = 840.dp
private val PaneHandleTargetSize = 48.dp
private val TimelineMinimumHeight = 120.dp
private const val MaximumTimelineFraction = 0.75f

/** Layout and actions follow launcher-gtk and preview-gtk, using Android native controls. */
@Composable
internal fun ProjectApp(timeline: TimelineSession, files: ProjectFiles, caption: WindowCaption = WindowCaption()) {
    val menuInputSource = remember { mutableStateOf("touch") }
    val event by timeline.snapshot
    val status = event.optJSONObject("status") ?: JSONObject()
    val project = status.optJSONObject("project")
    val workspaceState = rememberSaveableStateHolder()
    val workspaceKey = project?.getString("path") ?: "launcher"
    DisposableEffect(workspaceKey) { onDispose { workspaceState.removeState(workspaceKey) } }
    val playback = event.optJSONObject("playback") ?: JSONObject()
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    var busy by remember { mutableStateOf(false) }
    val operations = remember { Mutex() }
    var progress by remember { mutableStateOf("") }
    var exportSettings by rememberSaveable { mutableStateOf<String?>(null) }
    var pendingExport by rememberSaveable { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var preferences by rememberSaveable { mutableStateOf(false) }
    val servers = status.optJSONArray("project_servers")?.let { values ->
        List(values.length()) { values.getString(it) }
    }.orEmpty()
    var projectSources by remember { mutableStateOf<List<JSONObject>?>(null) }
    var version by remember { mutableStateOf("") }
    var projects by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var catalogRevision by remember { mutableIntStateOf(0) }
    var projectServer by rememberSaveable { mutableStateOf("") }
    var create by rememberSaveable { mutableStateOf(false) }
    var name by rememberSaveable { mutableStateOf("") }
    var canvasWidth by rememberSaveable { mutableStateOf("") }
    var canvasHeight by rememberSaveable { mutableStateOf("") }
    var fps by rememberSaveable { mutableStateOf("") }
    var tags by rememberSaveable { mutableStateOf("") }
    var schema by remember { mutableStateOf(JSONObject()) }

    fun useSettings(settings: JSONObject) {
        name = settings.optString("name")
        canvasWidth = settings.optString("canvas_width")
        canvasHeight = settings.optString("canvas_height")
        fps = settings.optString("fps")
        tags = settings.optJSONArray("tags")?.let { values -> (0 until values.length()).joinToString(", ") { values.getString(it) } } ?: ""
    }
    var choices by remember { mutableStateOf<List<File>>(emptyList()) }
    var choicesBottomSheet by rememberSaveable { mutableStateOf<Boolean?>(null) }
    var versionOpen by rememberSaveable { mutableStateOf<String?>(null) }
    var versionMessage by rememberSaveable { mutableStateOf("") }
    var comment by rememberSaveable { mutableStateOf(false) }
    var importTarget by rememberSaveable { mutableStateOf<String?>(null) }
    var pendingRemux by rememberSaveable(workspaceKey) { mutableStateOf<String?>(null) }
    var commentTarget by rememberSaveable { mutableStateOf<String?>(null) }
    var trackMenuTarget by rememberSaveable { mutableStateOf<String?>(null) }
    var inspectorFileTarget by rememberSaveable { mutableStateOf<String?>(null) }
    var pendingAudioRecord by rememberSaveable { mutableStateOf<String?>(null) }
    var lastActionToken by rememberSaveable { mutableStateOf<String?>(null) }
    var commentText by rememberSaveable { mutableStateOf("") }

    fun action(blocking: Boolean = true, block: suspend () -> Unit) {
        scope.launch { operations.withLock {
            // Picker results must queue behind startup/settings requests, never disappear
            // because rotation temporarily marks the new composition busy.
            if (blocking) busy = true
            try { block() } catch (failure: Exception) { error = failure.message ?: "The project operation failed." }
            finally { if (blocking) { busy = false; progress = "" } }
        } }
    }
    LaunchedEffect(project == null, servers, catalogRevision) {
        if (project != null) return@LaunchedEffect
        try {
            for (includeRemote in if (servers.isEmpty()) listOf(false) else listOf(false, true)) {
                val response = withContext(Dispatchers.IO) { JSONObject(NativeTimeline.listProjects(includeRemote)) }
                check(!response.has("error")) { response.getString("error") }
                projects = response.getJSONArray("projects").objects()
                projectSources = response.getJSONArray("servers").objects()
                version = response.getString("version")
            }
        } catch (cancelled: kotlinx.coroutines.CancellationException) {
            throw cancelled
        } catch (failure: Exception) {
            error = failure.message
        }
    }
    fun command(operation: String, arguments: Array<out Pair<String, Any>>) = action(blocking = false) {
        val result = timeline.request(operation, *arguments)
        if (result.has("show_in_files")) files.showInFiles(result.getString("show_in_files"))
        if (result.has("message")) android.widget.Toast.makeText(context, result.getString("message"), android.widget.Toast.LENGTH_SHORT).show()
    }
    suspend fun openRequest(request: JSONObject) {
        try {
            val response = timeline.request(request)
            response.optJSONObject("version_mismatch")?.let { mismatch ->
                versionMessage = mismatch.getString("body")
                versionOpen = request.put("force_version", mismatch.getJSONObject("identity")).toString()
            }
        }
        catch (failure: Exception) { files.cancelOpen(); throw failure }
    }
    suspend fun requestOpen(path: String, reconciled: Boolean = false) = openRequest(files.openRequest(path, reconciled))
    fun open(path: String) = action { requestOpen(path) }
    fun closeProject() = action {
        timeline.request("save")
        project?.optString("path")?.let { files.flush(it) }
        timeline.request("close")
    }

    val openFolder = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
        if (uri != null) action {
            context.contentResolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
            val found = files.openTree(uri) { file -> scope.launch { progress = file } }
            if (found.size == 1) requestOpen(found.single().absolutePath, reconciled = true) else choices = found
        }
    }
    val exportFolder = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
        if (uri != null && project != null) action {
            val path = project.getString("path")
            if (!project.optBoolean("remote")) {
                timeline.request("export", "project_path" to path)
                files.exportTree(path, uri) { file -> scope.launch { progress = file } }
            } else {
                val folder = withContext(Dispatchers.IO) {
                    java.nio.file.Files.createTempDirectory(files.projectDirectory().toPath(), "export-").toFile()
                }
                try {
                    val copy = File(folder, File(path).name).absolutePath
                    timeline.request("export", "project_path" to path, "path" to copy)
                    files.exportTree(copy, uri) { file -> scope.launch { progress = file } }
                } finally {
                    withContext(Dispatchers.IO) { folder.deleteRecursively() }
                }
            }
        }
    }
    val saveExport = rememberExportPicker { uri ->
        val pending = pendingExport
        pendingExport = null
        if (uri != null && pending != null) action {
            timeline.request(JSONObject(pending).put("destination", uri.toString()))
            exportSettings = null
        }
    }

    fun pickExport(kind: String, projectPath: String, values: Array<out Pair<String, Any>>) = action {
        if (pendingExport != null) return@action
        val command = JSONObject().put("op", "export_destination").put("kind", kind).put("project_path", projectPath)
        values.forEach { (key, value) -> command.put(key, value) }
        val destination = timeline.request(command)
        pendingExport = command.put("op", "export_start").toString()
        try {
            saveExport(destination)
        } catch (failure: Exception) {
            pendingExport = null
            throw failure
        }
    }
    fun importPicked(uri: android.net.Uri, destination: String, confirmedName: String? = null) = action {
        val target = targetArguments(destination)
        var confirmation: String? = null
        files.importFile(uri) { name, path, descriptor ->
            check(confirmedName == null || confirmedName == name) { "The selected file changed. Select it again." }
            val result = timeline.request("import_file", "path" to path, "name" to name, *target,
                "remux_confirmed" to (confirmedName != null), descriptor = descriptor)
            confirmation = result.optJSONObject("confirmation")?.let { prompt ->
                val placement = JSONObject(destination).put("position", result.get("position"))
                JSONObject().put("uri", uri.toString()).put("destination", placement.toString())
                    .put("name", name).put("prompt", prompt).toString()
            }
        }
        pendingRemux = confirmation
    }
    val importFile = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        val destination = importTarget
        importTarget = null
        if (uri != null && destination != null) importPicked(uri, destination)
    }
    pendingRemux?.let { encoded ->
        val pending = JSONObject(encoded)
        RemuxConfirmation(pending.getJSONObject("prompt")) { accepted ->
            pendingRemux = null
            if (accepted) importPicked(android.net.Uri.parse(pending.getString("uri")),
                pending.getString("destination"), pending.getString("name"))
        }
    }
    val inspectorFile = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        val pending = inspectorFileTarget
        inspectorFileTarget = null
        if (uri != null && pending != null) action {
            val target = JSONObject(pending)
            files.importFile(uri) { name, path, descriptor ->
                timeline.request("inspector_file", "project_path" to target.getString("project_path"),
                    "command" to target.getJSONObject("command"), "path" to path, "name" to name, descriptor = descriptor)
            }
        }
    }
    fun defaultTarget(kind: String? = null): String = JSONObject().apply {
        put("project_path", project?.getString("path") ?: error("Open a project first."))
        if (kind != null) { put("kind", kind); put("track", 0) }
    }.toString()
    val microphonePermissionMessage = tr("Microphone permission is required to record audio.")
    val microphonePermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        val target = pendingAudioRecord
        pendingAudioRecord = null
        if (granted && target != null) action { timeline.request("record_audio", *targetArguments(target)) }
        else if (!granted) error = microphonePermissionMessage
    }
    val trackAction = event.optJSONObject("timeline")?.optJSONObject("action")
    LaunchedEffect(trackAction?.optLong("serial")) {
        val serial = trackAction?.optLong("serial") ?: 0L
        val token = "${timeline.identity}:$serial"
        if (serial > 0 && token != lastActionToken && project != null) {
            lastActionToken = token
            val target = JSONObject(trackAction.toString())
            if (target.optString("project_path") != project.getString("path")) return@LaunchedEffect
            if (target.optString("service") == "audio_record") {
                if (context.checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) == android.content.pm.PackageManager.PERMISSION_GRANTED) {
                    action { timeline.request("record_audio", *targetArguments(target.toString())) }
                } else {
                    pendingAudioRecord = target.toString()
                    microphonePermission.launch(android.Manifest.permission.RECORD_AUDIO)
                }
            } else trackMenuTarget = target.toString()
        }
    }

    LaunchedEffect(Unit) { action {
        timeline.request("status")
        schema = timeline.request("settings_schema")
    } }
    LaunchedEffect(event.optString("error")) {
        event.optString("error").takeIf { it.isNotEmpty() && it != "null" }?.let { error = it }
    }
    LaunchedEffect(status.optString("watch_error")) {
        status.optString("watch_error").takeIf { it.isNotEmpty() && it != "null" }?.let { error = it }
    }
    DisposableEffect(project?.optString("path")) {
        val owner = files.attach(project?.optString("path")) { error = it }
        onDispose { files.detach(owner) }
    }
    BackHandler(project != null && !busy) { closeProject() }

    MenuInputScope(menuInputSource) {
    PreviewImageAction(timeline)
    TimelineService(timeline, files)
    VideoRecordingAction(timeline) { error = it }
    if (project != null) event.optJSONObject("timeline")?.optJSONObject("context")?.let { menu ->
        TimelineContextMenu(menu, !busy, timeline.menuOrigin) { operation, arguments ->
            command(operation, arrayOf("project_path" to project.getString("path"), "serial" to menu.getLong("serial"), *arguments))
        }
    }
    val workspace: @Composable (Boolean) -> Unit = { renderOnly -> workspaceState.SaveableStateProvider(workspaceKey) {
    Box(Modifier.fillMaxSize().then(if (project == null) Modifier.safeDrawingPadding() else Modifier).then(if (renderOnly) Modifier.clearAndSetSemantics {}.focusProperties { canFocus = false } else Modifier)) {
        if (project == null) {
            Launcher(projects, projectSources, version, !renderOnly && !busy && schema.has("defaults"),
                { useSettings(schema.getJSONObject("defaults")); projectServer = ""; create = true },
                { choicesBottomSheet = NativeLayout.menuUsesBottomSheet(menuInputSource.value); openFolder.launch(null) },
                { preferences = true },
                { entry -> action {
                    timeline.request("delete_project", "project" to entry)
                    catalogRevision++
                } },
                { entry ->
                    if (!entry.isNull("path")) open(entry.getString("path"))
                    else action {
                        openRequest(JSONObject().put("op", "open_remote")
                            .put("directory", files.projectDirectory().absolutePath).put("project", entry))
                    }
                })
        } else {
            Editor(timeline, files, project, playback, status, !renderOnly && !busy,
                onError = { error = it },
                syncError = event.optString("sync_error").takeUnless { event.isNull("sync_error") },
                selectFile = { control ->
                    inspectorFileTarget = JSONObject().put("project_path", project.getString("path"))
                        .put("command", control.getJSONObject("command")).toString()
                    inspectorFile.launch(arrayOf(control.getString("mime")))
                },
                command = ::command,
                activateProjectMenu = { callback -> action {
                    val path = project.getString("path")
                    val result = timeline.request("project_menu", "action" to callback, "project_path" to path)
                    when (val effect = result.optString("effect")) {
                        "" -> Unit
                        "flush" -> files.flush(path)
                        "pick_media" -> { importTarget = defaultTarget(); importFile.launch(arrayOf("*/*")) }
                        "show_preferences" -> preferences = true
                        "close_workspace" -> closeProject()
                        else -> error("Unknown Android project menu effect: $effect")
                    }
                } },
                export = { exportFolder.launch(null) },
                exportVideo = { exportSettings = "video" },
                exportCaptions = { exportSettings = "captions" },
                viewChange = { changes, finished -> action(blocking = false) {
                    try {
                        val saved = timeline.request("editor_layout", "changes" to changes)
                        val fields = changes.keys().asSequence().toList()
                        snapshotFlow { timeline.snapshot.value.optJSONObject("editor_layout") }.first { layout ->
                            layout != null && fields.all { layout.opt(it) == saved.opt(it) }
                        }
                    }
                    finally { finished() }
                } }, pendingOperation = if (busy) progress else null, renderOnly = renderOnly, windowCaption = caption)
        }
        if (busy && project == null) {
            Surface(Modifier.align(Alignment.BottomCenter).fillMaxWidth(), color = MaterialTheme.colorScheme.surfaceContainerHigh, tonalElevation = 4.dp) {
                Column(Modifier.padding(16.dp)) {
                    LinearProgressIndicator(Modifier.fillMaxWidth())
                    if (progress.isNotEmpty()) Text(progress, Modifier.padding(top = 8.dp), maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
        }
    }
    } }
    if (preferences) PreferencesScreen(timeline, files, {
        preferences = false
        catalogRevision++
    }) { workspace(true) } else workspace(false)
    versionOpen?.let { request ->
        val dismiss = { versionOpen = null; files.cancelOpen() }
        AlertDialog(onDismissRequest = dismiss, title = { Text(tr("Service Version Mismatch")) },
            text = { Text(versionMessage) },
            dismissButton = { TextButton(onClick = dismiss) { Text(tr("Close")) } },
            confirmButton = { TextButton(onClick = {
                versionOpen = null
                action { openRequest(JSONObject(request)) }
            }) { Text(tr("Force Open")) } })
    }
    if (create && schema.has("frame_rates")) AlertDialog(
        modifier = Modifier.imePadding(),
        onDismissRequest = { if (!busy) { create = false } },
        title = { Text(tr("Create Project")) },
        text = { Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            if (servers.isNotEmpty()) {
                ChoiceInput(tr("Location"), projectServer.ifEmpty { tr("On this device") },
                    listOf(tr("On this device")) + servers, !busy) { index ->
                    projectServer = servers.getOrNull(index - 1).orEmpty()
                }
            }
            OutlinedTextField(name, { name = it }, modifier = Modifier.fillMaxWidth(), label = { Text(tr("Project Name")) }, singleLine = true, enabled = !busy)
            OutlinedTextField(tags, { tags = it }, modifier = Modifier.fillMaxWidth(), label = { Text(tr("Tags")) }, enabled = !busy)
            val presets = schema.getJSONArray("presets").objects()
            val preset = presets.firstOrNull {
                it.getString("canvas_width") == canvasWidth && it.getString("canvas_height") == canvasHeight &&
                    it.getString("fps") == fps
            }
            ChoiceInput(tr("Preset"), preset?.getString("label") ?: tr("Custom"),
                presets.map { it.getString("label") }, !busy) { index ->
                val selected = presets[index]
                canvasWidth = selected.getString("canvas_width")
                canvasHeight = selected.getString("canvas_height")
                fps = selected.getString("fps")
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(canvasWidth, { canvasWidth = it }, label = { Text(tr("Width")) }, singleLine = true, modifier = Modifier.weight(1f), enabled = !busy)
                OutlinedTextField(canvasHeight, { canvasHeight = it }, label = { Text(tr("Height")) }, singleLine = true, modifier = Modifier.weight(1f), enabled = !busy)
            }
            val rates = schema.getJSONArray("frame_rates").objects()
            ChoiceInput(tr("FPS"), rates.first { it.getString("value") == fps }.getString("label"),
                rates.map { it.getString("label") }, !busy) { fps = rates[it].getString("value") }
        } },
        confirmButton = { TextButton(enabled = !busy && name.isNotBlank(), onClick = {
            action {
                timeline.request("create", "directory" to files.projectDirectory().absolutePath,
                    "name" to name, "canvas_width" to canvasWidth, "canvas_height" to canvasHeight,
                    "fps" to fps, "tags" to tags, "server" to projectServer)
                create = false
            }
        }) { Text(tr("Create")) } },
        dismissButton = { TextButton(onClick = { create = false }) { Text(tr("Cancel")) } })
    if (choices.isNotEmpty()) AdaptiveMenu(expanded = true, bottomSheet = checkNotNull(choicesBottomSheet), dismiss = { choices = emptyList(); files.cancelOpen() }) {
        ChoiceMenuItems(choices.map { it.nameWithoutExtension }) { index -> val file = choices[index]; choices = emptyList(); open(file.absolutePath) }
    }
    if (comment) AlertDialog(
        modifier = Modifier.imePadding(),
        onDismissRequest = { comment = false; commentTarget = null }, title = { Text(tr("Add Comment")) },
        text = { OutlinedTextField(commentText, { commentText = it }, label = { Text(tr("Comment")) }) },
        confirmButton = { TextButton(enabled = commentText.isNotBlank(), onClick = {
            comment = false
            val target = commentTarget
            commentTarget = null
            action { timeline.request("track_add", "action" to "Comment", "text" to commentText, *targetArguments(checkNotNull(target))) }
        }) { Text(tr("Add")) } }, dismissButton = { TextButton(onClick = { comment = false; commentTarget = null }) { Text(tr("Cancel")) } })
    trackMenuTarget?.let { savedTarget ->
        val destination = JSONObject(savedTarget)
        val entries = schema.optJSONObject("track_add")?.optJSONArray(destination.optString("kind"))
        AdaptiveMenu(expanded = true, bottomSheet = destination.getBoolean("bottom_sheet"), dismiss = { trackMenuTarget = null },
            offset = timeline.menuOrigin + DpOffset(destination.getDouble("x").dp, destination.getDouble("y").dp), atPoint = true) {
            for (index in 0 until (entries?.length() ?: 0)) {
                val entry = entries!!.getJSONObject(index)
                if (entry.optBoolean("separator")) HorizontalDivider()
                else DropdownMenuItem(text = { Text(tr(entry.getString("label"))) }, enabled = !busy, onClick = {
                    trackMenuTarget = null
                    when {
                        entry.optBoolean("needs_file") -> { importTarget = savedTarget; importFile.launch(arrayOf("*/*")) }
                        entry.optBoolean("needs_text") -> { commentTarget = savedTarget; commentText = ""; comment = true }
                        else -> action { timeline.request("track_add", "action" to entry.getString("action"), *targetArguments(savedTarget)) }
                    }
                })
            }
        }
    }
    error?.let { message -> AlertDialog(onDismissRequest = { error = null }, title = { Text(tr("Operation failed")) },
        text = { Text(message) }, confirmButton = { TextButton(onClick = { error = null }) { Text(tr("Close")) } }) }
    if (project != null) exportSettings?.let { kind ->
        val projectPath = project.getString("path")
        val submit: (Array<out Pair<String, Any>>) -> Unit = { values -> pickExport(kind, projectPath, values) }
        val enabled = !busy && pendingExport == null
        when (kind) {
            "video" -> VideoExportSettings(timeline, projectPath, enabled, { exportSettings = null }, submit)
            "captions" -> CaptionExportSettings(timeline, projectPath, enabled, { exportSettings = null }, submit)
            else -> error("Unknown export settings: $kind")
        }
    }
    event.optJSONObject("export")?.let { export ->
        ExportProgress(export, !busy, { destination -> action { showExportInFiles(context, destination) } }) { operation ->
            action { timeline.request(operation, "id" to export.getString("id")) }
        }
    }
    }
}

private fun targetArguments(saved: String): Array<Pair<String, Any>> {
    val target = JSONObject(saved)
    return listOf("project_path", "kind", "track", "track_id", "position", "import_targets").filter { target.has(it) && !target.isNull(it) }
        .map { it to target.get(it) }.toTypedArray()
}

@Composable
private fun Launcher(projects: List<JSONObject>, servers: List<JSONObject>?, version: String, enabled: Boolean,
    create: () -> Unit, open: () -> Unit, settings: () -> Unit, delete: (JSONObject) -> Unit, select: (JSONObject) -> Unit) {
    val resources = LocalResources.current
    val configuration = LocalConfiguration.current
    val theme = LocalContext.current.theme
    val logoSize = 56.dp
    val logoPixels = with(LocalDensity.current) { logoSize.roundToPx() }
    val logo = remember(resources, configuration, theme, logoPixels) {
        resources.getDrawable(R.mipmap.ic_launcher, theme).toBitmap(logoPixels, logoPixels).asImageBitmap()
    }
    var search by rememberSaveable { mutableStateOf("") }
    var showMenu by rememberSaveable { mutableStateOf(false) }
    var pendingDelete by remember { mutableStateOf<JSONObject?>(null) }
    val settingsMenu: @Composable () -> Unit = {
        Box {
            ActionIcon("open-menu-symbolic", "Main Menu", enabled) { showMenu = true }
            DropdownMenu(expanded = showMenu, onDismissRequest = { showMenu = false }) {
                DropdownMenuItem(text = { Text(tr("Settings")) }, onClick = { showMenu = false; settings() })
            }
        }
    }
    val sources = listOf("" to tr("All"), "local" to tr("Local")) +
        servers.orEmpty().map { it.getString("url") to it.getString("name") }
    var source by rememberSaveable { mutableStateOf("") }
    LaunchedEffect(servers) {
        if (servers != null && sources.none { it.first == source }) source = ""
    }
    val appIdentity: @Composable () -> Unit = {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Image(logo, contentDescription = null, modifier = Modifier.size(logoSize))
            Column {
                Text(tr("Shrimply"), style = MaterialTheme.typography.titleMedium)
                Text(version, style = MaterialTheme.typography.labelMedium)
            }
        }
    }
    val projectsPane: @Composable ColumnScope.() -> Unit = {
        Text(tr("Projects"), Modifier.padding(horizontal = 12.dp), style = MaterialTheme.typography.titleMedium)
        Row(Modifier.padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            OutlinedTextField(search, { search = it }, label = { Text(tr("Search Projects")) },
                singleLine = true, modifier = Modifier.weight(1f))
            ActionIcon("list-add-symbolic", "Create Project", enabled, action = create)
            ActionIcon("document-open-symbolic", "Open Project", enabled, action = open)
        }
        val query = search.trim()
        val visible = remember(projects, query, source) { projects.filter { project ->
            val server = project.optJSONObject("remote")?.getString("server")
            (source.isEmpty() || source == (server ?: "local")) &&
                sequenceOf(project.getString("name"), project.optString("path"), server.orEmpty(),
                    project.getJSONArray("tags").toString()).any { it.contains(query, ignoreCase = true) }
        } }
        if (visible.isEmpty()) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                Text(tr(if (query.isEmpty()) "No Projects" else "No Matching Projects"))
            }
        } else LazyColumn(Modifier.weight(1f), contentPadding = PaddingValues(start = 12.dp, end = 12.dp, bottom = 12.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp)) {
            items(visible) { project ->
                val server = project.optJSONObject("remote")?.getString("server")
                val access = project.optLong("opened_at").takeIf { it > 0 }?.let {
                    val date = java.text.DateFormat.getDateTimeInstance().format(java.util.Date(it * 1000))
                    tr("Last opened %{date}").replace("%{date}", date)
                }
                val details = listOfNotNull(access, server).joinToString(" · ")
                var rowMenu by remember(project) { mutableStateOf(false) }
                Card(shape = MaterialTheme.shapes.medium) {
                    ListItem(headlineContent = { Text(project.getString("name")) },
                        leadingContent = {
                            Icon(iconVector(if (server == null) "folder-open-symbolic" else "folder-remote-symbolic"),
                                null, Modifier.size(16.dp))
                        },
                        trailingContent = {
                            Box {
                                ActionIcon("view-more-symbolic", "Project options", enabled) { rowMenu = true }
                                DropdownMenu(expanded = rowMenu, onDismissRequest = { rowMenu = false }) {
                                    DropdownMenuItem(text = { Text(tr("Delete Project…")) },
                                        onClick = { rowMenu = false; pendingDelete = project })
                                }
                            }
                        },
                        colors = ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.surfaceContainerHigh),
                        supportingContent = details.takeIf { it.isNotEmpty() }?.let { value ->
                            { Text(value, maxLines = 2, overflow = TextOverflow.Ellipsis) }
                        },
                        modifier = Modifier.clickable(enabled = enabled) { select(project) })
                }
            }
        }
    }
    BoxWithConstraints(Modifier.fillMaxSize()) {
        if (maxWidth >= ExpandedWidth) Row(Modifier.fillMaxSize()) {
            Column(Modifier.width(212.dp).fillMaxHeight().padding(12.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp)) {
                appIdentity()
                LazyColumn(Modifier.weight(1f)) {
                    items(sources, key = { it.first }) { (key, label) ->
                        NavigationDrawerItem(label = { Text(label, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                            selected = source == key, onClick = { source = key })
                    }
                }
                settingsMenu()
            }
            Column(Modifier.weight(1f).fillMaxHeight().background(MaterialTheme.colorScheme.surfaceContainerLow),
                verticalArrangement = Arrangement.spacedBy(8.dp), content = projectsPane)
        } else Column(Modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(Modifier.padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Box(Modifier.weight(1f)) { appIdentity() }
                settingsMenu()
            }
            LazyRow(contentPadding = PaddingValues(horizontal = 12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                items(sources, key = { it.first }) { (key, label) ->
                    FilterChip(selected = source == key, onClick = { source = key }, label = { Text(label) })
                }
            }
            projectsPane()
        }
    }
    pendingDelete?.let { project ->
        val cancel = { pendingDelete = null }
        val message = if (project.optJSONObject("remote") == null) "Permanently delete this project file?"
                      else "Delete this project from the server for everyone?"
        AlertDialog(onDismissRequest = cancel, title = { Text(project.getString("name")) },
            text = { Text(tr(message)) }, dismissButton = { TextButton(onClick = cancel) { Text(tr("Cancel")) } },
            confirmButton = { TextButton(onClick = { cancel(); delete(project) }) { Text(tr("Delete Project")) } })
    }
}

@Composable
private fun ActionIcon(icon: String, description: String, enabled: Boolean = true,
    iconSize: Dp = 16.dp, targetSize: Dp = 48.dp, action: () -> Unit) {
    IconButton(onClick = action, enabled = enabled, modifier = Modifier.size(targetSize)) { Icon(iconVector(icon), tr(description), Modifier.size(iconSize)) }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Editor(timeline: TimelineSession, files: ProjectFiles, project: JSONObject, playback: JSONObject, status: JSONObject, enabled: Boolean,
    onError: (String) -> Unit, syncError: String?, selectFile: (JSONObject) -> Unit, command: (String, Array<out Pair<String, Any>>) -> Unit,
    activateProjectMenu: (String) -> Unit, export: () -> Unit, exportVideo: () -> Unit, exportCaptions: () -> Unit, viewChange: (JSONObject, () -> Unit) -> Unit,
    pendingOperation: String? = null, renderOnly: Boolean = false, windowCaption: WindowCaption = WindowCaption()) {
    var fullscreen by rememberSaveable(project.getString("path")) { mutableStateOf(false) }
    val inspectorState = rememberSaveableStateHolder()
    val paneDividerSize = remember { NativeLayout.panelDividerWidth().dp }
    val window = checkNotNull(LocalActivity.current).window
    BackHandler(fullscreen && !renderOnly) { fullscreen = false }
    DisposableEffect(window, fullscreen, renderOnly) {
        val immersive = fullscreen && !renderOnly
        val controller = checkNotNull(window.insetsController)
        val bars = WindowInsets.Type.systemBars()
        val behavior = controller.systemBarsBehavior
        if (immersive) {
            controller.systemBarsBehavior = WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            controller.hide(bars)
        }
        onDispose {
            if (immersive) {
                controller.systemBarsBehavior = behavior
                controller.show(bars)
            }
        }
    }
    val layout = timeline.snapshot.value.getJSONObject("editor_layout")
    var timelineDraft by remember { mutableStateOf<Float?>(null) }
    var inspectorDraft by remember { mutableStateOf<Float?>(null) }
    val timelineFraction = timelineDraft ?: layout.getDouble("timeline_fraction").toFloat()
    val inspectorWidth = inspectorDraft ?: layout.getDouble("inspector_width").toFloat()
    val currentTimelineFraction by rememberUpdatedState(timelineFraction)
    val currentInspectorWidth by rememberUpdatedState(inspectorWidth)
    val inspectorDragInteraction = remember { MutableInteractionSource() }
    val timelineDragInteraction = remember { MutableInteractionSource() }
    val inspectorOpen = layout.getBoolean("inspector_visible")
    val timelineOpen = layout.getBoolean("timeline_visible")
    val inspectSerial = timeline.snapshot.value.optJSONObject("timeline")?.optLong("inspect_serial") ?: 0L
    var inspectOwner by rememberSaveable { mutableStateOf(timeline.identity) }
    var consumedInspectSerial by rememberSaveable { mutableLongStateOf(inspectSerial) }
    LaunchedEffect(inspectSerial, renderOnly, timeline.identity) {
        if (!renderOnly && inspectOwner != timeline.identity) {
            inspectOwner = timeline.identity
            consumedInspectSerial = inspectSerial
        } else if (!renderOnly && inspectSerial > consumedInspectSerial) {
            consumedInspectSerial = inspectSerial
            if (!inspectorOpen) viewChange(JSONObject().put("inspector_visible", true)) {}
        }
    }
    LaunchedEffect(inspectorOpen, project.getString("path"), renderOnly) {
        if (!renderOnly) command("inspector_visible", arrayOf("visible" to inspectorOpen, "project_path" to project.getString("path")))
    }
    val density = LocalDensity.current.density
    val divider = MaterialTheme.colorScheme.outlineVariant
    val caption = if (!fullscreen) windowCaption else WindowCaption()
    val systemInsets = ComposeInsets.systemBars.union(ComposeInsets.displayCutout)
    val safeInsets = if (caption.height > 0) systemInsets.exclude(ComposeInsets.captionBar)
        .union(ComposeInsets.statusBars).union(ComposeInsets.displayCutout) else systemInsets
    BoxWithConstraints(Modifier.fillMaxSize().windowInsetsPadding(safeInsets).background(MaterialTheme.colorScheme.background)
        .fileDropTarget(timeline, files, project.getString("path").takeUnless { renderOnly }, false, onError)) {
        val availableWidth = maxWidth.value
        val minInspectorWidth = remember { NativeLayout.inspectorMinWidth() }
        val maxInspectorWidth = NativeLayout.inspectorMaxWidth(availableWidth)
        val expandedInspectorWidth = inspectorWidth.coerceIn(minInspectorWidth.coerceAtMost(maxInspectorWidth), maxInspectorWidth).dp
        val sideBySide = !NativeLayout.inspectorOverlay(availableWidth)
        val compact = NativeLayout.compactControls(maxWidth.value, maxHeight.value)
        val toolSize = if (compact) 40.dp else 48.dp
        val toolIconSize = if (compact) 14.dp else 16.dp
        Column(Modifier.fillMaxSize()) {
            if (!fullscreen) WindowCaptionHeader(caption, interactive = !renderOnly,
                left = { modifier, targetSize -> Row(modifier) {
                    IconToggleButton(checked = inspectorOpen, onCheckedChange = { viewChange(JSONObject().put("inspector_visible", it)) {} }, enabled = enabled, modifier = Modifier.size(targetSize)) {
                        Icon(iconVector("dock-left-symbolic"), tr("Toggle Inspector"), Modifier.size(16.dp))
                    }
                    IconToggleButton(checked = timelineOpen, onCheckedChange = { viewChange(JSONObject().put("timeline_visible", it)) {} }, enabled = enabled, modifier = Modifier.size(targetSize)) {
                        Icon(iconVector("dock-bottom-symbolic"), tr("Toggle Timeline"), Modifier.size(16.dp))
                    }
                } },
                right = { modifier, targetSize -> Row(modifier) {
                    AdaptiveMenuButton(enabled, Modifier.size(targetSize), icon = { Icon(iconVector("share-symbolic"), tr("Export"), Modifier.size(16.dp)) }) { dismiss ->
                        DropdownMenuItem(text = { Text(tr("Export Video")) }, onClick = { dismiss(); exportVideo() })
                        DropdownMenuItem(text = { Text(tr("Export Captions")) }, onClick = { dismiss(); exportCaptions() })
                        DropdownMenuItem(text = { Text(tr("Export Project")) }, onClick = { dismiss(); export() })
                    }
                    status.optJSONObject("project_menu")?.let { menu ->
                        AdaptiveMenuButton(enabled, Modifier.size(targetSize), icon = {
                            Icon(iconVector(menu.getString("icon")), menu.getString("label"), Modifier.size(16.dp))
                        }) { dismiss ->
                            ProjectMenuItems(menu, dismiss, activateProjectMenu)
                            if (!syncError.isNullOrEmpty()) {
                                HorizontalDivider()
                                Text(syncError, Modifier.padding(12.dp), color = MaterialTheme.colorScheme.error,
                                    style = MaterialTheme.typography.bodySmall)
                            }
                        }
                    }
                } })
            if (!fullscreen) HorizontalDivider()
            BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
                val availableHeight = NativeLayout.splitPaneExtent(maxHeight.value, paneDividerSize.value, 1f)
                val minimumTimelineFraction = if (availableHeight > 0f) (TimelineMinimumHeight.value / availableHeight).coerceAtMost(MaximumTimelineFraction) else MaximumTimelineFraction
                val inspectorSize = expandedInspectorWidth
                val timelineSize = NativeLayout.splitPaneExtent(maxHeight.value, paneDividerSize.value,
                    timelineFraction.coerceIn(minimumTimelineFraction, MaximumTimelineFraction)).dp
                val dividerSize = paneDividerSize
                EditorPanes(
                    inspectorWidth = expandedInspectorWidth.value,
                    timelineFraction = timelineFraction.coerceIn(minimumTimelineFraction, MaximumTimelineFraction),
                    inspectorVisible = inspectorOpen && !fullscreen,
                    timelineVisible = timelineOpen && !fullscreen,
                    preview = {
                        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.previewBackground) {
                            BoxWithConstraints(Modifier.fillMaxSize()) {
                                val previewCompact = NativeLayout.compactControls(maxWidth.value, maxHeight.value)
                                Column(Modifier.fillMaxSize()) {
                                    Row(Modifier.weight(1f).fillMaxWidth()) {
                                        if (!fullscreen) PreviewControls(timeline, compact = previewCompact, pendingOperation = pendingOperation)
                                        Box(Modifier.weight(1f).fillMaxHeight()) {
                                            NativePreview(timeline, Modifier.fillMaxSize())
                                            PreviewContextMenu(timeline)
                                        }
                                    }
                                    Transport(playback, enabled, command, previewCompact, fullscreen) { fullscreen = !fullscreen }
                                }
                            }
                        }
                    },
                    inspector = {
                        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.surfaceContainerLow) {
                            inspectorState.SaveableStateProvider("inspector") {
                                val timelineInset = if (timelineOpen) timelineSize + dividerSize else 0.dp
                                val inspectorIme = ComposeInsets.ime.exclude(ComposeInsets(bottom = timelineInset))
                                Inspector(timeline, timeline.snapshot.value.optJSONObject("inspector"), enabled,
                                    { arguments -> command("inspector", arrayOf("project_path" to project.getString("path"), *arguments)) },
                                    Modifier.requiredWidth(expandedInspectorWidth).fillMaxHeight().windowInsetsPadding(inspectorIme), selectFile)
                            }
                        }
                    },
                    timeline = {
                        Surface(Modifier.fillMaxSize().clipToBounds(), color = MaterialTheme.colorScheme.background) {
                            Column(Modifier.fillMaxSize()) {
                                HorizontalDivider(thickness = paneDividerSize, color = divider)
                                Row(Modifier.weight(1f).fillMaxWidth()) {
                                    val toolsProgress = timeline.snapshot.value.optJSONObject("timeline")?.getDouble("tools_progress")?.toFloat() ?: 0f
                                    Box(Modifier.width(toolSize * toolsProgress).fillMaxHeight().clipToBounds()) {
                                        CompositionLocalProvider(LocalMinimumInteractiveComponentSize provides toolSize) {
                                            Column(Modifier.requiredWidth(toolSize).fillMaxHeight().verticalScroll(rememberScrollState())) {
                                                val tools = timeline.snapshot.value.optJSONObject("timeline")?.optJSONArray("tools")
                                                for (index in 0 until (tools?.length() ?: 0)) {
                                                    val tool = checkNotNull(tools).getJSONObject(index)
                                                    if (index > 0 && tool.getInt("section") != tools.getJSONObject(index - 1).getInt("section")) HorizontalDivider(Modifier.padding(vertical = 4.dp))
                                                    IconToggleButton(checked = tool.getBoolean("active"), onCheckedChange = { command("timeline_tool", arrayOf("id" to tool.getString("id"))) }, enabled = enabled, modifier = Modifier.size(toolSize)) {
                                                        Icon(iconVector(tool.getString("icon")), tr(tool.getString("label")), Modifier.size(toolIconSize))
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Box(Modifier.weight(1f).fillMaxHeight()) {
                                        NativeSurface(timeline, Modifier.fillMaxSize()
                                            .fileDropTarget(timeline, files, project.getString("path").takeUnless { renderOnly }, true, onError))
                                        CanvasButtonIndication(timeline)
                                    }
                                }
                            }
                        }
                    },
                )
                if (sideBySide && inspectorOpen && !fullscreen) {
                    Box(Modifier.offset { IntOffset(inspectorSize.roundToPx(), 0) }.width(paneDividerSize)
                        .height(maxHeight - if (timelineOpen) timelineSize + dividerSize else 0.dp).background(divider))
                    VerticalDragHandle(
                        modifier = Modifier.align(Alignment.TopStart)
                            .offset(x = inspectorSize + paneDividerSize / 2 - PaneHandleTargetSize / 2,
                                y = (maxHeight - if (timelineOpen) timelineSize + dividerSize else 0.dp) / 2 - PaneHandleTargetSize / 2)
                            .size(PaneHandleTargetSize).zIndex(2f)
                            .draggable(
                                state = rememberDraggableState { delta ->
                                    inspectorDraft = ((inspectorDraft ?: currentInspectorWidth) + delta / density)
                                        .coerceIn(minInspectorWidth, maxInspectorWidth.coerceAtLeast(minInspectorWidth))
                                },
                                orientation = Orientation.Horizontal,
                                enabled = enabled,
                                interactionSource = inspectorDragInteraction,
                                onDragStopped = {
                                    inspectorDraft?.let { preferred -> viewChange(JSONObject().put("inspector_width", preferred)) { inspectorDraft = null } }
                                },
                            ),
                        interactionSource = inspectorDragInteraction,
                    )
                }
                if (timelineOpen && !fullscreen) {
                    Box(Modifier.align(Alignment.BottomCenter)
                        .offset(y = -timelineSize - dividerSize / 2 + PaneHandleTargetSize / 2)
                        .size(PaneHandleTargetSize).zIndex(2f)
                        .draggable(
                            state = rememberDraggableState { delta ->
                                if (availableHeight > 0f) timelineDraft = ((timelineDraft ?: currentTimelineFraction)
                                    .coerceIn(minimumTimelineFraction, MaximumTimelineFraction) - delta / density / availableHeight)
                                    .coerceIn(minimumTimelineFraction, MaximumTimelineFraction)
                            },
                            orientation = Orientation.Vertical,
                            enabled = enabled,
                            interactionSource = timelineDragInteraction,
                            onDragStopped = {
                                timelineDraft?.let { preferred -> viewChange(JSONObject().put("timeline_fraction", preferred)) { timelineDraft = null } }
                            },
                        ), contentAlignment = Alignment.Center) {
                        VerticalDragHandle(Modifier.rotate(90f), interactionSource = timelineDragInteraction)
                    }
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Transport(playback: JSONObject, enabled: Boolean, command: (String, Array<out Pair<String, Any>>) -> Unit,
    compact: Boolean, fullscreen: Boolean, toggleFullscreen: () -> Unit) {
    val frames = playback.optLong("total_frames").coerceAtLeast(0)
    val position = playback.optLong("position_frame").coerceIn(0, frames)
    var seeking by remember { mutableStateOf<Float?>(null) }
    val iconSize = if (compact) 12.dp else 16.dp
    val targetSize = if (compact) 40.dp else 48.dp
    val sliderInteraction = remember { MutableInteractionSource() }
    val seek: (Float) -> Unit = { seeking = it }
    val finishSeek: () -> Unit = { seeking?.let { command("seek", arrayOf("frame" to (it * frames).toLong())) }; seeking = null }
    val value = seeking ?: if (frames == 0L) 0f else position.toFloat() / frames
    val transport: @Composable () -> Unit = {
        ActionIcon("media-seek-backward-symbolic", "Step back one frame", enabled, iconSize, targetSize) { command("step", arrayOf("frames" to -1)) }
        val playing = playback.optBoolean("playing")
        ActionIcon(if (playing) "media-playback-pause-symbolic" else "media-playback-start-symbolic", if (playing) "Pause" else "Play", enabled && frames > 0, iconSize, targetSize) { command(if (playing) "pause" else "play", emptyArray()) }
        ActionIcon("media-seek-forward-symbolic", "Step forward one frame", enabled, iconSize, targetSize) { command("step", arrayOf("frames" to 1)) }
    }
    val seekSlider: @Composable RowScope.() -> Unit = {
        if (compact) Slider(value = value, onValueChange = seek, onValueChangeFinished = finishSeek,
            enabled = enabled && frames > 0, modifier = Modifier.weight(1f).height(32.dp), interactionSource = sliderInteraction,
            thumb = { SliderDefaults.Thumb(interactionSource = sliderInteraction, enabled = enabled && frames > 0, thumbSize = DpSize(4.dp, 16.dp)) },
            track = { state -> SliderDefaults.Track(sliderState = state, enabled = enabled && frames > 0, drawStopIndicator = null) })
        else Slider(value = value, onValueChange = seek, onValueChangeFinished = finishSeek, enabled = enabled && frames > 0, modifier = Modifier.weight(1f))
    }
    val fullscreenControl: @Composable () -> Unit = {
        ActionIcon(if (fullscreen) "view-restore-symbolic" else "view-fullscreen-symbolic",
            if (fullscreen) "Exit fullscreen" else "Fullscreen preview", enabled, iconSize, targetSize, toggleFullscreen)
    }
    CompositionLocalProvider(LocalMinimumInteractiveComponentSize provides targetSize) {
        BoxWithConstraints(Modifier.fillMaxWidth().padding(horizontal = 8.dp)) {
            val stacked = NativeLayout.transportStacked(maxWidth.value)
            val time: @Composable (Modifier) -> Unit = { modifier ->
                Text(playback.optString("time_label", "00:00:00.00 / 00:00:00.00"), modifier.padding(start = 8.dp),
                    maxLines = 1, overflow = TextOverflow.Ellipsis, textAlign = TextAlign.End,
                    fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.labelMedium)
            }
            Column(Modifier.fillMaxWidth()) {
                if (stacked) {
                    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) { seekSlider(); fullscreenControl() }
                    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) { transport(); time(Modifier.weight(1f)) }
                } else Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) { transport(); seekSlider(); time(Modifier); fullscreenControl() }
            }
        }
    }
}
