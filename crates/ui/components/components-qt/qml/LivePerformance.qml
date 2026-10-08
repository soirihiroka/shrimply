pragma ComponentBehavior: Bound

import QtQuick
import dev.shrimply.components
import QtQuick.Controls
import QtQuick.Layouts

Frame {
    id: root
    property bool expanded: false
    padding: 0
    implicitHeight: content.implicitHeight

    Binding on topPadding { when: Qt.platform.os === "windows"; value: 0 }
    Binding on bottomPadding { when: Qt.platform.os === "windows"; value: 0 }
    Binding on leftPadding { when: Qt.platform.os === "windows"; value: 0 }
    Binding on rightPadding { when: Qt.platform.os === "windows"; value: 0 }

    LivePerformanceBackend { id: backend }
    Timer {
        interval: backend.refreshInterval()
        repeat: true
        running: root.visible && root.expanded
        triggeredOnStart: true
        onTriggered: backend.refresh()
    }
    TextArea {
        id: clipboard
        visible: false
    }

    contentItem: ColumnLayout {
        id: content
        spacing: 0

        Item {
            Layout.fillWidth: true
            implicitHeight: headerContent.implicitHeight + 12

            Rectangle {
                anchors.fill: parent
                color: headerHover.hovered ? root.palette.alternateBase : "transparent"
            }

            HoverHandler { id: headerHover }
            MouseArea {
                anchors.fill: parent
                onClicked: root.expanded = !root.expanded
            }

            RowLayout {
                id: headerContent
                anchors.fill: parent
                anchors.leftMargin: 10
                anchors.rightMargin: 6
                spacing: 4

                Label {
                    text: ComponentTranslations.text("Live Performance")
                    Layout.fillWidth: true
                }
                ToolButton {
                    icon.source: "qrc:/qt/qml/dev/shrimply/components/icons/clear.svg"
                    icon.color: palette.buttonText
                    display: AbstractButton.IconOnly
                    background: Item {}
                    ToolTip.visible: hovered
                    ToolTip.text: ComponentTranslations.text("Clear")
                    onClicked: {
                        backend.clear()
                        if (root.expanded)
                            backend.refresh()
                    }
                }
                ToolButton {
                    icon.source: "qrc:/qt/qml/dev/shrimply/components/icons/copy.svg"
                    icon.color: palette.buttonText
                    display: AbstractButton.IconOnly
                    background: Item {}
                    ToolTip.visible: hovered
                    ToolTip.text: ComponentTranslations.text("Copy JSON")
                    onClicked: {
                        clipboard.text = backend.reportJson()
                        clipboard.selectAll()
                        clipboard.copy()
                        clipboard.deselect()
                    }
                }
                ToolButton {
                    icon.source: "qrc:/qt/qml/dev/shrimply/components/icons/disclosure.svg"
                    icon.color: palette.buttonText
                    display: AbstractButton.IconOnly
                    background: Item {}
                    rotation: root.expanded ? 270 : 90
                    Behavior on rotation { NumberAnimation { duration: 180 } }
                    ToolTip.visible: hovered
                    ToolTip.text: root.expanded
                        ? ComponentTranslations.text("Collapse")
                        : ComponentTranslations.text("Expand")
                    onClicked: root.expanded = !root.expanded
                }
            }
        }

        Loader {
            Layout.fillWidth: true
            active: root.expanded
            visible: active
            Layout.preferredHeight: active && item ? item.implicitHeight : 0
            sourceComponent: Component {
                ColumnLayout {
                    spacing: 0
                    Repeater {
                        model: backend.titles.length
                        delegate: ColumnLayout {
                            required property int index
                            Layout.fillWidth: true
                            Layout.leftMargin: 10
                            Layout.rightMargin: 10
                            Layout.topMargin: 6
                            Layout.bottomMargin: 6
                            spacing: 2
                            Label {
                                text: backend.titles[index]
                                Layout.fillWidth: true
                            }
                            Label {
                                text: backend.subtitles[index]
                                color: palette.placeholderText
                                font.pixelSize: Math.max(10, Application.font.pixelSize - 2)
                                wrapMode: Text.Wrap
                                Layout.fillWidth: true
                            }
                            Rectangle {
                                Layout.fillWidth: true
                                height: 1
                                color: palette.mid
                            }
                        }
                    }
                }
            }
        }
    }
}
