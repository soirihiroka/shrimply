pragma ComponentBehavior: Bound

import QtQuick
import dev.shrimply.components
import QtQuick.Controls
import QtQuick.Layouts

AdaptiveMenu {
    id: root
    property var labels: []
    property var searchTerms: labels
    property var tooltips: []
    property int selectedIndex: -1
    property bool searchEnabled: true
    property string placeholderText: ComponentTranslations.text("Search")
    property real minimumListHeight: 0
    property real maximumListHeight: 280
    property bool navigationActive: false
    readonly property var matchingIndices: backend.rankedMatchingIndices(
        labels, searchTerms, search.text)
    signal activated(int index)

    preferredWidth: 280
    padding: 6
    focus: true
    closePolicy: Popup.CloseOnEscape | (bottomSheet ? Popup.CloseOnPressOutside : Popup.CloseOnPressOutsideParent)

    function choose(row) {
        if (row < 0 || row >= matchingIndices.length)
            return
        activated(Number(matchingIndices[row]))
        close()
    }

    function moveSelection(direction) {
        navigationActive = true
        if (root.matchingIndices.length === 0)
            return
        const next = (choices.currentIndex + direction
            + root.matchingIndices.length) % root.matchingIndices.length
        choices.currentIndex = next
        choices.positionViewAtIndex(next, ListView.Contain)
    }

    onOpened: {
        search.clear()
        navigationActive = false
        choices.currentIndex = selectedIndex >= 0
            ? backend.matchingIndex(root.matchingIndices, String(selectedIndex))
            : root.matchingIndices.length > 0 ? 0 : -1
        if (searchEnabled)
            search.forceActiveFocus(Qt.PopupFocusReason)
        else
            choices.forceActiveFocus(Qt.PopupFocusReason)
    }

    SelectorBackend { id: backend }

    contentItem: ColumnLayout {
        spacing: 6

        TextField {
            id: search
            visible: root.searchEnabled
            Layout.fillWidth: true
            placeholderText: root.placeholderText
            selectByMouse: true
            onTextChanged: {
                root.navigationActive = false
                choices.currentIndex = root.matchingIndices.length > 0 ? 0 : -1
            }
            Keys.onDownPressed: function(event) {
                root.navigationActive = true
                choices.currentIndex = root.matchingIndices.length > 0 ? 0 : -1
                choices.forceActiveFocus(Qt.TabFocusReason)
                event.accepted = true
            }
            Keys.onUpPressed: function(event) {
                root.navigationActive = true
                choices.currentIndex = root.matchingIndices.length - 1
                choices.forceActiveFocus(Qt.TabFocusReason)
                event.accepted = true
            }
            Keys.onEscapePressed: function(event) {
                root.close()
                event.accepted = true
            }
            onAccepted: root.choose(choices.currentIndex)
        }

        ListView {
            id: choices
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(
                root.maximumListHeight,
                Math.max(root.minimumListHeight, contentHeight))
            clip: true
            model: root.matchingIndices
            keyNavigationEnabled: false
            delegate: ItemDelegate {
                required property int index
                required property string modelData
                readonly property int sourceIndex: Number(modelData)
                width: choices.width
                text: root.labels[sourceIndex]
                icon.source: root.selectedIndex === sourceIndex
                    ? "qrc:/shrimply/icons/object-select-symbolic.svg"
                    : ""
                icon.color: palette.buttonText
                highlighted: root.navigationActive && choices.currentIndex === index
                ToolTip.visible: hovered
                    && sourceIndex < root.tooltips.length
                    && root.tooltips[sourceIndex].length > 0
                ToolTip.text: sourceIndex < root.tooltips.length
                    ? root.tooltips[sourceIndex] : ""
                onHoveredChanged: if (hovered) {
                    root.navigationActive = true
                    choices.currentIndex = index
                }
                onClicked: root.choose(index)
            }
            Keys.onDownPressed: function(event) {
                root.moveSelection(1)
                event.accepted = true
            }
            Keys.onUpPressed: function(event) {
                root.moveSelection(-1)
                event.accepted = true
            }
            Keys.onReturnPressed: function(event) {
                root.choose(currentIndex)
                event.accepted = true
            }
            Keys.onEnterPressed: function(event) {
                root.choose(currentIndex)
                event.accepted = true
            }
            Keys.onEscapePressed: function(event) {
                root.close()
                event.accepted = true
            }
        }
    }
}
