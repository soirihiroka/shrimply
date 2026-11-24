package dev.shrimply.app

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.unit.Constraints
import kotlin.math.roundToInt

/** Rust supplies pane geometry. Each pane animates only its own visibility. */
@Composable
internal fun EditorPanes(
    inspectorWidth: Float,
    timelineFraction: Float,
    inspectorVisible: Boolean,
    timelineVisible: Boolean,
    preview: @Composable () -> Unit,
    inspector: @Composable () -> Unit,
    timeline: @Composable () -> Unit,
) {
    Layout(modifier = Modifier.clipToBounds(), content = {
        Box { preview() }
        Box {
            AnimatedVisibility(visible = inspectorVisible, modifier = Modifier.fillMaxSize(),
                enter = slideInHorizontally { -it }, exit = slideOutHorizontally { -it }) { inspector() }
        }
        Box {
            AnimatedVisibility(visible = timelineVisible, modifier = Modifier.fillMaxSize(),
                enter = slideInVertically { it }, exit = slideOutVertically { it }) { timeline() }
        }
    }) { measurables, constraints ->
        val bounds = NativeLayout.editorPaneBounds(constraints.maxWidth / density, constraints.maxHeight / density,
            inspectorWidth, timelineFraction, inspectorVisible, timelineVisible).map { (it * density).roundToInt() }
        val panes = measurables.mapIndexed { index, measurable ->
            measurable.measure(Constraints.fixed(bounds[index * 4 + 2], bounds[index * 4 + 3]))
        }
        layout(constraints.maxWidth, constraints.maxHeight) {
            panes.forEachIndexed { index, pane -> pane.place(bounds[index * 4], bounds[index * 4 + 1]) }
        }
    }
}
