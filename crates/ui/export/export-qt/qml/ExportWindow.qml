import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.shrimply.components
import dev.shrimply.export

Item {
    id: root

    property var owner
    readonly property bool busy: exportBackend.busy

    function openVideo() {
        if (exportBackend.busy)
            return
        exportBackend.resetVideo()
        videoWindow.show()
        videoWindow.raise()
        videoWindow.requestActivate()
    }

    function openCaptions() {
        if (exportBackend.busy)
            return
        mergeCaptions.checked = true
        captionsWindow.show()
        captionsWindow.raise()
        captionsWindow.requestActivate()
    }

    function exportJson() {
        if (exportBackend.startJson())
            progressWindow.open()
    }

    ExportBackend {
        id: exportBackend

        onSucceeded: function(title) {
            progressWindow.close()
            successWindow.message = title
            successWindow.open()
        }
        onFailed: function(heading, body) {
            progressWindow.close()
            errorWindow.heading = heading
            errorWindow.message = body
            errorWindow.open()
        }
        onCanceled: progressWindow.close()
        onOpenPath: function(url) { Qt.openUrlExternally(url) }
    }

    Timer {
        interval: 100
        repeat: true
        running: exportBackend.busy
        onTriggered: exportBackend.poll()
    }

    ApplicationWindow {
        id: videoWindow
        readonly property var form: exportBackend.videoForm.length ? JSON.parse(exportBackend.videoForm) : null
        title: form ? form.title : ""
        transientParent: root.owner
        modality: Qt.WindowModal
        flags: Qt.Dialog
        palette: root.owner.palette
        width: 760
        height: 820
        minimumWidth: 600
        minimumHeight: 560

        ColumnLayout {
            anchors.fill: parent
            spacing: 0

            ScrollView {
                id: videoScroll
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                padding: 18
                contentWidth: availableWidth

                ColumnLayout {
                    width: videoScroll.availableWidth
                    spacing: 16

                    Repeater {
                        model: videoWindow.form ? videoWindow.form.groups.length : 0

                        GroupBox {
                            id: groupBox
                            required property int index
                            readonly property var group: videoWindow.form.groups[index]
                            title: group.label
                            visible: group.visible
                            Layout.fillWidth: true

                            ColumnLayout {
                                anchors.fill: parent
                                spacing: 12

                                Repeater {
                                    model: videoWindow.form.fields.length

                                    Loader {
                                        id: fieldLoader
                                        required property int index
                                        readonly property var field: videoWindow.form.fields[index]
                                        visible: field.visible && field.group === groupBox.group.label
                                        active: visible
                                        enabled: field.enabled
                                        Layout.fillWidth: true
                                        sourceComponent: field.widget === "choice" ? choiceField
                                            : field.widget === "toggle" ? toggleField : numberField

                                        function setValue(value) {
                                            exportBackend.setVideoField(field.id, JSON.stringify(value))
                                        }

                                        Component {
                                            id: choiceField
                                            ControlRow {
                                                label: fieldLoader.field.label
                                                AdaptiveComboBox {
                                                    readonly property var choices: fieldLoader.field.choices
                                                    model: choices.map(choice => choice.label)
                                                    currentIndex: choices.findIndex(choice => choice.value === String(fieldLoader.field.value))
                                                    onActivated: fieldLoader.setValue(choices[currentIndex].value)
                                                }
                                            }
                                        }

                                        Component {
                                            id: numberField
                                            ControlRow {
                                                label: fieldLoader.field.label
                                                subtitle: fieldLoader.field.subtitle || ""
                                                NumberPicker {
                                                    minimum: fieldLoader.field.range[0]
                                                    maximum: fieldLoader.field.range[1]
                                                    dragStep: fieldLoader.field.range[2]
                                                    digits: 0
                                                    value: fieldLoader.field.value
                                                    onEdited: function(value) { fieldLoader.setValue(Math.round(value)) }
                                                }
                                            }
                                        }

                                        Component {
                                            id: toggleField
                                            SwitchRow {
                                                label: fieldLoader.field.label
                                                active: fieldLoader.field.value
                                                onToggled: function(active) { fieldLoader.setValue(active) }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: palette.mid }
            RowLayout {
                Layout.fillWidth: true
                Layout.margins: 12
                Item { Layout.fillWidth: true }
                Button { text: videoWindow.form ? videoWindow.form.cancel_label : ""; onClicked: videoWindow.close() }
                Button {
                    text: videoWindow.form ? videoWindow.form.submit_label : ""
                    highlighted: true
                    onClicked: {
                        if (exportBackend.startVideo()) {
                            videoWindow.close()
                            progressWindow.open()
                        }
                    }
                }
            }
        }
    }

    ApplicationWindow {
        id: captionsWindow
        title: exportBackend.translate("Export Captions")
        transientParent: root.owner
        modality: Qt.WindowModal
        flags: Qt.Dialog
        palette: root.owner.palette
        width: 440
        height: 230

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 18
            spacing: 12

            RadioButton {
                id: mergeCaptions
                text: exportBackend.translate("Merge into one file")
                checked: true
            }
            RadioButton {
                text: exportBackend.translate("Export each track separately")
            }
            Item { Layout.fillHeight: true }
            RowLayout {
                Layout.fillWidth: true
                Item { Layout.fillWidth: true }
                Button { text: exportBackend.translate("Cancel"); onClicked: captionsWindow.close() }
                Button {
                    text: exportBackend.translate("Export YTT")
                    highlighted: true
                    onClicked: {
                        if (exportBackend.startCaptions(!mergeCaptions.checked)) {
                            captionsWindow.close()
                            progressWindow.open()
                        }
                    }
                }
            }
        }
    }

    Dialog {
        id: progressWindow
        title: exportBackend.translate("Exporting")
        parent: Overlay.overlay
        modal: true
        width: Math.min(520, root.owner.width - 48)
        anchors.centerIn: parent
        closePolicy: Popup.NoAutoClose
        onClosed: {
            if (exportBackend.busy)
                exportBackend.cancel()
        }

        contentItem: ColumnLayout {
            spacing: 16

            Label {
                Layout.fillWidth: true
                text: exportBackend.status
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
            }
            ProgressBar {
                Layout.fillWidth: true
                from: 0
                to: 1
                value: exportBackend.progress
                indeterminate: !exportBackend.progressDeterminate
            }
            Label {
                Layout.fillWidth: true
                text: exportBackend.progressText
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
            }
        }

        footer: DialogButtonBox {
            Button {
                text: exportBackend.translate("Cancel")
                enabled: exportBackend.busy
                DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
                onClicked: exportBackend.cancel()
            }
        }
    }

    Dialog {
        id: successWindow
        property string message
        title: exportBackend.translate("Export Complete")
        parent: Overlay.overlay
        modal: true
        width: Math.min(480, root.owner.width - 48)
        anchors.centerIn: parent

        contentItem: ColumnLayout {
            spacing: 16
            Label { Layout.fillWidth: true; text: successWindow.message; wrapMode: Text.Wrap }
        }

        footer: DialogButtonBox {
            Button {
                text: exportBackend.translate("Close")
                DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
                onClicked: successWindow.close()
            }
            Button {
                text: exportBackend.translate("Show in Files")
                highlighted: true
                DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole
                onClicked: {
                    successWindow.close()
                    exportBackend.revealLastOutput()
                }
            }
        }
    }

    Dialog {
        id: errorWindow
        property string heading
        property string message
        title: heading
        parent: Overlay.overlay
        modal: true
        width: Math.min(520, root.owner.width - 48)
        anchors.centerIn: parent

        contentItem: ColumnLayout {
            spacing: 16
            Label { Layout.fillWidth: true; text: errorWindow.message; wrapMode: Text.Wrap }
        }

        footer: DialogButtonBox {
            Button {
                text: exportBackend.translate("Close")
                DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
                onClicked: errorWindow.close()
            }
        }
    }
}
