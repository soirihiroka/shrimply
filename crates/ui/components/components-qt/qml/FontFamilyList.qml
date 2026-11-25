pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Effects
import QtQuick.Layouts
import dev.shrimply.components

ColumnLayout {
    id: root
    property string value: "[]"
    property var browserBackend: null
    property int categoryIndex: -1
    property int itemIndex: -1
    property int controlIndex: -1
    readonly property var families: parseFamilies(value)
    readonly property int browserRevision: browserBackend
        ? browserBackend.fontBrowserRevision : 0
    property int editedIndex: -1
    property bool activating: false
    signal edited(string value)
    spacing: 2

    component CloudIcon: Image {
        id: cloud
        Layout.preferredWidth: 14
        Layout.preferredHeight: 14
        source: "qrc:/shrimply/icons/cloud-symbolic.svg"
        Accessible.name: ComponentTranslations.text("Google Fonts")
        layer.enabled: true
        layer.effect: MultiEffect {
            colorization: 1
            colorizationColor: cloud.palette.text
        }
    }

    onValueChanged: {
        if (activating && browserBackend) {
            browserBackend.cancelControlFontActivation()
            activating = false
            browser.close()
        }
    }

    function parseFamilies(serialized) {
        try {
            const parsed = JSON.parse(serialized)
            return Array.isArray(parsed) ? parsed : []
        } catch (error) {
            return []
        }
    }

    function familyName(family) {
        if (family && family.local)
            return family.local.name
        if (family && family.google_fonts)
            return family.google_fonts.name
        return ComponentTranslations.text("Choose font")
    }

    function isGoogle(family) {
        return Boolean(family && family.google_fonts)
    }

    function nextValue(index, family) {
        return browserBackend
            ? String(browserBackend.fontListWithChoice(
                value, index, JSON.stringify(family))) : ""
    }

    function openBrowser(index) {
        if (!browserBackend)
            return
        editedIndex = index
        activating = false
        search.clear()
        browserBackend.openFontBrowser()
        browserBackend.searchFontBrowser("")
        browser.open()
    }

    function choose(choice) {
        const serialized = String(browserBackend.fontBrowserValue(choice))
        if (serialized.length === 0)
            return
        const family = JSON.parse(serialized)
        const next = nextValue(editedIndex, family)
        if (next.length === 0)
            return
        if (!isGoogle(family)) {
            edited(next)
            browser.close()
            return
        }
        activating = true
        browserBackend.activateControlFont(
            categoryIndex, itemIndex, controlIndex, serialized, next)
    }

    function selectedChoice(serialized) {
        if (editedIndex < 0 || editedIndex >= families.length)
            return false
        const candidate = JSON.parse(serialized)
        const selected = families[editedIndex]
        return familyName(candidate).toLocaleLowerCase()
                === familyName(selected).toLocaleLowerCase()
            && isGoogle(candidate) === isGoogle(selected)
    }

    function move(index, offset) {
        const next = browserBackend
            ? String(browserBackend.moveFontListValue(value, index, offset)) : ""
        if (next.length > 0)
            edited(next)
    }

    function remove(index) {
        const next = browserBackend
            ? String(browserBackend.removeFontListValue(value, index)) : ""
        if (next.length > 0)
            edited(next)
    }

    Label {
        visible: root.families.length === 0
        text: ComponentTranslations.text("System default")
        opacity: 0.6
        Layout.fillWidth: true
    }

    Repeater {
        model: root.families
        delegate: RowLayout {
            id: familyRow
            required property int index
            required property var modelData
            spacing: 2
            Layout.fillWidth: true

            Button {
                text: root.familyName(familyRow.modelData)
                Layout.fillWidth: true
                onClicked: root.openBrowser(familyRow.index)
            }
            CloudIcon { visible: root.isGoogle(familyRow.modelData) }
            ToolButton {
                icon.source: "qrc:/shrimply/icons/go-up-symbolic.svg"
                enabled: familyRow.index > 0
                onClicked: root.move(familyRow.index, -1)
                ToolTip.visible: hovered
                ToolTip.text: ComponentTranslations.text("Move up")
            }
            ToolButton {
                icon.source: "qrc:/shrimply/icons/go-down-symbolic.svg"
                enabled: familyRow.index + 1 < root.families.length
                onClicked: root.move(familyRow.index, 1)
                ToolTip.visible: hovered
                ToolTip.text: ComponentTranslations.text("Move down")
            }
            ToolButton {
                icon.source: "qrc:/shrimply/icons/user-trash-symbolic.svg"
                onClicked: root.remove(familyRow.index)
                ToolTip.visible: hovered
                ToolTip.text: ComponentTranslations.text("Remove")
            }
        }
    }

    ToolButton {
        icon.source: "qrc:/shrimply/icons/list-add-symbolic.svg"
        display: AbstractButton.IconOnly
        Layout.alignment: Qt.AlignRight
        onClicked: root.openBrowser(-1)
        ToolTip.visible: hovered
        ToolTip.text: ComponentTranslations.text("Add font")
    }

    Connections {
        target: root.browserBackend
        function onFontBrowserRevisionChanged() {
            choices.requestedFirst = -1
            choices.requestedCount = -1
            Qt.callLater(choices.requestVisiblePreviews)
            if (!root.activating || root.browserBackend.fontBrowserBusy())
                return
            root.activating = false
            if (root.browserBackend.fontBrowserStatus().length === 0)
                browser.close()
        }
    }

    Dialog {
        id: browser
        modal: true
        anchors.centerIn: Overlay.overlay
        title: ComponentTranslations.text("Fonts")
        width: Math.min(1180, Overlay.overlay.width - 48)
        height: Math.min(760, Overlay.overlay.height - 48)
        standardButtons: Dialog.Close
        closePolicy: Popup.CloseOnEscape

        onClosed: {
            if (root.activating && root.browserBackend)
                root.browserBackend.cancelControlFontActivation()
            root.activating = false
            searchDelay.stop()
        }

        contentItem: ColumnLayout {
            spacing: 8

            TextField {
                id: search
                Layout.preferredWidth: 400
                Layout.maximumWidth: 400
                Layout.alignment: Qt.AlignHCenter
                placeholderText: ComponentTranslations.text(
                    "Search fonts or paste a Google Fonts specimen URL")
                selectByMouse: true
                onTextChanged: searchDelay.restart()
                onAccepted: {
                    searchDelay.stop()
                    root.browserBackend.searchFontBrowser(text)
                }
            }

            RowLayout {
                visible: root.browserRevision >= 0 && root.browserBackend
                    && root.browserBackend.fontBrowserStatus().length > 0
                Layout.fillWidth: true
                BusyIndicator {
                    running: root.browserRevision >= 0 && root.browserBackend
                        && root.browserBackend.fontBrowserBusy()
                    implicitWidth: 24
                    implicitHeight: 24
                }
                Label {
                    text: root.browserRevision >= 0 && root.browserBackend
                        ? root.browserBackend.fontBrowserStatus() : ""
                    opacity: 0.7
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                }
            }

            GridView {
                id: choices
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                leftMargin: 16
                rightMargin: 16
                topMargin: 16
                bottomMargin: 16
                readonly property int columnCount: Math.max(1,
                    Math.floor((width - leftMargin - rightMargin + 8) / 248))
                property int requestedFirst: -1
                property int requestedCount: -1
                cellWidth: Math.max(1,
                    (width - leftMargin - rightMargin + 8) / columnCount)
                cellHeight: 224
                model: root.browserBackend && root.browserRevision >= 0
                    ? root.browserBackend.fontBrowserCount() : 0
                ScrollBar.vertical: ScrollBar {}

                function requestVisiblePreviews() {
                    if (!root.browserBackend || model <= 0
                            || height <= 0 || cellHeight <= 0) {
                        requestedFirst = 0
                        requestedCount = 0
                        return
                    }
                    // topMargin is represented by negative contentY; rows start at 0.
                    const firstRow = Math.max(0,
                        Math.floor(Math.max(0, contentY) / cellHeight))
                    const lastRow = Math.max(firstRow, Math.ceil(
                        Math.max(0, contentY + height) / cellHeight))
                    const first = Math.min(model, firstRow * columnCount)
                    const count = Math.min(model - first,
                        (lastRow - firstRow) * columnCount)
                    if (first === requestedFirst && count === requestedCount)
                        return
                    requestedFirst = first
                    requestedCount = count
                    root.browserBackend.requestFontBrowserPreviews(first, count)
                }

                onContentYChanged: requestVisiblePreviews()
                onHeightChanged: requestVisiblePreviews()
                onWidthChanged: requestVisiblePreviews()
                onModelChanged: {
                    requestedFirst = -1
                    requestedCount = -1
                    requestVisiblePreviews()
                }

                delegate: ItemDelegate {
                    id: choice
                    required property int index
                    readonly property string familyName:
                        root.browserRevision >= 0
                            ? root.browserBackend.fontBrowserLabel(index) : ""
                    readonly property bool google:
                        root.browserRevision >= 0
                            && root.browserBackend.fontBrowserGoogle(index)
                    readonly property string serialized:
                        root.browserRevision >= 0
                            ? root.browserBackend.fontBrowserValue(index) : ""
                    readonly property bool selected:
                        serialized.length > 0 && root.selectedChoice(serialized)
                    width: GridView.view.cellWidth - 8
                    height: GridView.view.cellHeight - 8
                    checkable: true
                    checked: selected
                    enabled: !root.activating
                    onClicked: root.choose(index)

                    FontLoader {
                        id: specimenFont
                        source: root.browserRevision >= 0
                            ? root.browserBackend.fontBrowserPreviewSource(choice.index) : ""
                    }

                    contentItem: Item {
                        Label {
                            id: specimen
                            anchors {
                                left: parent.left
                                right: parent.right
                                top: parent.top
                                bottom: details.top
                            }
                            text: "Aa"
                            horizontalAlignment: Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter
                            font.family: specimenFont.status === FontLoader.Ready
                                ? specimenFont.name : choice.familyName
                            font.pixelSize: 128
                            minimumPixelSize: 32
                            fontSizeMode: Text.Fit
                            visible: specimenFont.status !== FontLoader.Loading
                        }
                        BusyIndicator {
                            anchors.centerIn: specimen
                            width: 38
                            height: 38
                            running: specimenFont.status === FontLoader.Loading
                            visible: running
                        }

                        RowLayout {
                            id: details
                            anchors {
                                left: parent.left
                                right: parent.right
                                bottom: parent.bottom
                            }
                            spacing: 8

                            Label {
                                text: choice.familyName
                                font.pixelSize: 15
                                elide: Text.ElideRight
                                Layout.fillWidth: true
                            }
                            CloudIcon { visible: choice.google }
                            Label {
                                visible: choice.selected
                                text: "✓"
                                font.pixelSize: 20
                            }
                        }
                    }
                }
            }
        }

        Timer {
            id: searchDelay
            interval: 500
            onTriggered: root.browserBackend.searchFontBrowser(search.text)
        }
    }
}
