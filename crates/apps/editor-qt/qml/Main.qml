import QtCore
import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import QtQml.Models
import dev.shrimply.editor
import dev.shrimply.export
import dev.shrimply.inspector

ApplicationWindow {
    id: window

    width: 1800
    height: 1100
    minimumWidth: 960
    minimumHeight: 640
    visible: true
    title: backend.projectTitle
    palette.inactive.windowText: palette.active.windowText
    palette.inactive.buttonText: palette.active.buttonText
    palette.inactive.text: palette.active.text
    property bool inspectorVisible: true
    property bool timelineVisible: true
    property bool fullscreenPreview: false
    property int visibilityBeforeFullscreen: Window.Windowed
    property string destinationTitle
    property string destinationName

    function setPreviewFullscreen(fullscreen) {
        if (fullscreen === fullscreenPreview)
            return
        if (fullscreen) {
            visibilityBeforeFullscreen = visibility === Window.FullScreen ? Window.Windowed : visibility
            fullscreenPreview = true
            visibility = Window.FullScreen
        } else {
            fullscreenPreview = false
            visibility = visibilityBeforeFullscreen
        }
    }

    onVisibilityChanged: {
        if (fullscreenPreview && visibility !== Window.FullScreen)
            fullscreenPreview = false
    }

    EditorBackend {
        id: backend
    }

    Component.onCompleted: Qt.callLater(backend.begin)

    Timer {
        interval: 16
        repeat: true
        running: true
        onTriggered: backend.poll()
    }

    Connections {
        target: backend

        function onRequestKdenlive() { kdenliveDialog.open() }
        function onRequestOtio() { otioDialog.open() }
        function onRequestRepair() { repairDialog.open() }
        function onRequestDestination(title, suggestedName) {
            window.destinationTitle = title
            window.destinationName = suggestedName
            Qt.callLater(function() {
                const path = StandardPaths.writableLocation(StandardPaths.DocumentsLocation) + "/" + window.destinationName
                const selected = backend.showFileSaveDialog(
                    path,
                    window.destinationTitle,
                    backend.translate("Shrimply projects (*.shrimp)"),
                    "shrimp")
                backend.chooseDestination(selected.toString().length > 0, selected)
            })
        }
        function onRequestWarnings(body) {
            warningDialog.text = body
            warningDialog.open()
        }
        function onRequestLock(pid) {
            lockDialog.pid = pid
            lockDialog.open()
        }
        function onShowError(heading, body) {
            errorDialog.title = heading
            errorDialog.text = body
            errorDialog.open()
        }
        function onShowPlaybackError(body) {
            audioErrorDialog.text = body
            audioErrorDialog.open()
        }
        function onCanceled() { Qt.quit() }
    }

    menuBar: MenuBar {
        visible: backend.ready && !window.fullscreenPreview

        Menu {
            title: backend.translate("File")
            popupType: Popup.Native

            Action { text: backend.translate("Save"); shortcut: StandardKey.Save; onTriggered: backend.save() }
            Action {
                text: backend.translate("Save As…")
                shortcut: StandardKey.SaveAs
                onTriggered: Qt.callLater(backend.showSaveAsDialog)
            }
            MenuSeparator {}
            Menu {
                title: backend.translate("Export")
                enabled: !exportWindow.busy

                Action {
                    text: backend.translate("Export video")
                    shortcut: "Ctrl+E"
                    onTriggered: exportWindow.openVideo()
                }
                Action {
                    text: backend.translate("Export captions (YTT)")
                    onTriggered: exportWindow.openCaptions()
                }
                Action {
                    text: backend.translate("Export JSON")
                    onTriggered: exportWindow.exportJson()
                }
            }
            MenuSeparator {}
            Action { text: backend.translate("Quit"); shortcut: StandardKey.Quit; onTriggered: Qt.quit() }
        }

        Menu {
            title: backend.translate("Edit")
            popupType: Popup.Native

            Action { text: backend.translate("Undo"); shortcut: StandardKey.Undo; onTriggered: backend.undo() }
            Action { text: backend.translate("Redo"); shortcut: StandardKey.Redo; onTriggered: backend.redo() }
            MenuSeparator {}
            Action {
                text: backend.translate("Preferences…")
                shortcut: StandardKey.Preferences
                onTriggered: preferencesWindow.openPreferences()
            }
        }

        Menu {
            id: viewMenu
            title: backend.translate("View")
            popupType: Popup.Native
            onAboutToShow: {
                inspectorMenuItem.checked = window.inspectorVisible
                timelineMenuItem.checked = window.timelineVisible
                console.info("View menu synchronized:",
                    "inspectorVisible=" + window.inspectorVisible,
                    "inspectorChecked=" + inspectorMenuItem.checked,
                    "timelineVisible=" + window.timelineVisible,
                    "timelineChecked=" + timelineMenuItem.checked)
            }

            MenuItem {
                id: inspectorMenuItem
                text: backend.translate("Inspector")
                checkable: true
                checked: true
                onClicked: {
                    window.inspectorVisible = !window.inspectorVisible
                    checked = window.inspectorVisible
                    Qt.callLater(function() {
                        console.info("Inspector view toggle settled:",
                            "visible=" + window.inspectorVisible,
                            "checked=" + inspectorMenuItem.checked,
                            "paneVisible=" + inspectorPane.visible)
                    })
                }
            }
            MenuItem {
                id: timelineMenuItem
                text: backend.translate("Timeline")
                checkable: true
                checked: true
                onClicked: {
                    window.timelineVisible = !window.timelineVisible
                    checked = window.timelineVisible
                    Qt.callLater(function() {
                        console.info("Timeline view toggle settled:",
                            "visible=" + window.timelineVisible,
                            "checked=" + timelineMenuItem.checked,
                            "paneVisible=" + timelinePane.visible)
                    })
                }
            }
            Action {
                text: backend.translate("Fullscreen Preview")
                shortcut: "F11"
                checkable: true
                Binding on checked { value: window.fullscreenPreview }
                onTriggered: window.setPreviewFullscreen(!window.fullscreenPreview)
            }
        }

        Menu {
            title: backend.translate("Help")
            popupType: Popup.Native

            Action { text: backend.translate("Keyboard Shortcuts") }
            Action {
                text: backend.translate("About Shrimply")
                onTriggered: aboutWindow.openAbout()
            }
        }
    }

    AboutWindow {
        id: aboutWindow
        backend: backend
        owner: window
    }

    ExportWindow {
        id: exportWindow
        owner: window
    }

    PreferencesWindow {
        id: preferencesWindow
        backend: backend
        owner: window
    }

    Pane {
        anchors.fill: parent
        visible: !backend.ready

        ColumnLayout {
            anchors.centerIn: parent
            spacing: 14

            BusyIndicator {
                Layout.alignment: Qt.AlignHCenter
                running: true
            }
            Label {
                Layout.alignment: Qt.AlignHCenter
                text: backend.translate("Loading project…")
                font.pointSize: 18
                font.bold: true
            }
            Label {
                Layout.alignment: Qt.AlignHCenter
                text: backend.loadingText
                opacity: 0.7
            }
        }
    }

    ColumnLayout {
        anchors.fill: parent
        visible: backend.ready
        spacing: 0

        SplitView {
            id: verticalSplit
            Layout.fillWidth: true
            Layout.fillHeight: true
            orientation: Qt.Vertical

            SplitView {
                SplitView.fillWidth: true
                SplitView.fillHeight: true
                SplitView.preferredHeight: 660
                orientation: Qt.Horizontal

                Pane {
                    id: inspectorPane
                    padding: 0
                    visible: window.inspectorVisible && !window.fullscreenPreview
                    SplitView.preferredWidth: inspectorView.implicitWidth
                    SplitView.minimumWidth: inspectorView.implicitWidth

                    InspectorView {
                        id: inspectorView
                        anchors.fill: parent
                        onError: function(body) {
                            errorDialog.title = backend.translate("Inspector edit failed")
                            errorDialog.text = body
                            errorDialog.open()
                        }
                        onConfirmation: function(body) {
                            inspectorConfirmation.text = body
                            inspectorConfirmation.open()
                        }
                    }
                }

                ColumnLayout {
                    SplitView.fillWidth: true
                    SplitView.fillHeight: true
                    spacing: 0

                    RowLayout {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        spacing: 0

                        ToolBar {
                            id: previewToolbar
                            visible: !window.fullscreenPreview

                            Layout.fillHeight: true
                            Layout.preferredWidth: 44

                            ColumnLayout {
                                id: previewToolColumn

                                readonly property int statusTextPixelSize: Math.round(Qt.application.font.pixelSize * 0.75)

                                anchors.top: parent.top
                                anchors.left: parent.left
                                anchors.right: parent.right
                                ToolButton {
                                    id: previewStatusButton
                                    Layout.alignment: Qt.AlignHCenter
                                    icon.name: "task-complete"
                                    text: backend.translate("Ready")
                                    display: AbstractButton.IconOnly
                                    enabled: false
                                }
                                Label {
                                    Layout.fillWidth: true
                                    Layout.preferredHeight: previewStatusButton.implicitHeight
                                    horizontalAlignment: Text.AlignHCenter
                                    verticalAlignment: Text.AlignVCenter
                                    text: backend.frameRateLabel
                                    font.family: backend.fixedFontFamily
                                    font.pixelSize: previewToolColumn.statusTextPixelSize
                                    ToolTip.visible: fpsHover.hovered
                                    ToolTip.text: backend.translate("Frame rate")
                                    HoverHandler { id: fpsHover }
                                }
                                Label {
                                    Layout.fillWidth: true
                                    Layout.preferredHeight: previewStatusButton.implicitHeight
                                    horizontalAlignment: Text.AlignHCenter
                                    verticalAlignment: Text.AlignVCenter
                                    text: backend.playbackSpeedLabel
                                    font.family: backend.fixedFontFamily
                                    font.pixelSize: previewToolColumn.statusTextPixelSize
                                    ToolTip.visible: speedHover.hovered
                                    ToolTip.text: backend.translate("Playback speed")
                                    HoverHandler { id: speedHover }
                                }
                                ToolButton {
                                    icon.name: "show-guides"
                                    text: backend.translate("Guides")
                                    display: AbstractButton.IconOnly
                                    checkable: true
                                    checked: previewLoader.item ? previewLoader.item.guidesVisible : false
                                    onClicked: if (previewLoader.item) previewLoader.item.guidesVisible = checked
                                }
                                ToolSeparator {}
                                ToolButton { icon.name: "draw-freehand"; text: backend.translate("Pen"); display: AbstractButton.IconOnly }
                                ToolButton { icon.name: "fill-color"; text: backend.translate("Fill"); display: AbstractButton.IconOnly }
                                ToolButton { icon.name: "transform-move"; text: backend.translate("Transform"); display: AbstractButton.IconOnly }
                                ToolButton { icon.name: "draw-eraser"; text: backend.translate("Eraser"); display: AbstractButton.IconOnly }
                            }
                        }

                        Loader {
                            id: previewLoader
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            active: backend.ready
                            sourceComponent: Component {
                                PreviewSurface {
                                    anchors.fill: parent
                                    fullscreenPreview: window.fullscreenPreview
                                }
                            }
                        }
                    }

                    ToolBar {
                        Layout.fillWidth: true

                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: 8
                            anchors.rightMargin: 8

                            ToolButton { icon.name: "media-seek-backward"; onClicked: backend.stepFrame(-1) }
                            ToolButton {
                                icon.name: backend.playing ? "media-playback-pause" : "media-playback-start"
                                onClicked: backend.togglePlaying()
                            }
                            ToolButton { icon.name: "media-seek-forward"; onClicked: backend.stepFrame(1) }
                            Slider {
                                Layout.fillWidth: true
                                from: 0
                                to: Math.max(1, backend.durationFrame)
                                value: backend.positionFrame
                                onMoved: backend.seekFrame(Math.round(value))
                            }
                            Label {
                                text: backend.timeLabel
                                font.family: backend.fixedFontFamily
                            }
                            ToolButton {
                                icon.name: window.fullscreenPreview ? "view-restore" : "view-fullscreen"
                                text: window.fullscreenPreview ? backend.translate("Exit Fullscreen Preview") : backend.translate("Fullscreen Preview")
                                display: AbstractButton.IconOnly
                                onClicked: window.setPreviewFullscreen(!window.fullscreenPreview)
                            }
                        }
                    }
                }
            }

            RowLayout {
                id: timelinePane
                visible: window.timelineVisible && !window.fullscreenPreview
                SplitView.fillWidth: true
                SplitView.preferredHeight: 410
                SplitView.minimumHeight: 180
                spacing: 0

                ToolBar {
                    Layout.fillHeight: true
                    Layout.preferredWidth: 44

                    ColumnLayout {
                        id: timelineToolColumn

                        anchors.top: parent.top
                        anchors.horizontalCenter: parent.horizontalCenter
                        ToolButton {
                            icon.name: "snap"
                            text: backend.translate("Magnet")
                            display: AbstractButton.IconOnly
                            checkable: true
                            Binding on checked { value: timelineLoader.item ? timelineLoader.item.magnetEnabled : false }
                            onClicked: if (timelineLoader.item) timelineLoader.item.magnetEnabled = checked
                        }
                        ToolButton {
                            icon.name: "view-grid"
                            text: backend.translate("Beat Grid")
                            display: AbstractButton.IconOnly
                            checkable: true
                            Binding on checked { value: timelineLoader.item ? timelineLoader.item.beatGridEnabled : false }
                            onClicked: if (timelineLoader.item) timelineLoader.item.beatGridEnabled = checked
                        }
                        ToolSeparator {}
                        ToolButton {
                            icon.name: "edit-select"
                            text: backend.translate("Pointer")
                            display: AbstractButton.IconOnly
                            checkable: true
                            Binding on checked {
                                value: timelineLoader.item ? !timelineLoader.item.cutEnabled : false
                            }
                            onClicked: if (checked && timelineLoader.item)
                                timelineLoader.item.cutEnabled = false
                        }
                        ToolButton {
                            icon.name: "edit-cut"
                            text: backend.translate("Cut")
                            display: AbstractButton.IconOnly
                            checkable: true
                            Binding on checked {
                                value: timelineLoader.item ? timelineLoader.item.cutEnabled : false
                            }
                            onClicked: if (checked && timelineLoader.item)
                                timelineLoader.item.cutEnabled = true
                        }
                        ToolSeparator {}
                        ToolButton {
                            icon.name: "timeline-mode-overwrite"
                            text: backend.translate("Overwrite/Insert")
                            display: AbstractButton.IconOnly
                            checkable: true
                            Binding on checked {
                                value: timelineLoader.item ? timelineLoader.item.overwriteMode : false
                            }
                            onClicked: if (checked && timelineLoader.item)
                                timelineLoader.item.selectOverwriteMode()
                        }
                        ToolButton {
                            icon.name: "dialog-cancel"
                            text: backend.translate("Block")
                            display: AbstractButton.IconOnly
                            checkable: true
                            Binding on checked {
                                value: timelineLoader.item ? timelineLoader.item.blockMode : false
                            }
                            onClicked: if (checked && timelineLoader.item)
                                timelineLoader.item.selectBlockMode()
                        }
                        ToolButton {
                            icon.name: "selection-move-to-layer-above"
                            text: backend.translate("New Track")
                            display: AbstractButton.IconOnly
                            checkable: true
                            Binding on checked {
                                value: timelineLoader.item ? timelineLoader.item.newTrackMode : false
                            }
                            onClicked: if (checked && timelineLoader.item)
                                timelineLoader.item.selectNewTrackMode()
                        }
                    }
                }

                Loader {
                    id: timelineLoader
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    active: backend.ready
                    sourceComponent: Component {
                        TimelineSurface { anchors.fill: parent; focus: true }
                    }
                }

                Loader {
                    Layout.preferredWidth: 54
                    Layout.fillHeight: true
                    active: backend.ready
                    sourceComponent: Component {
                        AudioMeterSurface { anchors.fill: parent }
                    }
                }
            }
        }
    }

    Menu {
        id: timelineTrackAddMenu
        popupType: Popup.Window

        Instantiator {
            model: timelineLoader.item ? timelineLoader.item.trackAddMenuItems : []
            delegate: DelegateChooser {
                role: "kind"
                DelegateChoice {
                    roleValue: 1
                    delegate: MenuItem {
                        required property var modelData
                        text: backend.translate(modelData.label)
                        icon.source: "qrc:/qt/qml/dev/shrimply/editor/track-add-icons/" + modelData.icon + ".svg"
                        icon.color: highlighted ? palette.highlightedText : palette.buttonText
                        onTriggered: timelineLoader.item.activateTrackAddMenuItem(modelData.index)
                    }
                }
                DelegateChoice {
                    roleValue: 2
                    delegate: MenuSeparator {}
                }
            }
            onObjectAdded: (index, object) => timelineTrackAddMenu.insertItem(index, object)
            onObjectRemoved: (index, object) => timelineTrackAddMenu.takeItem(index)
        }
    }

    Item {
        id: timelineTrackAddAnchor
        parent: timelineLoader.item
        width: 26
        height: 26
        visible: false
    }

    Menu {
        id: timelineContextMenu
        popupType: Popup.Window

        Instantiator {
            model: timelineLoader.item ? timelineLoader.item.contextMenuItems : []
            delegate: DelegateChooser {
                role: "kind"
                DelegateChoice {
                    roleValue: 1
                    delegate: MenuItem {
                        required property var modelData
                        text: modelData.label
                        enabled: modelData.enabled
                        icon.name: modelData.label === "Copy" ? "edit-copy"
                            : modelData.label === "Cut" ? "edit-cut"
                            : modelData.label === "Paste" ? "edit-paste" : ""
                        onTriggered: timelineLoader.item.activateContextMenuItem(modelData.index)
                    }
                }
                DelegateChoice {
                    roleValue: 2
                    delegate: MenuSeparator {}
                }
                DelegateChoice {
                    roleValue: 3
                    delegate: timelineContextControl
                }
                DelegateChoice {
                    roleValue: 4
                    delegate: timelineContextControl
                }
            }
            onObjectAdded: (index, object) => timelineContextMenu.insertItem(index, object)
            onObjectRemoved: (index, object) => timelineContextMenu.takeItem(index)
        }
    }

    Item {
        id: timelineContextAnchor
        parent: timelineLoader.item
        width: 1
        height: 1
        visible: false
    }

    Component {
        id: timelineContextControl

        MenuItem {
            id: controlItem
            required property var modelData
            property bool mixed: modelData.mixed
            property real currentValue: mixed ? 0 : modelData.value
            implicitWidth: 300
            implicitHeight: controlLayout.implicitHeight + 16

            contentItem: ColumnLayout {
                id: controlLayout
                spacing: 4

                Label {
                    Layout.fillWidth: true
                    text: modelData.label + (controlItem.mixed
                        ? backend.translate(" — Mixed")
                        : modelData.kind === 3
                            ? " — " + Number(Math.pow(2, controlItem.currentValue).toFixed(2)) + "×"
                            : " — " + Number(controlItem.currentValue.toFixed(1)) + " dB")
                }

                Slider {
                    Layout.fillWidth: true
                    from: modelData.minimum
                    to: modelData.maximum
                    stepSize: modelData.step
                    value: controlItem.currentValue
                    onMoved: {
                        controlItem.mixed = false
                        controlItem.currentValue = value
                        timelineLoader.item.setContextMenuControl(modelData.index, value)
                    }
                }
            }
        }
    }

    Connections {
        target: timelineLoader.item
        function onContextMenuRequested(x, y) {
            timelineContextAnchor.x = x
            timelineContextAnchor.y = y
            timelineContextMenu.popup(timelineContextAnchor, 0, 0)
        }
        function onTrackAddMenuRequested(x, y) {
            timelineTrackAddAnchor.x = x
            timelineTrackAddAnchor.y = y
            timelineTrackAddMenu.popup(timelineTrackAddAnchor, 0, 0)
        }
        function onTrackImportRequested() {
            const selected = backend.showOpenFileDialog(
                "",
                backend.translate("Import to Track"),
                backend.translate("All files (*)"))
            if (selected.toString().length > 0)
                timelineLoader.item.importTrackFile(selected)
        }
        function onSaveFrameRequested() {
            const selected = backend.showFileSaveDialog(
                "",
                backend.translate("Save Selected Frame"),
                backend.translate("PNG image (*.png)"),
                "png")
            if (selected.toString().length > 0)
                timelineLoader.item.saveContextFrame(selected)
        }
        function onContextActionFailed(message) {
            timelineContextError.text = message
            timelineContextError.open()
        }
        function onDeleteTrackRequested(clipCount) {
            timelineDeleteTrackDialog.text = backend.translate("%1 clips are about to be deleted. Are you sure?").arg(clipCount)
            timelineDeleteTrackDialog.open()
        }
    }

    MessageDialog {
        id: timelineContextError
        title: backend.translate("Timeline Action Failed")
        buttons: MessageDialog.Ok
    }

    MessageDialog {
        id: timelineDeleteTrackDialog
        title: backend.translate("Delete Track?")
        buttons: MessageDialog.Yes | MessageDialog.Cancel
        onAccepted: timelineLoader.item.deleteContextFoldedTrack()
    }

    MessageDialog {
        id: kdenliveDialog
        title: backend.translate("Convert Kdenlive Project?")
        text: backend.translate("Shrimply supports only some Kdenlive features. Unsupported content may be changed or omitted.")
        buttons: MessageDialog.Ok | MessageDialog.Cancel
        onAccepted: backend.confirmKdenlive(true)
        onRejected: backend.confirmKdenlive(false)
    }

    Dialog {
        id: otioDialog
        title: backend.translate("OTIO Project Settings")
        modal: true
        anchors.centerIn: parent
        standardButtons: Dialog.Ok | Dialog.Cancel
        onAccepted: backend.chooseOtio(true, otioWidth.value, otioHeight.value, fpsNumerator.value, fpsDenominator.value)
        onRejected: backend.chooseOtio(false, 0, 0, 0, 0)

        GridLayout {
            columns: 2
            Label { text: backend.translate("Width") }
            SpinBox { id: otioWidth; from: 1; to: 16384; value: 1920 }
            Label { text: backend.translate("Height") }
            SpinBox { id: otioHeight; from: 1; to: 16384; value: 1080 }
            Label { text: backend.translate("FPS numerator") }
            SpinBox { id: fpsNumerator; from: 1; to: 240000; value: 30 }
            Label { text: backend.translate("FPS denominator") }
            SpinBox { id: fpsDenominator; from: 1; to: 1001; value: 1 }
        }
    }

    MessageDialog {
        id: repairDialog
        title: backend.translate("Project Timing Needs Repair")
        text: backend.translate("Some clips are not aligned to the project frame grid. Fixing them will save a new project without changing the original.")
        buttons: MessageDialog.Ok | MessageDialog.Cancel
        onAccepted: backend.confirmRepair(true)
        onRejected: backend.confirmRepair(false)
    }

    MessageDialog {
        id: warningDialog
        title: backend.translate("OTIO imported with limitations")
        buttons: MessageDialog.Ok
        onAccepted: backend.acknowledgeWarnings()
    }

    Dialog {
        id: lockDialog
        property int pid: 0
        title: backend.translate("Project is in use")
        modal: true
        anchors.centerIn: parent

        contentItem: Label {
            text: backend.translate("The project lock is held by another editor process (PID %1).").arg(lockDialog.pid)
            wrapMode: Text.Wrap
        }
        footer: DialogButtonBox {
            Button {
                text: backend.translate("Close")
                DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
            }
            Button {
                text: backend.translate("Stop Other Editor")
                DialogButtonBox.buttonRole: DialogButtonBox.DestructiveRole
            }
            Button {
                text: backend.translate("Retry")
                highlighted: true
                DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole
            }
            onRejected: { lockDialog.close(); backend.resolveLock(0) }
            onDiscarded: { lockDialog.close(); backend.resolveLock(2) }
            onAccepted: { lockDialog.close(); backend.resolveLock(1) }
        }
    }

    MessageDialog {
        id: errorDialog
        buttons: MessageDialog.Close
        onAccepted: Qt.quit()
    }

    Popup {
        id: inspectorConfirmation
        property alias text: confirmationLabel.text
        x: Math.round((window.width - width) / 2)
        y: window.height - height - 24
        padding: 12
        modal: false
        closePolicy: Popup.NoAutoClose
        onOpened: confirmationTimer.restart()

        contentItem: Label {
            id: confirmationLabel
        }

        Timer {
            id: confirmationTimer
            interval: 2500
            onTriggered: inspectorConfirmation.close()
        }
    }

    MessageDialog {
        id: audioErrorDialog
        title: backend.translate("Audio playback stopped")
        buttons: MessageDialog.Close
    }

    Shortcut { sequence: "Space"; enabled: backend.ready; onActivated: backend.togglePlaying() }
    Shortcut { sequence: "Left"; enabled: backend.ready; onActivated: backend.stepFrame(-1) }
    Shortcut { sequence: "Right"; enabled: backend.ready; onActivated: backend.stepFrame(1) }
    Shortcut {
        sequence: "Escape"
        enabled: window.fullscreenPreview
        onActivated: window.setPreviewFullscreen(false)
    }
}
