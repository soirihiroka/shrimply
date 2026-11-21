pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import dev.shrimply.components

ComboBox {
    id: root
    property var labelForIndex: null
    MenuInput { id: menuInput }
    onPressedChanged: if (pressed) {
        choices.bottomSheet = choices.usesBottomSheet(menuInput.takeTouch())
        choices.position()
    }

    popup: SearchMenu {
        id: choices
        preferredWidth: Math.max(root.width, 240)
        selectedIndex: root.currentIndex
        searchEnabled: root.count > 0 && selector.searchable(root.count)
        labels: {
            const count = root.count
            const labels = []
            if (root.model === null) return labels
            for (let index = 0; index < count; ++index)
                labels.push(root.labelForIndex ? root.labelForIndex(index) : root.textAt(index))
            return labels
        }
        pointerAnchor: root
        pointerOffsetY: root.height
        onActivated: function(index) {
            selector.activateComboIndex(root, index)
        }
        onClosed: root.forceActiveFocus(Qt.PopupFocusReason)
    }
    SelectorBackend { id: selector }
}
