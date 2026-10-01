import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Frame {
    id: root
    property string title: ""
    property bool expanded: true
    property bool autoToggleExpansion: true
    property bool resetVisible: true
    property bool accented: false
    property alias beforeReset: beforeReset.data
    property alias afterReset: afterReset.data
    default property alias controls: body.content
    signal expandedRequested(bool expanded)
    signal resetRequested()
    signal focusRequested(Item focusedItem, bool bodyClicked)
    padding: 0
    implicitHeight: content.implicitHeight

    Binding on topPadding { when: Qt.platform.os === "windows"; value: 0 }
    Binding on bottomPadding { when: Qt.platform.os === "windows"; value: 0 }
    Binding on leftPadding { when: Qt.platform.os === "windows"; value: 0 }
    Binding on rightPadding { when: Qt.platform.os === "windows"; value: 0 }

    function ownsFocus(item) {
        while (item) {
            if (item === root)
                return true
            item = item.parent
        }
        return false
    }

    function descendantAt(item, x, y) {
        const child = item.childAt(x, y)
        if (!child)
            return item
        const position = child.mapFromItem(item, x, y)
        return descendantAt(child, position.x, position.y)
    }

    function isBodyDescendant(item) {
        while (item && item !== root) {
            if (item === body)
                return true
            item = item.parent
        }
        return false
    }

    PointHandler {
        acceptedButtons: Qt.AllButtons
        onActiveChanged: if (active) {
            const position = body.mapFromItem(root, point.position.x, point.position.y)
            const bodyClicked = position.x >= 0 && position.y >= 0
                && position.x < body.width && position.y < body.height
            root.focusRequested(bodyClicked
                ? root.descendantAt(body, position.x, position.y) : root,
                bodyClicked)
        }
    }

    Connections {
        target: root.Window.window
        function onActiveFocusItemChanged() {
            if (root.ownsFocus(root.Window.activeFocusItem))
                root.focusRequested(root.Window.activeFocusItem,
                    root.isBodyDescendant(root.Window.activeFocusItem))
        }
    }

    background: Rectangle {
        color: "transparent"
        border.color: root.palette.highlight
        border.width: root.accented ? 2 : 0
        radius: 3
    }

    contentItem: ColumnLayout {
        id: content
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: 8
            Layout.rightMargin: 8
            Layout.topMargin: 6
            Layout.bottomMargin: 6
            spacing: 4

            ToolButton {
                id: disclosureButton
                icon.source: "qrc:/qt/qml/dev/shrimply/components/icons/disclosure.svg"
                icon.color: palette.buttonText
                display: AbstractButton.IconOnly
                rotation: root.expanded ? 90 : 0
                Behavior on rotation { NumberAnimation { duration: 180 } }
                ToolTip {
                    visible: disclosureButton.hovered
                    text: root.expanded ? qsTr("Collapse") : qsTr("Expand")
                    popupType: Popup.Item
                }
                onClicked: {
                    const requested = !root.expanded
                    if (root.autoToggleExpansion)
                        root.expanded = requested
                    root.expandedRequested(requested)
                }
            }
            Label {
                text: root.title
                font.bold: true
                Layout.fillWidth: true
            }
            RowLayout {
                id: beforeReset
                spacing: 4
            }
            ToolButton {
                id: resetButton
                visible: root.resetVisible
                icon.source: "qrc:/qt/qml/dev/shrimply/components/icons/reset.svg"
                icon.color: palette.buttonText
                display: AbstractButton.IconOnly
                ToolTip {
                    visible: resetButton.hovered
                    text: qsTr("Reset")
                    popupType: Popup.Item
                }
                onClicked: root.resetRequested()
            }
            RowLayout {
                id: afterReset
                spacing: 4
            }
        }

        CollapsibleSection {
            id: body
            expanded: root.expanded
            Layout.fillWidth: true
            Layout.leftMargin: 12
            Layout.rightMargin: 12
            Layout.topMargin: 4
            Layout.bottomMargin: 12
            spacing: 8
        }
    }
}
