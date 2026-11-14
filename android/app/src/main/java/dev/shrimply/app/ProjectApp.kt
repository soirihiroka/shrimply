package dev.shrimply.app

import android.content.Intent
import androidx.compose.animation.core.Easing
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.tween
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.IntOffset
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

private val ExpandedWidth = 840.dp
private val DividerSize = 12.dp
private val InspectorMinimumWidth = 320.dp
private val TimelineMinimumHeight = 120.dp
private const val MaximumTimelineFraction = 0.75f
private const val InspectorMaximumWidthFraction = 0.45f

/** Layout and actions follow launcher-gtk and preview-gtk, using Android native controls. */
@Composable
internal fun ProjectApp(timeline: TimelineSession, files: ProjectFiles) {
    val event by timeline.snapshot
    val status = event.optJSONObject("status") ?: JSONObject()
    val project = status.optJSONObject("project")
    val playback = event.optJSONObject("playback") ?: JSONObject()
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    var busy by remember { mutableStateOf(false) }
    val operations = remember { Mutex() }
    var progress by remember { mutableStateOf("") }
    var exportSettings by rememberSaveable { mutableStateOf<String?>(null) }
    var exportDestination by rememberSaveable { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var create by rememberSaveable { mutableStateOf(false) }
    var name by rememberSaveable { mutableStateOf("") }
    var canvasWidth by rememberSaveable { mutableStateOf("") }
    var canvasHeight by rememberSaveable { mutableStateOf("") }
    var fps by rememberSaveable { mutableStateOf("") }
    var tags by rememberSaveable { mutableStateOf("") }
    var schema by remember { mutableStateOf(JSONObject()) }
    var presetsOpen by remember { mutableStateOf(false) }
    var ratesOpen by remember { mutableStateOf(false) }

    fun useSettings(settings: JSONObject) {
        name = settings.optString("name")
        canvasWidth = settings.optString("canvas_width")
        canvasHeight = settings.optString("canvas_height")
        fps = settings.optString("fps")
        tags = settings.optJSONArray("tags")?.let { values -> (0 until values.length()).joinToString(", ") { values.getString(it) } } ?: ""
    }
    var choices by remember { mutableStateOf<List<File>>(emptyList()) }
    var comment by rememberSaveable { mutableStateOf(false) }
    var importTarget by rememberSaveable { mutableStateOf<String?>(null) }
    var commentTarget by rememberSaveable { mutableStateOf<String?>(null) }
    var trackMenuTarget by rememberSaveable { mutableStateOf<String?>(null) }
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

    Box(Modifier.fillMaxSize().safeDrawingPadding()) {
        if (project == null) {
            Launcher(status, !busy && schema.has("defaults"), { useSettings(schema.getJSONObject("defaults")); create = true }, { openFolder.launch(null) }, ::open,
                { action { timeline.request("clear_recents") } })
        } else {
            Editor(timeline, project, playback, status, schema, !busy,
                close = ::closeProject,
                command = { operation, arguments -> action {
                    val result = timeline.request(operation, *arguments)
                    if (result.has("clipboard")) context.getSystemService(android.content.ClipboardManager::class.java)
                        .setPrimaryClip(android.content.ClipData.newPlainText("", result.getString("clipboard")))
                    if (result.has("message")) android.widget.Toast.makeText(context, result.getString("message"), android.widget.Toast.LENGTH_SHORT).show()
                } },
                save = { action { timeline.request("save"); files.flush(project.getString("path")) } },
                importFile = { importTarget = defaultTarget(); importFile.launch(arrayOf("*/*")) },
                export = { exportFolder.launch(null) },
                exportVideo = { exportSettings = "video" },
                exportCaptions = { exportSettings = "captions" },
                addComment = { commentTarget = defaultTarget("comment"); comment = true })
        }
        if (busy) {
            Surface(Modifier.align(Alignment.BottomCenter).fillMaxWidth(), color = MaterialTheme.colorScheme.surfaceContainerHigh, tonalElevation = 4.dp) {
                Column(Modifier.padding(16.dp)) {
                    LinearProgressIndicator(Modifier.fillMaxWidth())
                    if (progress.isNotEmpty()) Text(progress, Modifier.padding(top = 8.dp), maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
        }
    }
    if (create) AlertDialog(
        modifier = Modifier.imePadding(),
        onDismissRequest = { if (!busy) { create = false } },
        title = { Text(tr("Create Project")) },
        text = { Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            OutlinedTextField(name, { name = it }, label = { Text(tr("Project Name")) }, singleLine = true, enabled = !busy)
            OutlinedTextField(tags, { tags = it }, label = { Text(tr("Tags")) }, enabled = !busy)
            Box {
                OutlinedButton(onClick = { presetsOpen = true }, enabled = !busy) { Text(tr("Preset")) }
                DropdownMenu(presetsOpen, { presetsOpen = false }) {
                    val presets = schema.optJSONArray("presets")
                    for (index in 0 until (presets?.length() ?: 0)) {
                        val preset = presets!!.getJSONObject(index)
                        DropdownMenuItem(text = { Text(preset.getString("label")) }, onClick = {
                            canvasWidth = preset.getString("canvas_width"); canvasHeight = preset.getString("canvas_height")
                            fps = preset.getString("fps"); presetsOpen = false
                        })
                    }
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(canvasWidth, { canvasWidth = it }, label = { Text(tr("Width")) }, singleLine = true, modifier = Modifier.weight(1f), enabled = !busy)
                OutlinedTextField(canvasHeight, { canvasHeight = it }, label = { Text(tr("Height")) }, singleLine = true, modifier = Modifier.weight(1f), enabled = !busy)
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                OutlinedTextField(fps, { fps = it }, label = { Text(tr("Frame Rate")) }, singleLine = true, modifier = Modifier.weight(1f), enabled = !busy)
                Box {
                    TextButton(onClick = { ratesOpen = true }, enabled = !busy) { Text(tr("Choose…")) }
                    DropdownMenu(ratesOpen, { ratesOpen = false }) {
                        val rates = schema.optJSONArray("frame_rates")
                        for (index in 0 until (rates?.length() ?: 0)) {
                            val rate = rates!!.getJSONObject(index)
                            DropdownMenuItem(text = { Text(rate.getString("label")) }, onClick = { fps = rate.getString("value"); ratesOpen = false })
                        }
                    }
                }
            }
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
    if (choices.isNotEmpty()) AlertDialog(
        onDismissRequest = { choices = emptyList() }, title = { Text(tr("Open Project")) },
        text = { Column(Modifier.verticalScroll(rememberScrollState())) {
            choices.forEach { file -> TextButton(onClick = { choices = emptyList(); open(file.absolutePath) }) { Text(file.nameWithoutExtension) } }
        } }, confirmButton = { TextButton(onClick = { choices = emptyList() }) { Text(tr("Cancel")) } })
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
        AlertDialog(onDismissRequest = { trackMenuTarget = null },
            text = { Column(Modifier.verticalScroll(rememberScrollState())) {
                for (index in 0 until (entries?.length() ?: 0)) {
                    val entry = entries!!.getJSONObject(index)
                    if (entry.optBoolean("separator")) HorizontalDivider()
                    else TextButton(enabled = !busy, onClick = {
                        trackMenuTarget = null
                        when {
                            entry.optBoolean("needs_file") -> { importTarget = savedTarget; importFile.launch(arrayOf("*/*")) }
                            entry.optBoolean("needs_text") -> { commentTarget = savedTarget; commentText = ""; comment = true }
                            else -> action { timeline.request("track_add", "action" to entry.getString("action"), *targetArguments(savedTarget)) }
                        }
                    }) { Text(tr(entry.getString("label"))) }
                }
            } }, confirmButton = { TextButton(onClick = { trackMenuTarget = null }) { Text(tr("Cancel")) } })
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

private fun targetArguments(saved: String): Array<Pair<String, Any>> {
    val target = JSONObject(saved)
    return listOf("project_path", "kind", "track", "track_id").filter { target.has(it) && !target.isNull(it) }
        .map { it to target.get(it) }.toTypedArray()
}

@Composable
private fun Launcher(status: JSONObject, enabled: Boolean, create: () -> Unit, open: () -> Unit, recent: (String) -> Unit, clear: () -> Unit) {
    var search by rememberSaveable { mutableStateOf("") }
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
                    TextButton(onClick = clear, enabled = enabled) { Text(tr("Clear History")) }
                }
                val recents = status.optJSONArray("recents")
                val visible = (0 until (recents?.length() ?: 0)).map { recents!!.getJSONObject(it) }
                    .filter { search.isBlank() || it.toString().contains(search, ignoreCase = true) }
                if (visible.isEmpty()) Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Text(tr("No Recent Projects"), style = MaterialTheme.typography.titleLarge) }
                else Column(Modifier.verticalScroll(rememberScrollState())) {
                    visible.forEach { project -> ListItem(
                        headlineContent = { Text(project.getString("name")) },
                        supportingContent = { Text(project.getString("path"), maxLines = 2, overflow = TextOverflow.Ellipsis) },
                        leadingContent = { Icon(painterResource(R.drawable.document_open_recent), null) },
                        modifier = Modifier.clickable(enabled = enabled) { recent(project.getString("path")) })
                    }
                }
            }
        }
    }
}

@Composable
private fun ActionIcon(resource: Int, description: String, enabled: Boolean = true, action: () -> Unit) {
    IconButton(onClick = action, enabled = enabled) { Icon(painterResource(resource), tr(description), Modifier.size(16.dp)) }
}

@Composable
private fun Editor(timeline: TimelineSession, project: JSONObject, playback: JSONObject, status: JSONObject, schema: JSONObject, enabled: Boolean,
    close: () -> Unit, command: (String, Array<out Pair<String, Any>>) -> Unit, save: () -> Unit,
    importFile: () -> Unit, export: () -> Unit, exportVideo: () -> Unit, exportCaptions: () -> Unit, addComment: () -> Unit) {
    var timelineFraction by rememberSaveable { mutableFloatStateOf(0.4f) }
    var inspectorWidth by rememberSaveable { mutableFloatStateOf(320f) }
    var inspectorOpen by rememberSaveable { mutableStateOf(true) }
    val inspectSerial = timeline.snapshot.value.optJSONObject("timeline")?.optLong("inspect_serial") ?: 0L
    LaunchedEffect(inspectSerial) { if (inspectSerial > 0L) inspectorOpen = true }
    LaunchedEffect(inspectorOpen, project.getString("path")) {
        command("inspector_visible", arrayOf("visible" to inspectorOpen, "project_path" to project.getString("path")))
    }
    var timelineOpen by rememberSaveable { mutableStateOf(true) }
    var exportOpen by remember { mutableStateOf(false) }
    var menu by remember { mutableStateOf(false) }
    var tracks by remember { mutableStateOf(false) }
    val density = LocalDensity.current.density
    val divider = MaterialTheme.colorScheme.outlineVariant
    BoxWithConstraints(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
        val sideBySide = maxWidth >= ExpandedWidth
        val availableWidth = maxWidth.value
        Column(Modifier.fillMaxSize()) {
            Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                IconToggleButton(checked = inspectorOpen, onCheckedChange = { inspectorOpen = it }, enabled = enabled) {
                    Icon(painterResource(R.drawable.dock_left), tr("Toggle Inspector"), Modifier.size(16.dp))
                }
                IconToggleButton(checked = timelineOpen, onCheckedChange = { timelineOpen = it }, enabled = enabled) {
                    Icon(painterResource(R.drawable.dock_bottom), tr("Toggle Timeline"), Modifier.size(16.dp))
                }
                Text(project.getString("name"), Modifier.weight(1f), style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Box {
                    ActionIcon(R.drawable.share, "Export", enabled) { exportOpen = true }
                    DropdownMenu(exportOpen, { exportOpen = false }) {
                        DropdownMenuItem(text = { Text(tr("Export Video")) }, onClick = { exportOpen = false; exportVideo() })
                        DropdownMenuItem(text = { Text(tr("Export Captions")) }, onClick = { exportOpen = false; exportCaptions() })
                        DropdownMenuItem(text = { Text(tr("Export Project")) }, onClick = { exportOpen = false; export() })
                    }
                }
                Box {
                    ActionIcon(R.drawable.view_more, "Project options", enabled) { menu = true }
                    DropdownMenu(menu, { menu = false }) {
                        DropdownMenuItem(text = { Text(tr("Undo")) }, enabled = status.optBoolean("can_undo"), onClick = { menu = false; command("undo", emptyArray()) })
                        DropdownMenuItem(text = { Text(tr("Redo")) }, enabled = status.optBoolean("can_redo"), onClick = { menu = false; command("redo", emptyArray()) })
                        DropdownMenuItem(text = { Text(tr("Save")) }, onClick = { menu = false; save() })
                        DropdownMenuItem(text = { Text(tr("Import")) }, onClick = { menu = false; importFile() })
                        DropdownMenuItem(text = { Text(tr("Close")) }, onClick = { menu = false; close() })
                    }
                }
            }
            HorizontalDivider()
            BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
                val availableHeight = (maxHeight.value - DividerSize.value).coerceAtLeast(0f)
                val minimumTimelineFraction = if (availableHeight > 0f) (TimelineMinimumHeight.value / availableHeight).coerceAtMost(MaximumTimelineFraction) else MaximumTimelineFraction
                val expandedInspectorWidth = inspectorWidth.coerceIn(InspectorMinimumWidth.value.coerceAtMost(availableWidth), if (sideBySide) availableWidth * InspectorMaximumWidthFraction else availableWidth).dp
                val animation = tween<androidx.compose.ui.unit.Dp>(durationMillis = NativeLayout.panelAnimationDurationMillis(), easing = Easing { NativeLayout.panelAnimationProgress(it) })
                val inspectorSize by animateDpAsState(if (inspectorOpen) expandedInspectorWidth else 0.dp, animation, label = "inspector")
                val timelineSize by animateDpAsState(if (timelineOpen) (availableHeight * timelineFraction.coerceIn(minimumTimelineFraction, MaximumTimelineFraction)).dp else 0.dp, animation, label = "timeline")
                val dividerSize by animateDpAsState(if (timelineOpen) DividerSize else 0.dp, animation, label = "timeline divider")
                Column(Modifier.fillMaxSize()) {
                    Box(Modifier.weight(1f).fillMaxWidth()) {
                        Row(Modifier.fillMaxSize()) {
                            if (sideBySide) Spacer(Modifier.width(inspectorSize + if (inspectorOpen || inspectorSize > 0.dp) DividerSize else 0.dp))
                            // Preview identity and its surface survive panel changes on every width.
                            Column(Modifier.weight(1f).fillMaxHeight()) {
                                NativePreview(timeline, Modifier.weight(1f).fillMaxWidth())
                                Transport(playback, enabled, command)
                            }
                        }
                        Surface(Modifier.width(inspectorSize).fillMaxHeight().clipToBounds().align(Alignment.TopStart), shadowElevation = if (sideBySide) 0.dp else 6.dp) {
                            if (inspectorSize > 0.dp) Inspector(timeline, timeline.snapshot.value.optJSONObject("inspector"), enabled,
                                { arguments -> command("inspector", arrayOf("project_path" to project.getString("path"), *arguments)) },
                                Modifier.requiredWidth(expandedInspectorWidth).fillMaxHeight())
                        }
                        if (sideBySide && inspectorOpen) Box(Modifier.offset { IntOffset(inspectorSize.roundToPx(), 0) }.width(DividerSize).fillMaxHeight().pointerInput(density, availableWidth) {
                            detectDragGestures { change, delta -> change.consume(); inspectorWidth = (inspectorWidth + delta.x / density).coerceIn(InspectorMinimumWidth.value, availableWidth * InspectorMaximumWidthFraction) }
                        }, contentAlignment = Alignment.Center) { Box(Modifier.width(1.dp).fillMaxHeight().background(divider)) }
                    }
                    Box(Modifier.fillMaxWidth().height(dividerSize).clipToBounds().pointerInput(density, availableHeight) {
                        detectDragGestures { change, delta -> change.consume(); if (availableHeight > 0f) timelineFraction = (timelineFraction.coerceIn(minimumTimelineFraction, MaximumTimelineFraction) - delta.y / density / availableHeight).coerceIn(minimumTimelineFraction, MaximumTimelineFraction) }
                    }, contentAlignment = Alignment.Center) { Box(Modifier.fillMaxWidth().height(1.dp).background(divider)) }
                    Row(Modifier.fillMaxWidth().height(timelineSize).clipToBounds()) {
                        Column(Modifier.width(48.dp).fillMaxHeight().verticalScroll(rememberScrollState())) {
                            val tools = timeline.snapshot.value.optJSONObject("timeline")?.optJSONArray("tools")
                            for (index in 0 until (tools?.length() ?: 0)) {
                                val tool = checkNotNull(tools).getJSONObject(index)
                                if (index > 0 && tool.getInt("section") != tools.getJSONObject(index - 1).getInt("section")) HorizontalDivider(Modifier.padding(vertical = 4.dp))
                                IconToggleButton(checked = tool.getBoolean("active"), onCheckedChange = { command("timeline_tool", arrayOf("id" to tool.getString("id"))) }, enabled = enabled) {
                                    Icon(painterResource(timelineToolIcon(tool.getString("icon"))), tr(tool.getString("label")), Modifier.size(16.dp))
                                }
                            }

                            Box {
                                ActionIcon(R.drawable.list_add, "Add Track", enabled) { tracks = true }
                                DropdownMenu(tracks, { tracks = false }) {
                                    (schema.optJSONObject("track_add")?.keys()?.asSequence()?.toList() ?: emptyList()).forEach { kind -> DropdownMenuItem(text = { Text(tr(kind.replaceFirstChar(Char::uppercase))) }, onClick = { tracks = false; command("add_track", arrayOf("kind" to kind)) }) }
                                    DropdownMenuItem(text = { Text(tr("Add Comment")) }, onClick = { tracks = false; addComment() })
                                }
                            }
                        }
                        Box(Modifier.weight(1f).fillMaxHeight()) {
                            NativeSurface(timeline, Modifier.fillMaxSize())
                            timeline.snapshot.value.optJSONObject("timeline")?.optJSONObject("context")?.let { menu ->
                                TimelineContextMenu(menu, enabled) { operation, arguments ->
                                    command(operation, arrayOf("project_path" to project.getString("path"), "serial" to menu.getLong("serial"), *arguments))
                                }
                            }
                        }
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

@Composable
private fun Transport(playback: JSONObject, enabled: Boolean, command: (String, Array<out Pair<String, Any>>) -> Unit) {
    val frames = playback.optLong("total_frames").coerceAtLeast(0)
    val position = playback.optLong("position_frame").coerceIn(0, frames)
    var seeking by remember { mutableStateOf<Float?>(null) }
    Column(Modifier.fillMaxWidth().padding(horizontal = 8.dp)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            ActionIcon(R.drawable.media_seek_backward, "Step back one frame", enabled) { command("step", arrayOf("frames" to -1)) }
            val playing = playback.optBoolean("playing")
            ActionIcon(if (playing) R.drawable.media_playback_pause else R.drawable.media_playback_start, if (playing) "Pause" else "Play", enabled && frames > 0) { command(if (playing) "pause" else "play", emptyArray()) }
            ActionIcon(R.drawable.media_seek_forward, "Step forward one frame", enabled) { command("step", arrayOf("frames" to 1)) }
            Slider(value = seeking ?: if (frames == 0L) 0f else position.toFloat() / frames, onValueChange = { seeking = it },
                onValueChangeFinished = { seeking?.let { command("seek", arrayOf("frame" to (it * frames).toLong())) }; seeking = null },
                enabled = enabled && frames > 0, modifier = Modifier.weight(1f))
        }
        Text(playback.optString("time_label", "00:00:00.00 / 00:00:00.00"), Modifier.align(Alignment.End).padding(bottom = 8.dp), fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.labelMedium)
    }
}
