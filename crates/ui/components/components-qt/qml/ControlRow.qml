import QtQuick
import dev.shrimply.components
import QtQuick.Controls
import QtQuick.Layouts

RowLayout {
    id: root
    property string label: ""
    property string subtitle: ""
    readonly property int labelWidthCharacters: 13
    default property alias control: holder.data
    spacing: 6
    Layout.fillWidth: true

    ColumnLayout {
        id: labelColumn
        spacing: 1
        readonly property real labelWidth: labelMetrics.averageCharacterWidth
            * root.labelWidthCharacters
        Layout.preferredWidth: labelWidth
        Layout.minimumWidth: labelWidth
        Layout.maximumWidth: labelWidth
        FontMetrics { id: labelMetrics; font: label.font }
        Label {
            id: label
            text: ComponentTranslations.text(root.label)
            opacity: 0.72
            horizontalAlignment: Text.AlignLeft
            verticalAlignment: Text.AlignVCenter
            elide: Text.ElideRight
            Layout.fillWidth: true
        }
        Label {
            visible: root.subtitle.length > 0
            text: ComponentTranslations.text(root.subtitle)
            opacity: 0.56
            wrapMode: Text.Wrap
            font.pixelSize: Math.max(9, Application.font.pixelSize - 2)
            Layout.fillWidth: true
        }
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
}
