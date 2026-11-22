import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

ColumnLayout {
    id: root
    property alias value: editor.value
    property alias output: result.text
    property string error: ""
    property var diagnosticProvider: null
    property var completionProvider: null
    property int diagnosticDebounce: 250
    property int completionDebounce: diagnosticDebounce
    signal edited(string value)
    signal committed(string value)
    spacing: 6

    RowLayout {
        Layout.fillWidth: true
        Layout.preferredHeight: 86
        spacing: 6

        CodeEditor {
            id: editor
            Layout.fillWidth: true
            Layout.fillHeight: true
            diagnosticProvider: root.diagnosticProvider
            completionProvider: root.completionProvider
            diagnosticDebounce: root.diagnosticDebounce
            completionDebounce: root.completionDebounce
            onEdited: function(value) { root.edited(value) }
            onCommitted: function(value) { root.committed(value) }
        }
        Button {
            Layout.alignment: Qt.AlignTop
            display: AbstractButton.IconOnly
            icon.source: "qrc:/shrimply/icons/view-fullscreen-symbolic.svg"
            text: qsTr("Open larger editor")
            onClicked: largeEditor.open()
        }
    }
    Label {
        Layout.fillWidth: true
        visible: editor.diagnostic.length > 0
        text: editor.diagnostic
        color: palette.brightText
        wrapMode: Text.Wrap
    }
    ReadOnlyField {
        id: result
        Layout.fillWidth: true
        ToolTip.visible: hovered && root.error.length > 0
        ToolTip.text: root.error
    }

    Dialog {
        id: largeEditor
        title: qsTr("Expression")
        modal: true
        anchors.centerIn: Overlay.overlay
        width: Math.min(720, Overlay.overlay.width - 24)
        height: Math.min(460, Overlay.overlay.height - 24)
        standardButtons: Dialog.Close
        onClosed: expandedEditor.commit()

        contentItem: ColumnLayout {
            spacing: 6
            CodeEditor {
                id: expandedEditor
                Layout.fillWidth: true
                Layout.fillHeight: true
                value: root.value
                showLineNumbers: true
                diagnosticProvider: root.diagnosticProvider
                completionProvider: root.completionProvider
                diagnosticDebounce: root.diagnosticDebounce
                completionDebounce: root.completionDebounce
                onEdited: function(value) {
                    editor.synchronize(value)
                    root.edited(value)
                }
                onCommitted: function(value) { root.committed(value) }
            }
            Label {
                Layout.fillWidth: true
                visible: expandedEditor.diagnostic.length > 0
                text: expandedEditor.diagnostic
                color: palette.brightText
                wrapMode: Text.Wrap
            }
        }
        onOpened: {
            expandedEditor.synchronize(editor.currentText())
            expandedEditor.forceEditorFocus()
        }
    }
}
