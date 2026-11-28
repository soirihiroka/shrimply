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

    readonly property int sidebarWidth: 140
    readonly property int pageMargin: 12

    LauncherBackend {
        id: backend
    }

    Timer {
        interval: 2000
        running: window.active
        repeat: true
        triggeredOnStart: true
        onTriggered: backend.refreshDaemonStatus()
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

        ColumnLayout {
            Layout.preferredWidth: window.sidebarWidth
            Layout.fillHeight: true
            Layout.margins: window.pageMargin
            Image {
                Layout.alignment: Qt.AlignHCenter
                Layout.preferredWidth: 48
                Layout.preferredHeight: 48
                source: "qrc:/shrimply/icons/dev.shrimply.Shrimply.svg"
            }
            Label {
                Layout.alignment: Qt.AlignHCenter
                text: window.title
                font.bold: true
            }
            Label {
                Layout.alignment: Qt.AlignHCenter
                text: backend.version()
                opacity: 0.75
            }
            Label {
                Layout.alignment: Qt.AlignHCenter
                text: backend.daemonStatus
                opacity: 0.75
            }
            Item { Layout.fillHeight: true }
            ToolButton {
                id: mainMenuButton
                text: backend.text("Main Menu")
                display: AbstractButton.IconOnly
                icon.source: "qrc:/shrimply/icons/open-menu-symbolic.svg"
                ToolTip.visible: hovered
                ToolTip.text: text
                MenuInput { id: mainMenuInput }
                onClicked: mainMenu.presentForInput(mainMenuButton, mainMenuInput.takeTouch())
            }
        }
        Pane {
            Layout.fillWidth: true
            Layout.fillHeight: true
            ColumnLayout {
                anchors.fill: parent
                Label {
                    text: backend.text("Projects")
                    font.bold: true
                }
                RowLayout {
                    Layout.fillWidth: true
                    TextField {
                        id: search
                        Layout.fillWidth: true
                        placeholderText: backend.text("Search Projects")
                        selectByMouse: true
                        onTextChanged: backend.setSearch(text)
                    }
                    ToolButton {
                        text: backend.text("Create Project")
                        display: AbstractButton.IconOnly
                        icon.source: "qrc:/shrimply/icons/list-add-symbolic.svg"
                        ToolTip.visible: hovered
                        ToolTip.text: text
                        onClicked: createDialog.open()
                    }
                    ToolButton {
                        text: backend.text("Open Project")
                        display: AbstractButton.IconOnly
                        icon.source: "qrc:/shrimply/icons/document-open-symbolic.svg"
                        ToolTip.visible: hovered
                        ToolTip.text: text
                        onClicked: backend.chooseProject()
                    }
                }
                Item {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    ListView {
                        id: projects
                        anchors.fill: parent
                        clip: true
                        model: backend.projects
                        spacing: 8
                        delegate: Rectangle {
                            required property var modelData
                            width: projects.width - (projectScrollBar.visible ? projectScrollBar.width : 0)
                            height: card.implicitHeight + 8
                            radius: 8
                            color: window.palette.alternateBase
                            RowLayout {
                                id: card
                                anchors.fill: parent
                                anchors.margins: 4
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
                        }
                        ScrollBar.vertical: ScrollBar { id: projectScrollBar; policy: ScrollBar.AsNeeded }
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
        onSettingsClosed: backend.reloadProjects()
    }

    Shortcut {
        sequence: StandardKey.Preferences
        onActivated: preferencesWindow.openPreferences()
    }

    AdaptiveMenu {
        id: mainMenu
        pointerPopupType: Qt.platform.os === "windows" ? Popup.Item : Popup.Window
        Action { text: backend.text("Settings"); onTriggered: preferencesWindow.openPreferences() }
        Action { text: backend.text("About Shrimply"); onTriggered: aboutWindow.openAbout() }
    }

    AdaptiveMenu {
        id: projectMenu
        property var project: ({})
        pointerPopupType: mainMenu.pointerPopupType

        Action {
            text: backend.text("Project Information")
            onTriggered: {
                errorDialog.title = projectMenu.project.name
                errorBody.text = projectMenu.project.details
                errorDialog.open()
            }
        }
        Action {
            text: backend.text("Show in Files")
            enabled: Boolean(projectMenu.project.cached)
            onTriggered: Qt.openUrlExternally(projectMenu.project.folder)
        }
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
