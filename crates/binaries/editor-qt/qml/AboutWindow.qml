pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

ApplicationWindow {
    id: root

    required property var backend
    required property var owner

    title: backend.translate("About Shrimply")
    transientParent: owner
    modality: Qt.WindowModal
    flags: Qt.Dialog
    width: 700
    height: 560
    minimumWidth: 600
    minimumHeight: 460

    function openAbout() {
        show()
        raise()
        requestActivate()
    }

    Shortcut { sequence: StandardKey.Cancel; onActivated: root.close() }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        Pane {
            Layout.fillWidth: true
            padding: 24

            RowLayout {
                anchors.fill: parent
                spacing: 24

                Image {
                    Layout.preferredWidth: 96
                    Layout.preferredHeight: 96
                    source: "qrc:/qt/qml/dev/shrimply/editor/shrimply.svg"
                    fillMode: Image.PreserveAspectFit
                    asynchronous: true
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 4

                    Label {
                        text: "Shrimply"
                        font.pointSize: 22
                        font.bold: true
                    }
                    Label {
                        text: root.backend.translate("Version") + " " + root.backend.applicationVersion()
                        opacity: 0.7
                    }
                    Label {
                        Layout.fillWidth: true
                        text: root.backend.translate("A simple yet powerful cross platform video editor.")
                        wrapMode: Text.Wrap
                    }
                    Label {
                        text: "Copyright © 2026 Soiri Hiroka"
                        opacity: 0.7
                    }
                }
            }
        }

        TabBar {
            id: tabs
            Layout.fillWidth: true

            TabButton { text: root.backend.translate("About") }
            TabButton { text: root.backend.translate("Credits") }
            TabButton { text: root.backend.translate("License") }
        }

        StackLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            currentIndex: tabs.currentIndex

            Pane {
                padding: 24

                ColumnLayout {
                    anchors.fill: parent
                    spacing: 16

                    Label {
                        Layout.fillWidth: true
                        text: root.backend.translate("A simple yet powerful cross platform video editor.")
                        font.pointSize: 14
                        wrapMode: Text.Wrap
                    }
                    GridLayout {
                        columns: 2

                        Repeater {
                            model: root.backend.aboutLinkLabels()

                            Button {
                                id: linkButton

                                required property int index
                                required property string modelData

                                text: linkButton.modelData
                                onClicked: Qt.openUrlExternally(root.backend.aboutLinkUrl(linkButton.index))
                            }
                        }
                    }
                    Item { Layout.fillHeight: true }
                }
            }

            Pane {
                padding: 24

                ColumnLayout {
                    anchors.fill: parent
                    spacing: 12

                    Repeater {
                        model: root.backend.creditHeadings()

                        ColumnLayout {
                            id: creditSection

                            required property int index
                            required property string modelData

                            Label { text: creditSection.modelData; font.bold: true }
                            Label { text: root.backend.creditEntries(creditSection.index) }
                        }
                    }
                    Item { Layout.fillHeight: true }
                }
            }

            ScrollView {
                clip: true

                TextArea {
                    text: root.backend.licenseText()
                    readOnly: true
                    selectByMouse: true
                    wrapMode: TextEdit.NoWrap
                    font.family: root.backend.fixedFontFamily
                    background: null
                }
            }
        }

        DialogButtonBox {
            Layout.fillWidth: true
            standardButtons: DialogButtonBox.Close
            onRejected: root.close()
        }
    }
}
