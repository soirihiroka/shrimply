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
    property int stagedWidth: initialWidth
    property int stagedHeight: initialHeight
    property string stagedFpsNumerator: initialFpsNumerator
    property string stagedFpsDenominator: initialFpsDenominator
    property bool initialized: false
    property string pendingResetSourceKey: ""
    readonly property string sourceKey: initialWidth + ":" + initialHeight + ":"
        + initialFpsNumerator + ":" + initialFpsDenominator
    readonly property bool changed: stagedWidth !== initialWidth
        || stagedHeight !== initialHeight
        || stagedFpsNumerator !== initialFpsNumerator
        || stagedFpsDenominator !== initialFpsDenominator
    signal applyRequested(int width, int height, string fpsNumerator, string fpsDenominator)
    spacing: 8

    function resetDraft() {
        if (initialWidth < backend.minimumDimension()
                || initialHeight < backend.minimumDimension()
                || initialFpsNumerator.length === 0
                || initialFpsDenominator.length === 0)
            return
        stagedWidth = initialWidth
        stagedHeight = initialHeight
        stagedFpsNumerator = initialFpsNumerator
        stagedFpsDenominator = initialFpsDenominator
        backend.configure(stagedWidth, stagedHeight, stagedFpsNumerator, stagedFpsDenominator)
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
        label: ComponentTranslations.text("FPS")
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
                root.stagedFpsNumerator = backend.fpsNumerator()
                root.stagedFpsDenominator = backend.fpsDenominator()
            }
        }
    }

    ControlRow {
        label: ComponentTranslations.text("Resolution")
        Number2Picker {
            first: root.stagedWidth
            second: root.stagedHeight
            minimum: backend.minimumDimension()
            maximum: backend.maximumDimension()
            dragStep: 1
            digits: 0
            widthCharacters: 7
            firstPrefix: "W"
            secondPrefix: "H"
            enableLock: true
            onValuesEdited: function(width, height, component) {
                root.stagedWidth = Math.round(width)
                root.stagedHeight = Math.round(height)
            }
        }
    }

    RowLayout {
        Layout.alignment: Qt.AlignRight
        visible: root.changed
        Button {
            text: ComponentTranslations.text("Discard")
            onClicked: root.resetDraft()
        }
        Button {
            text: ComponentTranslations.text("Apply")
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
        title: ComponentTranslations.text("Change Project Settings?")
        standardButtons: Dialog.NoButton

        contentItem: Label {
            text: ComponentTranslations.text(
                "Changing the frame rate or resolution can affect timing, visual layout, and rendered output. Existing media and effects may no longer match the project."
            )
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
                text: ComponentTranslations.text("Cancel")
                DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
            }
            Button {
                text: ComponentTranslations.text("Apply")
                highlighted: true
                DialogButtonBox.buttonRole: DialogButtonBox.DestructiveRole
            }
            onRejected: confirmation.close()
            onDiscarded: {
                confirmation.close()
                root.applyRequested(
                    root.stagedWidth,
                    root.stagedHeight,
                    root.stagedFpsNumerator,
                    root.stagedFpsDenominator
                )
            }
        }
        onOpened: cancel.forceActiveFocus()
    }
}
