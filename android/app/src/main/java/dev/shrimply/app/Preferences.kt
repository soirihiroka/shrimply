package dev.shrimply.app

import android.app.Activity
import android.content.Intent
import android.graphics.Typeface
import android.provider.DocumentsContract
import androidx.activity.BackEventCompat
import androidx.activity.compose.PredictiveBackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.json.JSONObject
import java.io.File

private const val RootPage = "preferences:root"
private const val BackScale = 0.1f
private const val BackAnimationMillis = 180
private val BackTranslation = 24.dp

@Composable
internal fun PreferencesScreen(timeline: TimelineSession, files: ProjectFiles, dismiss: () -> Unit, workspace: @Composable () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var description by remember { mutableStateOf<JSONObject?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var page by rememberSaveable { mutableStateOf<String?>(null) }
    var section by rememberSaveable { mutableStateOf<String?>(null) }
    var pendingSource by rememberSaveable { mutableStateOf<String?>(null) }
    var deliveredSource by rememberSaveable { mutableStateOf<String?>(null) }
    val pages = rememberSaveableStateHolder()
    val progress = remember { Animatable(0f) }
    var edge by remember { mutableIntStateOf(BackEventCompat.EDGE_LEFT) }
    var backPage by remember { mutableStateOf<String?>(null) }
    val requests = remember { Mutex() }
    suspend fun request(command: JSONObject) = requests.withLock {
        val interactive = command.optString("action") !in listOf("font_visible", "font_search")
        if (interactive) busy = true
        try {
            val result = timeline.request("preferences", *command.keys().asSequence().map { it to command.get(it) }.toList().toTypedArray())
            description = result
            if (result.has("show_in_files")) files.showInFiles(result.getString("show_in_files"))
            if (interactive) failure = null
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (error: Exception) {
            failure = error.message
        } finally {
            if (interactive) busy = false
        }
    }
    val send: (JSONObject) -> Unit = { command -> scope.launch { request(command) } }
    val sourcePicker = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        val source = pendingSource?.let(::JSONObject)
        pendingSource = null
        if (source != null) scope.launch {
            val uri = result.data?.data?.takeIf { result.resultCode == Activity.RESULT_OK }
            if (uri == null) request(source.getJSONObject("cancel"))
            else try {
                request(JSONObject(source.getJSONObject("complete").toString())
                    .put("path", files.sourcePath(uri) ?: JSONObject.NULL))
            } catch (error: Exception) { failure = error.message }
        }
    }
    val source = description?.optJSONObject("source_picker")
    LaunchedEffect(source?.getString("id")) {
        val picker = source ?: return@LaunchedEffect
        if (deliveredSource == picker.getString("id")) return@LaunchedEffect
        deliveredSource = picker.getString("id")
        pendingSource = picker.toString()
        val folder = picker.getBoolean("folder")
        val intent = Intent(if (folder) Intent.ACTION_OPEN_DOCUMENT_TREE else Intent.ACTION_OPEN_DOCUMENT)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            .putExtra(Intent.EXTRA_TITLE, picker.getString("title"))
            .putExtra(DocumentsContract.EXTRA_INITIAL_URI, ProjectDocumentsProvider.directoryUri(context, File(context.filesDir, "projects")))
        if (!folder) intent.addCategory(Intent.CATEGORY_OPENABLE).type = "*/*"
        sourcePicker.launch(intent)
    }
    LaunchedEffect(timeline.identity) {
        try { timeline.request("inspector_visible", "visible" to false) }
        catch (error: Exception) { failure = error.message }
        while (isActive) {
            requests.withLock {
                try {
                    description = timeline.request("preferences_schema")
                } catch (cancelled: CancellationException) {
                    throw cancelled
                } catch (error: Exception) {
                    failure = error.message
                }
            }
            delay(1_000)
        }
    }
    val model = description
    val groups = model?.getJSONArray("controls")?.objects().orEmpty()
    val title = model?.getString("title") ?: tr("Settings")
    val backLabel = model?.getString("back_label") ?: tr("Back")
    val font = model?.optJSONObject("font_picker")
    val shift = with(LocalDensity.current) { BackTranslation.toPx() }
    val colors = MaterialTheme.colorScheme

    BoxWithConstraints(Modifier.fillMaxSize().background(colors.surfaceContainer)) {
        val wide = model != null && maxWidth >= model.getInt("wide_width").dp
        val category = groups.firstOrNull { it.getString("id") == page }
            ?: groups.firstOrNull().takeIf { wide }
        val sections = category?.getJSONArray("controls")?.objects().orEmpty()
        val detail = sections.firstOrNull { it.getString("id") == section }
        val back: suspend () -> Unit = {
            when {
                font != null -> request(font.getJSONObject("close"))
                detail != null -> section = null
                category != null && !wide -> page = null
                else -> {
                    model?.optJSONObject("close_command")?.let { request(it) }
                    dismiss()
                }
            }
        }
        val stack = buildList {
            if (!wide || category == null) add(RootPage to model)
            category?.let { add(it.getString("id") to it) }
            detail?.let { add(it.getString("id") to it) }
            font?.let { add(it.getString("id") to it) }
        }
        val swiping = progress.value > 0f && backPage == stack.last().first
        val leavingSettings = swiping && stack.size == 1
        val transform = Modifier.graphicsLayer {
            val fraction = progress.value
            scaleX = 1f - BackScale * fraction
            scaleY = scaleX
            translationX = shift * fraction * if (edge == BackEventCompat.EDGE_LEFT) 1f else -1f
        }

        val categories: @Composable (Boolean) -> Unit = { active ->
            SettingsGroup(modifier = Modifier.selectableGroup()) {
                groups.forEach { group ->
                    val id = group.getString("id")
                    val selected = wide && id == category?.getString("id")
                    SettingsRow(group.getString("label"), group.getString("summary"), active,
                        modifier = Modifier
                            .then(if (selected) Modifier.background(colors.secondaryContainer) else Modifier)
                            .selectable(selected = selected, enabled = active, role = Role.Button) {
                                scope.launch {
                                    font?.let { request(it.getJSONObject("close")) }
                                    page = id
                                    section = null
                                }
                            },
                        leading = { SettingsIcon(group.getString("icon"), group.getString("color")) })
                }
            }
        }
        val controls: @Composable (List<JSONObject>, Boolean) -> Unit = { entries, enabled ->
            entries.forEach { control ->
                key(control.getString("id")) {
                    InspectorControl(timeline, control, enabled, send,
                        { error("Preference file controls require a native picker command") }, settings = true)
                }
            }
        }
        val content: @Composable (Boolean) -> Unit = { animatePage ->
            stack.forEachIndexed { index, (id, document) ->
                val active = index == stack.lastIndex
                if (active || (swiping && index == stack.lastIndex - 1)) pages.SaveableStateProvider(id) {
                    val root = id == RootPage
                    val fonts = document === font && font != null
                    val enabled = active && !busy
                    val modifier = when {
                        !active -> Modifier.clearAndSetSemantics {}.focusProperties { canFocus = false }
                        animatePage && swiping -> transform
                        else -> Modifier
                    }
                    PreferencesPage(
                        title = when {
                            root -> title
                            fonts -> document!!.getString("title")
                            else -> document!!.getString("label")
                        },
                        modifier = modifier,
                        backLabel = backLabel,
                        back = if (!wide && active) ({ scope.launch { back() } }) else null,
                        backEnabled = enabled,
                        scrollable = !fonts,
                    ) {
                        if (active) failure?.let { Text(it, Modifier.padding(horizontal = 16.dp), color = colors.error) }
                        when {
                            document == null -> LinearProgressIndicator(Modifier.fillMaxWidth())
                            root -> categories(enabled)
                            fonts -> FontPreferences(document, enabled, send)
                            document === detail -> {
                                val (actions, rows) = document.getJSONArray("controls").objects()
                                    .partition { it.optBoolean("standalone") }
                                actions.forEach { action -> SettingsGroup { controls(listOf(action), enabled) } }
                                if (rows.isNotEmpty()) SettingsGroup { controls(rows, enabled) }
                            }
                            sections.size <= 1 -> controls(sections, enabled)
                            else -> SettingsGroup {
                                sections.forEach { entry ->
                                    SettingsRow(entry.getString("label"), enabled = enabled,
                                        modifier = Modifier.clickable(enabled = enabled, role = Role.Button) {
                                            page = id
                                            section = entry.getString("id")
                                        },
                                        trailing = { Icon(iconVector("go-next-symbolic"), null, Modifier.size(16.dp)) })
                                }
                            }
                        }
                    }
                }
            }
        }
        if (leavingSettings) workspace()
        Box(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing)) {
            if (wide && category != null) {
                Row(Modifier.fillMaxSize().then(if (leavingSettings) transform else Modifier)) {
                    pages.SaveableStateProvider(RootPage) {
                        PreferencesPage(title, Modifier.weight(0.36f), containerColor = colors.surface) {
                            categories(!busy)
                        }
                    }
                    Box(Modifier.weight(0.64f).fillMaxHeight()) { content(!leavingSettings) }
                }
            } else content(true)
        }
        PredictiveBackHandler {
            backPage = stack.last().first
            try {
                it.collect { event -> edge = event.swipeEdge; progress.snapTo(event.progress) }
                if (progress.value > 0f) progress.animateTo(1f, tween(BackAnimationMillis))
                back()
                progress.snapTo(0f)
            } catch (cancelled: CancellationException) {
                withContext(NonCancellable) { progress.animateTo(0f, tween(BackAnimationMillis)) }
                throw cancelled
            } finally {
                backPage = null
            }
        }
    }
    model?.optJSONObject("dialog")?.let { dialog ->
        val close = { send(dialog.getJSONObject("command")) }
        AlertDialog(onDismissRequest = close, title = { Text(dialog.getString("title")) },
            text = { Text(dialog.getString("body")) },
            confirmButton = { TextButton(onClick = close) { Text(dialog.getString("close_label")) } })
    }
}

@Composable
private fun SettingsIcon(icon: String, color: String) {
    Surface(
        Modifier.size(40.dp), shape = CircleShape,
        color = Color(android.graphics.Color.parseColor(color)),
        contentColor = Color.White,
    ) {
        Box(contentAlignment = Alignment.Center) { Icon(iconVector(icon), null, Modifier.size(20.dp)) }
    }
}

@Composable
internal fun SettingsGroup(title: String = "", modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Column(modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, bottom = 16.dp)) {
        if (title.isNotEmpty()) Text(title, Modifier.padding(start = 16.dp, top = 8.dp, bottom = 8.dp),
            color = MaterialTheme.colorScheme.primary, style = MaterialTheme.typography.titleSmall)
        Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)),
            verticalArrangement = Arrangement.spacedBy(2.dp), content = content)
    }
}

@Composable
internal fun ColumnScope.FontPreferences(model: JSONObject, enabled: Boolean, send: (JSONObject) -> Unit) {
    var query by rememberSaveable(model.getString("id")) { mutableStateOf("") }
    val choices = model.getJSONArray("choices").objects()
    val list = rememberLazyListState()
    LaunchedEffect(model.getString("id"), query) {
        delay(model.getLong("search_delay_ms"))
        if (list.firstVisibleItemIndex != 0 || list.firstVisibleItemScrollOffset != 0) list.scrollToItem(0)
        send(JSONObject(model.getJSONObject("search").toString()).put("query", query))
    }
    LaunchedEffect(list, model.getJSONArray("choices").toString(), model.getLong("preview_revision")) {
        snapshotFlow { list.layoutInfo.visibleItemsInfo.let { items -> (items.firstOrNull()?.index ?: 0) to ((items.lastOrNull()?.index ?: -1) + 1) } }
            .distinctUntilChanged().collect { (start, end) -> send(JSONObject(model.getJSONObject("visible").toString()).put("start", start).put("end", end)) }
    }
    OutlinedTextField(query, { query = it }, singleLine = true, label = { Text(model.getString("search_label")) },
        modifier = Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 8.dp))
    if (model.getBoolean("busy")) LinearProgressIndicator(Modifier.fillMaxWidth())
    if (model.getString("status").isNotEmpty()) Text(model.getString("status"), Modifier.padding(horizontal = 24.dp), color = MaterialTheme.colorScheme.onSurfaceVariant)
    LazyColumn(Modifier.weight(1f).fillMaxWidth(), state = list) {
        itemsIndexed(choices) { _, choice ->
            val name = choice.getString("label")
            val preview = choice.optString("preview_file").takeIf { !choice.isNull("preview_file") }
            val index = choice.getInt("preview_index")
            val family by produceState<FontFamily>(FontFamily.Default, preview, index) {
                value = FontFamily.Default
                if (preview != null) {
                    val typeface = withContext(Dispatchers.IO) {
                        runCatching { Typeface.Builder(preview).setTtcIndex(index).build() }.getOrNull()
                    }
                    if (typeface != null) value = FontFamily(typeface)
                }
            }
            Row(Modifier.fillMaxWidth().clickable(enabled = enabled) { send(choice.getJSONObject("command")) }
                .padding(start = 24.dp, end = 16.dp, top = 16.dp, bottom = 16.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(name, Modifier.weight(1f), style = MaterialTheme.typography.titleMedium,
                    fontFamily = family, maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (choice.getBoolean("google")) GoogleFontIcon()
                RadioButton(choice.getBoolean("selected"), onClick = null, enabled = enabled)
            }
        }
    }
}

@Composable
internal fun GoogleFontIcon() {
    Icon(iconVector("cloud-symbolic"), tr("Google Fonts"), Modifier.size(14.dp),
        tint = MaterialTheme.colorScheme.onSurfaceVariant)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PreferencesPage(
    title: String,
    modifier: Modifier = Modifier,
    backLabel: String = "",
    back: (() -> Unit)? = null,
    backEnabled: Boolean = true,
    scrollable: Boolean = true,
    containerColor: Color = MaterialTheme.colorScheme.surfaceContainer,
    content: @Composable ColumnScope.() -> Unit,
) {
    Scaffold(
        modifier = modifier.fillMaxSize(),
        containerColor = containerColor,
        contentWindowInsets = WindowInsets(0, 0, 0, 0),
        topBar = {
            TopAppBar(
                title = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                navigationIcon = {
                    if (back != null) IconButton(onClick = back, enabled = backEnabled) {
                        Icon(iconVector("go-previous-symbolic"), backLabel, Modifier.size(24.dp))
                    }
                },
                windowInsets = WindowInsets(0, 0, 0, 0),
                colors = TopAppBarDefaults.topAppBarColors(containerColor = containerColor),
            )
        },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)
            .then(if (scrollable) Modifier.verticalScroll(rememberScrollState()) else Modifier), content = content)
    }
}
