package dev.shrimply.app

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.graphics.vector.group
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.unit.dp
import org.json.JSONObject
import java.util.concurrent.ConcurrentHashMap

internal object NativeI18n {
    init { System.loadLibrary("shrimply_android") }
    external fun initializeLocale(languageTags: String)
    external fun text(key: String, arguments: String): String
}

@Composable
internal fun tr(key: String, vararg arguments: Pair<String, Any>): String {
    val languageTags = LocalConfiguration.current.locales.toLanguageTags()
    val values = JSONObject()
    arguments.forEach { (name, value) -> values.put(name, value.toString()) }
    val encoded = values.toString()
    return remember(languageTags, key, encoded) { NativeI18n.text(key, encoded) }
}

private val bundledIconCache = ConcurrentHashMap<String, ImageVector>()

/** Build a Compose vector directly from Rust's parsed SVG path data. */
internal fun iconVector(name: String): ImageVector = bundledIconCache.computeIfAbsent(name) { key ->
    val spec = JSONObject(NativeLayout.iconVector(key))
    val width = spec.getDouble("width").toFloat()
    val height = spec.getDouble("height").toFloat()
    val paths = spec.getJSONArray("paths")
    ImageVector.Builder(
        name = key,
        defaultWidth = width.dp,
        defaultHeight = height.dp,
        viewportWidth = width,
        viewportHeight = height,
    ).apply {
        val brush = SolidColor(Color.Black)
        for (index in 0 until paths.length()) {
            val path = paths.getJSONObject(index)
            val nodes = PathParser().parsePathString(path.getString("d")).toNodes()
            val tx = path.getDouble("tx").toFloat()
            val ty = path.getDouble("ty").toFloat()
            val sx = path.optDouble("sx", 1.0).toFloat()
            val sy = path.optDouble("sy", 1.0).toFloat()
            val fillAlpha = path.getDouble("fill_alpha").toFloat()
            val strokeAlpha = path.getDouble("stroke_alpha").toFloat()
            val fillType = if (path.getBoolean("even_odd")) PathFillType.EvenOdd else PathFillType.NonZero
            val strokeCap = when (path.getString("stroke_cap")) {
                "round" -> StrokeCap.Round
                "square" -> StrokeCap.Square
                else -> StrokeCap.Butt
            }
            val strokeJoin = when (path.getString("stroke_join")) {
                "round" -> StrokeJoin.Round
                "bevel" -> StrokeJoin.Bevel
                else -> StrokeJoin.Miter
            }
            group(translationX = tx, translationY = ty, scaleX = sx, scaleY = sy) {
                addPath(
                    pathData = nodes,
                    pathFillType = fillType,
                    fill = if (fillAlpha > 0f) brush else null,
                    fillAlpha = fillAlpha,
                    stroke = if (strokeAlpha > 0f) brush else null,
                    strokeAlpha = strokeAlpha,
                    strokeLineWidth = path.getDouble("stroke_width").toFloat(),
                    strokeLineCap = strokeCap,
                    strokeLineJoin = strokeJoin,
                )
            }
        }
    }.build()
}

internal object NativeLayout {
    init { System.loadLibrary("shrimply_android") }
    external fun panelDividerWidth(): Float
    external fun menuUsesBottomSheet(inputSource: String): Boolean
    external fun compactControls(width: Float, height: Float): Boolean
    external fun iconVector(name: String): String
    external fun inspectorMinWidth(): Float
    external fun inspectorMaxWidth(width: Float): Float
    external fun inspectorOverlay(width: Float): Boolean
    external fun splitPaneExtent(total: Float, divider: Float, fraction: Float): Float
    external fun editorPaneBounds(width: Float, height: Float, inspectorWidth: Float, timelineFraction: Float,
        inspectorVisible: Boolean, timelineVisible: Boolean): FloatArray
    external fun transportStacked(width: Float): Boolean
    external fun choiceSearchable(count: Int): Boolean
    external fun matchingChoices(labels: String, query: String): String
}

internal object NativeColor {
    init { System.loadLibrary("shrimply_android") }
    external fun request(request: String): String
}
