import QtQuick
import dev.shrimply.components
import QtQuick.Controls
import QtQuick.Layouts

ColumnLayout {
    id: root
    property int initialWidth: backend.minimumDimension()
    property int initialHeight: backend.minimumDimension()
    property string initialFpsNumerator: "30"
    property string initialFpsDenominator: "1"
    property bool initialized: false
    property string pendingResetSourceKey: ""
    readonly property string sourceKey: initialWidth + ":" + initialHeight + ":"
        + initialFpsNumerator + ":" + initialFpsDenominator
    signal applyRequested(int width, int height, string fpsNumerator, string fpsDenominator)
    spacing: 8

    function resetDraft() {
        if (initialWidth < backend.minimumDimension()
                || initialHeight < backend.minimumDimension()
                || initialFpsNumerator.length === 0
                || initialFpsDenominator.length === 0)
            return
        backend.configure(initialWidth, initialHeight, initialFpsNumerator, initialFpsDenominator)
    }

    onSourceKeyChanged: {
        if (!initialized)
            return
        pendingResetSourceKey = sourceKey
        resetDraftTimer.restart()
    }
    Component.onCompleted: {
        initialized = true
        resetDraft()
    }

    Timer {
        id: resetDraftTimer
        interval: 0
        repeat: false
        onTriggered: if (root.sourceKey === root.pendingResetSourceKey)
            root.resetDraft()
    }

    ControlRow {
        label: backend.settingsText("fps")
        Dropdown {
            value: String(backend.frameRate)
            values: {
                const count = backend.frameRateCount
                return count >= 0 ? backend.frameRateValues() : []
            }
            labels: {
                const count = backend.frameRateCount
                return count >= 0 ? backend.frameRateLabels() : []
            }
            onSelected: function(value) {
                backend.setFrameRateValue(Number(value))
            }
        }
    }

    ControlRow {
        label: backend.settingsText("resolution")
        Number2Picker {
            first: backend.width
            second: backend.height
            minimum: backend.minimumDimension()
            maximum: backend.maximumDimension()
            dragStep: 1
            digits: 0
            widthCharacters: 7
            firstPrefix: "W"
            secondPrefix: "H"
            enableLock: true
            onValuesEdited: function(width, height, component) {
                backend.setWidthValue(Math.round(width))
                backend.setHeightValue(Math.round(height))
            }
        }
    }

    RowLayout {
        Layout.alignment: Qt.AlignRight
        visible: backend.changed
        Button {
            text: backend.settingsText("discard")
            onClicked: backend.discard()
        }
        Button {
            text: backend.settingsText("apply")
            highlighted: true
            onClicked: confirmation.open()
        }
    }

    ProjectSettingsBackend { id: backend }

    Dialog {
        id: confirmation
        modal: true
        readonly property Item windowsDialogAnchor: Overlay.overlay
            || (root.Window.window ? root.Window.window.contentItem : root)
        anchors.centerIn: Qt.platform.os === "windows"
            ? windowsDialogAnchor : Overlay.overlay
        title: backend.settingsText("title")
        standardButtons: Dialog.NoButton

        contentItem: Label {
            text: backend.settingsText("warning")
            wrapMode: Text.Wrap
            width: Qt.platform.os === "windows"
                ? Math.max(0, Math.min(460, confirmation.windowsDialogAnchor.width - 48))
                : Math.min(460, Overlay.overlay.width - 48)
        }

        Binding on implicitHeight {
            when: Qt.platform.os === "windows"
            value: Math.max(
                confirmation.implicitBackgroundHeight
                    + confirmation.topInset + confirmation.bottomInset,
                confirmation.topPadding
                    + confirmation.bottomPadding
                    + confirmation.contentItem.implicitHeight
                    + (confirmation.implicitHeaderHeight > 0
                        ? confirmation.implicitHeaderHeight + confirmation.spacing : 0)
                    + (confirmation.implicitFooterHeight > 0
                        ? confirmation.implicitFooterHeight + confirmation.spacing : 0))
        }

        footer: DialogButtonBox {
            Button {
                id: cancel
                text: backend.settingsText("cancel")
                DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
            }
            Button {
                text: backend.settingsText("apply")
                highlighted: true
                DialogButtonBox.buttonRole: DialogButtonBox.DestructiveRole
            }
            onRejected: confirmation.close()
            onDiscarded: {
                confirmation.close()
                root.applyRequested(
                    backend.width,
                    backend.height,
                    backend.fpsNumerator(),
                    backend.fpsDenominator()
                )
            }
        }
        onOpened: cancel.forceActiveFocus()
    }
}
