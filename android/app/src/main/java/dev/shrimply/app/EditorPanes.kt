package dev.shrimply.app

import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.layout.Box
import androidx.compose.material3.adaptive.ExperimentalMaterial3AdaptiveApi
import androidx.compose.material3.adaptive.ExperimentalMaterial3AdaptiveComponentOverrideApi
import androidx.compose.material3.adaptive.layout.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt

/** Native pane motion/focus/state; Rust supplies the editor's two-dimensional arrangement. */
@OptIn(ExperimentalMaterial3AdaptiveApi::class, ExperimentalMaterial3AdaptiveComponentOverrideApi::class)
@Composable
internal fun EditorPanes(
    inspectorWidth: Float,
    timelineFraction: Float,
    inspectorVisible: Boolean,
    timelineVisible: Boolean,
    overlay: Boolean,
    preview: @Composable () -> Unit,
    inspector: @Composable () -> Unit,
    timeline: @Composable () -> Unit,
) {
    val currentInspectorWidth by rememberUpdatedState(inspectorWidth)
    val currentTimelineFraction by rememberUpdatedState(timelineFraction)
    val currentInspectorVisible by rememberUpdatedState(inspectorVisible)
    val currentTimelineVisible by rememberUpdatedState(timelineVisible)
    val arrangement = remember { object : ThreePaneScaffoldOverride {
        @Composable
        override fun ThreePaneScaffoldOverrideScope.ThreePaneScaffold() {
            Layout(modifier = modifier.clipToBounds(), content = {
                Box { primaryPane() }
                Box { secondaryPane() }
                Box { tertiaryPane?.invoke() }
            }) { measurables, constraints ->
                val bounds = NativeLayout.editorPaneBounds(constraints.maxWidth / density, constraints.maxHeight / density,
                    currentInspectorWidth, currentTimelineFraction, currentInspectorVisible, currentTimelineVisible)
                    .map { (it * density).roundToInt() }
                val panes = measurables.mapIndexed { index, measurable ->
                    measurable.measure(Constraints.fixed(bounds[index * 4 + 2], bounds[index * 4 + 3]))
                }
                layout(constraints.maxWidth, constraints.maxHeight) {
                    panes.forEachIndexed { index, pane -> pane.place(bounds[index * 4], bounds[index * 4 + 1]) }
                }
            }
        }
    } }
    val value = ThreePaneScaffoldValue(
        primary = PaneAdaptedValue.Expanded,
        secondary = if (!inspectorVisible) PaneAdaptedValue.Hidden
            else if (overlay) PaneAdaptedValue.Levitated(Alignment.TopStart) else PaneAdaptedValue.Expanded,
        tertiary = if (timelineVisible) PaneAdaptedValue.Reflowed(ThreePaneScaffoldRole.Primary) else PaneAdaptedValue.Hidden,
    )
    CompositionLocalProvider(LocalThreePaneScaffoldOverride provides arrangement) {
        ListDetailPaneScaffold(
            directive = PaneScaffoldDirective(2, 0.dp, 2, 0.dp, inspectorWidth.dp, emptyList()),
            value = value,
            detailPane = { preview() },
            listPane = { AnimatedPane(enterTransition = slideInHorizontally { -it }, exitTransition = slideOutHorizontally { -it }) { inspector() } },
            extraPane = { AnimatedPane(enterTransition = slideInVertically { it }, exitTransition = slideOutVertically { it }) { timeline() } },
        )
    }
}
