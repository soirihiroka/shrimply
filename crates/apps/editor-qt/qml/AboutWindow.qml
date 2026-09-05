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
                        text: backend.translate("Version") + " " + backend.applicationVersion()
                        opacity: 0.7
                    }
                    Label {
                        Layout.fillWidth: true
                        text: backend.translate("A simple video editor")
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

            TabButton { text: backend.translate("About") }
            TabButton { text: backend.translate("Authors") }
            TabButton { text: backend.translate("License") }
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
                        text: backend.translate("A simple video editor")
                        font.pointSize: 14
                        wrapMode: Text.Wrap
                    }
                    Label {
                        Layout.fillWidth: true
                        text: "Shrimply brings your ideas to life."
                        wrapMode: Text.Wrap
                    }
                    RowLayout {
                        Button {
                            text: backend.translate("Website")
                            icon.name: "internet-services-symbolic"
                            onClicked: Qt.openUrlExternally("https://github.com/soirihiroka/shrimply")
                        }
                        Button {
                            text: backend.translate("Report an Issue")
                            icon.name: "tools-report-bug"
                            onClicked: Qt.openUrlExternally("https://github.com/soirihiroka/shrimply/issues/new")
                        }
                        Item { Layout.fillWidth: true }
                    }
                    Item { Layout.fillHeight: true }
                }
            }

            Pane {
                padding: 24

                ColumnLayout {
                    anchors.fill: parent
                    spacing: 8

                    Label { text: "Soiri Hiroka"; font.bold: true }
                    Label { text: backend.translate("Developer and maintainer"); opacity: 0.7 }
                    Label { text: "Codex"; font.bold: true; Layout.topMargin: 12 }
                    Label { text: backend.translate("AI development agent"); opacity: 0.7 }
                    Label { text: "Gemini"; font.bold: true; Layout.topMargin: 12 }
                    Label { text: backend.translate("AI development agent"); opacity: 0.7 }
                    Item { Layout.fillHeight: true }
                }
            }

            ScrollView {
                clip: true

                TextArea {
                    text: backend.licenseText()
                    readOnly: true
                    selectByMouse: true
                    wrapMode: TextEdit.NoWrap
                    font.family: backend.fixedFontFamily
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
