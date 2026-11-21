import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtCore
import dev.shrimply.components
import dev.shrimply.components.demo

ApplicationWindow {
    id: window
    width: 920
    height: 900
    visible: true
    title: "Shrimply Qt Components"
    property string events: ""

    readonly property url homeUrl: StandardPaths.writableLocation(StandardPaths.HomeLocation)
    readonly property string homePath: homeUrl.toString().slice(7)

    function log(message) {
        events = message + "\n" + events
    }

    Tabs {
        anchors.fill: parent
        titles: ["General", "Info", "Log"]
        icons: ["preferences-system", "dialog-information", "utilities-terminal"]

        ScrollView {
        contentWidth: availableWidth
        ColumnLayout {
            width: parent.width
            spacing: 10

            Label { text: "Inputs"; font.bold: true }
            ControlRow {
                label: "Number"
                NumberPicker {
                    value: 12.5; minimum: -100; maximum: 100; dragStep: 0.25; digits: 2; unitName: "px"
                    onEdited: function(value) { window.log("number changed " + value) }
                    onCommitted: function(value) { window.log("number committed " + value) }
                }
            }
            ControlRow {
                label: "Pair"
                Number2Picker {
                    first: 1920; second: 1080; minimum: 1; maximum: 16384; digits: 0
                    firstPrefix: "W"; secondPrefix: "H"; unitName: "px"; enableLock: true
                    onFirstEdited: function(value) { window.log("pair first " + value) }
                    onSecondEdited: function(value) { window.log("pair second " + value) }
                }
            }
            ControlRow {
                label: "Vector"
                Number3Picker {
                    first: 1; second: 2; third: 3; prefixes: ["X", "Y", "Z"]; enableLock: true
                    onEdited: function(axis, value) { window.log("vector " + axis + " " + value) }
                }
            }
            ControlRow {
                label: "Single line"
                SingleLineTextInput {
                    value: "Editable text"; placeholderText: "Type here"; maximumLength: 40
                    onCommitted: function(value) { window.log("text committed " + value) }
                }
            }
            ControlRow {
                label: "Multiline"
                MultilineTextInput {
                    value: "Try a typo such as teh."; maximumLength: 240; minimumContentHeight: 110
                    onCommitted: function(value) { window.log("multiline committed") }
                }
            }
            ControlRow {
                label: "Searchable dropdown"
                Dropdown {
                    values: ["one", "two", "three", "four", "five", "six", "seven", "eight"]
                    labels: ["One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight"]
                    value: "two"
                    onSelected: function(value) { window.log("selected " + value) }
                }
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0
                InspectorCard {
                    Layout.fillWidth: true
                    title: "Transform"
                    expanded: true
                    onResetRequested: {
                        positionRow.resetPair(960, 540)
                        anchorRow.resetPair(960, 540)
                        scaleRow.resetPair(1, 1)
                        shearRow.resetPair(0, 0)
                        rotationRow.resetValue(0)
                        window.log("transform reset")
                    }

                    DemoPairGraphProperty {
                        id: positionRow
                        label: "Position"
                        initialGraphValue: 960
                        initialSecondValue: 540
                        firstPrefix: "X"
                        secondPrefix: "Y"
                        unitName: "px"
                        digits: 0
                        keyframes: true
                        expression: true
                        expressionValue: DemoLogic.expressionSource
                        expressionOutput: DemoLogic.expressionOutput(expressionValue)
                        onGraphPlaybackToggled: window.log("toggle playback")
                        onExpressionEdited: function(value) {
                            expressionOutput = DemoLogic.expressionOutput(value)
                            window.log("expression edited (" + value.length + " chars)")
                        }
                    }
                    DemoPairGraphProperty {
                        id: anchorRow
                        label: "Anchor"
                        initialGraphValue: 960
                        initialSecondValue: 540
                        firstPrefix: "X"
                        secondPrefix: "Y"
                        unitName: "px"
                        digits: 0
                        expressionValue: DemoLogic.expressionSource
                        expressionOutput: DemoLogic.expressionOutput(expressionValue)
                        onExpressionEdited: function(value) { expressionOutput = DemoLogic.expressionOutput(value) }
                    }
                    DemoPairGraphProperty {
                        id: scaleRow
                        label: "Scale"
                        initialGraphValue: 1
                        initialSecondValue: 1
                        firstPrefix: "X"
                        secondPrefix: "Y"
                        unitName: "x"
                        digits: 2
                        minimum: 0
                        enableLock: true
                        expressionValue: DemoLogic.expressionSource
                        expressionOutput: DemoLogic.expressionOutput(expressionValue)
                        onExpressionEdited: function(value) { expressionOutput = DemoLogic.expressionOutput(value) }
                    }
                    DemoPairGraphProperty {
                        id: shearRow
                        label: "Shear"
                        initialGraphValue: 0
                        initialSecondValue: 0
                        firstPrefix: "X"
                        secondPrefix: "Y"
                        digits: 2
                        expressionValue: DemoLogic.expressionSource
                        expressionOutput: DemoLogic.expressionOutput(expressionValue)
                        onExpressionEdited: function(value) { expressionOutput = DemoLogic.expressionOutput(value) }
                    }
                    DemoGraphProperty {
                        id: rotationRow
                        label: "Rotation"
                        initialGraphValue: 0
                        expressionValue: DemoLogic.expressionSource
                        expressionOutput: DemoLogic.expressionOutput(expressionValue)
                        onGraphValueChanged: rotationEditor.value = graphValue
                        onBaseValueEdited: graphValue = value
                        onExpressionEdited: function(value) { expressionOutput = DemoLogic.expressionOutput(value) }
                        NumberPicker {
                            id: rotationEditor
                            value: rotationRow.graphValue
                            digits: 1
                            dragStep: 0.1
                            unitName: "°"
                            prefixIconSource: "qrc:/qt/qml/dev/shrimply/components/icons/arrow3-up-symbolic.svg"
                            prefixIconRotates: true
                            onEdited: function(value) { rotationRow.editValue(value) }
                        }
                    }
                }
                ModifierMenuButton {
                    text: ComponentTranslations.text("Add modifier")
                    searchPlaceholderText: ComponentTranslations.text("Search modifiers")
                    Layout.alignment: Qt.AlignHCenter
                    values: DemoLogic.modifierValues
                    labels: DemoLogic.modifierLabels
                    onSelected: function(value) { window.log("add modifier " + value) }
                }
            }
            LivePerformance {
                Layout.fillWidth: true
            }
            SwitchRow { label: "Enabled"; tooltip: "Toggle this option"; active: true; onToggled: function(value) { window.log("switch " + value) } }
            ControlRow {
                label: "Color"
                ColorPicker { color: "#3584e4cc"; withAlpha: true; onSelected: function(value) { window.log("color " + value) } }
            }
            ControlRow {
                label: "Split"
                SplitButton { text: "Primary"; secondaryText: "Secondary"; onTriggered: window.log("primary"); onSecondaryTriggered: window.log("secondary") }
            }
            ControlRow {
                label: "Progress"
                RowLayout {
                    ProgressButton { text: "Idle" }
                    ProgressButton { text: "Working"; progressState: ProgressButton.Indeterminate }
                    ProgressButton { text: "Half"; progressState: ProgressButton.Progress; progress: 0.5 }
                }
            }
            ControlRow {
                label: "Playback keys"
                PlaybackShortcuts {
                    implicitHeight: 44
                    onTogglePlaying: window.log("toggle playback")
                    onStepPlaybackSpeedForward: window.log("step playback speed")
                    Rectangle {
                        anchors.fill: parent
                        color: "transparent"
                        border.color: palette.mid
                        Label { anchors.centerIn: parent; text: "Click, then press Space or L" }
                    }
                }
            }

        }
    }

        ScrollView {
            contentWidth: availableWidth
            ColumnLayout {
                width: parent.width
                spacing: 10
                ControlRow {
                    label: "Selected item"
                    ReadOnlyField {
                        text: "Example clip · 00:00:02:00"
                        horizontalAlignment: Text.AlignRight
                    }
                }
                ControlRow {
                    label: "Component package"
                    ReadOnlyField {
                        text: "shrimply-components-qt"
                        horizontalAlignment: Text.AlignRight
                    }
                }
                ControlRow {
                    label: "Frame graph"
                    ReadOnlyField {
                        text: "Shared Rust renderer"
                        horizontalAlignment: Text.AlignRight
                    }
                }
                ControlRow {
                    label: "Home folder"
                    ReadOnlyField {
                        text: window.homePath
                        horizontalAlignment: Text.AlignRight
                        actionIconSource: "qrc:/qt/qml/dev/shrimply/components/demo/icons/folder-open.svg"
                        actionText: "Show in Folder"
                        onActionTriggered: Qt.openUrlExternally(window.homeUrl)
                    }
                }
            }
        }

        ScrollView {
            TextArea {
                width: parent.width
                height: parent.height
                readOnly: true
                text: window.events
                font.family: "monospace"
            }
        }
    }
}
