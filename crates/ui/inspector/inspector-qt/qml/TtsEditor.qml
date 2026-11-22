pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.shrimply.components
import dev.shrimply.inspector

ColumnLayout {
    id: root
    property string audioId: ""
    property int documentRevision: 0
    signal error(string message)
    signal openPath(url url)
    spacing: 10

    TtsEditorBackend {
        id: backend
        audioId: root.audioId
        onError: function(message) { root.error(message) }
        onOpenPath: function(url) { root.openPath(url) }
    }

    Component.onCompleted: backend.refresh()
    onAudioIdChanged: backend.refresh()
    onDocumentRevisionChanged: backend.refresh()

    Timer {
        interval: 50
        repeat: true
        running: root.visible
        onTriggered: backend.poll()
    }

    ControlRow {
        label: backend.label("model")
        Dropdown {
            enabled: backend.modelEnabled
            value: {
                const revision = backend.revision
                return revision >= 0 ? backend.modelValue() : ""
            }
            values: {
                const revision = backend.revision
                return revision >= 0 ? backend.modelValues() : []
            }
            labels: {
                const revision = backend.revision
                return revision >= 0 ? backend.modelLabels() : []
            }
            onSelected: function(value) { backend.setModel(value) }
        }
    }

    Repeater {
        model: {
            const revision = backend.revision
            return revision >= 0 ? backend.controlCount() : 0
        }

        Loader {
            id: controlLoader
            required property int index
            readonly property int kind: {
                const revision = backend.revision
                return revision >= 0 ? backend.controlKind(index) : -1
            }
            readonly property string label: {
                const revision = backend.revision
                return revision >= 0 ? backend.controlLabel(index) : ""
            }
            readonly property string value: {
                const revision = backend.revision
                return revision >= 0 ? backend.controlValue(index) : ""
            }
            enabled: backend.inputsEnabled
            Layout.fillWidth: true
            sourceComponent: kind === 0 ? singleTextControl
                : kind === 1 ? multilineTextControl
                : kind === 2 ? selectorControl
                : kind === 3 ? audioControl
                : kind === 4 ? toggleControl
                : kind === 5 ? numberControl
                : tableControl

            Component {
                id: singleTextControl
                ControlRow {
                    label: controlLoader.label
                    SingleLineTextInput {
                        value: controlLoader.value
                        maximumLength: backend.controlMaximumLength(controlLoader.index)
                        onEdited: function(value) {
                            backend.setControlValue(controlLoader.index, value)
                        }
                        onCommitted: function(value) { backend.commitControl() }
                    }
                }
            }

            Component {
                id: multilineTextControl
                ControlRow {
                    label: controlLoader.label
                    MultilineTextInput {
                        value: controlLoader.value
                        maximumLength: backend.controlMaximumLength(controlLoader.index)
                        onEdited: function(value) {
                            backend.setControlValue(controlLoader.index, value)
                        }
                        onCommitted: function(value) { backend.commitControl() }
                    }
                }
            }

            Component {
                id: selectorControl
                ControlRow {
                    label: controlLoader.label
                    Dropdown {
                        value: controlLoader.value
                        values: {
                            const revision = backend.revision
                            return revision >= 0
                                ? backend.controlChoiceValues(controlLoader.index) : []
                        }
                        labels: {
                            const revision = backend.revision
                            return revision >= 0
                                ? backend.controlChoiceLabels(controlLoader.index) : []
                        }
                        onSelected: function(value) {
                            backend.setControlValue(controlLoader.index, value)
                        }
                    }
                }
            }

            Component {
                id: audioControl
                ControlRow {
                    label: controlLoader.label
                    RowLayout {
                        spacing: 6
                        Label {
                            text: controlLoader.value.length > 0
                                ? controlLoader.value : backend.label("audio_empty")
                            elide: Text.ElideMiddle
                            opacity: controlLoader.value.length > 0 ? 1 : 0.56
                            Layout.fillWidth: true
                        }
                        Button {
                            text: backend.label("audio_choose")
                            onClicked: backend.chooseControlAudio(controlLoader.index)
                        }
                        Button {
                            icon.source: "qrc:/shrimply/icons/down-symbolic.svg"
                            display: AbstractButton.IconOnly
                            ToolTip.visible: hovered
                            ToolTip.text: backend.label("audio_actions")
                            onClicked: audioActions.popup()
                            Menu {
                                id: audioActions
                                MenuItem {
                                    text: backend.label("audio_clear")
                                    enabled: controlLoader.value.length > 0
                                    onTriggered: backend.clearControlAudio(controlLoader.index)
                                }
                                MenuItem {
                                    text: backend.label("audio_reveal")
                                    enabled: controlLoader.value.length > 0
                                    onTriggered: backend.showControlAudio(controlLoader.index)
                                }
                            }
                        }
                    }
                }
            }

            Component {
                id: toggleControl
                SwitchRow {
                    label: controlLoader.label
                    active: controlLoader.value === "true"
                    onToggled: function(active) {
                        backend.setControlToggle(controlLoader.index, active)
                    }
                }
            }

            Component {
                id: numberControl
                ControlRow {
                    label: controlLoader.label
                    NumberPicker {
                        value: Number(controlLoader.value)
                        minimum: backend.controlMinimum(controlLoader.index)
                        maximum: backend.controlMaximum(controlLoader.index)
                        dragStep: backend.controlStep(controlLoader.index)
                        digits: backend.controlDigits(controlLoader.index)
                        onFractionEdited: function(numerator, denominator) {
                            backend.setControlNumber(
                                controlLoader.index, numerator, denominator)
                        }
                        onCommitted: function(value) { backend.commitControl() }
                    }
                }
            }

            Component {
                id: tableControl
                ColumnLayout {
                    Layout.fillWidth: true
                    RowLayout {
                        Layout.fillWidth: true
                        Label {
                            text: controlLoader.label
                            opacity: 0.72
                            Layout.fillWidth: true
                        }
                        Button {
                            icon.source: "qrc:/shrimply/icons/list-add-symbolic.svg"
                            display: AbstractButton.IconOnly
                            ToolTip.visible: hovered
                            ToolTip.text: backend.label("add_row")
                            onClicked: backend.addTableRow(controlLoader.index)
                        }
                    }
                    Repeater {
                        model: {
                            const revision = backend.revision
                            return revision >= 0
                                ? backend.tableRowCount(controlLoader.index) : 0
                        }
                        RowLayout {
                            id: tableRow
                            required property int index
                            readonly property int rowIndex: index
                            Layout.fillWidth: true
                            Repeater {
                                model: {
                                    const revision = backend.revision
                                    return revision >= 0
                                        ? backend.tableColumnCount(controlLoader.index) : 0
                                }
                                SingleLineTextInput {
                                    required property int index
                                    readonly property int columnIndex: index
                                    value: {
                                        const revision = backend.revision
                                        return revision >= 0 ? backend.tableValue(
                                            controlLoader.index, tableRow.rowIndex,
                                            columnIndex) : ""
                                    }
                                    placeholderText: backend.tableColumnLabel(
                                        controlLoader.index, columnIndex)
                                    maximumLength: backend.tableColumnMaximumLength(
                                        controlLoader.index, columnIndex)
                                    Layout.fillWidth: true
                                    onEdited: function(value) {
                                        backend.setTableValue(controlLoader.index,
                                            tableRow.rowIndex, columnIndex, value)
                                    }
                                    onCommitted: function(value) { backend.commitControl() }
                                }
                            }
                            Button {
                                icon.source: "qrc:/shrimply/icons/list-remove-symbolic.svg"
                                display: AbstractButton.IconOnly
                                ToolTip.visible: hovered
                                ToolTip.text: backend.label("remove_row")
                                onClicked: backend.removeTableRow(
                                    controlLoader.index, tableRow.rowIndex)
                            }
                        }
                    }
                }
            }
        }
    }

    RowLayout {
        Layout.fillWidth: true
        BusyIndicator {
            visible: backend.busy
            running: visible
            implicitWidth: 24
            implicitHeight: 24
            Layout.preferredWidth: 24
            Layout.preferredHeight: 24
            Layout.alignment: Qt.AlignVCenter
        }
        Label {
            text: backend.status
            wrapMode: Text.Wrap
            Layout.fillWidth: true
            ToolTip.visible: statusHover.hovered && backend.statusTooltip.length > 0
            ToolTip.text: backend.statusTooltip
            HoverHandler { id: statusHover }
        }
        Button {
            visible: backend.generateVisible
            enabled: backend.generateEnabled
            highlighted: true
            text: backend.generateLabel
            onClicked: backend.generate()
        }
    }
}
