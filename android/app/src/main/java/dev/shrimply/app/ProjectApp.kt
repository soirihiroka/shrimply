package dev.shrimply.app

import android.content.Intent
import androidx.activity.compose.LocalActivity
import android.view.WindowInsets
import androidx.compose.foundation.layout.WindowInsets as ComposeInsets
import android.view.WindowInsetsController
import androidx.compose.animation.core.Easing
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.text.selection.SelectionContainer
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.DpOffset
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

private val ExpandedWidth = 840.dp
private val PaneHandleTargetSize = 48.dp
private val InspectorMinimumWidth = 320.dp
private val TimelineMinimumHeight = 120.dp
private const val MaximumTimelineFraction = 0.75f
private const val InspectorMaximumWidthFraction = 0.45f

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
    var exportDestination by rememberSaveable { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var preferences by rememberSaveable { mutableStateOf(false) }
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
    var comment by rememberSaveable { mutableStateOf(false) }
    var importTarget by rememberSaveable { mutableStateOf<String?>(null) }
    var commentTarget by rememberSaveable { mutableStateOf<String?>(null) }
    var trackMenuTarget by rememberSaveable { mutableStateOf<String?>(null) }
    var inspectorFileTarget by rememberSaveable { mutableStateOf<String?>(null) }
    var pendingAudioRecord by rememberSaveable { mutableStateOf<String?>(null) }
    var lastActionToken by rememberSaveable { mutableStateOf<String?>(null) }
    var commentText by rememberSaveable { mutableStateOf("") }

    fun action(block: suspend () -> Unit) {
        scope.launch { operations.withLock {
            // Picker results must queue behind startup/settings requests, never disappear
            // because rotation temporarily marks the new composition busy.
            busy = true
            try { block() } catch (failure: Exception) { error = failure.message ?: "The project operation failed." }
            finally { busy = false; progress = "" }
        } }
    }
    fun command(operation: String, arguments: Array<out Pair<String, Any>>) = action {
        val result = timeline.request(operation, *arguments)
        if (result.has("show_in_files")) files.showInFiles(result.getString("show_in_files"))
        if (result.has("message")) android.widget.Toast.makeText(context, result.getString("message"), android.widget.Toast.LENGTH_SHORT).show()
    }
    fun open(path: String) = action { timeline.request("open", "path" to path) }
    fun closeProject() = action {
        timeline.request("save")
        project?.optString("path")?.let { files.flush(it) }
        timeline.request("close")
    }

    val openFolder = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
        if (uri != null) action {
            context.contentResolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
            val found = files.openTree(uri) { file -> scope.launch { progress = file } }
            if (found.size == 1) timeline.request("open", "path" to found.single().absolutePath) else choices = found
        }
    }
    val exportFolder = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
        if (uri != null && project != null) action {
            timeline.request("export")
            files.exportTree(project.getString("path"), uri) { file -> scope.launch { progress = file } }
        }
    }
    fun saveCompletedExport(uri: android.net.Uri?) {
        val pending = exportDestination
        exportDestination = null
        if (uri != null && pending != null) action {
            val export = JSONObject(pending)
            if (export.getBoolean("separate")) files.exportCompletedFolder(export.getString("directory"), uri, export.getString("mime")) { file -> scope.launch { progress = file } }
            else files.exportCompletedFile(export.getString("path"), uri)
            timeline.request("export_saved", "id" to export.getString("id"))
        }
    }
    val saveExport = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        saveCompletedExport(result.data?.data?.takeIf { result.resultCode == android.app.Activity.RESULT_OK })
    }
    val saveExportFolder = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri -> saveCompletedExport(uri) }
    val importFile = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        val destination = importTarget
        importTarget = null
        if (uri != null && destination != null) action {
            val target = targetArguments(destination)
            files.importFile(uri) { name, path, descriptor ->
                timeline.request("import_probe", "name" to name, *target)
                timeline.request("import_file", "path" to path, "name" to name, *target, descriptor = descriptor)
            }
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
    PreviewImageAction(timeline, files)
    TimelineService(timeline, files)
    ScreenRecordingAction(timeline) { error = it }
    if (project != null) event.optJSONObject("timeline")?.optJSONObject("context")?.let { menu ->
        TimelineContextMenu(menu, !busy, timeline.menuOrigin) { operation, arguments ->
            command(operation, arrayOf("project_path" to project.getString("path"), "serial" to menu.getLong("serial"), *arguments))
        }
    }
    val workspace: @Composable (Boolean) -> Unit = { renderOnly -> workspaceState.SaveableStateProvider(workspaceKey) {
    Box(Modifier.fillMaxSize().then(if (project == null) Modifier.safeDrawingPadding() else Modifier).then(if (renderOnly) Modifier.clearAndSetSemantics {}.focusProperties { canFocus = false } else Modifier)) {
        if (project == null) {
            Launcher(status, !renderOnly && !busy && schema.has("defaults"), { useSettings(schema.getJSONObject("defaults")); create = true }, { choicesBottomSheet = NativeLayout.menuUsesBottomSheet(menuInputSource.value); openFolder.launch(null) }, ::open,
                { action { timeline.request("clear_recents") } },
                command = { command -> action { timeline.request(command) } },
                showInFiles = { path -> action { files.showInFiles(path) } })
        } else {
            Editor(timeline, project, playback, status, !renderOnly && !busy,
                selectFile = { control ->
                    inspectorFileTarget = JSONObject().put("project_path", project.getString("path"))
                        .put("command", control.getJSONObject("command")).toString()
                    inspectorFile.launch(arrayOf(control.getString("mime")))
                },
                preferences = { preferences = true },
                close = ::closeProject,
                command = ::command,
                save = { action { timeline.request("save"); files.flush(project.getString("path")) } },
                importFile = { importTarget = defaultTarget(); importFile.launch(arrayOf("*/*")) },
                export = { exportFolder.launch(null) },
                exportVideo = { exportSettings = "video" },
                exportCaptions = { exportSettings = "captions" },
                viewChange = { changes, finished -> action {
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
    if (preferences) PreferencesScreen(timeline, files, { preferences = false }) { workspace(true) } else workspace(false)
    if (create) AlertDialog(
        modifier = Modifier.imePadding(),
        onDismissRequest = { if (!busy) { create = false } },
        title = { Text(tr("Create Project")) },
        text = { Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            OutlinedTextField(name, { name = it }, label = { Text(tr("Project Name")) }, singleLine = true, enabled = !busy)
            OutlinedTextField(tags, { tags = it }, label = { Text(tr("Tags")) }, enabled = !busy)
            AdaptiveMenuAnchor(anchor = { open ->
                OutlinedButton(onClick = open, enabled = !busy) { Text(tr("Preset")) }
            }) { dismiss ->
                val presets = checkNotNull(schema.optJSONArray("presets"))
                ChoiceMenuItems((0 until presets.length()).map { presets.getJSONObject(it).getString("label") }) { index ->
                    val preset = presets.getJSONObject(index)
                    canvasWidth = preset.getString("canvas_width"); canvasHeight = preset.getString("canvas_height")
                    fps = preset.getString("fps"); dismiss()
                }
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
            create = false
            action {
                val width = canvasWidth.toIntOrNull() ?: throw IllegalArgumentException("Width must be a whole number.")
                val height = canvasHeight.toIntOrNull() ?: throw IllegalArgumentException("Height must be a whole number.")
                val values = arrayOf<Pair<String, Any>>("name" to name.trim(), "canvas_width" to width, "canvas_height" to height,
                    "fps" to fps.trim(), "tags" to JSONArray(tags.split(',').map(String::trim).filter(String::isNotEmpty)))
                timeline.request("create", "path" to files.newPath(name).absolutePath, *values)
            }
        }) { Text(tr("Create")) } },
        dismissButton = { TextButton(onClick = { create = false }) { Text(tr("Cancel")) } })
    if (choices.isNotEmpty()) AdaptiveMenu(expanded = true, bottomSheet = checkNotNull(choicesBottomSheet), dismiss = { choices = emptyList() }) {
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
        val submit: (Array<out Pair<String, Any>>) -> Unit = { values ->
            exportSettings = null
            action { timeline.request(if (kind == "captions") "export_captions" else "export_video", "project_path" to project.getString("path"), *values) }
        }
        when (kind) {
            "video" -> VideoExportSettings(timeline, project.getString("path"), { exportSettings = null }, submit)
            "captions" -> CaptionExportSettings(timeline, project.getString("path"), { exportSettings = null }, submit)
            else -> error("Unknown export settings: $kind")
        }
    }
    event.optJSONObject("export")?.let { export ->
        ExportProgress(export, !busy,
            command = { operation -> action { timeline.request(operation, "id" to export.getString("id")) } },
            save = {
                exportDestination = export.toString()
                if (export.getBoolean("separate")) saveExportFolder.launch(null)
                else saveExport.launch(Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE)
                    .setType(export.getString("mime")).putExtra(Intent.EXTRA_TITLE, export.getString("filename")))
            })
    }
    }
}

private fun targetArguments(saved: String): Array<Pair<String, Any>> {
    val target = JSONObject(saved)
    return listOf("project_path", "kind", "track", "track_id", "position", "import_targets").filter { target.has(it) && !target.isNull(it) }
        .map { it to target.get(it) }.toTypedArray()
}

@Composable
private fun Launcher(status: JSONObject, enabled: Boolean, create: () -> Unit, open: () -> Unit, recent: (String) -> Unit, clear: () -> Unit,
    command: (JSONObject) -> Unit, showInFiles: (String) -> Unit) {
    var search by rememberSaveable { mutableStateOf("") }
    var info by remember { mutableStateOf<JSONObject?>(null) }
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val expanded = maxWidth >= ExpandedWidth
        Row(Modifier.fillMaxSize()) {
            if (expanded) Column(Modifier.width(200.dp).fillMaxHeight().background(MaterialTheme.colorScheme.surfaceContainerLow)) {
                Button(onClick = create, enabled = enabled, modifier = Modifier.padding(12.dp).fillMaxWidth()) { Text(tr("Create Project")) }
                OutlinedButton(onClick = open, enabled = enabled, modifier = Modifier.padding(horizontal = 12.dp).fillMaxWidth()) { Text(tr("Open Project")) }
            }
            Column(Modifier.weight(1f).fillMaxHeight()) {
                if (expanded) {
                    Text(tr("History"), Modifier.padding(20.dp), style = MaterialTheme.typography.titleMedium)
                    HorizontalDivider()
                }
                if (!expanded) Row(Modifier.padding(12.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Button(onClick = create, enabled = enabled, modifier = Modifier.weight(1f)) { Text(tr("Create Project")) }
                    OutlinedButton(onClick = open, enabled = enabled, modifier = Modifier.weight(1f)) { Text(tr("Open Project")) }
                }
                Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    OutlinedTextField(search, { search = it }, label = { Text(tr("Search history")) }, singleLine = true, modifier = Modifier.weight(1f))
                    IconButton(onClick = clear, enabled = enabled) {
                        Icon(painterResource(R.drawable.user_trash), tr("Clear History"), Modifier.size(16.dp))
                    }
                }
                val recents = status.optJSONArray("recents")
                val visible = (0 until (recents?.length() ?: 0)).map { recents!!.getJSONObject(it) }
                    .filter { project -> search.isBlank() || sequenceOf(project.getString("name"), project.getString("path"),
                        project.getJSONArray("tags").toString()).any { it.contains(search, ignoreCase = true) } }
                if (visible.isEmpty()) Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Text(tr("No Recent Projects"), style = MaterialTheme.typography.titleLarge) }
                else Column(Modifier.verticalScroll(rememberScrollState())) {
                    visible.forEach { project -> key(project.getString("path")) { ListItem(
                        headlineContent = { Text(project.getString("name")) },
                        supportingContent = { Text(project.getString("last_edited"), maxLines = 1, overflow = TextOverflow.Ellipsis) },
                        trailingContent = {
                            AdaptiveMenuButton(enabled, icon = { Icon(painterResource(R.drawable.view_more), project.getString("menu_label"), Modifier.size(16.dp)) }) { dismiss ->
                                project.getJSONArray("menu").objects().forEach { item ->
                                    DropdownMenuItem(text = { Text(item.getString("label")) },
                                        leadingIcon = { Icon(painterResource(inspectorIcon(item.getString("icon"))), null, Modifier.size(16.dp)) }, onClick = {
                                        dismiss()
                                        when (item.getString("service")) {
                                            "info" -> info = item.getJSONObject("info")
                                            "show_in_files" -> showInFiles(item.getString("path"))
                                            "remove_recent" -> command(item.getJSONObject("command"))
                                            else -> error("Unknown recent project service")
                                        }
                                    })
                                }
                            }
                        }, modifier = Modifier.clickable(enabled = enabled) { recent(project.getString("path")) }) }
                    }
                }
            }
        }
    }
    info?.let { details -> AlertDialog(onDismissRequest = { info = null }, title = { Text(details.getString("title")) },
        text = { SelectionContainer { Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            details.getJSONArray("fields").objects().forEach { field -> Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(field.getString("label"), style = MaterialTheme.typography.labelMedium)
                    Text(field.getString("value"))
                }
                field.optJSONObject("action")?.let { action -> IconButton(onClick = { showInFiles(action.getString("path")) }) {
                    Icon(painterResource(inspectorIcon(action.getString("icon"))), action.getString("label"), Modifier.size(16.dp))
                } }
            } }
        } } }, confirmButton = { TextButton(onClick = { info = null }) { Text(details.getString("close_label")) } }) }
}

@Composable
private fun ActionIcon(resource: Int, description: String, enabled: Boolean = true,
    iconSize: Dp = 16.dp, targetSize: Dp = 48.dp, action: () -> Unit) {
    IconButton(onClick = action, enabled = enabled, modifier = Modifier.size(targetSize)) { Icon(painterResource(resource), tr(description), Modifier.size(iconSize)) }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Editor(timeline: TimelineSession, project: JSONObject, playback: JSONObject, status: JSONObject, enabled: Boolean,
    selectFile: (JSONObject) -> Unit, preferences: () -> Unit, close: () -> Unit, command: (String, Array<out Pair<String, Any>>) -> Unit, save: () -> Unit,
    importFile: () -> Unit, export: () -> Unit, exportVideo: () -> Unit, exportCaptions: () -> Unit, viewChange: (JSONObject, () -> Unit) -> Unit,
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
    val safeInsets = if (caption.height > 0) ComposeInsets.safeDrawing.exclude(ComposeInsets.captionBar)
        .union(ComposeInsets.statusBars).union(ComposeInsets.displayCutout) else ComposeInsets.safeDrawing
    BoxWithConstraints(Modifier.fillMaxSize().windowInsetsPadding(safeInsets).background(MaterialTheme.colorScheme.background)) {
        val availableWidth = maxWidth.value
        val expandedInspectorWidth = inspectorWidth.coerceIn(InspectorMinimumWidth.value.coerceAtMost(availableWidth), availableWidth).dp
        val sideBySide = !NativeLayout.inspectorOverlay(availableWidth, expandedInspectorWidth.value)
        val compact = NativeLayout.compactControls(maxWidth.value, maxHeight.value)
        val toolSize = if (compact) 40.dp else 48.dp
        val toolIconSize = if (compact) 14.dp else 16.dp
        Column(Modifier.fillMaxSize()) {
            if (!fullscreen) WindowCaptionHeader(caption, interactive = !renderOnly,
                left = { modifier, targetSize -> Row(modifier) {
                    IconToggleButton(checked = inspectorOpen, onCheckedChange = { viewChange(JSONObject().put("inspector_visible", it)) {} }, enabled = enabled, modifier = Modifier.size(targetSize)) {
                        Icon(painterResource(R.drawable.dock_left), tr("Toggle Inspector"), Modifier.size(16.dp))
                    }
                    IconToggleButton(checked = timelineOpen, onCheckedChange = { viewChange(JSONObject().put("timeline_visible", it)) {} }, enabled = enabled, modifier = Modifier.size(targetSize)) {
                        Icon(painterResource(R.drawable.dock_bottom), tr("Toggle Timeline"), Modifier.size(16.dp))
                    }
                } },
                right = { modifier, targetSize -> Row(modifier) {
                    AdaptiveMenuButton(enabled, Modifier.size(targetSize), icon = { Icon(painterResource(R.drawable.share), tr("Export"), Modifier.size(16.dp)) }) { dismiss ->
                        DropdownMenuItem(text = { Text(tr("Export Video")) }, onClick = { dismiss(); exportVideo() })
                        DropdownMenuItem(text = { Text(tr("Export Captions")) }, onClick = { dismiss(); exportCaptions() })
                        DropdownMenuItem(text = { Text(tr("Export Project")) }, onClick = { dismiss(); export() })
                    }
                    AdaptiveMenuButton(enabled, Modifier.size(targetSize), icon = { Icon(painterResource(R.drawable.view_more), tr("Project options"), Modifier.size(16.dp)) }) { dismiss ->
                        DropdownMenuItem(text = { Text(tr("Undo")) }, enabled = status.optBoolean("can_undo"), onClick = { dismiss(); command("undo", emptyArray()) })
                        DropdownMenuItem(text = { Text(tr("Redo")) }, enabled = status.optBoolean("can_redo"), onClick = { dismiss(); command("redo", emptyArray()) })
                        DropdownMenuItem(text = { Text(tr("Save")) }, onClick = { dismiss(); save() })
                        DropdownMenuItem(text = { Text(tr("Import")) }, onClick = { dismiss(); importFile() })
                        DropdownMenuItem(text = { Text(tr("Preferences")) }, onClick = { dismiss(); preferences() })
                        DropdownMenuItem(text = { Text(tr("Close")) }, onClick = { dismiss(); close() })
                    }
                } })
            if (!fullscreen) HorizontalDivider()
            BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
                val availableHeight = NativeLayout.splitPaneExtent(maxHeight.value, paneDividerSize.value, 1f)
                val minimumTimelineFraction = if (availableHeight > 0f) (TimelineMinimumHeight.value / availableHeight).coerceAtMost(MaximumTimelineFraction) else MaximumTimelineFraction
                val animation = tween<Float>(durationMillis = NativeLayout.panelAnimationDurationMillis(), easing = Easing { NativeLayout.panelAnimationProgress(it) })
                val inspectorProgress by animateFloatAsState(if (inspectorOpen && !fullscreen) 1f else 0f, animation, label = "inspector")
                val timelineProgress by animateFloatAsState(if (timelineOpen && !fullscreen) 1f else 0f, animation, label = "timeline")
                val inspectorSize = expandedInspectorWidth * inspectorProgress
                val timelineSize = (NativeLayout.splitPaneExtent(maxHeight.value, paneDividerSize.value,
                    timelineFraction.coerceIn(minimumTimelineFraction, MaximumTimelineFraction)) * timelineProgress).dp
                val dividerSize = paneDividerSize * timelineProgress
                Column(Modifier.fillMaxSize()) {
                    Box(Modifier.weight(1f).fillMaxWidth().clipToBounds()) {
                        Row(Modifier.fillMaxSize()) {
                            if (sideBySide) Spacer(Modifier.width(inspectorSize + if (!fullscreen && (inspectorOpen || inspectorSize > 0.dp)) paneDividerSize else 0.dp))
                            // Preview identity and its surface survive panel changes on every width.
                            Surface(Modifier.weight(1f).fillMaxHeight(), color = MaterialTheme.colorScheme.background) {
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
                        }
                        Surface(Modifier.width(inspectorSize).fillMaxHeight().align(Alignment.TopStart).zIndex(1f),
                            color = MaterialTheme.colorScheme.surfaceContainerLow, shadowElevation = if (sideBySide) 0.dp else 6.dp) {
                            Box(Modifier.fillMaxSize().clipToBounds()) {
                                if (inspectorSize > 0.dp) inspectorState.SaveableStateProvider("inspector") {
                                    Inspector(timeline, timeline.snapshot.value.optJSONObject("inspector"), enabled,
                                        { arguments -> command("inspector", arrayOf("project_path" to project.getString("path"), *arguments)) },
                                        Modifier.requiredWidth(expandedInspectorWidth).fillMaxHeight(), selectFile)
                                }
                            }
                        }
                        if (sideBySide && inspectorOpen && !fullscreen) {
                            Box(Modifier.offset { IntOffset(inspectorSize.roundToPx(), 0) }.width(paneDividerSize).fillMaxHeight().background(divider))
                            VerticalDragHandle(
                                modifier = Modifier.align(Alignment.CenterStart)
                                    .offset(x = inspectorSize + paneDividerSize / 2 - PaneHandleTargetSize / 2)
                                    .size(PaneHandleTargetSize).zIndex(2f)
                                    .draggable(
                                        state = rememberDraggableState { delta ->
                                            inspectorDraft = ((inspectorDraft ?: currentInspectorWidth) + delta / density)
                                                .coerceIn(InspectorMinimumWidth.value, (availableWidth * InspectorMaximumWidthFraction).coerceAtLeast(InspectorMinimumWidth.value))
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
                    }
                    Box(Modifier.fillMaxWidth().height(dividerSize).background(divider))
                    Surface(Modifier.fillMaxWidth().height(timelineSize).clipToBounds(), color = MaterialTheme.colorScheme.background) {
                        Row(Modifier.fillMaxSize()) {
                            val toolsProgress = timeline.snapshot.value.optJSONObject("timeline")?.getDouble("tools_progress")?.toFloat() ?: 0f
                            Box(Modifier.width(toolSize * toolsProgress).fillMaxHeight().clipToBounds()) {
                                CompositionLocalProvider(LocalMinimumInteractiveComponentSize provides toolSize) {
                                    Column(Modifier.requiredWidth(toolSize).fillMaxHeight().verticalScroll(rememberScrollState())) {
                                        val tools = timeline.snapshot.value.optJSONObject("timeline")?.optJSONArray("tools")
                                        for (index in 0 until (tools?.length() ?: 0)) {
                                            val tool = checkNotNull(tools).getJSONObject(index)
                                            if (index > 0 && tool.getInt("section") != tools.getJSONObject(index - 1).getInt("section")) HorizontalDivider(Modifier.padding(vertical = 4.dp))
                                            IconToggleButton(checked = tool.getBoolean("active"), onCheckedChange = { command("timeline_tool", arrayOf("id" to tool.getString("id"))) }, enabled = enabled, modifier = Modifier.size(toolSize)) {
                                                Icon(painterResource(timelineToolIcon(tool.getString("icon"))), tr(tool.getString("label")), Modifier.size(toolIconSize))
                                            }
                                        }
                                    }
                                }
                            }
                            Box(Modifier.weight(1f).fillMaxHeight()) {
                                NativeSurface(timeline, Modifier.fillMaxSize())
                            }
                        }
                    }
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

private fun timelineToolIcon(name: String): Int = when (name) {
    "magnet-tilted-symbolic" -> R.drawable.magnet_tilted
    "metronome-symbolic" -> R.drawable.metronome
    "pointer-left-symbolic" -> R.drawable.pointer_left
    "cut-symbolic" -> R.drawable.cut
    "track-insert-symbolic" -> R.drawable.track_insert
    "track-block-symbolic" -> R.drawable.track_block
    "track-move-above-symbolic" -> R.drawable.track_move_above
    else -> error("Unknown shared timeline icon: $name")
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
        ActionIcon(R.drawable.media_seek_backward, "Step back one frame", enabled, iconSize, targetSize) { command("step", arrayOf("frames" to -1)) }
        val playing = playback.optBoolean("playing")
        ActionIcon(if (playing) R.drawable.media_playback_pause else R.drawable.media_playback_start, if (playing) "Pause" else "Play", enabled && frames > 0, iconSize, targetSize) { command(if (playing) "pause" else "play", emptyArray()) }
        ActionIcon(R.drawable.media_seek_forward, "Step forward one frame", enabled, iconSize, targetSize) { command("step", arrayOf("frames" to 1)) }
    }
    val seekSlider: @Composable RowScope.() -> Unit = {
        if (compact) Slider(value = value, onValueChange = seek, onValueChangeFinished = finishSeek,
            enabled = enabled && frames > 0, modifier = Modifier.weight(1f).height(32.dp), interactionSource = sliderInteraction,
            thumb = { SliderDefaults.Thumb(interactionSource = sliderInteraction, enabled = enabled && frames > 0, thumbSize = DpSize(4.dp, 16.dp)) },
            track = { state -> SliderDefaults.Track(sliderState = state, enabled = enabled && frames > 0, drawStopIndicator = null) })
        else Slider(value = value, onValueChange = seek, onValueChangeFinished = finishSeek, enabled = enabled && frames > 0, modifier = Modifier.weight(1f))
    }
    val fullscreenControl: @Composable () -> Unit = {
        ActionIcon(if (fullscreen) R.drawable.view_restore else R.drawable.view_fullscreen,
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
