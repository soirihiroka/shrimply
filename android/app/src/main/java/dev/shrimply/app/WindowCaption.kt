package dev.shrimply.app

import android.graphics.Rect
import android.os.Build
import android.view.WindowInsets
import androidx.compose.foundation.layout.*
import androidx.compose.material3.LocalMinimumInteractiveComponentSize
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.ceil
import kotlin.math.floor

/** Actual window-manager geometry, in window pixels; never inferred from device type. */
internal data class WindowCaption(val height: Int = 0, val controls: List<Rect> = emptyList())

internal fun windowCaption(insets: WindowInsets, supported: Boolean): WindowCaption {
    if (!supported || Build.VERSION.SDK_INT < 35 || !insets.isVisible(WindowInsets.Type.captionBar())) return WindowCaption()
    val height = insets.getInsets(WindowInsets.Type.captionBar()).top
    val controls = insets.getBoundingRects(WindowInsets.Type.captionBar()).map { Rect(it) }
    // Legacy shells report the whole opaque bar, even when the appearance flag is accepted.
    // Keep the regular inset header unless the OS actually exposes usable caption space.
    if (height <= 0 || controls.isEmpty() || captionSpan(Rect(0, 0, insets.frame.width, height), controls).isEmpty) return WindowCaption()
    return WindowCaption(height, controls)
}

/** Native reserved rectangles are projected into the actual laid-out header, not screen coordinates. */
private fun captionSpan(header: Rect, controls: List<Rect>): Rect {
    var start = header.left
    var best = Rect(header.left, header.top, header.left, header.bottom)
    for (control in controls.filter { it.bottom > header.top && it.top < header.bottom }
        .sortedBy { it.left }) {
        val end = control.left.coerceIn(header.left, header.right)
        if (end - start > best.width()) best = Rect(start, header.top, end, header.bottom)
        start = maxOf(start, control.right.coerceIn(header.left, header.right))
    }
    if (header.right - start > best.width()) best = Rect(start, header.top, header.right, header.bottom)
    return best
}

/** Only the two real action groups claim caption input. Empty space keeps native window dragging. */
@Composable
@OptIn(ExperimentalMaterial3Api::class)
internal fun WindowCaptionHeader(caption: WindowCaption, interactive: Boolean,
    left: @Composable (Modifier, Dp) -> Unit, right: @Composable (Modifier, Dp) -> Unit) {
    val view = LocalView.current
    val density = LocalDensity.current
    val integrated = caption.height > 0
    var bounds by remember { mutableStateOf(Rect()) }
    var leftBounds by remember { mutableStateOf(Rect()) }
    var rightBounds by remember { mutableStateOf(Rect()) }
    val height = if (integrated) with(density) { caption.height.toDp() } else 56.dp
    val targetSize = if (integrated) minOf(height, 48.dp) else 48.dp
    val span = if (integrated && !bounds.isEmpty) captionSpan(bounds, caption.controls) else bounds
    val startPadding = if (integrated) with(density) { (span.left - bounds.left).coerceAtLeast(0).toDp() } else 8.dp
    val endPadding = if (integrated) with(density) { (bounds.right - span.right).coerceAtLeast(0).toDp() } else 8.dp
    fun capture(update: (Rect) -> Unit) = Modifier.onGloballyPositioned { coordinates ->
        val rect = coordinates.boundsInWindow()
        update(Rect(ceil(rect.left).toInt(), ceil(rect.top).toInt(), floor(rect.right).toInt(), floor(rect.bottom).toInt()))
    }
    SideEffect {
        if (interactive) {
            val location = IntArray(2)
            view.getLocationInWindow(location)
            view.systemGestureExclusionRects = if (!integrated) emptyList() else listOf(leftBounds, rightBounds)
                .mapNotNull { source -> Rect(source).takeIf { it.intersect(span) && !it.isEmpty }?.apply { offset(-location[0], -location[1]) } }
        }
    }
    DisposableEffect(view, interactive) { onDispose { if (interactive) view.systemGestureExclusionRects = emptyList() } }
    Box(Modifier.fillMaxWidth().height(height).then(capture { bounds = it })) {
        // Wait for real coordinates before putting controls beside the system's own controls.
        if (!integrated || !bounds.isEmpty) CompositionLocalProvider(
            LocalMinimumInteractiveComponentSize provides if (integrated) 0.dp else LocalMinimumInteractiveComponentSize.current
        ) {
            Row(Modifier.fillMaxSize().absolutePadding(left = startPadding, right = endPadding).clipToBounds(), verticalAlignment = Alignment.CenterVertically) {
                left(if (integrated) capture { leftBounds = it } else Modifier, targetSize)
                Spacer(Modifier.weight(1f))
                right(if (integrated) capture { rightBounds = it } else Modifier, targetSize)
            }
        }
    }
}
