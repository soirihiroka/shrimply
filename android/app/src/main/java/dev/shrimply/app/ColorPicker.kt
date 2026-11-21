package dev.shrimply.app

import android.content.Context
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.clickable
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.RoundRect
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.clipPath
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.setProgress
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import org.json.JSONArray
import org.json.JSONObject

private val ColorDialogWidth = 880.dp
private val ColorWideWidth = 780.dp
private val ColorSelectionWidth = 352.dp
private val ColorBarWidth = 40.dp

/** The native view renders the same Rust picker state used by GTK. */
@Composable
internal fun ColorInput(control: JSONObject, enabled: Boolean, send: (JSONObject) -> Unit) {
    val context = LocalContext.current
    val storage = remember(context) { context.getSharedPreferences("color-picker", Context.MODE_PRIVATE) }
    var picker by remember { mutableStateOf<JSONObject?>(null) }
    val input = LocalMenuInputSource.current
    var bottomSheet by remember { mutableStateOf(false) }
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(control.getString("label"), Modifier.weight(1f), style = MaterialTheme.typography.labelMedium)
        FilledTonalButton(enabled = enabled, onClick = {
            bottomSheet = NativeLayout.menuUsesBottomSheet(input.value)
            val request = JSONObject(control.getJSONObject("picker").toString())
                .put("recent", JSONArray(storage.getString("recent", "[]")))
            picker = JSONObject(NativeColor.request(request.toString()))
        }, contentPadding = PaddingValues(horizontal = 10.dp, vertical = 6.dp)) {
            Canvas(Modifier.size(22.dp).clip(RoundedCornerShape(4.dp))) {
                checkerboard(control)
                drawRect(Color(control.getLong("argb").toInt()))
            }
            Spacer(Modifier.width(8.dp))
            Text(control.getString("display"), fontFamily = FontFamily.Monospace, maxLines = 1)
        }
    }
    picker?.let { initial ->
        ColorPicker(initial, bottomSheet, dismiss = { picker = null }) { result ->
            storage.edit().putString("recent", result.getJSONObject("state").getJSONArray("recent").toString()).apply()
            send(JSONObject(control.getJSONObject("command").toString()).put("components", result.getJSONArray("values")))
            picker = null
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ColorPicker(initial: JSONObject, bottomSheet: Boolean, dismiss: () -> Unit, confirm: (JSONObject) -> Unit) {
    var model by remember(initial) { mutableStateOf(initial) }
    var hex by remember(initial) { mutableStateOf(initial.getString("hex")) }
    var screenError by remember { mutableStateOf<String?>(null) }
    val update: (JSONObject) -> Unit = { event ->
        model = JSONObject(NativeColor.request(event.put("state", model.getJSONObject("state")).toString()))
        if (event.getString("action") != "hex") hex = model.getString("hex")
    }
    val labels = model.getJSONObject("labels")
    val content: @Composable () -> Unit = {
        val pickScreen = rememberScreenColorPicker(labels, { color ->
            screenError = null
            update(JSONObject().put("action", "argb").put("value", color.toUInt().toLong()))
        }, { screenError = it })
        Column(Modifier.fillMaxWidth()) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = pickScreen) {
                    Icon(painterResource(R.drawable.color_select), labels.getString("pick"), Modifier.size(20.dp))
                }
                Text(labels.getString("title"), Modifier.weight(1f), style = MaterialTheme.typography.titleLarge)
                IconButton(onClick = dismiss) {
                    Icon(painterResource(R.drawable.window_close), labels.getString("cancel"), Modifier.size(20.dp))
                }
            }
            screenError?.let { Text(it, Modifier.padding(horizontal = 16.dp), color = MaterialTheme.colorScheme.error) }
            BoxWithConstraints(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(16.dp)) {
                val selection: @Composable () -> Unit = {
                    ColorSelection(model, hex, { hex = it; update(JSONObject().put("action", "hex").put("text", it)) }, update)
                }
                val palette: @Composable () -> Unit = {
                    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        ColorPalette(model, update)
                        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                            FilledIconButton(onClick = {
                                val result = JSONObject(NativeColor.request(JSONObject().put("state", model.getJSONObject("state")).put("action", "confirm").toString()))
                                check(result.getBoolean("confirmed"))
                                confirm(result)
                            }) { Icon(painterResource(R.drawable.check_plain), labels.getString("confirm"), Modifier.size(20.dp)) }
                        }
                    }
                }
                if (maxWidth >= ColorWideWidth) Row(horizontalArrangement = Arrangement.spacedBy(24.dp)) {
                    Box(Modifier.width(ColorSelectionWidth)) { selection() }
                    VerticalDivider(Modifier.height(model.getJSONObject("metrics").getDouble("plane").dp + 96.dp))
                    Box(Modifier.weight(1f)) { palette() }
                } else Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.TopCenter) { selection() }
                    HorizontalDivider()
                    palette()
                }
            }
        }
    }
    if (bottomSheet) ModalBottomSheet(onDismissRequest = dismiss, sheetMaxWidth = ColorDialogWidth, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Box(Modifier.navigationBarsPadding()) { content() }
    } else Dialog(onDismissRequest = dismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.padding(16.dp).widthIn(max = ColorDialogWidth).fillMaxWidth().heightIn(max = 640.dp),
            shape = MaterialTheme.shapes.extraLarge, tonalElevation = 6.dp) { content() }
    }
}

@Composable
private fun ColorSelection(model: JSONObject, hex: String, editHex: (String) -> Unit, update: (JSONObject) -> Unit) {
    val labels = model.getJSONObject("labels")
    val metrics = model.getJSONObject("metrics")
    val error = model.optString("error").takeUnless { it.isEmpty() || it == "null" }
    Column(Modifier.widthIn(max = ColorSelectionWidth).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Canvas(Modifier.size(32.dp).clip(RoundedCornerShape(6.dp))) {
                checkerboard(metrics)
                drawRect(Color(model.getLong("argb").toInt()))
            }
            OutlinedTextField(hex, editHex, Modifier.weight(1f), singleLine = true,
                label = { Text(labels.getString("hex")) }, isError = error != null,
                supportingText = error?.let { message -> { Text(message) } },
                textStyle = MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace))
        }
        BoxWithConstraints(Modifier.fillMaxWidth()) {
            val planeSize = (maxWidth - ColorBarWidth - 10.dp).coerceAtMost(metrics.getDouble("plane").dp)
            Row(Modifier.height(planeSize), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                val hueStops = model.getJSONArray("hue_stops")
                val stops = (0 until hueStops.length()).map { index ->
                    val stop = hueStops.getJSONArray(index)
                    stop.getDouble(0).toFloat() to Color(stop.getLong(1).toInt())
                }.toTypedArray()
                ColorBar(labels.getString("hue"), "hue", model.getDouble("hue_position").toFloat(),
                    Modifier.width(ColorBarWidth).fillMaxHeight(), true, metrics, update) { start, end -> Brush.linearGradient(*stops, start = start, end = end) }
                val position = model.getJSONArray("position")
                val gradients = model.getJSONObject("plane_stops")
                val vertical = gradients.getJSONArray("vertical").colors()
                val horizontal = gradients.getJSONArray("horizontal").colors()
                Canvas(Modifier.weight(1f).fillMaxHeight().clip(RoundedCornerShape(6.dp))
                    .colorKeys("plane", update)
                    .semantics { contentDescription = labels.getString("plane") }
                    .colorPointer { point, bounds ->
                        update(JSONObject().put("action", "plane")
                            .put("x", (point.x / bounds.width).coerceIn(0f, 1f))
                            .put("y", (point.y / bounds.height).coerceIn(0f, 1f)))
                    }) {
                    drawRect(Brush.verticalGradient(vertical))
                    drawRect(Brush.horizontalGradient(horizontal))
                    selectionMarker(Offset(size.width * position.getDouble(0).toFloat(), size.height * position.getDouble(1).toFloat()), metrics.getDouble("thumb").dp.toPx())
                }
            }
        }
        if (model.getJSONObject("state").getBoolean("with_alpha")) Row {
            Spacer(Modifier.width(ColorBarWidth + 10.dp))
            val colors = model.getJSONArray("alpha_stops").colors()
            ColorBar(labels.getString("alpha"), "alpha", model.getDouble("alpha_position").toFloat(),
                Modifier.weight(1f).height(ColorBarWidth), false, metrics, update, checker = true) { start, end -> Brush.linearGradient(colors, start, end) }
        }
    }
}

@Composable
private fun ColorBar(label: String, target: String, position: Float, modifier: Modifier, vertical: Boolean, metrics: JSONObject,
    update: (JSONObject) -> Unit, checker: Boolean = false, brush: (Offset, Offset) -> Brush) {
    val inset = with(LocalDensity.current) { metrics.getDouble("thumb").dp.toPx() }
    Canvas(modifier.colorKeys(target, update).semantics {
        contentDescription = label
        progressBarRangeInfo = ProgressBarRangeInfo(position, 0f..1f)
        setProgress { update(JSONObject().put("action", target).put("position", it)); true }
    }.colorPointer { point, bounds ->
        update(JSONObject().put("action", target).put("position",
            if (vertical) ((point.y - inset) / (bounds.height - inset * 2)).coerceIn(0f, 1f)
            else ((point.x - inset) / (bounds.width - inset * 2)).coerceIn(0f, 1f)))
    }) {
        val track = metrics.getDouble("track").dp.toPx()
        val start = if (vertical) Offset(size.width / 2, inset) else Offset(inset, size.height / 2)
        val end = if (vertical) Offset(size.width / 2, size.height - inset) else Offset(size.width - inset, size.height / 2)
        val topLeft = if (vertical) Offset((size.width - track) / 2, inset) else Offset(inset, (size.height - track) / 2)
        val trackSize = if (vertical) Size(track, size.height - inset * 2) else Size(size.width - inset * 2, track)
        val path = Path().apply { addRoundRect(RoundRect(Rect(topLeft, trackSize), CornerRadius(track / 2))) }
        clipPath(path) {
            if (checker) checkerboard(metrics, topLeft, trackSize)
            drawRect(brush(start, end), topLeft, trackSize)
        }
        selectionMarker(start + (end - start) * position, inset)
    }
}

@Composable
private fun ColorPalette(model: JSONObject, update: (JSONObject) -> Unit) {
    val palette = model.getJSONArray("palette").objects()
    val recent = model.getJSONArray("recent").objects()
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        FlowRow(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            palette.groupBy { it.getInt("column") }.toSortedMap().values.forEach { column ->
                Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    column.sortedBy { it.getInt("row") }.forEach { color -> ColorSwatch(color, model.getJSONObject("metrics"), update, Modifier.size(36.dp)) }
                }
            }
        }
        if (recent.isNotEmpty()) {
            Text(model.getJSONObject("labels").getString("recent"), style = MaterialTheme.typography.labelMedium)
            FlowRow(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                recent.forEach { color -> ColorSwatch(color, model.getJSONObject("metrics"), update, Modifier.size(36.dp)) }
            }
        }
    }
}

@Composable
private fun ColorSwatch(color: JSONObject, metrics: JSONObject, update: (JSONObject) -> Unit, modifier: Modifier) {
    val selected = color.getBoolean("selected")
    Box(modifier.clip(RoundedCornerShape(4.dp))
        .semantics { contentDescription = color.getString("label"); this.selected = selected }
        .clickable { update(JSONObject().put("action", "color").put("color", color.getString("hex"))) }, contentAlignment = Alignment.Center) {
        Canvas(Modifier.fillMaxSize()) { checkerboard(metrics); drawRect(Color(color.getLong("argb").toInt())) }
        if (selected) {
            Icon(painterResource(R.drawable.check_plain), null, Modifier.size(18.dp), tint = Color.Black)
            Icon(painterResource(R.drawable.check_plain), null, Modifier.size(16.dp), tint = Color.White)
        }
    }
}

/** Forward native pointer positions; color interpretation remains in Rust. */
@Composable
private fun Modifier.colorPointer(update: (Offset, IntSize) -> Unit): Modifier {
    val currentUpdate by rememberUpdatedState(update)
    return pointerInput(Unit) { awaitEachGesture {
        val down = awaitFirstDown()
        currentUpdate(down.position, size)
        down.consume()
        do {
            val change = awaitPointerEvent().changes.firstOrNull { it.id == down.id } ?: break
            currentUpdate(change.position, size)
            change.consume()
        } while (change.pressed)
    } }
}

private fun Modifier.colorKeys(target: String, update: (JSONObject) -> Unit): Modifier = onKeyEvent { event ->
    val key = when (event.key) { Key.DirectionLeft -> "left"; Key.DirectionRight -> "right"; Key.DirectionUp -> "up"; Key.DirectionDown -> "down"; else -> null }
    if (event.type != KeyEventType.KeyDown || key == null) false
    else { update(JSONObject().put("action", "key").put("target", target).put("key", key)); true }
}.focusable()

private fun JSONArray.colors(): List<Color> = (0 until length()).map { Color(getLong(it).toInt()) }

private fun DrawScope.selectionMarker(center: Offset, radius: Float) {
    drawCircle(Color.Black, radius, center, style = Stroke(3.dp.toPx()))
    drawCircle(Color.White, radius, center, style = Stroke(1.5.dp.toPx()))
}

private fun DrawScope.checkerboard(metrics: JSONObject, origin: Offset = Offset.Zero, area: Size = size) {
    val cell = metrics.getDouble("checker").dp.toPx()
    val dark = metrics.getDouble("checker_dark").toFloat()
    val light = metrics.getDouble("checker_light").toFloat()
    drawRect(Color(dark, dark, dark), origin, area)
    for (y in 0..(area.height / cell).toInt()) for (x in 0..(area.width / cell).toInt()) {
        if ((x + y) % 2 == 0) drawRect(Color(light, light, light), origin + Offset(x * cell, y * cell),
            Size(cell.coerceAtMost(area.width - x * cell), cell.coerceAtMost(area.height - y * cell)))
    }
}
