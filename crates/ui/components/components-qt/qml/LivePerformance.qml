pragma ComponentBehavior: Bound

import QtQuick
import dev.shrimply.components
import QtQuick.Controls
import QtQuick.Layouts

InspectorCard {
    id: root
    title: ComponentTranslations.text("Live Performance")
    expanded: false
    resetVisible: false

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

    afterReset: [
        ToolButton {
            icon.source: "qrc:/shrimply/icons/edit-clear-symbolic.svg"
            icon.color: palette.buttonText
            display: AbstractButton.IconOnly
            ToolTip.visible: hovered
            ToolTip.text: ComponentTranslations.text("Clear")
            onClicked: {
                backend.clear()
                if (root.expanded)
                    backend.refresh()
            }
        },
        ToolButton {
            icon.source: "qrc:/shrimply/icons/edit-copy-symbolic.svg"
            icon.color: palette.buttonText
            display: AbstractButton.IconOnly
            ToolTip.visible: hovered
            ToolTip.text: ComponentTranslations.text("Copy JSON")
            onClicked: {
                clipboard.text = backend.reportJson()
                clipboard.selectAll()
                clipboard.copy()
                clipboard.deselect()
            }
        }
    ]

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
