import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

ControlRow {
    id: root
    property bool keyframes: false
    property bool expression: false
    property bool keyframeAvailable: true
    property bool expressionAvailable: true
    default property alias editor: holder.data
    property alias trailing: trailingControls.data
    signal keyframesToggled(bool enabled)
    signal expressionToggled(bool enabled)

    RowLayout {
        Layout.minimumWidth: Qt.platform.os === "windows" ? visibleChildMinimumWidth : 0
        readonly property real visibleChildMinimumWidth: {
            let minimum = 0
            let visibleChildren = 0
            for (let index = 0; index < children.length; ++index) {
                const child = children[index]
                if (!child.visible)
                    continue
                minimum += Math.max(child.Layout.minimumWidth || 0,
                    child.implicitWidth || 0)
                visibleChildren += 1
            }
            return minimum + Math.max(0, visibleChildren - 1) * spacing
        }
        Item {
            id: holder
            Layout.fillWidth: true
            Layout.minimumWidth: Qt.platform.os === "windows" ? visibleChildMinimumWidth : 0
            readonly property real visibleChildMinimumWidth: {
                let minimum = 0
                for (let index = 0; index < children.length; ++index) {
                    const child = children[index]
                    if (child.visible)
                        minimum = Math.max(minimum, child.Layout.minimumWidth || 0)
                }
                return minimum
            }
            implicitHeight: childrenRect.height
            clip: true
            onChildrenChanged: {
                for (let index = 0; index < children.length; ++index)
                    children[index].width = Qt.binding(function() { return holder.width })
            }
        }
        ToolButton {
            visible: root.keyframeAvailable
            checkable: true
            checked: root.keyframes
            flat: true
            icon.source: "qrc:/qt/qml/dev/shrimply/components/icons/keyframe.svg"
            icon.color: checked || highlighted
                ? palette.highlightedText
                : palette.buttonText
            display: AbstractButton.IconOnly
            ToolTip.visible: hovered
            ToolTip.text: qsTr("Keyframes")
            onClicked: root.keyframesToggled(checked)
        }
        ToolButton {
            visible: root.expressionAvailable
            checkable: true
            checked: root.expression
            flat: true
            icon.source: "qrc:/qt/qml/dev/shrimply/components/icons/code.svg"
            icon.color: checked || highlighted
                ? palette.highlightedText
                : palette.buttonText
            display: AbstractButton.IconOnly
            ToolTip.visible: hovered
            ToolTip.text: qsTr("Expression")
            onClicked: root.expressionToggled(checked)
        }
        RowLayout {
            id: trailingControls
            visible: children.length > 0
            spacing: 6
        }
    }
}
