import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

pragma ComponentBehavior: Bound

Item {
    id: root
    property var titles: []
    property var icons: []
    default property alias pages: pageStack.data
    implicitHeight: layout.implicitHeight

    ColumnLayout {
        id: layout
        anchors.fill: parent
        spacing: 6

        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: 16
            Layout.rightMargin: 16
            Layout.topMargin: 12
            spacing: 0
            Repeater {
                model: root.titles
                Button {
                    required property int index
                    required property string modelData
                    property int tabIndex: index
                    Layout.fillWidth: true
                    checkable: true
                    autoExclusive: true
                    checked: index === 0
                    text: modelData
                    icon.source: index < root.icons.length && root.icons[index].length > 0
                        ? "qrc:/shrimply/icons/" + root.icons[index] + ".svg" : ""
                    display: AbstractButton.TextBesideIcon
                    onClicked: pageStack.currentIndex = index
                }
            }
        }

        StackLayout {
            id: pageStack
            Layout.fillWidth: true
            Layout.fillHeight: true
            currentIndex: 0
        }
    }
}
