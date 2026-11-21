pragma ComponentBehavior: Bound

import QtQuick
import dev.shrimply.components
import QtQuick.Controls
import QtQuick.Layouts

Button {
    id: root
    property var values: []
    property var labels: []
    property var searchTerms: labels
    property string searchPlaceholderText: ""
    signal selected(string value)
    flat: true
    icon.source: "qrc:/qt/qml/dev/shrimply/components/icons/add-modifier.svg"
    icon.color: palette.buttonText
    display: AbstractButton.TextBesideIcon
    onClicked: menu.presentForInput(anchor, menuInput.takeTouch())

    SelectorBackend { id: backend }
    MenuInput { id: menuInput }
    Item {
        id: anchor
        parent: root
        x: (root.width - menu.width) / 2
        y: root.height
        width: 1
        height: 1
        visible: false
    }
    SearchMenu {
        id: menu
        preferredWidth: 280
        labels: root.labels
        searchTerms: root.searchTerms
        placeholderText: root.searchPlaceholderText
        onActivated: function(index) {
            const value = backend.valueAt(root.values, index)
            if (value.length > 0)
                root.selected(value)
        }
    }
}
