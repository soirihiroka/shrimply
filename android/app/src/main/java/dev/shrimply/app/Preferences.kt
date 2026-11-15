package dev.shrimply.app

import androidx.activity.BackEventCompat
import androidx.activity.compose.PredictiveBackHandler
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.tween
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

private const val RootPage = "preferences:root"
private const val BackScale = 0.1f
private const val BackAnimationMillis = 180
private val BackTranslation = 24.dp
private val BackCornerRadius = 28.dp

/** Material navigation and rows display the same Rust preferences document. */
@Composable
internal fun PreferencesScreen(timeline: TimelineSession, dismiss: () -> Unit, workspace: @Composable () -> Unit) {
    val scope = rememberCoroutineScope()
    var description by remember { mutableStateOf<JSONObject?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var page by rememberSaveable { mutableStateOf<String?>(null) }
    val pages = rememberSaveableStateHolder()
    val progress = remember { Animatable(0f) }
    var edge by remember { mutableIntStateOf(BackEventCompat.EDGE_LEFT) }
    val back = { if (page == null) dismiss() else page = null }
    val currentBusy by rememberUpdatedState(busy)
    LaunchedEffect(timeline.identity) {
        try { timeline.request("inspector_visible", "visible" to false) }
        catch (error: Exception) { failure = error.message }
        while (isActive) {
            if (!currentBusy) {
                try { description = timeline.request("preferences_schema") }
                catch (error: Exception) { failure = error.message }
            }
            delay(1_000)
        }
    }
    val send: (JSONObject) -> Unit = { command -> scope.launch {
        busy = true
        try {
            description = timeline.request("preferences", *command.keys().asSequence().map { it to command.get(it) }.toList().toTypedArray())
            failure = null
        } catch (error: Exception) { failure = error.message }
        finally { busy = false }
    } }
    val model = description
    val groups = model?.getJSONArray("controls")?.objects().orEmpty()
    val category = groups.firstOrNull { it.getString("id") == page }
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
    Box(Modifier.fillMaxSize()) {
        if (category == null && progress.value > 0f) {
            workspace()
            Surface(Modifier.fillMaxSize(), color = Color.Transparent) {}
        }
        if (category == null || progress.value > 0f) pages.SaveableStateProvider(RootPage) {
            PreferencesPage(
                title = model?.getString("title") ?: tr("Preferences"),
                backLabel = model?.getString("back_label") ?: tr("Back"), back = back,
                modifier = if (category == null) transform else Modifier.clearAndSetSemantics {}.focusProperties { canFocus = false },
                enabled = category == null,
            ) {
                failure?.let { Text(it, Modifier.padding(24.dp), color = MaterialTheme.colorScheme.error) }
                if (model == null && failure == null) LinearProgressIndicator(Modifier.fillMaxWidth())
                groups.forEach { group ->
                    ListItem(headlineContent = { Text(group.getString("label")) },
                        supportingContent = { Text(group.getString("summary")) },
                        leadingContent = { Icon(painterResource(inspectorIcon(group.getString("icon"))), null, Modifier.size(24.dp)) },
                        colors = ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.surface),
                        modifier = Modifier.fillMaxWidth().clickable(enabled = category == null) { page = group.getString("id") }
                            .padding(start = 8.dp).heightIn(min = 88.dp))
                }
            }
        }
        if (category != null) {
            if (progress.value > 0f) Surface(Modifier.fillMaxSize(), color = Color.Transparent) {}
            pages.SaveableStateProvider(category.getString("id")) {
                PreferencesPage(category.getString("label"), model!!.getString("back_label"), back, transform) {
                    failure?.let { Text(it, Modifier.padding(24.dp), color = MaterialTheme.colorScheme.error) }
                    category.getJSONArray("controls").objects().forEach { control -> key(control.getString("id")) {
                        InspectorControl(timeline, control, !busy, send,
                            { error("File selection is not supported by this preferences description") }, settings = true)
                    } }
                }
            }
        }
    }
    // Register after the previous page's content so its back callback cannot win.
    PredictiveBackHandler { events ->
        try {
            events.collect { event ->
                edge = event.swipeEdge
                progress.snapTo(event.progress)
            }
            if (progress.value > 0f) progress.animateTo(1f, tween(BackAnimationMillis))
            progress.snapTo(0f)
            back()
        } catch (cancelled: CancellationException) {
            withContext(NonCancellable) { progress.animateTo(0f, tween(BackAnimationMillis)) }
            throw cancelled
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PreferencesPage(title: String, backLabel: String, back: () -> Unit,
    modifier: Modifier = Modifier, enabled: Boolean = true, content: @Composable ColumnScope.() -> Unit) {
    val scroll = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
    Scaffold(modifier = modifier.fillMaxSize().nestedScroll(scroll.nestedScrollConnection),
        containerColor = MaterialTheme.colorScheme.surface,
        topBar = {
            LargeTopAppBar(title = { Text(title) }, scrollBehavior = scroll,
                colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.surface),
                navigationIcon = { IconButton(onClick = back, enabled = enabled) {
                    Icon(painterResource(R.drawable.go_previous), backLabel, Modifier.size(24.dp))
                } })
        }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).consumeWindowInsets(padding)
            .verticalScroll(rememberScrollState()).navigationBarsPadding().padding(bottom = 24.dp), content = content)
    }
}
