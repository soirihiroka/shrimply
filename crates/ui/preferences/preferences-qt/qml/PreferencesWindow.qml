pragma ComponentBehavior: Bound

import dev.shrimply.components
import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts

ApplicationWindow {
    id: root

    required property var backend
    required property var owner
    signal settingsClosed()
    property string serialized: ""
    property var document: ({})
    property int selected: 0
    property bool finishWhenIdle: false
    readonly property var pages: document.controls || []
    readonly property var page: pages[selected]
    readonly property bool wide: width >= (document.wide_width || 780)

    title: document.title || backend.translate("Settings")
    transientParent: owner
    modality: Qt.WindowModal
    flags: Qt.Dialog
    width: 900
    height: 610
    minimumWidth: 460
    minimumHeight: 390

    function refresh() {
        const next = backend.settingsDocument()
        if (serialized !== next)
            update(next)
        if (finishWhenIdle && !document.pending) {
            finishWhenIdle = false
            settingsClosed()
        }
    }

    function update(next) {
        serialized = next
        document = JSON.parse(next)
    }

    function openPreferences(pageId) {
        refresh()
        const index = pages.findIndex(page => page.id === pageId)
        if (index >= 0)
            selected = index
        finishWhenIdle = false
        show()
        raise()
        requestActivate()
    }

    function send(command) {
        const next = backend.applySettings(JSON.stringify(command))
        const result = JSON.parse(next)
        if (result.error) {
            notice.heading = backend.translate("Settings")
            notice.message = result.error
            notice.dismiss = null
            notice.open()
            return
        }
        update(next)
        if (result.dialog) {
            notice.heading = result.dialog.title
            notice.message = result.dialog.body
            notice.dismiss = result.dialog.command
            notice.open()
        }
        if (result.source_picker) {
            const picker = result.source_picker.folder ? folder : file
            picker.settings = result.source_picker
            picker.title = result.source_picker.title
            picker.open()
        }
        if (result.show_in_files) {
            const url = backend.revealSettingsPath(result.show_in_files)
            if (url.toString().length)
                Qt.openUrlExternally(url)
        }
    }

    onClosing: {
        if (document.pending)
            finishWhenIdle = true
        else
            settingsClosed()
    }
    Shortcut { sequence: StandardKey.Cancel; onActivated: root.close() }
    Timer { interval: 400; repeat: true; running: root.visible || root.finishWhenIdle; onTriggered: root.refresh() }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        Pane {
            visible: root.wide
            Layout.fillHeight: true
            Layout.preferredWidth: root.document.sidebar_width || 200

            ColumnLayout {
                width: parent.width
                Repeater {
                    model: root.pages
                    ItemDelegate {
                        required property var modelData
                        required property int index
                        Layout.fillWidth: true
                        text: modelData.label
                        icon.source: "qrc:/shrimply/icons/" + modelData.icon + ".svg"
                        icon.color: modelData.color
                        highlighted: root.selected === index
                        onClicked: root.selected = index
                    }
                }
            }
        }

        Rectangle { visible: root.wide; Layout.fillHeight: true; Layout.preferredWidth: 1; color: palette.mid }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            AdaptiveComboBox {
                visible: !root.wide
                Layout.fillWidth: true
                Layout.margins: 16
                model: root.pages.map(page => page.label)
                currentIndex: root.selected
                onActivated: index => root.selected = index
            }

            ScrollView {
                id: scroll
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                padding: 20
                contentWidth: availableWidth

                ColumnLayout {
                    width: scroll.availableWidth
                    spacing: 16
                    Label {
                        visible: root.wide
                        text: root.page?.label || ""
                        font.pixelSize: 22
                        font.bold: true
                    }
                    Repeater {
                        model: root.page?.controls || []
                        GroupBox {
                            id: section
                            required property var modelData
                            Layout.fillWidth: true
                            title: modelData.label
                            ColumnLayout {
                                anchors.fill: parent
                                Repeater {
                                    model: section.modelData.controls
                                    SettingRow {}
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    component SettingRow: RowLayout {
        id: row
        required property var modelData
        readonly property string kind: modelData.widget
        Layout.fillWidth: true
        enabled: modelData.enabled !== false
        ColumnLayout {
            Layout.fillWidth: true
            visible: row.kind !== "button" || !!row.modelData.subtitle
            Label {
                text: row.kind === "button" ? row.modelData.subtitle : row.modelData.label
                textFormat: Text.PlainText
                wrapMode: Text.WrapAnywhere
                Layout.fillWidth: true
            }
            Label {
                visible: row.kind !== "button" && text.length > 0
                text: row.modelData.subtitle || ""
                textFormat: Text.PlainText
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
                opacity: 0.7
            }
        }
        Switch {
            visible: row.kind === "toggle"
            checked: row.modelData.selected === true
            Accessible.name: row.modelData.label
            onToggled: root.send(row.modelData.command)
        }
        AdaptiveComboBox {
            visible: row.kind === "choice"
            Layout.maximumWidth: 270
            model: (row.modelData.choices || []).map(choice => choice.label)
            currentIndex: (row.modelData.choices || []).findIndex(choice => choice.label === row.modelData.display)
            displayText: row.modelData.display || ""
            Accessible.name: row.modelData.label
            onActivated: index => root.send(row.modelData.choices[index].command)
        }
        Button {
            visible: row.kind === "editor" || row.kind === "font" || row.kind === "button"
            Layout.maximumWidth: 270
            text: row.kind === "button" ? row.modelData.label : row.modelData.display || root.backend.translate("Choose…")
            onClicked: {
                if (row.kind === "editor") {
                    editor.settings = row.modelData.editor
                    editor.open()
                } else if (row.kind === "font") {
                    fontDialog.open()
                } else {
                    root.send(row.modelData.command)
                }
            }
        }
        Label {
            visible: row.kind === "text"
            text: row.modelData.display || ""
            textFormat: Text.PlainText
            wrapMode: Text.WrapAnywhere
            Layout.maximumWidth: 330
            opacity: 0.7
        }
        BusyIndicator { visible: row.kind === "loading"; running: visible }
    }

    Dialog {
        id: editor
        property var settings: ({})
        modal: true
        anchors.centerIn: parent
        width: Math.min(root.width - 32, 420)
        title: settings.title || ""
        contentItem: ColumnLayout {
            Repeater {
                id: inputs
                model: editor.settings.fields || []
                TextField {
                    required property var modelData
                    Layout.fillWidth: true
                    placeholderText: modelData.label || ""
                    text: modelData.value || ""
                    echoMode: modelData.keyboard === "password" ? TextInput.Password : TextInput.Normal
                    Accessible.name: modelData.label || editor.title
                }
            }
        }
        footer: DialogButtonBox {
            Button {
                text: editor.settings.cancel_label || ""
                DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
            }
            Button {
                text: editor.settings.confirm_label || ""
                DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole
            }
        }
        onAccepted: {
            const values = []
            for (let index = 0; index < inputs.count; ++index)
                values.push(inputs.itemAt(index).text)
            root.send(Object.assign({}, settings.command, {values: values}))
        }
        onClosed: settings = ({})
    }

    Dialog {
        id: notice
        property string heading: ""
        property string message: ""
        property var dismiss: null
        anchors.centerIn: parent
        width: Math.min(root.width - 32, 440)
        title: heading
        modal: true
        standardButtons: Dialog.Close
        contentItem: Label { text: notice.message; textFormat: Text.PlainText; wrapMode: Text.Wrap }
        onClosed: if (dismiss) root.send(dismiss)
    }
    FileDialog {
        id: file
        property var settings
        onAccepted: root.send(Object.assign({}, settings.complete, {path: root.backend.settingsPath(selectedFile)}))
        onRejected: root.send(settings.cancel)
    }
    FolderDialog {
        id: folder
        property var settings
        onAccepted: root.send(Object.assign({}, settings.complete, {path: root.backend.settingsPath(selectedFolder)}))
        onRejected: root.send(settings.cancel)
    }
    FontDialog {
        id: fontDialog
        onAccepted: root.send({action: "desktop_font", name: selectedFont.family})
    }
}
