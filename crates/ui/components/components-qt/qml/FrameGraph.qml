import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.shrimply.components

FocusScope {
    id: root
    signal togglePlayback()
    signal editFinished()
    signal playheadChanged(int component, var numerator, var denominator)
    signal keysChanged(int component, var times, var values)
    signal keysMoved(int component, var oldTimes, var times, var values)
    signal keysDeleted(int component, var times)
    signal keyAdded(int component, var numerator, var denominator, real value)
    signal keysPasted(int component, var times, var values)
    signal copyRequested(int component, var times)
    signal pasteRequested(int component, var numerator, var denominator)
    signal textInterpolationRequested(int component, string ownerId, real x, real y)
    signal interpolationChanged(int component, string ownerId, int index)
    signal textInterpolationChanged(int component, string ownerId, int index)
    property var textInterpolationLabels: []
    property var textInterpolationTooltips: []
    property var textInterpolationIndexForOwner: null
    readonly property real graphValue: graph.graphValue
    readonly property var interpolationLabels: {
        const labels = []
        for (let index = 0; index < graph.interpolationCount; ++index)
            labels.push(graph.interpolationLabel(index))
        return labels
    }

    function editValue(value) { graph.editGraphValue(value) }
    function configureCurrentValue(value) { graph.configureGraphCurrentValue(value) }
    function configureValue(value) { graph.configureGraphValue(value) }
    function editComponent(component, value) {
        graph.editGraphComponentValue(component, value)
    }
    function editPair(first, second, activeComponent, firstChanged, secondChanged) {
        graph.editGraphPair(first, second, activeComponent, firstChanged, secondChanged)
    }
    function configurePair(first, second, activeComponent) {
        graph.configureGraphPair(first, second, activeComponent)
    }
    function replaceStepGraph(component, times, values) {
        graph.replaceStepGraph(component, times, values)
    }
    function reconcileStepMoves(component, oldTimes, rawTimes, times) {
        graph.reconcileStepGraphMoves(component, oldTimes, rawTimes, times)
    }
    function rollbackStepMoves(component, oldTimes, rawTimes) {
        graph.rollbackStepGraphMoves(component, oldTimes, rawTimes)
    }
    function replaceRawGraph(component, pointTimes, pointValues, segments, staticValue) {
        graph.replaceRawGraph(component, pointTimes, pointValues, segments, staticValue)
    }
    function replaceSpeedGraph(component, keys, segments, staticValue) {
        graph.replaceSpeedGraph(component, keys, segments, staticValue)
    }
    function setRange(startNumerator, startDenominator, endNumerator, endDenominator) {
        graph.setGraphRange(startNumerator, startDenominator, endNumerator, endDenominator)
    }
    function setFrameStep(numerator, denominator) {
        graph.setGraphFrameStep(numerator, denominator)
    }
    function setPlayhead(numerator, denominator) {
        graph.setGraphPlayhead(numerator, denominator)
    }
    function setSnapping(enabled, radiusPx) { graph.setGraphSnapping(enabled, radiusPx) }
    function setExternalClipboard(enabled) { graph.setGraphExternalClipboard(enabled) }
    function setTextInterpolation(enabled) { graph.setGraphTextInterpolation(enabled) }
    function activateComponent(component) { graph.activateGraphComponent(component) }

    implicitWidth: 640
    implicitHeight: controls.implicitHeight + graph.implicitHeight

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        RowLayout {
            id: controls
            Layout.fillWidth: true
            spacing: 6

            Item {
                Layout.fillWidth: true
            }
            ToolButton {
                enabled: graph.canPrevious
                icon.source: "qrc:/qt/qml/dev/shrimply/components/icons/previous.svg"
                icon.color: palette.buttonText
                display: AbstractButton.IconOnly
                ToolTip.visible: hovered
                ToolTip.text: "Previous keyframe"
                onClicked: {
                    graph.previousKey()
                    graph.forceActiveFocus()
                }
            }
            ToolButton {
                icon.source: graph.keyAtPlayhead
                             ? "qrc:/qt/qml/dev/shrimply/components/icons/remove.svg"
                             : "qrc:/qt/qml/dev/shrimply/components/icons/add.svg"
                icon.color: palette.buttonText
                display: AbstractButton.IconOnly
                ToolTip.visible: hovered
                ToolTip.text: graph.keyAtPlayhead
                              ? "Delete keyframe at playhead"
                              : "Add keyframe at playhead"
                onClicked: {
                    graph.toggleKey()
                    graph.forceActiveFocus()
                }
            }
            ToolButton {
                enabled: graph.canNext
                icon.source: "qrc:/qt/qml/dev/shrimply/components/icons/next.svg"
                icon.color: palette.buttonText
                display: AbstractButton.IconOnly
                ToolTip.visible: hovered
                ToolTip.text: "Next keyframe"
                onClicked: {
                    graph.nextKey()
                    graph.forceActiveFocus()
                }
            }
        }

        FrameGraphItem {
            id: graph
            Layout.fillWidth: true
            Layout.fillHeight: true
            implicitHeight: preferredHeight
            mirrorVertically: true
            onTogglePlayback: root.togglePlayback()
            onEditFinished: root.editFinished()
            onPlayheadChanged: function(component, numerator, denominator) {
                root.playheadChanged(component, numerator, denominator)
            }
            onKeysChanged: function(component, times, values) {
                root.keysChanged(component, times, values)
            }
            onKeysMoved: function(component, oldTimes, times, values) {
                root.keysMoved(component, oldTimes, times, values)
            }
            onKeysDeleted: function(component, times) {
                root.keysDeleted(component, times)
            }
            onKeyAdded: function(component, numerator, denominator, value) {
                root.keyAdded(component, numerator, denominator, value)
            }
            onKeysPasted: function(component, times, values) {
                root.keysPasted(component, times, values)
            }
            onCopyRequested: function(component, times) {
                root.copyRequested(component, times)
            }
            onPasteRequested: function(component, numerator, denominator) {
                root.pasteRequested(component, numerator, denominator)
            }
            onTextInterpolationRequested: function(component, ownerId, x, y) {
                root.textInterpolationRequested(component, ownerId, x, y)
                if (root.textInterpolationLabels.length === 0)
                    return
                textInterpolationAnchor.x = x
                textInterpolationAnchor.y = controls.height + y
                textInterpolationMenu.selectedIndex = root.textInterpolationIndexForOwner
                    ? root.textInterpolationIndexForOwner(ownerId) : -1
                textInterpolationMenu.popup(textInterpolationAnchor, 0, 0)
            }
            onInterpolationRequested: function(component, ownerId, index, x, y) {
                interpolationAnchor.x = x
                interpolationAnchor.y = controls.height + y
                interpolationMenu.selectedIndex = index
                interpolationMenu.popup(interpolationAnchor, 0, 0)
            }
            onInterpolationChanged: function(component, ownerId, index) {
                root.interpolationChanged(component, ownerId, index)
            }
            onTextInterpolationChanged: function(component, ownerId, index) {
                root.textInterpolationChanged(component, ownerId, index)
            }

            Keys.onPressed: function(event) {
                let key = -1
                if ((event.modifiers & Qt.ControlModifier) && event.key === Qt.Key_C)
                    key = 7
                else if ((event.modifiers & Qt.ControlModifier) && event.key === Qt.Key_V)
                    key = 8
                else if (event.key === Qt.Key_Left) key = 0
                else if (event.key === Qt.Key_Right) key = 1
                else if (event.key === Qt.Key_Home) key = 2
                else if (event.key === Qt.Key_End) key = 3
                else if (event.key === Qt.Key_Plus || event.key === Qt.Key_Equal) key = 4
                else if (event.key === Qt.Key_Minus) key = 5
                else if (event.key === Qt.Key_Delete || event.key === Qt.Key_Backspace) key = 6
                else if (event.key === Qt.Key_Space) key = 9
                if (key >= 0) {
                    graph.handleKey(key)
                    event.accepted = true
                }
            }

            MouseArea {
                anchors.fill: parent
                acceptedButtons: Qt.LeftButton | Qt.MiddleButton | Qt.RightButton
                hoverEnabled: true
                preventStealing: true

                onPressed: function(mouse) {
                    mouse.accepted = true
                    graph.forceActiveFocus()
                    const button = mouse.button === Qt.LeftButton ? 0
                                 : mouse.button === Qt.MiddleButton ? 1 : 2
                    if (button !== 2)
                        graph.begin(
                            button, mouse.x, mouse.y,
                            (mouse.modifiers & Qt.ControlModifier) !== 0,
                            (mouse.modifiers & Qt.ShiftModifier) !== 0)
                }
                onPositionChanged: function(mouse) {
                    graph.pointerMoved(mouse.x, mouse.y)
                    if (pressedButtons !== Qt.NoButton)
                        graph.updatePointer(mouse.x, mouse.y)
                }
                onReleased: function(mouse) {
                    if (mouse.button === Qt.RightButton)
                        graph.begin(
                            2, mouse.x, mouse.y,
                            (mouse.modifiers & Qt.ControlModifier) !== 0,
                            (mouse.modifiers & Qt.ShiftModifier) !== 0)
                    graph.endPointer()
                }
                onCanceled: {
                    graph.endPointer()
                }
                onExited: if (pressedButtons === Qt.NoButton) graph.pointerLeft()
                onWheel: function(wheel) {
                    wheel.accepted = graph.scroll(
                        wheel.pixelDelta, wheel.angleDelta,
                        wheel.x, wheel.y,
                        (wheel.modifiers & Qt.ControlModifier) !== 0)
                }
            }
        }
    }

    Item {
        id: interpolationAnchor
        width: 1
        height: 1
    }

    Item {
        id: textInterpolationAnchor
        width: 1
        height: 1
    }

    SearchMenu {
        id: interpolationMenu
        width: 280
        labels: root.interpolationLabels
        placeholderText: ComponentTranslations.text("Search interpolations")
        minimumListHeight: 180
        maximumListHeight: 240
        onActivated: function(index) { graph.setInterpolation(index) }
        onClosed: graph.forceActiveFocus(Qt.PopupFocusReason)
    }

    SearchMenu {
        id: textInterpolationMenu
        width: 280
        labels: root.textInterpolationLabels
        tooltips: root.textInterpolationTooltips
        placeholderText: ComponentTranslations.text("Search interpolations")
        minimumListHeight: 180
        maximumListHeight: 240
        onActivated: function(index) { graph.setTextInterpolation(index) }
        onClosed: graph.forceActiveFocus(Qt.PopupFocusReason)
    }
}
