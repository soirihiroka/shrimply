package dev.shrimply.app

import android.os.Bundle
import android.os.Build
import android.app.ActivityManager
import android.content.pm.PackageManager
import android.view.WindowInsetsController
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.AndroidExternalSurface
import androidx.compose.foundation.AndroidEmbeddedExternalSurface
import androidx.compose.foundation.AndroidExternalSurfaceScope
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import android.view.InputDevice
import android.view.MotionEvent
import android.view.KeyEvent
import androidx.compose.foundation.focusable
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.pointer.pointerInteropFilter
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.unit.DpOffset
import androidx.compose.ui.unit.dp
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.core.view.WindowCompat

private class EditorModel(context: android.content.Context, stress: Boolean, benchmark: Boolean) : ViewModel() {
    val timeline = TimelineSession(stress, benchmark)
    val projects = ProjectFiles(context.applicationContext)
    override fun onCleared() { timeline.close(); projects.close() }
}

class MainActivity : ComponentActivity() {
    private lateinit var timeline: TimelineSession
    private lateinit var projects: ProjectFiles
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // App-sharing pickers can launch the current app above its permission activity.
        if (!isTaskRoot && intent.action == android.content.Intent.ACTION_MAIN &&
            intent.hasCategory(android.content.Intent.CATEGORY_LAUNCHER)) {
            finish()
            return
        }
        val fontDirectories = android.graphics.fonts.SystemFonts.getAvailableFonts()
            .mapNotNull { it.file?.parentFile?.absolutePath }.distinct().sorted()
        NativeTimeline.initialize(applicationContext, java.io.File(filesDir, "config").absolutePath,
            cacheDir.absolutePath, org.json.JSONArray(fontDirectories).toString())
        NativeI18n.initializeLocale(resources.configuration.locales.toLanguageTags())
        val stress = intent.getBooleanExtra("stress", false)
        val benchmark = intent.getBooleanExtra("benchmark", false)
        val model = ViewModelProvider(this, object : ViewModelProvider.Factory {
            override fun <T : ViewModel> create(modelClass: Class<T>): T =
                modelClass.cast(EditorModel(applicationContext, stress, benchmark))!!
        })[EditorModel::class.java]
        timeline = model.timeline
        projects = model.projects
        if (applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE != 0) intent.getStringExtra("throughput_project")?.let { path ->
            window.addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            Thread {
                try {
                    val report = NativeTimeline.throughput(path, intent.getStringExtra("throughput_scene") ?: "project",
                        intent.getIntExtra("throughput_frames", 600))
                    java.io.File(cacheDir, "pipeline-throughput.json").writeText(report)
                    android.util.Log.i("ShrimplyTimeline", "throughput_done")
                } catch (failure: Exception) {
                    android.util.Log.e("ShrimplyTimeline", "throughput_failed", failure)
                } finally {
                    runOnUiThread { window.clearFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) }
                }
            }.start()
        }
        val customCaptionSupported = Build.VERSION.SDK_INT >= 35 &&
            packageManager.hasSystemFeature(PackageManager.FEATURE_FREEFORM_WINDOW_MANAGEMENT)
        val caption = mutableStateOf(WindowCaption())
        window.decorView.setOnApplyWindowInsetsListener { view, insets ->
            caption.value = windowCaption(insets, customCaptionSupported)
            view.onApplyWindowInsets(insets)
        }
        setContent {
            if (applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE != 0) {
                val project = intent.getStringExtra("debug_project")
                LaunchedEffect(timeline.identity, project) {
                    if (project != null) {
                        timeline.request("open", "path" to project)
                        timeline.request("remove_recent", "path" to project)
                        if (intent.getBooleanExtra("debug_playback_capture", false)) {
                            window.addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                            try {
                                timeline.request("performance_clear")
                                timeline.request("play")
                                kotlinx.coroutines.delay(java.time.Duration.ofMinutes(1).toMillis())
                                timeline.request("pause")
                                val report = timeline.request("performance_copy").getString("clipboard")
                                java.io.File(cacheDir, "playback-performance.json").writeText(report)
                            } finally { window.clearFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) }
                        }
                    }
                }
            }
            val currentCaption = caption.value
            val context = LocalContext.current
            val dark = isSystemInDarkTheme()
            val colors = if (dark) dynamicDarkColorScheme(context)
                else dynamicLightColorScheme(context)
            LaunchedEffect(window, dark, colors.primary, colors.surfaceContainerLowest, currentCaption.height > 0) {
                WindowCompat.getInsetsController(window, window.decorView).apply {
                    isAppearanceLightStatusBars = !dark
                    isAppearanceLightNavigationBars = !dark
                }
                if (Build.VERSION.SDK_INT >= 35) {
                    val transparent = WindowInsetsController.APPEARANCE_TRANSPARENT_CAPTION_BAR_BACKGROUND
                    val light = WindowInsetsController.APPEARANCE_LIGHT_CAPTION_BARS
                    val appearance = if (customCaptionSupported) transparent or (if (dark) 0 else light) else 0
                    window.insetsController?.setSystemBarsAppearance(appearance, transparent or light)
                }
                if (!customCaptionSupported) {
                    // Developer-forced freeform can expose a legacy opaque caption shell.
                    setTaskDescription(ActivityManager.TaskDescription.Builder()
                        .setLabel(title.toString()).setIcon(R.mipmap.ic_launcher)
                        .setPrimaryColor(colors.primary.toArgb())
                        .setStatusBarColor(colors.surfaceContainerLowest.toArgb()).build())
                }
            }
            val palette = arrayOf(
                "window_bg" to colors.surfaceContainerLowest, "window_fg" to colors.onSurface,
                "view_bg" to colors.surfaceContainerLow, "view_fg" to colors.onSurface,
                "alternate_bg" to colors.surfaceContainer, "button_bg" to colors.surfaceContainerHigh,
                "button_fg" to colors.onSurface, "border" to colors.outlineVariant,
                "accent_bg" to colors.primary, "accent_fg" to colors.onPrimary,
            ).map { (key, color) -> key to color.toArgb().toUInt().toLong() }
            LaunchedEffect(timeline.identity, dark, palette) {
                timeline.request("native_palette", "dark" to dark, *palette.toTypedArray())
            }
            MaterialTheme(colorScheme = colors) {
                Surface(Modifier.fillMaxSize()) {
                    if (benchmark) NativeSurface(timeline, Modifier.fillMaxSize())
                    else ProjectApp(timeline, projects, currentCaption)
                }
            }
        }
        window.decorView.requestApplyInsets()
    }
    override fun onResume() { super.onResume(); if (::timeline.isInitialized) timeline.active(true) }
    override fun onPause() { if (::timeline.isInitialized) timeline.active(false); super.onPause() }
}

@OptIn(ExperimentalComposeUiApi::class)
@Composable
internal fun NativeSurface(timeline: TimelineSession, modifier: Modifier, role: Int = 0, enabled: Boolean = true) {
    val graph = role >= 2
    val visible = remember(timeline.identity, role) { mutableStateOf(!graph) }
    val density = LocalDensity.current.density
    val viewConfiguration = android.view.ViewConfiguration.get(LocalContext.current)
    val focus = remember { FocusRequester() }
    val mouseButton = remember { intArrayOf(-1) }
    val surfaceModifier = modifier
        .onGloballyPositioned { coordinates ->
            if (graph) {
                val next = !coordinates.boundsInWindow(clipBounds = true).isEmpty
                if (visible.value != next) { visible.value = next; timeline.visible(role, next) }
            }
            val origin = coordinates.positionInRoot()
            if (!graph) timeline.menuOrigin = DpOffset((origin.x / density).dp, (origin.y / density).dp)
        }
        .focusRequester(focus)
        .onPreviewKeyEvent { key ->
            val event = key.nativeKeyEvent
            if (!enabled) return@onPreviewKeyEvent false
            if (event.action != KeyEvent.ACTION_DOWN || event.isAltPressed || event.isMetaPressed) return@onPreviewKeyEvent false
            val character = when (event.keyCode) {
                KeyEvent.KEYCODE_ESCAPE -> 27
                KeyEvent.KEYCODE_DEL -> 8
                KeyEvent.KEYCODE_FORWARD_DEL -> 127
                else -> event.getUnicodeChar(event.metaState and KeyEvent.META_CTRL_MASK.inv())
            }
            if (!graph && character == 0) return@onPreviewKeyEvent false
            val flags = (if (event.metaState and KeyEvent.META_CTRL_MASK != 0) 1 else 0) or (if (event.metaState and KeyEvent.META_SHIFT_MASK != 0) 2 else 0)
            timeline.input(floatArrayOf(if (graph) 28f else 8f, if (graph) event.keyCode.toFloat() else 0f, 0f, character.toFloat(), flags.toFloat(), 0f), role)
        }
        .focusable(enabled)
        .pointerInteropFilter { event ->
            if (!enabled) return@pointerInteropFilter false
            val action = event.actionMasked
            if (action == MotionEvent.ACTION_DOWN) focus.requestFocus()
            val mouse = event.isFromSource(InputDevice.SOURCE_MOUSE) || event.getToolType(0) == MotionEvent.TOOL_TYPE_STYLUS
            val flags = (if (event.metaState and KeyEvent.META_CTRL_MASK != 0) 1 else 0) or (if (event.metaState and KeyEvent.META_SHIFT_MASK != 0) 2 else 0)
            if (mouse || action == MotionEvent.ACTION_SCROLL) {
                val kind = when (action) {
                    MotionEvent.ACTION_DOWN -> {
                        mouseButton[0] = when {
                            event.buttonState and MotionEvent.BUTTON_SECONDARY != 0 -> if (graph) 2 else 3
                            event.buttonState and MotionEvent.BUTTON_TERTIARY != 0 -> 1
                            else -> 0
                        }
                        if (graph) 25f else if (mouseButton[0] == 3) 10f else 5f
                    }
                    MotionEvent.ACTION_UP -> if (graph) 26f else if (mouseButton[0] == 3) return@pointerInteropFilter true else 6f
                    MotionEvent.ACTION_MOVE, MotionEvent.ACTION_HOVER_MOVE, MotionEvent.ACTION_HOVER_ENTER -> if (graph) 24f else 4f
                    MotionEvent.ACTION_SCROLL -> if (graph) 27f else 7f
                    MotionEvent.ACTION_HOVER_EXIT -> if (graph) 30f else 9f
                    MotionEvent.ACTION_CANCEL -> if (graph) 29f else 3f
                    else -> return@pointerInteropFilter true
                }
                val touchpad = event.isFromSource(InputDevice.SOURCE_TOUCHPAD)
                val horizontal = event.getAxisValue(MotionEvent.AXIS_HSCROLL) * (if (touchpad) viewConfiguration.scaledHorizontalScrollFactor / density else 1f)
                val vertical = -event.getAxisValue(MotionEvent.AXIS_VSCROLL) * (if (touchpad) viewConfiguration.scaledVerticalScrollFactor / density else 1f)
                timeline.input(floatArrayOf(kind, event.x / density, event.y / density, if (kind == 7f || kind == 27f) horizontal else mouseButton[0].toFloat(), (flags or (if (touchpad) 4 else 0)).toFloat(), vertical), role)
                if (action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_CANCEL) mouseButton[0] = -1
            } else {
                val batch = FloatArray(event.pointerCount * 6)
                for (index in 0 until event.pointerCount) {
                    val phase = when {
                        action == MotionEvent.ACTION_CANCEL -> 3f
                        index == event.actionIndex && (action == MotionEvent.ACTION_DOWN || action == MotionEvent.ACTION_POINTER_DOWN) -> 0f
                        index == event.actionIndex && (action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_POINTER_UP) -> 2f
                        else -> 1f
                    }
                    val offset = index * 6
                    batch[offset] = phase + if (graph) 20f else 0f
                    batch[offset + 1] = event.getX(index) / density
                    batch[offset + 2] = event.getY(index) / density
                    batch[offset + 3] = event.getPointerId(index).toFloat()
                }
                timeline.input(batch, role)
            }
            true
        }
    val onInit: AndroidExternalSurfaceScope.() -> Unit = {
        onSurface { surface, width, height ->
            timeline.surface(role, surface, width, height, density)
            if (graph) timeline.visible(role, visible.value)
            surface.onChanged { w, h -> timeline.resize(role, w, h) }
            surface.onDestroyed { timeline.surface(role, null, 0, 0, density) }
        }
    }
    // A scrolling graph must be composited and clipped with its inspector ancestors.
    if (graph) AndroidEmbeddedExternalSurface(surfaceModifier.clipToBounds(), onInit = onInit)
    else AndroidExternalSurface(surfaceModifier, onInit = onInit)
}

@Composable
internal fun NativePreview(timeline: TimelineSession, modifier: Modifier) {
    val density = LocalDensity.current.density
    AndroidExternalSurface(modifier.previewInput(timeline)) {
        onSurface { surface, width, height ->
            timeline.surface(1, surface, width, height, density)
            surface.onChanged { w, h -> timeline.resize(1, w, h) }
            surface.onDestroyed { timeline.surface(1, null, 0, 0, density) }
        }
    }
}
