pragma ComponentBehavior: Bound

import QtQuick
import dev.shrimply.components
import QtQuick.Layouts

InspectorProperty {
    id: root
    property real initialGraphValue: 0
    property real graphValue: initialGraphValue
    property real initialSecondValue: 0
    property real initialThirdValue: 0
    property bool paired: false
    property bool tripled: false
    property bool graphValueDrivesEditor: true
    property string externalClipboardMarker: ""
    property real firstValue: initialGraphValue
    property real secondValue: initialSecondValue
    property real thirdValue: initialThirdValue
    property string expressionValue: ""
    property string expressionOutput: ""
    property string expressionError: ""
    property var expressionDiagnosticProvider: null
    property var expressionCompletionProvider: null
    property int expressionDiagnosticDebounce: 250
    property int expressionCompletionDebounce: expressionDiagnosticDebounce
    property var textInterpolationLabels: []
    property var textInterpolationTooltips: []
    property var textInterpolationIndexForOwner: null
    readonly property int graphComponent: editRouter.activeComponent
    signal graphLoaded()
    signal graphEditFinished()
    property var graphLifecycleHandler: null
    signal graphPlaybackToggled()
    signal graphPlayheadChanged(int component, var numerator, var denominator)
    signal graphKeysMoved(int component, var oldTimes, var times, var values)
    signal graphKeysDeleted(int component, var times)
    signal graphKeyAdded(int component, var numerator, var denominator)
    signal graphCopyRequested(int component, var times)
    signal graphPasteRequested(int component, var numerator, var denominator)
    signal graphInterpolationChanged(int component, string ownerId, int interpolation)
    signal graphTextInterpolationRequested(int component, string ownerId, real x, real y)
    signal graphTextInterpolationChanged(int component, string ownerId, int interpolation)
    signal expressionEdited(string value)
    signal expressionCommitted(string value)
    signal baseValueEdited(real value)
    signal basePairEdited(real first, real second)
    signal baseTripleEdited(real first, real second, real third)
    signal keyframeValueEdited(real value)
    signal keyframePairEdited(real first, real second, int component)
    signal keyframeTripleEdited(real first, real second, real third, int component)
    signal graphReset(int component, real value)

    function editValue(value) {
        editRouter.setModes(root.keyframes, root.expression)
        editRouter.editValue(value)
    }

    function editPair(first, second, component) {
        editRouter.setModes(root.keyframes, root.expression)
        editRouter.editPair(first, second, component)
    }

    function editTriple(first, second, third, component) {
        editRouter.setModes(root.keyframes, root.expression)
        editRouter.editTriple(first, second, third, component)
    }

    function resetValue(value) {
        if (graphLoader.graph)
            graphLoader.graph.configureValue(value)
        root.graphValue = value
        root.graphReset(0, value)
    }

    function resetPair(first, second) {
        editRouter.selectComponent(0)
        editRouter.configurePair(first, second)
        root.firstValue = first
        root.secondValue = second
        if (graphLoader.graph && graphValueDrivesEditor)
            graphLoader.graph.configurePair(first, second, 0)
        root.graphReset(0, first)
        root.graphReset(1, second)
    }

    function resetTriple(first, second, third) {
        editRouter.selectTripleComponent(0)
        editRouter.configureTriple(first, second, third)
        root.firstValue = first
        root.secondValue = second
        root.thirdValue = third
        root.graphReset(0, first)
        root.graphReset(1, second)
        root.graphReset(2, third)
    }

    function replaceRawGraphComponent(component, pointTimes, pointValues, segments, staticValue) {
        if (graphLoader.graph)
            graphLoader.graph.replaceRawGraph(
                component, pointTimes, pointValues, segments, staticValue)
    }
    function replaceStepGraphComponent(component, times, values) {
        if (graphLoader.graph)
            graphLoader.graph.replaceStepGraph(component, times, values)
    }
    function reconcileStepGraphMoves(component, oldTimes, rawTimes, times) {
        if (graphLoader.graph)
            graphLoader.graph.reconcileStepMoves(component, oldTimes, rawTimes, times)
    }
    function rollbackStepGraphMoves(component, oldTimes, rawTimes) {
        if (graphLoader.graph)
            graphLoader.graph.rollbackStepMoves(component, oldTimes, rawTimes)
    }
    function replaceSpeedGraphComponent(component, keys, segments, staticValue) {
        if (graphLoader.graph)
            graphLoader.graph.replaceSpeedGraph(component, keys, segments, staticValue)
    }
    function setGraphRange(startNumerator, startDenominator, endNumerator, endDenominator) {
        if (graphLoader.graph)
            graphLoader.graph.setRange(
                startNumerator, startDenominator, endNumerator, endDenominator)
    }
    function setGraphFrameStep(numerator, denominator) {
        if (graphLoader.graph)
            graphLoader.graph.setFrameStep(numerator, denominator)
    }
    function setGraphPlayhead(numerator, denominator) {
        if (graphLoader.graph)
            graphLoader.graph.setPlayhead(numerator, denominator)
    }
    function setGraphSnapping(enabled, radiusPx) {
        if (graphLoader.graph)
            graphLoader.graph.setSnapping(enabled, radiusPx)
    }
    function setGraphExternalClipboard(enabled) {
        if (graphLoader.graph)
            graphLoader.graph.setExternalClipboard(enabled)
    }
    function setTextInterpolation(enabled) {
        if (graphLoader.graph)
            graphLoader.graph.setTextInterpolation(enabled)
    }
    function copyExternalClipboardMarker() {
        if (root.externalClipboardMarker.length === 0)
            return
        clipboardBridge.text = root.externalClipboardMarker
        clipboardBridge.selectAll()
        clipboardBridge.copy()
        clipboardBridge.deselect()
    }

    Component.onCompleted: {
        editRouter.setModes(root.keyframes, root.expression)
        if (root.tripled)
            root.resetTriple(initialGraphValue, initialSecondValue, initialThirdValue)
        else if (root.paired)
            root.resetPair(initialGraphValue, initialSecondValue)
        else if (!root.keyframes)
            root.resetValue(initialGraphValue)
        else
            root.graphValue = initialGraphValue
    }
    onKeyframesChanged: editRouter.setModes(keyframes, expression)
    onExpressionChanged: editRouter.setModes(keyframes, expression)
    onInitialGraphValueChanged: {
        if (tripled) {
            editRouter.configureTriple(
                initialGraphValue, initialSecondValue, initialThirdValue)
            firstValue = initialGraphValue
            secondValue = initialSecondValue
            thirdValue = initialThirdValue
        } else if (paired) {
            editRouter.configurePair(initialGraphValue, initialSecondValue)
            firstValue = initialGraphValue
            secondValue = initialSecondValue
        } else if (!keyframes && graphLoader.graph) {
            graphLoader.graph.configureValue(initialGraphValue)
        }
        graphValue = initialGraphValue
    }
    onInitialSecondValueChanged: if (paired || tripled) {
        if (tripled) {
            editRouter.configureTriple(
                initialGraphValue, initialSecondValue, initialThirdValue)
            firstValue = initialGraphValue
            secondValue = initialSecondValue
            thirdValue = initialThirdValue
        } else {
            editRouter.configurePair(initialGraphValue, initialSecondValue)
            firstValue = initialGraphValue
            secondValue = initialSecondValue
            if (graphLoader.graph && graphValueDrivesEditor)
                graphLoader.graph.configurePair(
                    initialGraphValue, initialSecondValue, graphComponent)
        }
    }
    onInitialThirdValueChanged: if (tripled) {
        editRouter.configureTriple(initialGraphValue, initialSecondValue, initialThirdValue)
        firstValue = initialGraphValue
        secondValue = initialSecondValue
        thirdValue = initialThirdValue
    }
    onGraphComponentChanged: if (graphLoader.graph) {
        if (tripled)
            editRouter.selectTripleComponent(graphComponent)
        else
            editRouter.selectComponent(graphComponent)
        if (graphValueDrivesEditor)
            graphLoader.graph.activateComponent(graphComponent)
    }

    LayeredPropertyBackend {
        id: editRouter
        onBaseEdited: function(value) { root.baseValueEdited(value) }
        onBasePairEdited: function(first, second) {
            root.firstValue = first
            root.secondValue = second
            root.basePairEdited(first, second)
        }
        onKeyframeEdited: function(value) {
            if (graphLoader.graph)
                graphLoader.graph.configureCurrentValue(value)
            root.keyframeValueEdited(value)
        }
        onKeyframePairEdited: function(first, second, component, firstChanged, secondChanged) {
            root.firstValue = first
            root.secondValue = second
            if (graphLoader.graph && root.graphValueDrivesEditor)
                graphLoader.graph.editPair(
                    first, second, component, firstChanged, secondChanged)
            root.keyframePairEdited(first, second, component)
        }
        onBaseTripleEdited: function(first, second, third) {
            root.firstValue = first
            root.secondValue = second
            root.thirdValue = third
            root.baseTripleEdited(first, second, third)
        }
        onKeyframeTripleEdited: function(first, second, third, component) {
            root.firstValue = first
            root.secondValue = second
            root.thirdValue = third
            root.keyframeTripleEdited(first, second, third, component)
        }
    }

    keyframeContent: Loader {
        id: graphLoader
        property bool loadedOnce: false
        readonly property FrameGraph graph: item as FrameGraph
        active: root.keyframes || loadedOnce
        Layout.fillWidth: true
        onLoaded: {
            loadedOnce = true
            root.graphLoaded()
        }
        sourceComponent: Component {
            FrameGraph {
                textInterpolationLabels: root.textInterpolationLabels
                textInterpolationTooltips: root.textInterpolationTooltips
                textInterpolationIndexForOwner: root.textInterpolationIndexForOwner
                onGraphValueChanged: {
                    root.graphValue = graphValue
                    if (!root.graphValueDrivesEditor)
                        return
                    if (root.graphComponent === 0)
                        root.firstValue = graphValue
                    else if (root.graphComponent === 1)
                        root.secondValue = graphValue
                    else
                        root.thirdValue = graphValue
                }
                onTogglePlayback: root.graphPlaybackToggled()
                onEditFinished: {
                    if (root.graphLifecycleHandler) root.graphLifecycleHandler(1, false)
                    else root.graphEditFinished()
                }
                onEditStarted: if (root.graphLifecycleHandler) root.graphLifecycleHandler(0, false)
                onEditCancelled: if (root.graphLifecycleHandler) root.graphLifecycleHandler(2, false)
                onHistoryRequested: function(redo) {
                    if (root.graphLifecycleHandler) root.graphLifecycleHandler(3, redo)
                }
                onPlayheadChanged: function(component, numerator, denominator) {
                    root.graphPlayheadChanged(component, numerator, denominator)
                }
                onKeysMoved: function(component, oldTimes, times, values) {
                    root.graphKeysMoved(component, oldTimes, times, values)
                }
                onKeysDeleted: function(component, times) {
                    root.graphKeysDeleted(component, times)
                }
                onKeyAdded: function(component, numerator, denominator, value) {
                    root.graphKeyAdded(component, numerator, denominator)
                }
                onCopyRequested: function(component, times) {
                    root.graphCopyRequested(component, times)
                }
                onPasteRequested: function(component, numerator, denominator) {
                    clipboardBridge.text = ""
                    clipboardBridge.paste()
                    if (root.externalClipboardMarker.length === 0
                            || clipboardBridge.text === root.externalClipboardMarker) {
                        root.graphPasteRequested(component, numerator, denominator)
                    }
                }
                onInterpolationChanged: function(component, ownerId, interpolation) {
                    root.graphInterpolationChanged(component, ownerId, interpolation)
                }
                onTextInterpolationRequested: function(component, ownerId, x, y) {
                    root.graphTextInterpolationRequested(component, ownerId, x, y)
                }
                onTextInterpolationChanged: function(component, ownerId, interpolation) {
                    root.graphTextInterpolationChanged(component, ownerId, interpolation)
                }
            }
        }
    }
    TextEdit {
        id: clipboardBridge
        visible: false
    }
    expressionContent: Loader {
        active: root.expression
        Layout.fillWidth: true
        sourceComponent: Component {
            ExpressionEditor {
                value: root.expressionValue
                output: root.expressionOutput
                error: root.expressionError
                diagnosticProvider: root.expressionDiagnosticProvider
                completionProvider: root.expressionCompletionProvider
                diagnosticDebounce: root.expressionDiagnosticDebounce
                completionDebounce: root.expressionCompletionDebounce
                onEdited: function(value) { root.expressionEdited(value) }
                onCommitted: function(value) { root.expressionCommitted(value) }
            }
        }
    }
}
