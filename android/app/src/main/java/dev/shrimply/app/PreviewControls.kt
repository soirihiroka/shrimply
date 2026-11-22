package dev.shrimply.app

import android.view.MotionEvent
import android.view.InputDevice
import android.view.KeyEvent
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.focusable
import androidx.compose.foundation.background
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.ui.graphics.Color
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.input.pointer.pointerInteropFilter
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.DpOffset
import org.json.JSONObject
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch

/** Native widgets bind Rust's preview presentation and opaque commands. */
@Composable
internal fun PreviewControls(timeline: TimelineSession, modifier: Modifier = Modifier, compact: Boolean = false, pendingOperation: String? = null) {
    val snapshot by timeline.snapshot
    val controls = snapshot.optJSONObject("preview_controls") ?: return
    if (!controls.getBoolean("visible")) return
    val guide = controls.getJSONObject("guide")
    val size = if (compact) 40.dp else 48.dp
    val scope = rememberCoroutineScope()
    var failure by remember { mutableStateOf<String?>(null) }
    Surface(modifier.width(size), color = MaterialTheme.colorScheme.surfaceContainerLow) {
        Column(Modifier.verticalScroll(rememberScrollState()), horizontalAlignment = Alignment.CenterHorizontally) {
            Box(Modifier.size(size), contentAlignment = Alignment.Center) {
                if (controls.getBoolean("loading") || pendingOperation != null) {
                    CircularProgressIndicator(Modifier.size(16.dp).semantics {
                        contentDescription = pendingOperation?.ifEmpty { controls.getString("loading_status") } ?: controls.getString("status")
                    }, strokeWidth = 2.dp)
                } else {
                    Icon(iconVector("check-plain-symbolic"), controls.getString("status"), Modifier.size(16.dp))
                }
            }
            Box(Modifier.size(size).semantics { contentDescription = controls.getString("frame_rate_label") }, contentAlignment = Alignment.Center) {
                Text(controls.getString("frame_rate"), fontFamily = FontFamily.Monospace, fontSize = 11.sp)
            }
            Box(Modifier.size(size).semantics { contentDescription = controls.getString("speed_label") }, contentAlignment = Alignment.Center) {
                Text(controls.getString("speed"), fontSize = 11.sp)
            }
            HorizontalDivider()
            IconToggleButton(checked = guide.getBoolean("active"), enabled = guide.getBoolean("enabled"), onCheckedChange = {
                val command = guide.getJSONObject("command")
                scope.launch {
                    try {
                        timeline.request(command)
                    } catch (error: Exception) { failure = error.message }
                }
            }, modifier = Modifier.size(size)) {
                Icon(iconVector("ruler-angled-symbolic"), guide.getString("label"), Modifier.size(16.dp))
            }
            controls.optJSONObject("paint")?.let { paint ->
                PreviewPaintControls(paint, size, controls.getString("close_label")) { command ->
                    scope.launch {
                        try { timeline.request(command) } catch (error: Exception) { failure = error.message }
                    }
                }
            }
        }
    }
    failure?.let { message ->
        AlertDialog(onDismissRequest = { failure = null }, text = { Text(message) }, confirmButton = {
            TextButton(onClick = { failure = null }) { Text(controls.getString("close_label")) }
        })
    }
}

/** Paint controls carry Rust labels, values and commands; native code only renders them. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PreviewPaintControls(paint: JSONObject, size: androidx.compose.ui.unit.Dp, closeLabel: String, send: (JSONObject) -> Unit) {
    val buttons = paint.getJSONArray("buttons")
    for (index in 0 until buttons.length()) {
        val control = buttons.getJSONObject(index)
        if (control.getBoolean("separator")) HorizontalDivider()
        val anchor: @Composable (open: () -> Unit) -> Unit = { open ->
            IconToggleButton(checked = control.getBoolean("active"),
                colors = IconButtonDefaults.iconToggleButtonColors(checkedContainerColor = MaterialTheme.colorScheme.secondaryContainer),
                onCheckedChange = { if (control.isNull("editor")) send(control.getJSONObject("command")) else open() },
                modifier = Modifier.size(size)) {
                when {
                    !control.isNull("color") -> {
                        val rgba = control.getJSONArray("color")
                        Box(Modifier.size(18.dp).background(Color(rgba.getDouble(0).toFloat(), rgba.getDouble(1).toFloat(),
                            rgba.getDouble(2).toFloat(), rgba.getDouble(3).toFloat()), CircleShape)
                            .semantics { contentDescription = control.getString("label") })
                    }
                    !control.isNull("icon") -> Icon(iconVector(control.getString("icon")), control.getString("label"), Modifier.size(16.dp))
                    else -> Text(control.getString("text"), fontSize = 11.sp,
                        modifier = Modifier.semantics { contentDescription = control.getString("label") })
                }
            }
        }
        if (control.isNull("editor")) anchor {} else AdaptiveMenuAnchor(anchor = anchor) { dismiss ->
            val field = control.getJSONObject("editor")
            var value by remember { mutableFloatStateOf(field.getDouble("value").toFloat()) }
            Column(Modifier.widthIn(min = 240.dp).padding(horizontal = 24.dp, vertical = 16.dp)) {
                Text(field.getString("label"), style = MaterialTheme.typography.titleMedium)
                Slider(value = value, onValueChange = { value = it },
                    valueRange = field.getDouble("minimum").toFloat()..field.getDouble("maximum").toFloat(),
                    steps = ((field.getDouble("maximum") - field.getDouble("minimum")) / field.getDouble("step")).toInt() - 1,
                    onValueChangeFinished = { send(JSONObject(control.getJSONObject("command").toString()).put("value", value)) })
                TextButton(onClick = dismiss, modifier = Modifier.align(Alignment.End)) { Text(closeLabel) }
            }
        }
    }
}

/** Native widgets display Rust's menu contract without interpreting its actions. */
@Composable
internal fun PreviewContextMenu(timeline: TimelineSession) {
    val snapshot by timeline.snapshot
    val menu = snapshot.optJSONObject("preview_context") ?: return
    val scope = rememberCoroutineScope()
    var failure by remember { mutableStateOf<String?>(null) }
    val send: (JSONObject) -> Unit = { command -> scope.launch {
        try { timeline.request(command) } catch (error: Exception) { failure = error.message }
    } }
    AdaptiveMenu(expanded = true, bottomSheet = menu.getBoolean("bottom_sheet"),
        dismiss = { send(menu.getJSONObject("dismiss")) }, atPoint = true,
        offset = DpOffset(menu.getDouble("x").dp, menu.getDouble("y").dp)) {
        val items = menu.getJSONArray("items")
        for (index in 0 until items.length()) {
            val item = items.getJSONObject(index)
            DropdownMenuItem(text = { Text(item.getString("label")) }, enabled = item.getBoolean("enabled"),
                trailingIcon = if (item.isNull("checked")) null else { { Checkbox(checked = item.getBoolean("checked"), onCheckedChange = null) } },
                onClick = { send(item.getJSONObject("command")) })
        }
    }
    failure?.let { message -> AlertDialog(onDismissRequest = { failure = null }, text = { Text(message) },
        confirmButton = { TextButton(onClick = { failure = null }) { Text(snapshot.getJSONObject("preview_controls").getString("close_label")) } }) }
}

/** Raw contacts and buttons enter Rust's shared touch/guide handlers. */
@OptIn(ExperimentalComposeUiApi::class)
internal fun Modifier.previewInput(timeline: TimelineSession): Modifier = composed {
    val density = LocalDensity.current.density
    val viewConfiguration = android.view.ViewConfiguration.get(LocalContext.current)
    val focus = remember { FocusRequester() }
    var size by remember { mutableStateOf(IntSize.Zero) }
    var pressedButtons by remember { mutableIntStateOf(0) }
    var wasFocused by remember { mutableStateOf(false) }
    DisposableEffect(timeline.identity) {
        onDispose { timeline.input(floatArrayOf(14f, 0f, 0f, 0f, 0f, 0f)) }
    }
    focusRequester(focus).onPreviewKeyEvent { key ->
        val event = key.nativeKeyEvent
        if (event.isSystem || event.keyCode == KeyEvent.KEYCODE_TAB) return@onPreviewKeyEvent false
        if (event.action != KeyEvent.ACTION_DOWN && event.action != KeyEvent.ACTION_UP) return@onPreviewKeyEvent false
        val flags = (if (event.isShiftPressed) 4 else 0) or (if (event.isCtrlPressed) 8 else 0) or
            (if (event.isAltPressed) 16 else 0) or (if (event.isMetaPressed) 32 else 0)
        timeline.input(floatArrayOf(42f, event.keyCode.toFloat(),
            event.getUnicodeChar(event.metaState and KeyEvent.META_CTRL_MASK.inv()).toFloat(), flags.toFloat(),
            if (event.action == KeyEvent.ACTION_UP) 1f else 0f, event.repeatCount.toFloat()))
        true
    }.onFocusChanged { state ->
        if (wasFocused && !state.isFocused) {
            pressedButtons = 0
            timeline.input(floatArrayOf(14f, 0f, 0f, 0f, 0f, 0f))
        }
        wasFocused = state.isFocused
    }.focusable().onSizeChanged { size = it }.pointerInteropFilter { event ->
        val action = event.actionMasked
        if (action == MotionEvent.ACTION_DOWN) focus.requestFocus()
        val tool = event.getToolType(0)
        val pen = tool == MotionEvent.TOOL_TYPE_STYLUS || tool == MotionEvent.TOOL_TYPE_ERASER
        val touchpad = event.isFromSource(InputDevice.SOURCE_TOUCHPAD)
        val pointer = event.isFromSource(InputDevice.SOURCE_MOUSE) || touchpad || pen
        if (pointer) {
            if (action == MotionEvent.ACTION_DOWN) pressedButtons = event.buttonState
            val buttons = if (action == MotionEvent.ACTION_UP) pressedButtons else event.buttonState
            val flags = (if (pen) 1 else 0) or (if (buttons and MotionEvent.BUTTON_TERTIARY != 0) 2 else 0) or
                (if (event.metaState and KeyEvent.META_SHIFT_MASK != 0) 4 else 0) or (if (event.metaState and KeyEvent.META_CTRL_MASK != 0) 8 else 0) or
                (if (event.metaState and KeyEvent.META_ALT_MASK != 0) 16 else 0) or (if (event.metaState and KeyEvent.META_META_MASK != 0) 32 else 0) or
                (if (touchpad) 64 else 0) or (if (tool == MotionEvent.TOOL_TYPE_ERASER) 128 else 0)
            val kind = when (action) {
                MotionEvent.ACTION_DOWN -> if (event.buttonState and (MotionEvent.BUTTON_SECONDARY or MotionEvent.BUTTON_STYLUS_PRIMARY) != 0) 19f else 11f
                MotionEvent.ACTION_MOVE -> 12f
                MotionEvent.ACTION_UP -> 13f
                MotionEvent.ACTION_CANCEL -> 14f
                MotionEvent.ACTION_SCROLL -> 31f
                MotionEvent.ACTION_HOVER_MOVE, MotionEvent.ACTION_HOVER_ENTER -> 32f
                MotionEvent.ACTION_HOVER_EXIT -> 33f
                else -> return@pointerInteropFilter false
            }
            if (pen && kind != 19f && kind != 31f) {
                // Split full native history into the JNI queue's bounded packets.
                val historyCount = if (action == MotionEvent.ACTION_MOVE) event.historySize else 0
                var start = 0
                while (start <= historyCount) {
                    val count = minOf(MAX_PREVIEW_PEN_SAMPLES, historyCount + 1 - start)
                    val batch = FloatArray(count * PREVIEW_PEN_STRIDE)
                    for (index in 0 until count) {
                        val at = start + index
                        val historical = at < historyCount
                        val offset = index * PREVIEW_PEN_STRIDE
                        val time = if (historical) event.getHistoricalEventTime(at) else event.eventTime
                        batch[offset] = if (historical) 37f else when (kind) { 32f -> 40f; 33f -> 41f; else -> kind + 25f }
                        batch[offset + 1] = (if (historical) event.getHistoricalX(at) else event.x) / density
                        batch[offset + 2] = (if (historical) event.getHistoricalY(at) else event.y) / density
                        batch[offset + 3] = if (historical) event.getHistoricalPressure(at) else event.pressure
                        batch[offset + 4] = if (historical) event.getHistoricalAxisValue(MotionEvent.AXIS_TILT, at) else event.getAxisValue(MotionEvent.AXIS_TILT)
                        batch[offset + 5] = if (historical) event.getHistoricalOrientation(at) else event.orientation
                        batch[offset + 6] = size.width / density
                        batch[offset + 7] = size.height / density
                        batch[offset + 8] = flags.toFloat()
                        batch[offset + 9] = (time and 0xffffL).toFloat()
                        batch[offset + 10] = ((time ushr 16) and 0xffffL).toFloat()
                    }
                    timeline.input(batch)
                    start += count
                }
            } else {
                timeline.input(floatArrayOf(kind, event.x / density, event.y / density,
                    if (action == MotionEvent.ACTION_SCROLL) event.getAxisValue(MotionEvent.AXIS_HSCROLL) * (if (touchpad) viewConfiguration.scaledHorizontalScrollFactor / density else 1f) else size.width / density,
                    if (action == MotionEvent.ACTION_SCROLL) -event.getAxisValue(MotionEvent.AXIS_VSCROLL) * (if (touchpad) viewConfiguration.scaledVerticalScrollFactor / density else 1f) else size.height / density,
                    flags.toFloat()))
            }
        } else {
            val batch = FloatArray(event.pointerCount * 6)
            for (index in 0 until event.pointerCount) {
                val phase = when {
                    action == MotionEvent.ACTION_CANCEL -> 18f
                    index == event.actionIndex && (action == MotionEvent.ACTION_DOWN || action == MotionEvent.ACTION_POINTER_DOWN) -> 15f
                    index == event.actionIndex && (action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_POINTER_UP) -> 17f
                    else -> 16f
                }
                val offset = index * 6
                batch[offset] = phase
                batch[offset + 1] = event.getX(index) / density
                batch[offset + 2] = event.getY(index) / density
                batch[offset + 3] = event.getPointerId(index).toFloat()
                batch[offset + 4] = size.width / density
                batch[offset + 5] = size.height / density
            }
            timeline.input(batch)
        }
        true
    }
}

// Twelve values per sample, bounded by the native queue's 192-value packet limit.
private const val PREVIEW_PEN_STRIDE = 12
private const val MAX_PREVIEW_PEN_SAMPLES = 16
