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
import androidx.compose.foundation.BorderStroke
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
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
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
private val BackCornerRadius = 28.dp

/** Android Settings navigation displays the same pages, groups and controls as GTK. */
@Composable
internal fun PreferencesScreen(timeline: TimelineSession, files: ProjectFiles, dismiss: () -> Unit, workspace: @Composable () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var description by remember { mutableStateOf<JSONObject?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var page by rememberSaveable { mutableStateOf<String?>(null) }
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
    val category = groups.firstOrNull { it.getString("id") == page }
    val font = model?.optJSONObject("font_picker")
    val back: suspend () -> Unit = {
        if (font != null) request(font.getJSONObject("close"))
        else if (category != null) page = null
        else { model?.optJSONObject("close_command")?.let { request(it) }; dismiss() }
    }
    val stack = buildList {
        add(RootPage to model)
        category?.let { add(it.getString("id") to it) }
        font?.let { add(it.getString("id") to it) }
    }
    val density = LocalDensity.current
    val shift = with(density) { BackTranslation.toPx() }
    val transform = Modifier.graphicsLayer {
        val fraction = progress.value
        scaleX = 1f - BackScale * fraction
        scaleY = scaleX
        translationX = shift * fraction * if (edge == BackEventCompat.EDGE_LEFT) 1f else -1f
        shape = RoundedCornerShape(BackCornerRadius * fraction)
        clip = fraction > 0f
    }
    val showPage: @Composable (Pair<String, JSONObject?>, Modifier, Boolean) -> Unit = { (id, document), modifier, active ->
        pages.SaveableStateProvider(id) {
            val root = id == RootPage
            val fonts = document === font && font != null
            PreferencesPage(
                title = if (root) document?.getString("title") ?: tr("Preferences") else if (fonts) document!!.getString("title") else document!!.getString("label"),
                backLabel = model?.getString("back_label") ?: tr("Back"),
                back = { scope.launch { back() } }, modifier = modifier, enabled = active && !busy, scrollable = !fonts,
            ) {
                if (active) failure?.let { Text(it, Modifier.padding(horizontal = 24.dp), color = MaterialTheme.colorScheme.error) }
                if (document == null) LinearProgressIndicator(Modifier.fillMaxWidth())
                else if (root) SettingsGroup {
                    groups.forEach { group ->
                        SettingsRow(group.getString("label"), group.getString("summary"), active,
                            leading = { SettingsIcon(group.getString("icon")) },
                            trailing = { Icon(iconVector("go-next-symbolic"), null, Modifier.size(20.dp)) },
                            modifier = Modifier.clickable(enabled = active) { page = group.getString("id") })
                    }
                }
                else if (fonts) FontPreferences(document, active && !busy, send)
                else document.getJSONArray("controls").objects().forEach { control -> key(control.getString("id")) {
                    InspectorControl(timeline, control, active && !busy, send,
                        { error("Preference file controls require a native picker command") }, settings = true)
                } }
            }
        }
    }
    Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.surfaceContainerLowest)) {
        val swiping = progress.value > 0f && backPage == stack.last().first
        if (swiping && stack.size == 1) {
            workspace()
            Surface(Modifier.fillMaxSize(), color = Color.Transparent) {}
        }
        stack.forEachIndexed { index, entry ->
            val active = index == stack.lastIndex
            if (active || (swiping && index == stack.lastIndex - 1)) key(entry.first) {
                val modifier = if (active) {
                    if (swiping) transform else Modifier
                } else Modifier.clearAndSetSemantics {}.focusProperties { canFocus = false }
                showPage(entry, modifier, active)
            }
        }
    }
    model?.optJSONObject("dialog")?.let { dialog ->
        val close = { send(dialog.getJSONObject("command")) }
        AlertDialog(onDismissRequest = close, title = { Text(dialog.getString("title")) },
            text = { Text(dialog.getString("body")) },
            confirmButton = { TextButton(onClick = close) { Text(dialog.getString("close_label")) } })
    }
    PredictiveBackHandler {
        val target = back
        backPage = stack.last().first
        try {
            it.collect { event -> edge = event.swipeEdge; progress.snapTo(event.progress) }
            if (progress.value > 0f) progress.animateTo(1f, tween(BackAnimationMillis))
            target()
            progress.snapTo(0f)
        } catch (cancelled: CancellationException) {
            withContext(NonCancellable) { progress.animateTo(0f, tween(BackAnimationMillis)) }
            throw cancelled
        } finally {
            backPage = null
        }
    }
}

@Composable
private fun SettingsIcon(icon: String) {
    Surface(
        Modifier.size(36.dp), shape = CircleShape,
        color = MaterialTheme.colorScheme.secondaryContainer,
        contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
    ) {
        Box(contentAlignment = Alignment.Center) { Icon(iconVector(icon), null, Modifier.size(20.dp)) }
    }
}

@Composable
internal fun SettingsGroup(title: String = "", content: @Composable ColumnScope.() -> Unit) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)) {
        if (title.isNotEmpty()) Text(title, Modifier.padding(start = 4.dp, top = 8.dp, bottom = 8.dp),
            color = MaterialTheme.colorScheme.primary, style = MaterialTheme.typography.titleSmall)
        Column(Modifier.fillMaxWidth().clip(MaterialTheme.shapes.extraLarge),
            verticalArrangement = Arrangement.spacedBy(2.dp), content = content)
    }
}

@Composable
internal fun ColumnScope.FontPreferences(model: JSONObject, enabled: Boolean, send: (JSONObject) -> Unit) {
    var query by rememberSaveable(model.getString("id")) { mutableStateOf("") }
    val choices = model.getJSONArray("choices").objects()
    val list = rememberLazyListState()
    LaunchedEffect(query) {
        delay(model.getLong("search_delay_ms"))
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
            val family = remember(name, preview, choice.getInt("preview_index")) {
                when {
                    preview != null -> FontFamily(requireNotNull(Typeface.Builder(preview).setTtcIndex(choice.getInt("preview_index")).build()))
                    else -> FontFamily.Default
                }
            }
            Row(Modifier.fillMaxWidth().clickable(enabled = enabled) { send(choice.getJSONObject("command")) }
                .padding(start = 24.dp, end = 16.dp, top = 16.dp, bottom = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(name, style = MaterialTheme.typography.titleMedium, fontFamily = family)
                    Text(choice.getString("source"), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                RadioButton(choice.getBoolean("selected"), onClick = null, enabled = enabled)
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PreferencesPage(title: String, backLabel: String, back: () -> Unit,
    modifier: Modifier = Modifier, enabled: Boolean = true, scrollable: Boolean = true, content: @Composable ColumnScope.() -> Unit) {
    val scroll = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
    val densityForTitle = LocalDensity.current
    val configuration = LocalConfiguration.current
    var titleHeight by remember(title, configuration.screenWidthDp, configuration.fontScale) { mutableStateOf(0.dp) }
    Scaffold(modifier = modifier.fillMaxSize().nestedScroll(scroll.nestedScrollConnection),
        containerColor = MaterialTheme.colorScheme.surface,
        contentWindowInsets = WindowInsets.safeDrawing,
        topBar = {
            MaterialTheme(typography = MaterialTheme.typography.copy(headlineMedium = MaterialTheme.typography.displaySmall,
                titleLarge = MaterialTheme.typography.titleMedium)) {
                LargeTopAppBar(title = { Text(title, Modifier.padding(start = 8.dp, end = 16.dp), maxLines = 3,
                    onTextLayout = { result ->
                        val measured = with(densityForTitle) { result.size.height.toDp() }
                        if (measured > titleHeight) titleHeight = measured
                    }) }, scrollBehavior = scroll,
                    windowInsets = WindowInsets.safeDrawing.only(WindowInsetsSides.Top + WindowInsetsSides.Horizontal),
                    collapsedHeight = 56.dp, expandedHeight = maxOf(152.dp, 56.dp + titleHeight + 28.dp),
                    colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.surface,
                        scrolledContainerColor = MaterialTheme.colorScheme.surfaceContainer),
                    navigationIcon = {
                        OutlinedIconButton(onClick = back, enabled = enabled, modifier = Modifier.padding(start = 8.dp),
                            colors = IconButtonDefaults.outlinedIconButtonColors(containerColor = MaterialTheme.colorScheme.surfaceContainerHigh),
                            border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)) {
                            Icon(iconVector("go-previous-symbolic"), backLabel, Modifier.size(24.dp))
                        }
                    })
            }
        }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).consumeWindowInsets(padding).navigationBarsPadding().imePadding()
            .then(if (scrollable) Modifier.verticalScroll(rememberScrollState()) else Modifier)
            .padding(bottom = 24.dp), content = content)
    }
}
