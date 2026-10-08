import QtQuick
import dev.shrimply.components
import QtQuick.Controls
import QtQuick.Layouts

RowLayout {
    id: root
    property real first: 0
    property real second: 0
    property real third: 0
    property real minimum: -1000000
    property real maximum: 1000000
    property real dragStep: 1
    property int digits: 2
    property int widthCharacters: 8
    property var prefixes: ["", "", ""]
    property string unitName: ""
    property bool enableLock: false
    signal edited(int axis, real value)
    signal committed(int axis, real value)
    signal valuesEdited(real first, real second, real third, int component)
    spacing: 6
    Layout.minimumWidth: Qt.platform.os === "windows" ? implicitWidth : 0

    NumberGroupBackend {
        id: group
        onValueEdited: function(axis, value) { root.edited(axis, value) }
        onGroupEdited: function(axis) {
            root.valuesEdited(group.first, group.second, group.third, axis)
        }
    }
    Component.onCompleted: {
        group.configure(first, second, third, 3, enableLock)
        group.setBounds(minimum, maximum)
    }
    onFirstChanged: group.setExternalValue(0, first)
    onSecondChanged: group.setExternalValue(1, second)
    onThirdChanged: group.setExternalValue(2, third)
    onMinimumChanged: group.setBounds(minimum, maximum)
    onMaximumChanged: group.setBounds(minimum, maximum)
    onEnableLockChanged: group.updateLocked(enableLock && lock.checked)

    ToolButton {
        id: lock
        visible: root.enableLock
        checkable: true
        checked: true
        icon.source: checked
            ? "qrc:/qt/qml/dev/shrimply/components/icons/padlock.svg"
            : "qrc:/qt/qml/dev/shrimply/components/icons/padlock-open.svg"
        display: AbstractButton.IconOnly
        ToolTip.visible: hovered
        ToolTip.text: ComponentTranslations.text("Lock ratio")
        onToggled: group.updateLocked(checked)
    }
    NumberPicker {
        id: firstPicker
        Layout.fillWidth: true
        value: group.first; minimum: root.minimum; maximum: root.maximum; dragStep: root.dragStep; digits: root.digits
        widthCharacters: root.widthCharacters
        prefix: root.prefixes[0]; unitName: root.unitName
        onEdited: function(value) { group.edit(0, value) }
        onCommitted: function(value) { root.committed(0, value) }
    }
    NumberPicker {
        id: secondPicker
        Layout.fillWidth: true
        value: group.second; minimum: root.minimum; maximum: root.maximum; dragStep: root.dragStep; digits: root.digits
        widthCharacters: root.widthCharacters
        prefix: root.prefixes[1]; unitName: root.unitName
        onEdited: function(value) { group.edit(1, value) }
        onCommitted: function(value) { root.committed(1, value) }
    }
    NumberPicker {
        id: thirdPicker
        Layout.fillWidth: true
        value: group.third; minimum: root.minimum; maximum: root.maximum; dragStep: root.dragStep; digits: root.digits
        widthCharacters: root.widthCharacters
        prefix: root.prefixes[2]; unitName: root.unitName
        onEdited: function(value) { group.edit(2, value) }
        onCommitted: function(value) { root.committed(2, value) }
    }
}
