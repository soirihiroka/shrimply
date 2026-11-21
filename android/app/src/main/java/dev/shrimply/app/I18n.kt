package dev.shrimply.app

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalConfiguration
import org.json.JSONObject

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


internal object NativeLayout {
    init { System.loadLibrary("shrimply_android") }
    external fun panelDividerWidth(): Float
    external fun menuUsesBottomSheet(inputSource: String): Boolean
    external fun compactControls(width: Float, height: Float): Boolean
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
