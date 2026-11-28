import QtCore
import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Effects
import QtQuick.Layouts
import QtQml.Models
import Qt.labs.qmlmodels
import dev.shrimply.components
import dev.shrimply.editor
import dev.shrimply.export
import dev.shrimply.inspector
import dev.shrimply.preferences

ApplicationWindow {
    id: window

    width: 1800
    height: 1100
    minimumWidth: backend.previewMinimumWidth()
    minimumHeight: 640
    visible: true
    title: backend.projectTitle
    palette.inactive.windowText: palette.active.windowText
    palette.inactive.buttonText: palette.active.buttonText
    palette.inactive.text: palette.active.text
    readonly property bool inspectorVisible: backend.inspectorVisible
    readonly property bool timelineVisible: backend.timelineVisible
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

    property bool startupPending: true
    onFrameSwapped: {
        if (startupPending) {
            startupPending = false
            Qt.callLater(backend.begin)
        }
    }

    Timer {
        interval: 16
        repeat: true
        running: true
        onTriggered: backend.poll()
    }

    Connections {
        target: backend

        function onRequestTrust(explanation, details, files, folders) {
            trustDialog.explanation = explanation
            trustDialog.details = details
            trustDialog.fileCount = files
            trustDialog.folderCount = folders
            trustDialog.open()
        }
        function onRequestKdenlive() { kdenliveDialog.open() }
        function onRequestOtio() { otioDialog.open() }
        function onRequestRepair() { repairDialog.open() }
        function onRequestVersion(body, matchLabel) {
            versionDialog.body = body
            versionDialog.matchLabel = matchLabel
            versionDialog.open()
        }
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

    Dialog {
        id: trustDialog
        property string explanation
        property string details
        property int fileCount
        property int folderCount
        title: qsTr("Trust %1 executable files?").arg(fileCount)
        modal: true
        anchors.centerIn: parent
        width: Math.min(640, window.width - 40)
        closePolicy: Popup.NoAutoClose
        contentItem: ColumnLayout {
            Label { text: trustDialog.explanation; wrapMode: Text.WordWrap; Layout.fillWidth: true }
            CheckBox { id: trustDetails; text: qsTr("Show files and folders") }
            ScrollView {
                visible: trustDetails.checked
                Layout.fillWidth: true
                Layout.preferredHeight: 220
                TextArea { text: trustDialog.details; readOnly: true; wrapMode: Text.Wrap; textFormat: TextEdit.PlainText }
            }
        }
        footer: DialogButtonBox {
            Button { text: qsTr("Cancel"); DialogButtonBox.buttonRole: DialogButtonBox.RejectRole; onClicked: { trustDialog.close(); backend.confirmTrust(0) } }
            Button { text: qsTr("Trust %1 files").arg(trustDialog.fileCount); onClicked: { trustDialog.close(); backend.confirmTrust(1) } }
            Button { text: qsTr("Trust %1 folders").arg(trustDialog.folderCount); onClicked: { trustDialog.close(); backend.confirmTrust(2) } }
        }
    }

    menuBar: ToolBar {
        visible: backend.ready && !window.fullscreenPreview
        implicitHeight: visible ? menuLayout.implicitHeight : 0
        RowLayout {
            id: menuLayout
            anchors.fill: parent
            spacing: 0
            MenuBar {
                Layout.fillWidth: true
                delegate: MenuBarItem {
                    id: menuItem
                    MenuInput { id: menuInput }
                    onPressed: if (menu) menu.bottomSheet = menu.usesBottomSheet(menuInput.takeTouch())
                }

                AdaptiveMenu {
                    id: fileMenu
                    title: backend.translate("File")
                    pointerPopupType: Popup.Native

                    AdaptiveMenu {
                        bottomSheet: fileMenu.bottomSheet
                        pointerPopupType: Popup.Native
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

                AdaptiveMenu {
                    id: viewMenu
                    title: backend.translate("View")
                    pointerPopupType: Popup.Native
                    onAboutToShow: {
                        inspectorMenuItem.checked = window.inspectorVisible
                        timelineMenuItem.checked = window.timelineVisible
                    }

                    MenuItem {
                        id: inspectorMenuItem
                        text: backend.translate("Inspector")
                        checkable: true
                        checked: window.inspectorVisible
                        onClicked: backend.toggleInspector()
                    }
                    MenuItem {
                        id: timelineMenuItem
                        text: backend.translate("Timeline")
                        checkable: true
                        checked: window.timelineVisible
                        onClicked: backend.toggleTimeline()
                    }
                    Action {
                        text: backend.translate("Fullscreen Preview")
                        shortcut: "F11"
                        checkable: true
                        Binding on checked { value: window.fullscreenPreview }
                        onTriggered: window.setPreviewFullscreen(!window.fullscreenPreview)
                    }
                }
            }
            ToolButton {
                id: projectMenuButton
                display: AbstractButton.IconOnly
                property var document: JSON.parse(backend.projectMenu())
                text: document.label
                icon.source: "qrc:/shrimply/icons/" + document.icon + ".svg"
                MenuInput { id: projectMenuInput }
                onClicked: {
                    projectMenuButton.document = JSON.parse(backend.projectMenu())
                    projectOptions.presentForInput(projectMenuButton, projectMenuInput.takeTouch())
                }
                ToolTip.visible: hovered
                ToolTip.text: text
            }
        }
    }

    Shortcut { sequence: StandardKey.Save; enabled: backend.ready; onActivated: backend.save() }
    Shortcut { sequence: StandardKey.SaveAs; enabled: backend.ready; onActivated: Qt.callLater(backend.showSaveAsDialog) }
    Shortcut { sequences: [StandardKey.Undo]; enabled: backend.ready; onActivated: backend.undo() }
    Shortcut { sequences: [StandardKey.Redo]; enabled: backend.ready; onActivated: backend.redo() }
    Shortcut { sequence: StandardKey.Preferences; enabled: backend.ready; onActivated: preferencesWindow.openPreferences() }

    Connections {
        target: backend
        function onProjectPreferencesRequested() { preferencesWindow.openPreferences() }
        function onProjectAboutRequested() { aboutWindow.openAbout() }
    }

    component ProjectMenu: AdaptiveMenu {
        id: menu
        required property var entries
        pointerPopupType: Qt.platform.os === "windows" ? Popup.Item : Popup.Window
        Instantiator {
            model: menu.entries
            delegate: projectMenuDelegates
            onObjectAdded: (index, object) => {
                if (object instanceof Menu) menu.insertMenu(index, object)
                else menu.insertItem(index, object)
            }
            onObjectRemoved: (index, object) => {
                if (object instanceof Menu) menu.takeMenu(index)
                else menu.takeItem(index)
            }
        }
    }

    DelegateChooser {
        id: projectMenuDelegates
        role: "kind"
        DelegateChoice {
            roleValue: "action"
            delegate: MenuItem {
                required property var modelData
                text: modelData.label
                enabled: modelData.enabled
                onTriggered: Qt.callLater(backend.activateProjectMenu, modelData.action)
            }
        }
        DelegateChoice { roleValue: "separator"; delegate: MenuSeparator {} }
        DelegateChoice {
            roleValue: "submenu"
            delegate: ProjectMenu {
                required property var modelData
                title: modelData.label
                entries: modelData.entries
                bottomSheet: projectOptions.bottomSheet
            }
        }
    }

    ProjectMenu {
        id: projectOptions
        entries: projectMenuButton.document.entries
        pointerAnchor: projectMenuButton
        pointerOffsetY: projectMenuButton.height
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
                text: backend.translate(backend.loadingText)
                opacity: 0.7
            }
        }
    }

    component ThinSplitView: SplitView {
        id: split
        property real dividerExtent: 0
        handle: Rectangle {
            id: divider
            implicitWidth: 1
            implicitHeight: 1
            onWidthChanged: split.dividerExtent = split.orientation === Qt.Horizontal ? width : height
            onHeightChanged: split.dividerExtent = split.orientation === Qt.Horizontal ? width : height
            color: SplitHandle.pressed ? split.palette.highlight : split.palette.mid
            containmentMask: Item {
                readonly property bool vertical: split.orientation === Qt.Horizontal
                x: vertical ? (divider.width - width) / 2 : 0
                y: vertical ? 0 : (divider.height - height) / 2
                width: vertical ? Math.max(divider.width, Qt.styleHints.startDragDistance * 2) : divider.width
                height: vertical ? divider.height : Math.max(divider.height, Qt.styleHints.startDragDistance * 2)
            }
        }
    }

    ColumnLayout {
        anchors.fill: parent
        visible: backend.ready
        spacing: 0

        ThinSplitView {
            id: verticalSplit
            Layout.fillWidth: true
            Layout.fillHeight: true
            orientation: Qt.Vertical
            property bool trackingDivider: false
            onResizingChanged: {
                if (resizing) {
                    trackingDivider = true
                } else if (trackingDivider) {
                    if (timelinePane.visible && height > 0)
                        backend.saveTimelineFraction(editorTop.height, timelinePane.height)
                    trackingDivider = false
                }
            }

            Item {
                id: editorTop
                SplitView.fillWidth: true
                SplitView.fillHeight: true
                SplitView.preferredHeight: 660
                readonly property real inspectorWidth: Math.max(backend.inspectorWidth, inspectorView.implicitWidth)
                readonly property bool inspectorOverlay: backend.inspectorOverlay(window.width)

                ThinSplitView {
                    id: horizontalSplit
                    anchors.fill: parent
                    orientation: Qt.Horizontal
                    property bool trackingDivider: false
                    onResizingChanged: {
                        if (resizing) {
                            trackingDivider = true
                        } else if (trackingDivider) {
                            if (inspectorSlot.visible)
                                backend.saveInspectorWidth(inspectorSlot.width)
                            trackingDivider = false
                        }
                    }

                    Item {
                        id: inspectorSlot
                        visible: window.inspectorVisible && !window.fullscreenPreview && !editorTop.inspectorOverlay
                        Binding {
                            target: inspectorSlot.SplitView
                            property: "preferredWidth"
                            restoreMode: Binding.RestoreNone
                            value: editorTop.inspectorWidth
                            when: !horizontalSplit.resizing && !horizontalSplit.trackingDivider
                        }
                        SplitView.minimumWidth: inspectorView.implicitWidth
                    }

                ColumnLayout {
                    SplitView.fillWidth: true
                    SplitView.fillHeight: true
                    SplitView.minimumWidth: backend.previewMinimumWidth()
                    spacing: 0

                    RowLayout {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        spacing: 0

                        ToolBar {
                            id: previewToolbar
                            visible: !window.fullscreenPreview && backend.previewControlsVisible

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
                                    icon.source: "qrc:/shrimply/icons/checkbox-checked-symbolic.svg"
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
                                    icon.source: "qrc:/shrimply/icons/ruler-angled-symbolic.svg"
                                    text: backend.translate("Guides")
                                    display: AbstractButton.IconOnly
                                    checkable: true
                                    checked: previewLoader.item ? previewLoader.item.guidesVisible : false
                                    onClicked: if (previewLoader.item) previewLoader.item.guidesVisible = checked
                                }
                                ToolSeparator {}
                                ToolButton {
                                    icon.source: "qrc:/shrimply/icons/document-edit-symbolic.svg"
                                    text: backend.translate("Pen")
                                    display: AbstractButton.IconOnly
                                    checkable: true
                                    Binding on checked { value: previewLoader.item ? previewLoader.item.penToolActive : false }
                                    onClicked: if (checked && previewLoader.item) previewLoader.item.selectPenTool()
                                }
                                ToolButton {
                                    icon.source: "qrc:/shrimply/icons/fill-tool-symbolic.svg"
                                    text: backend.translate("Fill")
                                    display: AbstractButton.IconOnly
                                    checkable: true
                                    Binding on checked { value: previewLoader.item ? previewLoader.item.fillToolActive : false }
                                    onClicked: if (checked && previewLoader.item) previewLoader.item.selectFillTool()
                                }
                                ToolButton {
                                    icon.source: "qrc:/shrimply/icons/move-tool-symbolic.svg"
                                    text: backend.translate("Transform")
                                    display: AbstractButton.IconOnly
                                    checkable: true
                                    Binding on checked { value: previewLoader.item ? previewLoader.item.transformToolActive : false }
                                    onClicked: if (checked && previewLoader.item) previewLoader.item.selectTransformTool()
                                }
                                ToolButton {
                                    icon.source: "qrc:/shrimply/icons/eraser-symbolic.svg"
                                    text: backend.translate("Eraser")
                                    display: AbstractButton.IconOnly
                                    checkable: true
                                    Binding on checked { value: previewLoader.item ? previewLoader.item.eraserToolActive : false }
                                    onClicked: if (previewLoader.item) previewLoader.item.eraserToolActive = checked
                                }
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
                                    Keys.onPressed: function(event) {
                                        if (event.key === Qt.Key_Menu || (event.key === Qt.Key_F10 && (event.modifiers & Qt.ShiftModifier))) {
                                            previewContextAnchor.x = 0
                                            previewContextAnchor.y = 0
                                            previewContextMenu.show(false)
                                            event.accepted = true
                                        }
                                    }
                                    TapHandler {
                                        acceptedButtons: Qt.RightButton
                                        onTapped: function(point) {
                                            previewContextAnchor.x = point.position.x
                                            previewContextAnchor.y = point.position.y
                                            previewContextMenu.show(false)
                                        }
                                    }
                                    TapHandler {
                                        acceptedDevices: PointerDevice.TouchScreen | PointerDevice.Stylus
                                        onLongPressed: {
                                            previewContextAnchor.x = point.position.x
                                            previewContextAnchor.y = point.position.y
                                            previewContextMenu.show(true)
                                        }
                                    }
                                }
                            }
                        }
                    }

                    ToolBar {
                        Layout.fillWidth: true
                        implicitHeight: transportGrid.implicitHeight + topPadding + bottomPadding

                        GridLayout {
                            id: transportGrid
                            anchors.fill: parent
                            anchors.leftMargin: 8
                            anchors.rightMargin: 8
                            columns: 4
                            readonly property bool stacked: backend.transportStacked(width)

                            RowLayout {
                                Layout.row: transportGrid.stacked ? 1 : 0
                                Layout.column: 0
                                Layout.columnSpan: 1
                                Layout.alignment: Qt.AlignLeft
                                ToolButton { icon.source: "qrc:/shrimply/icons/media-seek-backward-symbolic.svg"; onClicked: backend.stepFrame(-1) }
                                ToolButton {
                                    icon.source: backend.playing
                                        ? "qrc:/shrimply/icons/media-playback-pause-symbolic.svg"
                                        : "qrc:/shrimply/icons/media-playback-start-symbolic.svg"
                                    onClicked: backend.togglePlaying()
                                }
                                ToolButton { icon.source: "qrc:/shrimply/icons/media-seek-forward-symbolic.svg"; onClicked: backend.stepFrame(1) }
                            }
                            Slider {
                                Layout.row: 0
                                Layout.column: transportGrid.stacked ? 0 : 1
                                Layout.columnSpan: transportGrid.stacked ? 3 : 1
                                Layout.fillWidth: true
                                from: 0
                                to: Math.max(1, backend.durationFrame)
                                value: backend.positionFrame
                                onMoved: backend.seekFrame(Math.round(value))
                            }
                            Label {
                                Layout.row: transportGrid.stacked ? 1 : 0
                                Layout.column: transportGrid.stacked ? 1 : 2
                                Layout.columnSpan: transportGrid.stacked ? 3 : 1
                                Layout.fillWidth: transportGrid.stacked
                                horizontalAlignment: transportGrid.stacked ? Text.AlignRight : Text.AlignLeft
                                elide: Text.ElideRight
                                text: backend.timeLabel
                                font.family: backend.fixedFontFamily
                            }
                            ToolButton {
                                Layout.row: 0
                                Layout.column: 3
                                icon.source: window.fullscreenPreview
                                    ? "qrc:/shrimply/icons/view-restore-symbolic.svg"
                                    : "qrc:/shrimply/icons/view-fullscreen-symbolic.svg"
                                text: window.fullscreenPreview ? backend.translate("Exit Fullscreen Preview") : backend.translate("Fullscreen Preview")
                                display: AbstractButton.IconOnly
                                onClicked: window.setPreviewFullscreen(!window.fullscreenPreview)
                            }
                        }
                    }
                }
                }
                Pane {
                    id: inspectorPane
                    parent: editorTop.inspectorOverlay ? editorTop : inspectorSlot
                    padding: 0
                    background: Rectangle {
                        color: Qt.rgba(inspectorPane.palette.window.r, inspectorPane.palette.window.g, inspectorPane.palette.window.b, 1)
                    }
                    layer.enabled: editorTop.inspectorOverlay
                    layer.effect: MultiEffect {
                        shadowEnabled: true
                        shadowColor: inspectorPane.palette.shadow
                        shadowOpacity: 0.3
                        blurMax: 12
                    }
                    visible: window.inspectorVisible && !window.fullscreenPreview
                    width: editorTop.inspectorOverlay ? Math.min(editorTop.inspectorWidth, editorTop.width) : inspectorSlot.width
                    height: parent.height
                    z: 1

                    InspectorView {
                        id: inspectorView
                        anchors.fill: parent
                        clip: true
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
            }

            RowLayout {
                id: timelinePane
                visible: window.timelineVisible && !window.fullscreenPreview
                SplitView.fillWidth: true
                Binding {
                    target: timelinePane.SplitView
                    property: "preferredHeight"
                    restoreMode: Binding.RestoreNone
                    value: backend.splitPaneExtent(verticalSplit.height, verticalSplit.dividerExtent, backend.timelineFraction)
                    when: !verticalSplit.resizing && !verticalSplit.trackingDivider
                }
                SplitView.minimumHeight: 180
                spacing: 0

                ToolBar {
                    Layout.fillHeight: true
                    Layout.preferredWidth: 44 * (timelineLoader.item ? timelineLoader.item.toolsProgress : 1)
                    clip: true
                    visible: Layout.preferredWidth > 0

                    ColumnLayout {
                        id: timelineToolColumn

                        anchors.top: parent.top
                        anchors.horizontalCenter: parent.horizontalCenter
                        ToolButton {
                            icon.source: "qrc:/shrimply/icons/magnet-tilted-symbolic.svg"
                            text: backend.translate("Magnet")
                            display: AbstractButton.IconOnly
                            checkable: true
                            Binding on checked { value: timelineLoader.item ? timelineLoader.item.magnetEnabled : false }
                            onClicked: if (timelineLoader.item) timelineLoader.item.magnetEnabled = checked
                        }
                        ToolButton {
                            icon.source: "qrc:/shrimply/icons/metronome-symbolic.svg"
                            text: backend.translate("Beat Grid")
                            display: AbstractButton.IconOnly
                            checkable: true
                            Binding on checked { value: timelineLoader.item ? timelineLoader.item.beatGridEnabled : false }
                            onClicked: if (timelineLoader.item) timelineLoader.item.beatGridEnabled = checked
                        }
                        ToolSeparator {
                            orientation: Qt.platform.os === "windows" ? Qt.Horizontal : Qt.Vertical
                            Layout.fillWidth: Qt.platform.os === "windows"
                        }
                        ToolButton {
                            icon.source: "qrc:/shrimply/icons/pointer-left-symbolic.svg"
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
                            icon.source: "qrc:/shrimply/icons/edit-cut-symbolic.svg"
                            text: backend.translate("Cut")
                            display: AbstractButton.IconOnly
                            checkable: true
                            Binding on checked {
                                value: timelineLoader.item ? timelineLoader.item.cutEnabled : false
                            }
                            onClicked: if (checked && timelineLoader.item)
                                timelineLoader.item.cutEnabled = true
                        }
                        ToolSeparator {
                            orientation: Qt.platform.os === "windows" ? Qt.Horizontal : Qt.Vertical
                            Layout.fillWidth: Qt.platform.os === "windows"
                        }
                        ToolButton {
                            icon.source: "qrc:/shrimply/icons/track-insert-symbolic.svg"
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
                            icon.source: "qrc:/shrimply/icons/track-block-symbolic.svg"
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
                            icon.source: "qrc:/shrimply/icons/track-move-above-symbolic.svg"
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
                    Layout.preferredWidth: 54 * (timelineLoader.item ? timelineLoader.item.audioMonitorProgress : 1)
                    clip: true
                    visible: Layout.preferredWidth > 0
                    Layout.fillHeight: true
                    active: backend.ready
                    sourceComponent: Component {
                        AudioMeterSurface { anchors.fill: parent }
                    }
                }
            }
        }
    }


    AdaptiveMenu {
        pointerPopupType: Qt.platform.os === "windows" ? Popup.Item : Popup.Window
        id: previewContextMenu
        property var labels: []

        function show(touch) {
            if (!previewLoader.item) return
            previewLoader.item.cancelPointerInput()
            previewLoader.item.forceActiveFocus()
            labels = backend.previewContextLabels()
            updateChecks()
            present(previewContextAnchor, backend.previewContextBottomSheet(touch))
        }

        function updateChecks() {
            for (let index = 0; index < count; ++index) {
                const item = itemAt(index)
                if (item) {
                    const state = backend.previewContextChecked(index)
                    item.checkable = state >= 0
                    item.checked = state === 1
                }
            }
        }

        Instantiator {
            model: previewContextMenu.labels
            delegate: MenuItem {
                required property int index
                required property string modelData
                text: modelData
                checkable: backend.previewContextChecked(index) >= 0
                checked: backend.previewContextChecked(index) === 1
                onTriggered: {
                    const request = backend.activatePreviewContext(index)
                    if (request > 0 && previewLoader.item)
                        previewLoader.item.captureContextImage(request === 2)
                }
            }
            onObjectAdded: (index, object) => previewContextMenu.insertItem(index, object)
            onObjectRemoved: (index, object) => previewContextMenu.takeItem(index)
        }
    }

    Item {
        id: previewContextAnchor
        parent: previewLoader.item
        width: 1
        height: 1
        visible: false
    }

    Connections {
        target: backend
        function onPreviewControlsVisibleChanged() { previewContextMenu.updateChecks() }
    }

    Connections {
        target: previewLoader.item
        function onPreviewImageSaveRequested() {
            previewLoader.item.saveCapturedImage(backend.showPreviewImageSaveDialog())
        }
        function onPreviewImageCaptureFailed(error) { backend.reportPreviewImageError(error) }
    }

    AdaptiveMenu {
        pointerPopupType: Qt.platform.os === "windows" ? Popup.Item : Popup.Window
        id: timelineTrackAddMenu

        Instantiator {
            model: timelineLoader.item ? timelineLoader.item.trackAddMenuItems : []
            delegate: DelegateChooser {
                role: "kind"
                DelegateChoice {
                    roleValue: 1
                    delegate: MenuItem {
                        required property var modelData
                        text: backend.translate(modelData.label)
                        icon.source: "qrc:/shrimply/icons/" + modelData.icon + ".svg"
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

    AdaptiveMenu {
        pointerPopupType: Qt.platform.os === "windows" ? Popup.Item : Popup.Window
        id: timelineContextMenu

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
                        checkable: modelData.checkable
                        checked: modelData.checked
                        icon.source: modelData.label === "Copy"
                            ? "qrc:/shrimply/icons/edit-copy-symbolic.svg"
                            : modelData.label === "Cut"
                            ? "qrc:/shrimply/icons/edit-cut-symbolic.svg"
                            : modelData.label === "Paste"
                            ? "qrc:/shrimply/icons/edit-paste-symbolic.svg" : ""
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
        function onContextMenuRequested(x, y, bottomSheet) {
            timelineContextAnchor.x = x
            timelineContextAnchor.y = y
            timelineContextMenu.present(timelineContextAnchor, bottomSheet)
        }
        function onTrackAddMenuRequested(x, y, bottomSheet) {
            timelineTrackAddAnchor.x = x
            timelineTrackAddAnchor.y = y
            timelineTrackAddMenu.present(timelineTrackAddAnchor, bottomSheet)
        }
        function onTrackImportRequested() {
            const selected = backend.showOpenFileDialog(
                "",
                backend.translate("Import to Track"),
                backend.translate("All files (*)"))
            if (selected.toString().length > 0)
                timelineLoader.item.importTrackFile(selected)
        }
        function onTrackRemuxRequested() { timelineRemuxDialog.open() }
        function onSourceDeletionRequested(path) {
            timelineSourceDeletionDialog.sourcePath = path
            timelineSourceDeletionDialog.open()
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

    Dialog {
        id: timelineRemuxDialog
        title: backend.translate("Remux MKV/WebM to MP4?")
        modal: true
        anchors.centerIn: parent
        closePolicy: Popup.CloseOnEscape
        onAccepted: timelineLoader.item.confirmTrackRemux(true)
        onRejected: timelineLoader.item.confirmTrackRemux(false)

        contentItem: Label {
            text: backend.translate("MP4 is the supported timeline format. The MP4 will be created beside the source. After import, you can choose whether to delete the original.")
            wrapMode: Text.Wrap
        }
        footer: DialogButtonBox {
            Button {
                text: backend.translate("Cancel")
                DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
            }
            Button {
                text: backend.translate("Remux")
                highlighted: true
                DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole
            }
            onRejected: timelineRemuxDialog.reject()
            onAccepted: timelineRemuxDialog.accept()
        }
    }

    Dialog {
        id: timelineSourceDeletionDialog
        property string sourcePath: ""
        title: backend.translate("Delete original file?")
        modal: true
        anchors.centerIn: parent
        closePolicy: Popup.CloseOnEscape
        contentItem: Label {
            text: backend.translate("The file was remuxed. Delete the original to keep only the MP4 copy?") + "\n\n" + timelineSourceDeletionDialog.sourcePath
            textFormat: Text.PlainText
            wrapMode: Text.Wrap
        }
        onOpened: keepOriginalButton.forceActiveFocus()
        onAccepted: timelineLoader.item.confirmSourceDeletion(true)
        onRejected: timelineLoader.item.confirmSourceDeletion(false)
        footer: DialogButtonBox {
            Button {
                id: keepOriginalButton
                text: backend.translate("Keep")
                highlighted: true
                DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
            }
            Button {
                text: backend.translate("Delete")
                DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole
            }
            onRejected: timelineSourceDeletionDialog.reject()
            onAccepted: timelineSourceDeletionDialog.accept()
        }
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

    Dialog {
        id: versionDialog
        title: backend.translate("Service Version Mismatch")
        anchors.centerIn: parent
        width: Math.min(480, window.width - 32)
        modal: true
        property string body: ""
        property string matchLabel: ""
        property int choice: 0
        onOpened: choice = 0
        onClosed: backend.confirmVersion(choice)
        contentItem: Label {
            text: versionDialog.body
            wrapMode: Text.Wrap
        }
        footer: DialogButtonBox {
            Button {
                text: backend.translate("Close")
                onClicked: versionDialog.close()
            }
            Button {
                text: backend.translate("Force Open")
                onClicked: { versionDialog.choice = 1; versionDialog.close() }
            }
            Button {
                visible: versionDialog.matchLabel.length > 0
                text: versionDialog.matchLabel
                highlighted: true
                onClicked: { versionDialog.choice = 2; versionDialog.close() }
            }
        }
    }

    MessageDialog {
        id: warningDialog
        title: backend.translate("OTIO imported with limitations")
        buttons: MessageDialog.Ok
        onAccepted: backend.acknowledgeWarnings()
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
