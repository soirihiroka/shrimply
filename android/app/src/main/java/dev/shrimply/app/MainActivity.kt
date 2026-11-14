package dev.shrimply.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.AndroidExternalSurface
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
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
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider

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
        setContent {
            MaterialTheme(colorScheme = darkColorScheme(primary = Color(0xff80b8ff))) {
                Surface(Modifier.fillMaxSize()) {
                    if (benchmark) NativeSurface(timeline, Modifier.fillMaxSize())
                    else ProjectApp(timeline, projects)
                }
            }
        }
    }
    override fun onResume() { super.onResume(); timeline.active(true) }
    override fun onPause() { timeline.active(false); super.onPause() }
}

@OptIn(ExperimentalComposeUiApi::class)
@Composable
internal fun NativeSurface(timeline: TimelineSession, modifier: Modifier) {
    val density = LocalDensity.current.density
    val viewConfiguration = android.view.ViewConfiguration.get(LocalContext.current)
    val focus = remember { FocusRequester() }
    val mouseButton = remember { intArrayOf(0) }
    AndroidExternalSurface(modifier
        .focusRequester(focus)
        .onPreviewKeyEvent { key ->
            val event = key.nativeKeyEvent
            if (event.action != KeyEvent.ACTION_DOWN || event.isAltPressed || event.isMetaPressed) return@onPreviewKeyEvent false
            val character = when (event.keyCode) {
                KeyEvent.KEYCODE_ESCAPE -> 27
                KeyEvent.KEYCODE_DEL -> 8
                KeyEvent.KEYCODE_FORWARD_DEL -> 127
                else -> event.getUnicodeChar(event.metaState and KeyEvent.META_CTRL_MASK.inv())
            }
            if (character == 0) return@onPreviewKeyEvent false
            val flags = (if (event.metaState and KeyEvent.META_CTRL_MASK != 0) 1 else 0) or (if (event.metaState and KeyEvent.META_SHIFT_MASK != 0) 2 else 0)
            timeline.input(floatArrayOf(8f, 0f, 0f, character.toFloat(), flags.toFloat(), 0f))
        }
        .focusable()
        .pointerInteropFilter { event ->
            val action = event.actionMasked
            if (action == MotionEvent.ACTION_DOWN) focus.requestFocus()
            val mouse = event.isFromSource(InputDevice.SOURCE_MOUSE) || event.getToolType(0) == MotionEvent.TOOL_TYPE_STYLUS
            val flags = (if (event.metaState and KeyEvent.META_CTRL_MASK != 0) 1 else 0) or (if (event.metaState and KeyEvent.META_SHIFT_MASK != 0) 2 else 0)
            if (mouse || action == MotionEvent.ACTION_SCROLL) {
                val kind = when (action) {
                    MotionEvent.ACTION_DOWN -> {
                        mouseButton[0] = when {
                            event.buttonState and MotionEvent.BUTTON_SECONDARY != 0 -> 3
                            event.buttonState and MotionEvent.BUTTON_TERTIARY != 0 -> 1
                            else -> 0
                        }
                        if (mouseButton[0] == 3) 10f else 5f
                    }
                    MotionEvent.ACTION_UP -> if (mouseButton[0] == 3) return@pointerInteropFilter true else 6f
                    MotionEvent.ACTION_MOVE, MotionEvent.ACTION_HOVER_MOVE, MotionEvent.ACTION_HOVER_ENTER -> 4f
                    MotionEvent.ACTION_SCROLL -> 7f
                    MotionEvent.ACTION_HOVER_EXIT -> 9f
                    MotionEvent.ACTION_CANCEL -> 3f
                    else -> return@pointerInteropFilter true
                }
                val touchpad = event.isFromSource(InputDevice.SOURCE_TOUCHPAD)
                val horizontal = event.getAxisValue(MotionEvent.AXIS_HSCROLL) * (if (touchpad) viewConfiguration.scaledHorizontalScrollFactor / density else 1f)
                val vertical = -event.getAxisValue(MotionEvent.AXIS_VSCROLL) * (if (touchpad) viewConfiguration.scaledVerticalScrollFactor / density else 1f)
                timeline.input(floatArrayOf(kind, event.x / density, event.y / density, if (kind == 7f) horizontal else mouseButton[0].toFloat(), (flags or (if (touchpad) 4 else 0)).toFloat(), vertical))
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
                    batch[offset] = phase
                    batch[offset + 1] = event.getX(index) / density
                    batch[offset + 2] = event.getY(index) / density
                    batch[offset + 3] = event.getPointerId(index).toFloat()
                }
                timeline.input(batch)
            }
            true
        }) {
        onSurface { surface, width, height ->
            timeline.surface(0, surface, width, height, density)
            surface.onChanged { w, h -> timeline.resize(0, w, h) }
            surface.onDestroyed { timeline.surface(0, null, 0, 0, density) }
        }
    }
}

@Composable
internal fun NativePreview(timeline: TimelineSession, modifier: Modifier) {
    val density = LocalDensity.current.density
    AndroidExternalSurface(modifier) {
        onSurface { surface, width, height ->
            timeline.surface(1, surface, width, height, density)
            surface.onChanged { w, h -> timeline.resize(1, w, h) }
            surface.onDestroyed { timeline.surface(1, null, 0, 0, density) }
        }
    }
}
