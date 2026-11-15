pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import dev.shrimply.components

Menu {
    id: root
    property bool bottomSheet: false
    property int pointerPopupType: Popup.Window
    property real preferredWidth: implicitWidth
    property Item pointerAnchor: null
    property real pointerOffsetY: 0
    parent: Overlay.overlay
    popupType: bottomSheet ? Popup.Item : pointerPopupType
    modal: bottomSheet
    dim: bottomSheet
    cascade: !bottomSheet
    transformOrigin: bottomSheet ? Item.Bottom : Item.TopLeft

    SelectorBackend { id: backend }

    function usesBottomSheet(touch) {
        return backend.menuUsesBottomSheet(touch)
    }

    // Menu and ComboBox also assign popup geometry internally while opening.
    function position() {
        if (bottomSheet) {
            parent = Overlay.overlay
            if (!parent) return
            width = parent.width
            height = Math.min(implicitHeight, parent.height)
            x = 0
            y = parent.height - height
        } else {
            width = preferredWidth
            height = implicitHeight
            if (pointerAnchor) {
                parent = pointerAnchor
                x = 0
                y = pointerOffsetY
            }
        }
    }

    function present(anchor, sheet) {
        pointerAnchor = anchor
        bottomSheet = sheet
        if (sheet) open()
        else popup(anchor, 0, 0)
    }

    function presentForInput(anchor, touch) {
        present(anchor, usesBottomSheet(touch))
    }

    Connections {
        target: root
        function onAboutToShow() { root.position() }
        function onOpened() { root.position() }
        function onImplicitHeightChanged() { if (root.visible) root.position() }
    }
    Connections {
        target: root.parent
        function onWidthChanged() { if (root.visible && root.bottomSheet) root.position() }
        function onHeightChanged() { if (root.visible && root.bottomSheet) root.position() }
    }
}
