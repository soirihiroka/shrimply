import QtCore
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.shrimply.launcher

ApplicationWindow {
    id: window

    width: 760
    height: 560
    minimumWidth: 560
    minimumHeight: 420
    visible: true
    title: backend.text("Shrimply")

    readonly property int sidebarWidth: 200
    readonly property int pageMargin: 12
    readonly property int revealFocusDelay: 120
    property url pendingDirectory

    LauncherBackend {
        id: backend
    }

    Connections {
        target: backend

        function onShowError(heading, body) {
            errorDialog.title = heading
            errorBody.text = body
            errorDialog.open()
        }

        function onEditorStarted() {
            window.hide()
        }

        function onEditorFinished() {
            Qt.quit()
        }

        function onOpenDirectory(url, afterReveal) {
            if (afterReveal) {
                window.pendingDirectory = url
                revealTimer.restart()
            } else {
                Qt.openUrlExternally(url)
            }
        }
    }

    Timer {
        id: revealTimer
        interval: window.revealFocusDelay
        repeat: false
        onTriggered: Qt.openUrlExternally(window.pendingDirectory)
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        Pane {
            Layout.preferredWidth: window.sidebarWidth
            Layout.minimumWidth: window.sidebarWidth
            Layout.maximumWidth: window.sidebarWidth
            Layout.fillHeight: true

            ColumnLayout {
                anchors.fill: parent
                spacing: 12

                Button {
                    Layout.fillWidth: true
                    text: backend.text("Create Project")
                    highlighted: true
                    onClicked: createDialog.open()
                }

                Button {
                    Layout.fillWidth: true
                    text: backend.text("Open Project")
                    onClicked: backend.chooseProject()
                }

                Item {
                    Layout.fillHeight: true
                }
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            ToolBar {
                Layout.fillWidth: true

                RowLayout {
                    anchors.fill: parent
                    anchors.margins: 6
                    spacing: 6

                    TextField {
                        id: search
                        Layout.fillWidth: true
                        placeholderText: backend.text("Search history")
                        selectByMouse: true
                        onTextChanged: backend.setSearch(text)
                    }

                    ToolButton {
                        icon.name: "edit-clear-history"
                        text: backend.text("Clear History")
                        display: AbstractButton.IconOnly
                        ToolTip.visible: hovered
                        ToolTip.text: text
                        onClicked: backend.clearHistory()
                    }
                }
            }

            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.margins: window.pageMargin

                ListView {
                    id: recentList
                    anchors.fill: parent
                    spacing: 8
                    clip: true
                    model: backend.recentCount
                    visible: count > 0

                    delegate: ItemDelegate {
                        id: recentDelegate
                        required property int index
                        width: recentList.width
                        height: 64
                        rightPadding: optionsButton.width + 12
                        text: backend.recentName(index) + "\n"
                            + backend.recentLastEdited(index)
                        onClicked: backend.openRecent(index)

                        ToolButton {
                            id: optionsButton
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            icon.name: "overflow-menu"
                            text: backend.text("Project options")
                            display: AbstractButton.IconOnly
                            ToolTip.visible: hovered
                            ToolTip.text: text
                            onClicked: options.popup(
                                optionsButton, Qt.point(0, optionsButton.height))

                            Menu {
                                id: options
                                popupType: Popup.Native

                                MenuItem {
                                    text: backend.text("Info")
                                    onTriggered: {
                                        infoDialog.recentIndex = recentDelegate.index
                                        infoDialog.open()
                                    }
                                }

                                MenuItem {
                                    text: backend.text("Show in Files")
                                    onTriggered: backend.showRecent(recentDelegate.index)
                                }

                                MenuItem {
                                    text: backend.text("Delete")
                                    onTriggered: backend.removeRecent(recentDelegate.index)
                                }
                            }
                        }
                    }

                    ScrollBar.vertical: ScrollBar {}
                }

                ColumnLayout {
                    anchors.centerIn: parent
                    visible: backend.recentCount === 0

                    ToolButton {
                        Layout.alignment: Qt.AlignHCenter
                        icon.name: "document-open-recent"
                        display: AbstractButton.IconOnly
                        enabled: false
                    }

                    Label {
                        Layout.alignment: Qt.AlignHCenter
                        text: search.text.trim().length === 0
                            ? backend.text("No Recent Projects")
                            : backend.text("No Matching Projects")
                        font.bold: true
                    }
                }
            }
        }
    }

    Dialog {
        id: errorDialog
        anchors.centerIn: parent
        width: Math.min(460, window.width - 48)
        modal: true
        standardButtons: Dialog.Close

        Label {
            id: errorBody
            width: parent.width
            wrapMode: Text.Wrap
        }
    }

    Dialog {
        id: infoDialog
        property int recentIndex: -1
        anchors.centerIn: parent
        width: Math.min(500, window.width - 48)
        modal: true
        title: recentIndex >= 0 ? backend.recentName(recentIndex) : ""
        standardButtons: Dialog.Close

        ColumnLayout {
            width: parent.width
            spacing: 14

            Label {
                text: backend.text("Last Edited")
                font.bold: true
            }

            Label {
                Layout.fillWidth: true
                text: infoDialog.recentIndex >= 0
                    ? backend.recentLastEdited(infoDialog.recentIndex)
                    : backend.text("Unavailable")
            }

            Label {
                text: backend.text("File Location")
                font.bold: true
            }

            RowLayout {
                Layout.fillWidth: true

                Label {
                    Layout.fillWidth: true
                    text: infoDialog.recentIndex >= 0
                        ? backend.recentPath(infoDialog.recentIndex) : ""
                    wrapMode: Text.WrapAnywhere
                }

                ToolButton {
                    icon.name: "document-open-folder"
                    text: backend.text("Show in Files")
                    display: AbstractButton.IconOnly
                    ToolTip.visible: hovered
                    ToolTip.text: text
                    onClicked: backend.showRecent(infoDialog.recentIndex)
                }
            }
        }
    }

    Dialog {
        id: createDialog
        anchors.centerIn: parent
        width: Math.min(500, window.width - 48)
        modal: true
        title: backend.text("Create Project")

        contentItem: ColumnLayout {
            spacing: 12

            Label {
                text: backend.text("Project Name")
                font.bold: true
            }

            TextField {
                id: projectName
                Layout.fillWidth: true
                text: backend.text("Untitled Project")
                selectByMouse: true
            }

            Label {
                text: backend.text("Preset")
                font.bold: true
            }

            ComboBox {
                id: preset
                Layout.fillWidth: true
                model: backend.presetCount
                currentIndex: 3
                displayText: currentIndex >= 0 ? backend.presetLabel(currentIndex) : ""
                delegate: ItemDelegate {
                    required property int index
                    width: preset.width
                    text: backend.presetLabel(index)
                    highlighted: preset.highlightedIndex === index
                }
                onActivated: function(index) {
                    if (index < backend.presetCount - 1) {
                        widthInput.value = backend.presetWidth(index)
                        heightInput.value = backend.presetHeight(index)
                        frameRate.currentIndex = backend.presetFrameRate(index)
                    }
                }
            }

            Label {
                text: backend.text("Project Settings")
                font.bold: true
            }

            RowLayout {
                Layout.fillWidth: true

                Label {
                    text: backend.text("Width")
                    Layout.fillWidth: true
                }

                SpinBox {
                    id: widthInput
                    from: 1
                    to: 16384
                    value: 1920
                    editable: true
                    onValueModified: preset.currentIndex = backend.presetCount - 1
                }
            }

            RowLayout {
                Layout.fillWidth: true

                Label {
                    text: backend.text("Height")
                    Layout.fillWidth: true
                }

                SpinBox {
                    id: heightInput
                    from: 1
                    to: 16384
                    value: 1080
                    editable: true
                    onValueModified: preset.currentIndex = backend.presetCount - 1
                }
            }

            RowLayout {
                Layout.fillWidth: true

                Label {
                    text: backend.text("Frame Rate")
                    Layout.fillWidth: true
                }

                ComboBox {
                    id: frameRate
                    model: backend.frameRateCount
                    currentIndex: 8
                    displayText: currentIndex >= 0
                        ? backend.frameRateLabel(currentIndex) : ""
                    delegate: ItemDelegate {
                        required property int index
                        width: frameRate.width
                        text: backend.frameRateLabel(index)
                        highlighted: frameRate.highlightedIndex === index
                    }
                    onActivated: preset.currentIndex = backend.presetCount - 1
                }
            }

            Button {
                Layout.alignment: Qt.AlignHCenter
                Layout.topMargin: 8
                text: backend.text("Create Project")
                highlighted: true
                enabled: projectName.text.trim().length > 0
                onClicked: {
                    createDialog.close()
                    const selectedFile = backend.chooseProjectDestination(
                        projectName.text.trim())
                    if (selectedFile.toString().length > 0) {
                        backend.requestCreateProject(
                            projectName.text.trim(),
                            widthInput.value,
                            heightInput.value,
                            frameRate.currentIndex,
                            selectedFile)
                    }
                }
            }
        }
    }
}
