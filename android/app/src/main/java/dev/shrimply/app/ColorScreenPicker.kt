package dev.shrimply.app

import android.app.Activity
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.media.projection.MediaProjectionManager
import android.os.Handler
import android.os.Looper
import android.view.View
import android.view.WindowManager
import androidx.activity.compose.LocalActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.key.onKeyEvent
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.DialogWindowProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.File
import java.util.UUID

/** Android consent, screenshot transport and pixel access; color interpretation stays in Rust. */
@Composable
internal fun rememberScreenColorPicker(labels: JSONObject, onColor: (Int) -> Unit, onError: (String) -> Unit): () -> Unit {
    val context = LocalContext.current
    val activity = checkNotNull(LocalActivity.current)
    val view = LocalView.current
    val keyboard = LocalSoftwareKeyboardController.current
    val root = view.rootView
    val window = (view.parent as? DialogWindowProvider)?.window
    val dimmed = remember(window) { window?.attributes?.flags?.and(WindowManager.LayoutParams.FLAG_DIM_BEHIND) != 0 }
    val show: (Boolean) -> Unit = { visible ->
        root.visibility = if (visible) View.VISIBLE else View.INVISIBLE
        if (visible && dimmed) window?.addFlags(WindowManager.LayoutParams.FLAG_DIM_BEHIND)
        else window?.clearFlags(WindowManager.LayoutParams.FLAG_DIM_BEHIND)
    }
    val owner = remember { UUID.randomUUID().toString() }
    val main = remember { Handler(Looper.getMainLooper()) }
    val scope = rememberCoroutineScope()
    val currentColor by rememberUpdatedState(onColor)
    val currentError by rememberUpdatedState(onError)
    var pending by remember { mutableStateOf<JSONObject?>(null) }
    var bitmap by remember { mutableStateOf<Bitmap?>(null) }
    val receive: (JSONObject) -> Unit = { event -> main.post {
        scope.launch {
            ScreenRecordingService.acknowledge(event.getString("id"))
            val config = pending
            pending = null
            show(true)
            try {
                if (event.getString("event") == "finished") bitmap = withContext(Dispatchers.IO) {
                    checkNotNull(BitmapFactory.decodeFile(event.getString("path")))
                }
                else if (event.getString("event") == "failed") currentError(event.getString("error"))
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (error: Exception) { currentError(error.message ?: labels.getString("sample_error")) }
            finally { config?.let { File(it.getString("path")).delete() } }
        }
    } }
    val consent = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        val config = pending
        if (config != null && result.resultCode == Activity.RESULT_OK && result.data != null) {
            ScreenRecordingService.start(context, owner, config, result.resultCode, result.data!!, receive)
        } else {
            pending = null
            show(true)
            config?.let { File(it.getString("path")).delete() }
        }
    }
    DisposableEffect(owner) {
        onDispose {
            show(true)
            ScreenRecordingService.stopOwner(owner)
            pending?.let { File(it.getString("path")).delete() }
        }
    }
    bitmap?.let { image ->
        var position by remember(image) { mutableStateOf(Offset(0.5f, 0.5f)) }
        var pixel by remember(image) { mutableStateOf(listOf(image.width / 2, image.height / 2)) }
        val focus = remember(image) { FocusRequester() }
        val select: (Offset, String?) -> Unit = { point, key ->
            val request = JSONObject().put("action", "sample").put("width", image.width).put("height", image.height)
                .put("x", point.x.toDouble()).put("y", point.y.toDouble())
            key?.let { request.put("key", it) }
            val selected = JSONObject(NativeColor.request(request.toString()))
            val coordinates = selected.getJSONArray("pixel")
            pixel = listOf(coordinates.getInt(0), coordinates.getInt(1))
            val normalized = selected.getJSONArray("position")
            position = Offset(normalized.getDouble(0).toFloat(), normalized.getDouble(1).toFloat())
        }
        val choose = { currentColor(image.getPixel(pixel[0], pixel[1])); bitmap = null }
        Dialog(onDismissRequest = { bitmap = null }, properties = DialogProperties(usePlatformDefaultWidth = false)) {
            Surface(Modifier.fillMaxSize()) {
                Column {
                    Row(Modifier.fillMaxWidth().padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text(labels.getString("pick"), Modifier.weight(1f), style = MaterialTheme.typography.titleMedium)
                        IconButton(onClick = { bitmap = null }) {
                            Icon(painterResource(R.drawable.window_close), labels.getString("cancel"))
                        }
                    }
                    BoxWithConstraints(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                        val ratio = image.width.toFloat() / image.height
                        val width = minOf(maxWidth, maxHeight * ratio)
                        Box(Modifier.size(width, width / ratio).focusRequester(focus)
                            .onKeyEvent { event ->
                                if (event.type != KeyEventType.KeyDown) false
                                else when (event.key) {
                                    Key.DirectionLeft -> { select(position, "left"); true }
                                    Key.DirectionRight -> { select(position, "right"); true }
                                    Key.DirectionUp -> { select(position, "up"); true }
                                    Key.DirectionDown -> { select(position, "down"); true }
                                    Key.Enter, Key.NumPadEnter, Key.DirectionCenter, Key.Spacebar -> { choose(); true }
                                    else -> false
                                }
                            }.focusable().semantics(mergeDescendants = true) {
                                contentDescription = labels.getString("pick")
                                onClick(labels.getString("confirm")) { choose(); true }
                                customActions = listOf("left", "right", "up", "down").map { direction ->
                                    CustomAccessibilityAction(labels.getString(direction)) { select(position, direction); true }
                                }
                            }.pointerInput(image) {
                                detectTapGestures { point ->
                                    select(Offset(point.x / size.width, point.y / size.height), null)
                                    choose()
                                }
                            }) {
                            Image(image.asImageBitmap(), null, Modifier.fillMaxSize())
                            Canvas(Modifier.fillMaxSize()) {
                                val center = Offset(position.x * size.width, position.y * size.height)
                                drawCircle(Color.Black, 7.dp.toPx(), center, style = Stroke(3.dp.toPx()))
                                drawCircle(Color.White, 7.dp.toPx(), center, style = Stroke(1.dp.toPx()))
                            }
                            LaunchedEffect(image) { focus.requestFocus() }
                        }
                    }
                }
            }
        }
    }
    return {
        // A new projection grant can revoke an existing recording before the service sees it.
        if (ScreenRecordingService.busy()) currentError(labels.getString("sample_error"))
        else if (pending == null && bitmap == null) {
            val bounds = activity.windowManager.maximumWindowMetrics.bounds
            val config = JSONObject().put("mode", "image").put("id", UUID.randomUUID().toString())
                .put("path", File.createTempFile("color-sample-", ".png", context.cacheDir).absolutePath)
                .put("screen_width", bounds.width()).put("screen_height", bounds.height())
                .put("density_dpi", activity.resources.configuration.densityDpi)
                .put("notification_title", labels.getString("notification_title"))
                .put("notification_text", labels.getString("notification_text"))
                .put("stop_label", labels.getString("stop"))
            pending = config
            keyboard?.hide()
            show(false)
            try { consent.launch(context.getSystemService(MediaProjectionManager::class.java).createScreenCaptureIntent()) }
            catch (error: Exception) {
                pending = null
                show(true)
                File(config.getString("path")).delete()
                currentError(error.message ?: labels.getString("sample_error"))
            }
        }
    }
}
