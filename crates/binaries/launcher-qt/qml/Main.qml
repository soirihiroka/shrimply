import dev.shrimply.components
import dev.shrimply.editor
import dev.shrimply.preferences
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
        function onProjectServersUpdated(success) {
            projectServersDialog.updating = false
            if (success) {
                projectServerRows.model = backend.projectServers(false)
                projectServersDialog.previous = ""
                newProjectServer.clear()
                projectUsername.clear()
            }
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

        ColumnLayout {
            Layout.preferredWidth: window.sidebarWidth
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
            Item { Layout.fillHeight: true }
        }
        Pane {
            Layout.fillWidth: true
            Layout.fillHeight: true
            ColumnLayout {
                anchors.fill: parent
                RowLayout {
                    Layout.fillWidth: true
                    Label {
                        Layout.fillWidth: true
                        text: backend.text("Projects")
                        font.bold: true
                    }
                    ToolButton {
                        id: mainMenuButton
                        text: backend.text("Main Menu")
                        display: AbstractButton.IconOnly
                        icon.source: "qrc:/shrimply/icons/open-menu-symbolic.svg"
                        MenuInput { id: mainMenuInput }
                        onClicked: mainMenu.presentForInput(mainMenuButton, mainMenuInput.takeTouch())
                    }
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
                        delegate: RowLayout {
                            required property var modelData
                            width: projects.width
                            spacing: 0

                            ItemDelegate {
                                Layout.fillWidth: true
                                text: [modelData.name, modelData.access, modelData.remote].filter(value => value).join("\n")
                                icon.source: modelData.remote
                                    ? "qrc:/shrimply/icons/folder-remote-symbolic.svg"
                                    : "qrc:/shrimply/icons/folder-open-symbolic.svg"
                                icon.width: 16
                                icon.height: 16
                                onClicked: backend.openListedProject(modelData.key)
                            }
                            ToolButton {
                                id: projectMenuButton
                                text: backend.text("Project options")
                                display: AbstractButton.IconOnly
                                icon.source: "qrc:/shrimply/icons/view-more-symbolic.svg"
                                MenuInput { id: rowMenuInput }
                                onClicked: {
                                    projectMenu.project = modelData
                                    projectMenu.presentForInput(projectMenuButton, rowMenuInput.takeTouch())
                                }
                            }
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
    }

    EditorBackend {
        id: settingsBackend
    }

    AboutWindow {
        id: aboutWindow
        backend: settingsBackend
        owner: window
    }

    PreferencesWindow {
        id: preferencesWindow
        backend: settingsBackend
        owner: window
    }

    Shortcut {
        sequence: StandardKey.Preferences
        onActivated: preferencesWindow.openPreferences()
    }

    Timer {
        interval: 100
        repeat: true
        running: preferencesWindow.visible
        onTriggered: settingsBackend.pollPreferenceTasks()
    }

    AdaptiveMenu {
        id: mainMenu
        pointerPopupType: Qt.platform.os === "windows" ? Popup.Item : Popup.Window
        Action { text: backend.text("Preferences"); onTriggered: preferencesWindow.openPreferences() }
        Action { text: backend.text("Project Servers"); onTriggered: projectServersDialog.open() }
        Action { text: backend.text("About Shrimply"); onTriggered: aboutWindow.openAbout() }
    }

    AdaptiveMenu {
        id: projectMenu
        property var project: ({})
        pointerPopupType: mainMenu.pointerPopupType

        Action {
            text: backend.text("Restart Local Service")
            enabled: Boolean(projectMenu.project.cached)
            onTriggered: backend.manageListedProject(projectMenu.project.key, false)
        }
        Action {
            text: backend.text("Delete Project…")
            onTriggered: {
                deleteDialog.projectKey = projectMenu.project.key
                deleteDialog.title = projectMenu.project.name
                deleteMessage.text = backend.text(projectMenu.project.remote
                    ? "Delete this project from the server for everyone?"
                    : "Permanently delete this project file?")
                deleteDialog.open()
            }
        }
    }

    Dialog {
        id: deleteDialog
        property string projectKey
        anchors.centerIn: parent
        modal: true
        width: Math.min(460, window.width - 48)
        standardButtons: Dialog.Cancel | Dialog.Discard
        onOpened: standardButton(Dialog.Discard).text = backend.text("Delete Project")
        onDiscarded: backend.manageListedProject(projectKey, true)
        Label {
            id: deleteMessage
            wrapMode: Text.Wrap
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
        property bool updating: false
        property string previous: ""
        onOpened: projectServerRows.model = backend.projectServers(false)
        onClosed: {
            previous = ""
            newProjectServer.clear()
            projectUsername.clear()
            projectPassword.clear()
        }

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

                    Label {
                        Layout.fillWidth: true
                        text: modelData
                        elide: Text.ElideRight
                    }

                    ToolButton {
                        icon.source: "qrc:/shrimply/icons/pencil-symbolic.svg"
                        text: backend.text("Edit")
                        display: AbstractButton.IconOnly
                        enabled: !projectServersDialog.updating
                        onClicked: {
                            projectServersDialog.previous = modelData
                            newProjectServer.text = modelData
                            projectPassword.clear()
                            projectUsername.forceActiveFocus()
                        }
                    }

                    ToolButton {
                        icon.source: "qrc:/shrimply/icons/user-trash-symbolic.svg"
                        text: backend.text("Remove Server")
                        display: AbstractButton.IconOnly
                        ToolTip.visible: hovered
                        ToolTip.text: text
                        enabled: !projectServersDialog.updating
                        onClicked: {
                            projectServersDialog.updating = true
                            backend.updateProjectServer(modelData, "", "", "")
                        }
                    }
                }
            }

            TextField {
                id: newProjectServer
                Layout.fillWidth: true
                enabled: !projectServersDialog.updating
                placeholderText: backend.text("Server URL")
            }
            TextField {
                id: projectUsername
                Layout.fillWidth: true
                enabled: !projectServersDialog.updating
                placeholderText: backend.text("Username")
            }
            TextField {
                id: projectPassword
                Layout.fillWidth: true
                enabled: !projectServersDialog.updating
                placeholderText: backend.text("Password")
                echoMode: TextInput.Password
                onAccepted: if (signIn.enabled) signIn.clicked()
            }
            Button {
                id: signIn
                text: backend.text("Sign In")
                enabled: !projectServersDialog.updating && newProjectServer.text.trim().length > 0 &&
                    projectUsername.text.trim().length > 0 && projectPassword.text.length > 0
                onClicked: {
                    projectServersDialog.updating = true
                    backend.updateProjectServer(projectServersDialog.previous, newProjectServer.text,
                                                projectUsername.text, projectPassword.text)
                    projectPassword.clear()
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

            ControlRow {
                label: "Location"
                AdaptiveComboBox {
                    id: projectDestination
                    enabled: count > 1
                }
            }

            ControlRow {
                label: "Project Name"
                TextField {
                    id: projectName
                    text: backend.text("Untitled Project")
                    selectByMouse: true
                }
            }

            Label {
                text: backend.text("Project Settings")
                font.bold: true
            }

            ProjectSettingsSelector {
                id: settings
                Layout.fillWidth: true
                creating: true
            }

            Button {
                Layout.alignment: Qt.AlignHCenter
                Layout.topMargin: 8
                text: backend.text("Create Project")
                highlighted: true
                enabled: projectName.text.trim().length > 0
                onClicked: {
                    createDialog.close()
                    backend.requestCreateProject(
                        projectName.text.trim(), settings.projectWidth, settings.projectHeight,
                        settings.frameRate,
                        projectDestination.currentIndex > 0 ? projectDestination.currentText : "")
                }
            }
        }
    }
}
