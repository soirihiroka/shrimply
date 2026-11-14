package dev.shrimply.tablet

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.AndroidExternalSurface
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp

class MainActivity : ComponentActivity() {
    private lateinit var timeline: TimelineSession
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        timeline = TimelineSession(intent.getBooleanExtra("stress", false), intent.getBooleanExtra("benchmark", false))
        setContent {
            MaterialTheme(colorScheme = darkColorScheme(primary = Color(0xff80b8ff))) {
                Editor(timeline)
            }
        }
    }
    override fun onResume() { super.onResume(); timeline.active(true) }
    override fun onPause() { timeline.active(false); super.onPause() }
    override fun onDestroy() { timeline.close(); super.onDestroy() }
}

@Composable
private fun Editor(timeline: TimelineSession) {
    var timelineFraction by rememberSaveable { mutableFloatStateOf(0.4f) }
    var inspectorWidth by rememberSaveable { mutableFloatStateOf(280f) }
    val density = LocalDensity.current.density
    val divider = MaterialTheme.colorScheme.outlineVariant
    Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).safeDrawingPadding()) {
        Row(Modifier.fillMaxWidth().height(52.dp).padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("Shrimply", style = MaterialTheme.typography.titleMedium)
            Spacer(Modifier.width(24.dp))
            Text("Tablet timeline study", color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        HorizontalDivider()
        BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
            val availableHeight = maxHeight.value
            val availableWidth = maxWidth.value
            val showInspector = maxWidth >= 720.dp
            Column(Modifier.fillMaxSize()) {
                Row(Modifier.weight(1f).fillMaxWidth()) {
                    if (showInspector) {
                        Column(Modifier.width(inspectorWidth.coerceIn(200f, availableWidth * 0.45f).dp).fillMaxHeight().padding(20.dp)) {
                            Text("Inspector", style = MaterialTheme.typography.titleMedium)
                            Spacer(Modifier.height(24.dp))
                            Text("Sample timeline", style = MaterialTheme.typography.bodyLarge)
                            Spacer(Modifier.height(12.dp))
                            Text("Comment · Caption · Video · Audio", color = MaterialTheme.colorScheme.onSurfaceVariant)
                            Spacer(Modifier.height(20.dp))
                            Text("Drag to pan. Pinch to zoom. Tap to position the playhead.", color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                        Box(Modifier.width(12.dp).fillMaxHeight().pointerInput(density, availableWidth) {
                            detectDragGestures { change, delta ->
                                change.consume()
                                inspectorWidth = (inspectorWidth + delta.x / density).coerceIn(200f, availableWidth * 0.45f)
                            }
                        }, contentAlignment = Alignment.Center) {
                            Box(Modifier.width(1.dp).fillMaxHeight().background(divider))
                        }
                    }
                    Box(Modifier.weight(1f).fillMaxHeight().padding(24.dp), contentAlignment = Alignment.Center) {
                        Column(horizontalAlignment = Alignment.CenterHorizontally) {
                            Text("Preview placeholder", style = MaterialTheme.typography.titleLarge)
                            Spacer(Modifier.height(12.dp))
                            Text("Video preview is deferred", color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                    }
                }
                Box(Modifier.fillMaxWidth().height(12.dp).pointerInput(density, availableHeight) {
                    detectDragGestures { change, delta ->
                        change.consume()
                        timelineFraction = (timelineFraction - delta.y / density / availableHeight).coerceIn(0.2f, 0.75f)
                    }
                }, contentAlignment = Alignment.Center) {
                    Box(Modifier.fillMaxWidth().height(1.dp).background(divider))
                }
                Column(Modifier.fillMaxWidth().height((availableHeight * timelineFraction).dp)) {
                    Row(Modifier.fillMaxWidth().height(40.dp).padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text("Timeline", style = MaterialTheme.typography.titleSmall)
                        Spacer(Modifier.weight(1f))
                        Text("Pan · Pinch to zoom · Tap playhead", style = MaterialTheme.typography.labelMedium)
                    }
                    NativeSurface(timeline, Modifier.weight(1f).fillMaxWidth())
                }
            }
        }
    }
}

@Composable
private fun NativeSurface(timeline: TimelineSession, modifier: Modifier) {
    val density = LocalDensity.current.density
    AndroidExternalSurface(modifier.pointerInput(timeline, density) {
        awaitEachGesture {
            val down = awaitFirstDown(requireUnconsumed = false)
            var moved = false
            var ruler = down.position.y / density < 44f
            do {
                val event = awaitPointerEvent()
                val pan = event.calculatePan()
                val zoom = event.calculateZoom()
                if (event.changes.size > 1) ruler = false
                if (pan.getDistance() > 0f || zoom != 1f) {
                    moved = true
                    val centroid = event.calculateCentroid(useCurrent = true)
                    if (centroid.x.isFinite() && centroid.y.isFinite()) {
                        timeline.input(floatArrayOf(if (ruler) 1f else 0f, centroid.x / density, centroid.y / density, pan.x / density, pan.y / density, zoom))
                    }
                    event.changes.forEach { it.consume() }
                }
            } while (event.changes.any { it.pressed })
            if (!moved) timeline.input(floatArrayOf(1f, down.position.x / density, down.position.y / density, 0f, 0f, 1f))
        }
    }) {
        onSurface { surface, width, height ->
            timeline.surface(surface, width, height, density)
            surface.onChanged { w, h -> timeline.resize(w, h) }
            surface.onDestroyed { timeline.surface(null, 0, 0, density) }
        }
    }
}
