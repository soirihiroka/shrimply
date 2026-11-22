package dev.shrimply.app

import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.PointerType
import androidx.compose.ui.input.pointer.PointerIcon
import androidx.compose.ui.input.pointer.pointerHoverIcon
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.launch
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.isActive
import kotlinx.coroutines.withContext
import org.json.JSONObject

/** Native fields render Rust values and forward raw input; Rust owns all math. */
@Composable
internal fun NumberInput(timeline: TimelineSession, control: JSONObject, enabled: Boolean) {
    val owner = control.getJSONObject("command").toString()
    val events = remember(timeline.identity, owner) { Channel<JSONObject>(Channel.UNLIMITED) }
    val command = remember(timeline.identity, owner) { control.getJSONObject("command") }
    var response by remember(owner) { mutableStateOf<JSONObject?>(null) }
    var failure by remember(owner) { mutableStateOf<String?>(null) }
    val captured by rememberUpdatedState(response?.optBoolean("capture") == true)
    val density = LocalDensity.current.density
    val context = LocalContext.current
    val resizeCursor = remember(context) { PointerIcon(android.view.PointerIcon.getSystemIcon(context, android.view.PointerIcon.TYPE_HORIZONTAL_DOUBLE_ARROW)) }
    val model = response?.optJSONObject("number") ?: control
    val fields = model.getJSONArray("fields")
    val editingAxis = response?.takeUnless { it.isNull("editing_axis") }?.getInt("editing_axis")
    var draft by remember(owner) { mutableStateOf(TextFieldValue()) }
    val focus = remember { FocusRequester() }

    LaunchedEffect(control.getJSONArray("fields").toString(), control.optJSONObject("lock")?.toString(), control.optBoolean("interaction_active")) {
        if (!control.optBoolean("interaction_active")) response = null
    }
    LaunchedEffect(enabled) {
        if (!enabled) events.send(JSONObject().put("event", "cancel"))
    }
    LaunchedEffect(editingAxis) {
        if (editingAxis != null) {
            val value = fields.getJSONObject(editingAxis).getString("edit_value")
            draft = TextFieldValue(value, TextRange(0, value.length))
            focus.requestFocus()
        }
    }
    suspend fun execute(event: JSONObject): JSONObject {
        val payload = JSONObject(command.toString())
        event.keys().forEach { payload.put(it, event.get(it)) }
        return timeline.request(payload)
    }
    suspend fun receive(event: JSONObject) {
        try { response = execute(event); failure = null }
        catch (cancelled: CancellationException) { throw cancelled }
        catch (error: Exception) { response = null; failure = error.message }
    }
    val scope = rememberCoroutineScope()
    // Queue text before the next native focus/selection event, without awaiting each keystroke.
    val submit: (JSONObject) -> Unit = { event -> scope.launch(start = CoroutineStart.UNDISPATCHED) { receive(event) } }
    LaunchedEffect(timeline.identity, owner) {
        var pending: JSONObject? = null
        try {
            while (isActive) {
                var event = pending ?: events.receive()
                pending = null
                // Replace consecutive motion snapshots while preserving edges.
                while (event.optString("event") == "move") {
                    val next = events.tryReceive().getOrNull() ?: break
                    if (next.optString("event") == "move" && next.optInt("contacts") == event.optInt("contacts")) event = next else { pending = next; break }
                }
                receive(event)
            }
        } finally {
            withContext(NonCancellable) {
                try {
                    pending?.let { execute(it) }
                    while (true) execute(events.tryReceive().getOrNull() ?: break)
                } catch (_: Exception) { /* Rust reports conflicting project edits. */ }
                finally {
                    try { execute(JSONObject().put("event", "release")) }
                    catch (_: Exception) { /* The owning project may already be closed. */ }
                }
            }
        }
    }
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        if (control.getString("label").isNotEmpty()) Text(control.getString("label"), style = MaterialTheme.typography.labelMedium)
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            model.optJSONObject("lock")?.let { lock ->
                IconToggleButton(checked = lock.getBoolean("checked"), enabled = enabled,
                    onCheckedChange = { events.trySend(JSONObject().put("event", "lock")) }, modifier = Modifier.size(40.dp)) {
                    Icon(iconVector(lock.getString("icon")), lock.getString("label"), Modifier.size(16.dp))
                }
            }
            for (axis in 0 until fields.length()) {
                val field = fields.getJSONObject(axis)
                Surface(Modifier.weight(1f), color = MaterialTheme.colorScheme.surfaceContainerHigh, contentColor = if (enabled) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant, shape = MaterialTheme.shapes.small) {
                    if (editingAxis == axis) {
                        var hadFocus by remember(owner, axis) { mutableStateOf(false) }
                        BasicTextField(value = draft, onValueChange = {
                            draft = it
                            submit(JSONObject().put("event", "text").put("axis", axis).put("text", it.text))
                        }, enabled = enabled, singleLine = true,
                            cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                            textStyle = MaterialTheme.typography.bodyMedium.copy(color = MaterialTheme.colorScheme.onSurface),
                            keyboardOptions = KeyboardOptions(keyboardType = if (field.getString("keyboard") == "decimal") KeyboardType.Decimal else KeyboardType.Text, imeAction = ImeAction.Done),
                            keyboardActions = KeyboardActions(onDone = { submit(JSONObject().put("event", "commit").put("axis", axis).put("text", draft.text)) }),
                            modifier = Modifier.fillMaxWidth().heightIn(min = 40.dp).padding(8.dp).focusRequester(focus)
                                .onFocusChanged { state ->
                                    if (hadFocus && !state.isFocused) submit(JSONObject().put("event", "commit").put("axis", axis).put("text", draft.text))
                                    hadFocus = state.isFocused
                                }.onPreviewKeyEvent {
                                    if (it.type == KeyEventType.KeyDown && it.key == Key.Escape) {
                                        events.trySend(JSONObject().put("event", "cancel")); true
                                    } else false
                                })
                    } else Row(Modifier.fillMaxWidth().heightIn(min = 40.dp).pointerHoverIcon(resizeCursor)
                        .semantics { if (enabled) onClick { events.trySend(JSONObject().put("event", "edit").put("axis", axis)); true } else disabled() }
                        .onPreviewKeyEvent {
                            if (enabled && it.type == KeyEventType.KeyDown && (it.key == Key.Enter || it.key == Key.NumPadEnter || it.key == Key.Spacebar)) {
                                events.trySend(JSONObject().put("event", "edit").put("axis", axis)); true
                            } else false
                        }.focusable(enabled)
                        .pointerInput(timeline.identity, owner, axis, enabled, density) {
                            if (!enabled) return@pointerInput
                            awaitEachGesture {
                                val down = awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial)
                                events.trySend(JSONObject().put("event", "begin").put("axis", axis)
                                    .put("x", down.position.x / density).put("y", down.position.y / density)
                                    .put("touch", down.type == PointerType.Touch).put("contacts", 1))
                                try {
                                    while (true) {
                                        val event = awaitPointerEvent(PointerEventPass.Initial)
                                        val change = event.changes.firstOrNull { it.id == down.id }
                                        if (change == null) { events.trySend(JSONObject().put("event", "cancel")); break }
                                        events.trySend(JSONObject().put("event", if (change.pressed) "move" else "end")
                                            .put("x", change.position.x / density).put("y", change.position.y / density)
                                            .put("contacts", event.changes.count { it.pressed }))
                                        if (captured) change.consume()
                                        if (!change.pressed) break
                                    }
                                } finally {
                                    // Native cancellation and disposal never retain a provisional edit.
                                    if (currentEvent.changes.any { it.id == down.id && it.pressed }) events.trySend(JSONObject().put("event", "cancel"))
                                }
                            }
                        }.padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                        if (field.getString("icon").isNotEmpty()) Icon(iconVector(field.getString("icon")), null,
                            Modifier.size(16.dp).graphicsLayer { rotationZ = field.getDouble("icon_rotation").toFloat() })
                        if (field.getString("prefix").isNotEmpty()) Text(field.getString("prefix"), color = MaterialTheme.colorScheme.onSurfaceVariant)
                        Text(field.getString("display"), Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyMedium.copy(fontFeatureSettings = "tnum"))
                        if (field.getString("suffix").isNotEmpty()) Text(field.getString("suffix"), color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
        }
        val message = failure ?: response?.optString("validation_message").orEmpty()
        if (message.isNotEmpty()) Text(message, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
    }
}
