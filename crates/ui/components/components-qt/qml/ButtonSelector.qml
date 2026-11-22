pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

RowLayout {
    id: root
    property var values: []
    property var labels: []
    property var icons: []
    property string value: ""
    property string selectedValue: value
    signal selected(string value)
    spacing: 0

    function choose(value) {
        if (selectedValue === value)
            return
        selectedValue = value
        selected(value)
    }

    onValueChanged: selectedValue = value

    ButtonGroup { id: group }

    Repeater {
        model: root.values
        ToolButton {
            required property int index
            required property string modelData
            Layout.fillWidth: true
            checkable: true
            checked: modelData === root.selectedValue
            ButtonGroup.group: group
            icon.source: index < root.icons.length && root.icons[index].length > 0
                ? "qrc:/shrimply/icons/" + root.icons[index] + ".svg" : ""
            text: index < root.labels.length ? root.labels[index] : ""
            display: icon.source.toString().length > 0
                ? AbstractButton.IconOnly : AbstractButton.TextOnly
            ToolTip.visible: hovered
            ToolTip.text: text
            onClicked: root.choose(modelData)
        }
    }
}
