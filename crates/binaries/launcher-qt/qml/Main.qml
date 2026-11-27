import dev.shrimply.components
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

    readonly property int sidebarWidth: 330
    readonly property int pageMargin: 12

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

    }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        Pane {
            Layout.preferredWidth: window.sidebarWidth
            Layout.fillHeight: true
            ColumnLayout {
                anchors.fill: parent
                Label {
                    text: backend.text("Projects")
                    font.bold: true
                }
                TextField {
                    id: search
                    Layout.fillWidth: true
                    placeholderText: backend.text("Search Projects")
                    selectByMouse: true
                    onTextChanged: backend.setSearch(text)
                }
                Item {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    ListView {
                        id: projects
                        anchors.fill: parent
                        clip: true
                        model: backend.projects
                        delegate: ItemDelegate {
                            required property string modelData
                            required property int index
                            width: projects.width
                            text: modelData
                            icon.source: backend.projectIsRemote(index)
                                ? "qrc:/shrimply/icons/folder-remote-symbolic.svg"
                                : "qrc:/shrimply/icons/folder-open-symbolic.svg"
                            icon.width: 16
                            icon.height: 16
                            onClicked: backend.openListedProject(index)
                        }
                        ScrollBar.vertical: ScrollBar {}
                    }
                    Label {
                        anchors.centerIn: parent
                        visible: projects.count === 0
                        text: backend.text(search.text.trim().length ? "No Matching Projects" : "No Projects")
                    }
                }
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.margins: window.pageMargin
            Item { Layout.fillHeight: true }
            Button {
                Layout.alignment: Qt.AlignHCenter
                text: backend.text("Create Project")
                highlighted: true
                onClicked: createDialog.open()
            }
            Button {
                Layout.alignment: Qt.AlignHCenter
                text: backend.text("Open Project")
                onClicked: backend.chooseProject()
            }
            Button {
                Layout.alignment: Qt.AlignHCenter
                text: backend.text("Project Servers")
                onClicked: projectServersDialog.open()
            }
            Item { Layout.fillHeight: true }
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
        id: projectServersDialog
        anchors.centerIn: parent
        width: Math.min(500, window.width - 48)
        title: backend.text("Project Servers")
        modal: true
        standardButtons: Dialog.Close
        onOpened: projectServerRows.model = backend.projectServers(false)

        ColumnLayout {
            width: parent.width

            ListView {
                id: projectServerRows
                Layout.fillWidth: true
                Layout.preferredHeight: Math.min(contentHeight, 220)
                clip: true
                delegate: RowLayout {
                    required property string modelData
                    width: projectServerRows.width

                    TextField {
                        id: savedUrl
                        Layout.fillWidth: true
                        text: modelData
                        onAccepted: {
                            if (text.trim().length > 0 && backend.updateProjectServer(modelData, text)) {
                                projectServerRows.model = backend.projectServers(false)
                            }
                        }
                    }

                    ToolButton {
                        icon.source: "qrc:/shrimply/icons/user-trash-symbolic.svg"
                        text: backend.text("Remove Server")
                        display: AbstractButton.IconOnly
                        ToolTip.visible: hovered
                        ToolTip.text: text
                        onClicked: {
                            if (backend.updateProjectServer(modelData, "")) {
                                projectServerRows.model = backend.projectServers(false)
                            }
                        }
                    }
                }
            }

            RowLayout {
                TextField {
                    id: newProjectServer
                    Layout.fillWidth: true
                    placeholderText: backend.text("Server URL")
                    onAccepted: addProjectServer.clicked()
                }

                Button {
                    id: addProjectServer
                    text: backend.text("Add Server")
                    enabled: newProjectServer.text.trim().length > 0
                    onClicked: {
                        if (backend.updateProjectServer("", newProjectServer.text)) {
                            newProjectServer.clear()
                            projectServerRows.model = backend.projectServers(false)
                        }
                    }
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
        onOpened: {
            projectDestination.model = backend.projectServers(true)
            projectDestination.currentIndex = 0
        }

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

            RowLayout {
                Layout.fillWidth: true
                Label {
                    text: backend.text("Location")
                    Layout.fillWidth: true
                }
                ComboBox {
                    id: projectDestination
                    Layout.fillWidth: true
                }
            }

            Label {
                text: backend.text("Preset")
                font.bold: true
            }

            AdaptiveComboBox {
                id: preset
                Layout.fillWidth: true
                model: backend.presetCount
                currentIndex: 3
                displayText: currentIndex >= 0 ? backend.presetLabel(currentIndex) : ""
                labelForIndex: function(index) { return backend.presetLabel(index) }
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

                AdaptiveComboBox {
                    id: frameRate
                    model: backend.frameRateCount
                    currentIndex: 8
                    displayText: currentIndex >= 0
                        ? backend.frameRateLabel(currentIndex) : ""
                    labelForIndex: function(index) { return backend.frameRateLabel(index) }
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
                    if (projectDestination.currentIndex > 0) {
                        backend.createRemoteProject(
                            projectDestination.currentText, projectName.text.trim(),
                            widthInput.value, heightInput.value, frameRate.currentIndex)
                        return
                    }
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
