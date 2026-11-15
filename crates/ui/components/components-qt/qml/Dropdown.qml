pragma ComponentBehavior: Bound

import QtQuick
import dev.shrimply.components
import QtQuick.Controls
import QtQuick.Layouts

Button {
    id: root
    property var values: []
    property var labels: []
    property string value: ""
    property string selectedValue: value
    property bool enableSearch: backend.searchable(labels.length)
    property string searchPlaceholder: ComponentTranslations.text("Search")
    signal selected(string value)
    text: currentLabel()

    function currentLabel() {
        const index = backend.selectedIndex(values, selectedValue)
        return index >= 0 && index < labels.length ? labels[index] : ""
    }

    function synchronize() {
        popup.selectedIndex = backend.selectedIndex(values, selectedValue)
    }

    function choose(index) {
        const next = backend.valueAt(values, index)
        if (index < 0 || index >= values.length)
            return
        if (next === selectedValue)
            return
        selectedValue = next
        selected(next)
    }

    SelectorBackend { id: backend }
    MenuInput { id: menuInput }

    // Match the editor timeline menus: QQuickMenu's Wayland positioner uses
    // the parent item's bottom-left anchor with bottom-right gravity.  A wide
    // parent therefore places the popup at the parent's right edge; using a
    // one-pixel anchor makes that edge the field's left edge.
    Item {
        id: popupAnchor
        parent: root
        x: 0
        y: root.height
        width: 1
        height: 1
        visible: false
    }

    Component.onCompleted: synchronize()
    onValuesChanged: synchronize()
    onValueChanged: selectedValue = value
    onSelectedValueChanged: synchronize()
    onClicked: {
        const touch = menuInput.takeTouch()
        if (popup.opened)
            popup.close()
        else
            popup.presentForInput(popupAnchor, touch)
    }

    contentItem: RowLayout {
        Label {
            text: root.text
            elide: Text.ElideRight
            Layout.fillWidth: true
        }
        Label { text: "⌄" }
    }

    SearchMenu {
        id: popup
        preferredWidth: Math.max(root.width, 240)
        labels: root.labels
        selectedIndex: backend.selectedIndex(root.values, root.selectedValue)
        searchEnabled: root.enableSearch
        placeholderText: root.searchPlaceholder
        onActivated: function(index) { root.choose(index) }
        onClosed: root.forceActiveFocus(Qt.PopupFocusReason)
    }
}
