package dev.shrimply.app

import androidx.compose.foundation.indication
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.PressInteraction
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ripple
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.flow.first

/** Visual indication only. Rust owns the target geometry, cancellation and activation. */
@Composable
internal fun CanvasButtonIndication(timeline: TimelineSession) {
    val feedback = timeline.snapshot.value.optJSONObject("timeline")?.optJSONObject("button_feedback") ?: return
    key(timeline.identity, feedback.getString("owner"), feedback.getLong("serial")) {
        val bounds = feedback.getJSONArray("bounds")
        val clip = feedback.getJSONArray("clip")
        val origin = feedback.getJSONArray("origin")
        val density = LocalDensity.current.density
        val interactions = remember { MutableInteractionSource() }
        val press = remember { PressInteraction.Press(Offset(origin.getDouble(0).toFloat() * density, origin.getDouble(1).toFloat() * density)) }
        val phase by rememberUpdatedState(feedback.getString("phase"))
        LaunchedEffect(interactions) {
            var finished = false
            try {
                interactions.emit(press)
                val terminal = snapshotFlow { phase }.first { it != "pressed" }
                interactions.emit(when (terminal) {
                    "released" -> PressInteraction.Release(press)
                    "cancelled" -> PressInteraction.Cancel(press)
                    else -> error("Unknown canvas button indication phase: $terminal")
                })
                finished = true
            } finally {
                if (!finished) interactions.tryEmit(PressInteraction.Cancel(press))
            }
        }
        Box(Modifier.offset(clip.getDouble(0).dp, clip.getDouble(1).dp)
            .size(clip.getDouble(2).dp, clip.getDouble(3).dp).clipToBounds()) {
            Box(Modifier
                .offset((bounds.getDouble(0) - clip.getDouble(0)).dp, (bounds.getDouble(1) - clip.getDouble(1)).dp)
                .size(bounds.getDouble(2).dp, bounds.getDouble(3).dp)
                .clip(RoundedCornerShape(feedback.getDouble("radius").dp))
                .indication(interactions, ripple(bounded = true)))
        }
    }
}
