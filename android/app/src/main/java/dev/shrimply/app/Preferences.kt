package dev.shrimply.app

import android.app.Activity
import android.content.Intent
import android.graphics.Typeface
import android.os.Bundle
import android.provider.DocumentsContract
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
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
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.repeatOnLifecycle
import androidx.lifecycle.viewModelScope
import androidx.window.embedding.SplitAttributes
import androidx.window.embedding.SplitController
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.json.JSONObject
import java.io.File

internal class SettingsSession(private val timeline: TimelineSession, scope: CoroutineScope) {
    private val document = MutableStateFlow<JSONObject?>(null)
    val snapshot = document.asStateFlow()
    private val requests = Mutex()
    private var encoded = ""
    var failure by mutableStateOf<String?>(null)
        private set

    init {
        scope.launch {
            document.subscriptionCount.map { it > 0 }.distinctUntilChanged().collectLatest { visible ->
                if (visible) while (isActive) {
                    try {
                        request()
                        failure = null
                    } catch (cancelled: CancellationException) {
                        throw cancelled
                    } catch (error: Exception) {
                        failure = error.message
                    }
                    delay(1_000)
                }
            }
        }
    }

    suspend fun request(command: JSONObject? = null): JSONObject = requests.withLock {
        val result = if (command == null) timeline.request("preferences_schema")
            else timeline.request(JSONObject(command.toString()).put("op", "preferences"))
        val json = withContext(Dispatchers.Default) { result.toString() }
        if (encoded != json) {
            encoded = json
            document.value = result
        }
        result
    }
}

class SettingsDetailActivity : SettingsActivity()

open class SettingsActivity : ComponentActivity() {
    private var editor: EditorModel? = null
    private val home get() = this !is SettingsDetailActivity

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val model = EditorModel.forTask(taskId) ?: run { finish(); return }
        editor = model
        enableEdgeToEdge()
        val splits = SplitController.getInstance(this).splitInfoList(this)
        val category = intent.getStringExtra("category")
        val fontPicker = intent.getLongExtra("font_picker", 0)
        setContent {
            val dark = isSystemInDarkTheme()
            val colors = remember(dark) { if (dark) dynamicDarkColorScheme(this) else dynamicLightColorScheme(this) }
            val splitInfo by splits.collectAsState(initial = emptyList())
            val split = splitInfo.any { it.splitAttributes.splitType != SplitAttributes.SplitType.SPLIT_TYPE_EXPAND }
            MaterialTheme(colorScheme = colors) {
                MenuInputScope(remember { mutableStateOf("touch") }) {
                    PreferencesScreen(model, split, category, fontPicker)
                }
            }
        }
    }

    private fun openPage(category: String, fontPicker: Long = 0) {
        editor?.settingsCategory = category
        startActivity(Intent(this, SettingsDetailActivity::class.java)
            .putExtra("category", category).putExtra("font_picker", fontPicker))
    }

    override fun onDestroy() {
        val fontPicker = intent.getLongExtra("font_picker", 0)
        if (isFinishing && (home || fontPicker != 0L)) editor?.let { model ->
            model.viewModelScope.launch {
                try {
                    if (home) model.settings.request(JSONObject().put("action", "preferences_close"))
                    else model.settings.request(JSONObject().put("action", "font_close").put("picker", fontPicker))
                } catch (cancelled: CancellationException) {
                    throw cancelled
                } catch (error: Exception) {
                    android.util.Log.w("Shrimply", "Could not close Settings", error)
                }
            }
        }
        super.onDestroy()
    }

    @Composable
    private fun PreferencesScreen(model: EditorModel, split: Boolean, categoryId: String?, fontPicker: Long) {
        val fontPage = fontPicker != 0L
        val lifecycleState by lifecycle.currentStateFlow.collectAsState()
        val handlesPrompts = !home && lifecycleState == Lifecycle.State.RESUMED
        val timeline = model.timeline
        val files = model.projects
        val context = LocalContext.current
        val scope = rememberCoroutineScope()
        val document by produceState(model.settings.snapshot.value, model) {
            lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
                model.settings.snapshot.collect { value = it }
            }
        }
        var failure by remember { mutableStateOf<String?>(null) }
        var busy by remember { mutableStateOf(false) }
        var pendingSource by rememberSaveable { mutableStateOf<String?>(null) }
        var deliveredSource by rememberSaveable { mutableStateOf<String?>(null) }
        suspend fun request(command: JSONObject): JSONObject? {
            val interactive = command.optString("action") !in listOf("font_visible", "font_search")
            if (interactive) busy = true
            return try {
                val result = model.settings.request(command)
                if (result.has("show_in_files")) files.showInFiles(result.getString("show_in_files"))
                if (interactive) failure = null
                result
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                failure = error.message
                null
            } finally {
                if (interactive) busy = false
            }
        }
        val send: (JSONObject) -> Unit = { command ->
            scope.launch {
                val result = request(command)
                if (result != null && command.optString("action") == "font_open") {
                    val picker = result.optJSONObject("font_picker")?.getJSONObject("close")?.getLong("picker")
                    val page = categoryId ?: model.settingsCategory
                    if (page != null && picker != null) openPage(page, picker)
                }
            }
        }
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
        val source = document?.optJSONObject("source_picker").takeIf { handlesPrompts }
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
        val current = document
        val pages = current?.getJSONArray("controls")?.objects().orEmpty()
        val category = if (home) null else pages.firstOrNull { it.getString("id") == categoryId }
            ?: pages.firstOrNull()
        val sections = category?.getJSONArray("controls")?.objects().orEmpty()
        val font = current?.optJSONObject("font_picker")?.takeIf {
            fontPage && it.getJSONObject("close").getLong("picker") == fontPicker
        }
        LaunchedEffect(current != null, font != null) {
            if (fontPage && current != null && font == null) finish()
        }
        val title = font?.getString("title") ?: category?.getString("label")
            ?: current?.getString("title") ?: tr("Settings")
        val background = if (home && split) MaterialTheme.colorScheme.surface else MaterialTheme.colorScheme.surfaceContainer
        val enabled = !busy
        if (category != null && handlesPrompts) SideEffect { model.settingsCategory = category.getString("id") }
        val controls: @Composable (List<JSONObject>) -> Unit = { entries ->
            entries.forEach { control -> key(control.getString("id")) {
                InspectorControl(timeline, control, enabled, send,
                    { error("Preference file controls require a native picker command") }, settings = true)
            } }
        }
        Surface(Modifier.fillMaxSize(), color = background) {
            PreferencesPage(
                title = title,
                modifier = Modifier.windowInsetsPadding(WindowInsets.safeDrawing),
                backLabel = current?.getString("back_label") ?: tr("Back"),
                back = if (split && categoryId == null) null else ::finish,
                scrollable = !fontPage,
                containerColor = background,
            ) {
                (failure ?: model.settings.failure)?.let { Text(it, Modifier.padding(horizontal = 16.dp), color = MaterialTheme.colorScheme.error) }
                when {
                    current == null || fontPage && font == null -> LinearProgressIndicator(Modifier.fillMaxWidth())
                    font != null -> FontPreferences(font, enabled, send)
                    home -> SettingsGroup(modifier = Modifier.selectableGroup()) {
                        pages.forEach { page ->
                            val id = page.getString("id")
                            val selected = split && id == model.settingsCategory
                            SettingsRow(page.getString("label"), page.getString("summary"), enabled,
                                modifier = Modifier
                                    .then(if (selected) Modifier.background(MaterialTheme.colorScheme.secondaryContainer) else Modifier)
                                    .selectable(selected = selected, enabled = enabled, role = Role.Button) { openPage(id) },
                                leading = { SettingsIcon(page.getString("icon"), page.getString("color")) })
                        }
                    }
                    else -> sections.forEach { section ->
                        val (actions, rows) = section.getJSONArray("controls").objects().partition { it.optBoolean("standalone") }
                        if (actions.isEmpty()) controls(listOf(section))
                        else {
                            SettingsGroup(section.getString("label")) { controls(actions) }
                            if (rows.isNotEmpty()) SettingsGroup { controls(rows) }
                        }
                    }
                }
            }
        }
        current?.optJSONObject("dialog").takeIf { handlesPrompts }?.let { dialog ->
            val close = { send(dialog.getJSONObject("command")) }
            AlertDialog(onDismissRequest = close, title = { Text(dialog.getString("title")) },
                text = { Text(dialog.getString("body")) },
                confirmButton = { TextButton(onClick = close) { Text(dialog.getString("close_label")) } })
        }
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
                    if (back != null) IconButton(onClick = back) {
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
