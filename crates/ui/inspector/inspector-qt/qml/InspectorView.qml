pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.shrimply.components
import dev.shrimply.inspector

Item {
    id: root
    implicitWidth: backend.minimumWidth()
    readonly property int expressionDiagnosticDebounce:
        backend.expressionDiagnosticDebounce()
    readonly property int expressionCompletionDebounce:
        backend.expressionCompletionDebounce()
    signal error(string body)
    signal confirmation(string body)

    function refreshed(value) {
        const revision = backend.revision
        return revision >= 0 ? value : value
    }

    function expressionDiagnostic(source) {
        return backend.expressionDiagnostic(source)
    }

    function expressionCompletion(category, item, control, source, cursor, automatic) {
        return backend.controlExpressionCompletion(
            category, item, control, source, cursor, automatic)
    }

    function bundledCategoryIcon(name) {
        switch (name) {
        case "blend-tool-symbolic":
        case "info-outline-symbolic":
        case "playback-speed-symbolic":
        case "sliders-horizontal-symbolic":
        case "sound-symbolic":
        case "speedometer-symbolic":
            return "qrc:/qt/qml/dev/shrimply/components/icons/" + name + ".svg"
        default:
            return ""
        }
    }

    InspectorBackend {
        id: backend
        onShowError: function(body) { root.error(body) }
        onShowConfirmation: function(body) { root.confirmation(body) }
        onOpenPath: function(url) { Qt.openUrlExternally(url) }
    }

    Timer {
        interval: 50
        repeat: true
        running: true
        triggeredOnStart: true
        onTriggered: {
            if (backend.targetChangePending())
                root.forceActiveFocus(Qt.OtherFocusReason)
            backend.poll(inspectorScroll.contentItem.contentY)
        }
    }

    Label {
        anchors.centerIn: parent
        visible: !backend.ready
        text: qsTr("Loading inspector…")
        opacity: 0.65
    }

    ColumnLayout {
        anchors.fill: parent
        visible: backend.ready
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: 16
            Layout.rightMargin: 16
            Layout.topMargin: 12
            Layout.bottomMargin: 8
            spacing: 0

            ButtonGroup { id: categories }
            Repeater {
                model: {
                    const revision = backend.documentRevision
                    return revision >= 0 ? backend.categoryKeys() : []
                }
                Button {
                    required property int index
                    readonly property string categoryIconName: root.refreshed(
                        backend.categoryIcon(index))
                    readonly property string categoryIconSource: root.bundledCategoryIcon(
                        categoryIconName)
                    Layout.fillWidth: true
                    checkable: true
                    ButtonGroup.group: categories
                    checked: index === backend.activeCategory
                    text: root.refreshed(backend.categoryLabel(index))
                    icon.name: categoryIconSource.length === 0 ? categoryIconName : ""
                    icon.source: categoryIconSource
                    display: AbstractButton.TextBesideIcon
                    onClicked: backend.activateCategory(index)
                }
            }
        }

        ScrollView {
            id: inspectorScroll
            Layout.fillWidth: true
            Layout.fillHeight: true
            leftPadding: 16
            rightPadding: 16
            topPadding: 4
            bottomPadding: 12
            contentWidth: availableWidth

            Connections {
                target: backend
                function onDocumentRevisionChanged() {
                    Qt.callLater(function() {
                        inspectorScroll.contentItem.contentY = backend.scrollPosition
                        inspectorScroll.contentItem.returnToBounds()
                    })
                }
            }

            Column {
                width: inspectorScroll.availableWidth
                spacing: 8

                Repeater {
                    model: {
                        const revision = backend.documentRevision
                        return revision >= 0 ? backend.itemKeys(backend.activeCategory) : []
                    }

                    Loader {
                        id: itemLoader
                        required property int index
                        readonly property int categoryIndex: backend.activeCategory
                        readonly property int revision: backend.revision
                        readonly property bool card: root.refreshed(
                            backend.itemIsCard(categoryIndex, index))
                        readonly property string identity: card ? root.refreshed(
                            backend.itemIdentity(categoryIndex, index)) : ""
                        width: parent.width
                        sourceComponent: card ? cardComponent : flatComponent

                        Component {
                            id: flatComponent
                            Loader {
                                width: itemLoader.width
                                sourceComponent: controlList
                                onLoaded: {
                                    item.categoryIndex = Qt.binding(function() {
                                        return itemLoader.categoryIndex
                                    })
                                    item.itemIndex = Qt.binding(function() {
                                        return itemLoader.index
                                    })
                                }
                            }
                        }

                        Component {
                            id: cardComponent
                            InspectorCard {
                                id: card
                                readonly property string identity: itemLoader.identity
                                property string loadedBodyIdentity: ""
                                autoToggleExpansion: false
                                accented: root.refreshed(backend.itemFocused(
                                    itemLoader.categoryIndex, itemLoader.index))
                                title: root.refreshed(backend.itemTitle(
                                    itemLoader.categoryIndex, itemLoader.index))
                                resetVisible: root.refreshed(backend.itemResetAvailable(
                                    itemLoader.categoryIndex, itemLoader.index))
                                expanded: root.refreshed(backend.itemExpanded(
                                    itemLoader.categoryIndex, itemLoader.index))
                                onExpandedRequested: function(expanded) {
                                    backend.setItemExpanded(itemLoader.categoryIndex,
                                        itemLoader.index, expanded)
                                }
                                onExpandedChanged: if (expanded)
                                    loadedBodyIdentity = identity
                                onIdentityChanged: {
                                    loadedBodyIdentity = ""
                                    const pendingIdentity = identity
                                    Qt.callLater(function() {
                                        if (card.identity === pendingIdentity && card.expanded)
                                            card.loadedBodyIdentity = pendingIdentity
                                    })
                                }
                                onResetRequested: backend.resetItem(
                                    itemLoader.categoryIndex, itemLoader.index)
                                onFocusRequested: function(focusedItem, bodyClicked) {
                                    focusPreview(focusedItem, bodyClicked)
                                }

                                function focusPreview(focusedItem, bodyClicked) {
                                    if (!backend.itemFocusAvailable(
                                            itemLoader.categoryIndex, itemLoader.index))
                                        return
                                    if (!bodyClicked) {
                                        backend.focusItem(
                                            itemLoader.categoryIndex, itemLoader.index)
                                        return
                                    }
                                    while (focusedItem && focusedItem !== card) {
                                        if (focusedItem.inspectorControlIndex !== undefined) {
                                            backend.focusControl(itemLoader.categoryIndex,
                                                itemLoader.index,
                                                focusedItem.inspectorControlIndex)
                                            return
                                        }
                                        focusedItem = focusedItem.parent
                                    }
                                    backend.focusItemBody(
                                        itemLoader.categoryIndex, itemLoader.index)
                                }

                                beforeReset: RowLayout {
                                    spacing: 4
                                    Switch {
                                        visible: root.refreshed(backend.itemHasToggle(
                                            itemLoader.categoryIndex, itemLoader.index))
                                        checked: root.refreshed(backend.itemToggleActive(
                                            itemLoader.categoryIndex, itemLoader.index))
                                        ToolTip.visible: hovered
                                        ToolTip.text: root.refreshed(backend.itemToggleTooltip(
                                            itemLoader.categoryIndex, itemLoader.index))
                                        onToggled: backend.setItemToggle(
                                            itemLoader.categoryIndex, itemLoader.index, checked)
                                    }
                                    ToolButton {
                                        visible: root.refreshed(backend.itemHasButtonToggle(
                                            itemLoader.categoryIndex, itemLoader.index))
                                        checkable: true
                                        checked: root.refreshed(backend.itemButtonToggleActive(
                                            itemLoader.categoryIndex, itemLoader.index))
                                        icon.name: root.refreshed(backend.itemButtonToggleIcon(
                                            itemLoader.categoryIndex, itemLoader.index))
                                        ToolTip.visible: hovered
                                        ToolTip.text: root.refreshed(
                                            backend.itemButtonToggleTooltip(
                                                itemLoader.categoryIndex, itemLoader.index))
                                        onToggled: backend.setItemButtonToggle(
                                            itemLoader.categoryIndex, itemLoader.index, checked)
                                    }
                                }

                                afterReset: RowLayout {
                                    spacing: 4
                                    Repeater {
                                        model: root.refreshed(backend.itemActionCount(
                                            itemLoader.categoryIndex, itemLoader.index))
                                        ToolButton {
                                            required property int index
                                            enabled: root.refreshed(
                                                backend.itemActionSensitive(
                                                    itemLoader.categoryIndex,
                                                    itemLoader.index, index))
                                            icon.name: root.refreshed(backend.itemActionIcon(
                                                itemLoader.categoryIndex,
                                                itemLoader.index, index))
                                            ToolTip.visible: hovered
                                            ToolTip.text: root.refreshed(
                                                backend.itemActionTooltip(
                                                    itemLoader.categoryIndex,
                                                    itemLoader.index, index))
                                            onClicked: backend.triggerItemAction(
                                                itemLoader.categoryIndex, itemLoader.index, index)
                                        }
                                    }
                                }

                                Loader {
                                    Layout.fillWidth: true
                                    sourceComponent: card.identity.length > 0
                                            && card.loadedBodyIdentity === card.identity
                                        ? controlList : undefined
                                    onLoaded: {
                                        item.categoryIndex = Qt.binding(function() {
                                            return itemLoader.categoryIndex
                                        })
                                        item.itemIndex = Qt.binding(function() {
                                            return itemLoader.index
                                        })
                                    }
                                }
                                Component.onCompleted: if (expanded)
                                    loadedBodyIdentity = identity
                            }
                        }
                    }
                }
            }
        }
    }

    Component {
        id: controlList
        ColumnLayout {
            id: controlColumn
            property int categoryIndex: 0
            property int itemIndex: 0
            spacing: 8

            Repeater {
                model: root.refreshed(
                    backend.controlCount(parent.categoryIndex, parent.itemIndex))
                Loader {
                    id: controlLoader
                    required property int index
                    readonly property int inspectorControlIndex: index
                    readonly property int categoryIndex: controlColumn.categoryIndex
                    readonly property int itemIndex: controlColumn.itemIndex
                    readonly property int kind: root.refreshed(backend.controlKind(
                        categoryIndex, itemIndex, index))
                    readonly property string label: root.refreshed(backend.controlLabel(
                        categoryIndex, itemIndex, index))
                    readonly property bool transformLive: root.refreshed(
                        backend.controlTransformLive(categoryIndex, itemIndex, index))
                    readonly property int rowRole: root.refreshed(
                        backend.controlRowRole(categoryIndex, itemIndex, index))
                    readonly property bool inlineRowPrimary:
                        rowRole === InspectorBackend.Primary
                    readonly property bool inlineRowTail:
                        rowRole === InspectorBackend.Auxiliary
                        || rowRole === InspectorBackend.TrailingAction
                    readonly property string value: {
                        const revision = kind === InspectorBackend.AudioCache
                            || kind === InspectorBackend.VisualCache
                            ? backend.documentRevision + backend.cacheRevision
                            : backend.revision
                        return revision >= 0 ? backend.controlValue(
                            categoryIndex, itemIndex, index) : ""
                    }
                    readonly property bool editable: root.refreshed(backend.controlEditable(
                        categoryIndex, itemIndex, index))
                    readonly property bool sensitive: {
                        const revision = kind === InspectorBackend.Analysis
                            ? backend.documentRevision + backend.analysisRevision
                            : kind === InspectorBackend.AudioCache
                            || kind === InspectorBackend.AudioCachePreset
                            || kind === InspectorBackend.VisualCache
                            || kind === InspectorBackend.VisualCacheQuality
                            ? backend.documentRevision + backend.cacheRevision
                            : backend.revision
                        return revision >= 0 && backend.controlSensitive(
                            categoryIndex, itemIndex, index)
                    }
                    visible: !inlineRowTail && root.refreshed(
                        backend.controlVisible(categoryIndex, itemIndex, index))
                    enabled: kind === InspectorBackend.ReadOnly
                        || kind === InspectorBackend.Selector
                        || kind === InspectorBackend.Performance
                        || kind === InspectorBackend.InfoHeading
                        || kind === InspectorBackend.InfoArtwork
                        || kind === InspectorBackend.FileLocation
                        || kind === InspectorBackend.InfoLoading
                        || kind === InspectorBackend.Action
                        || kind === InspectorBackend.Analysis
                        ? sensitive : editable && sensitive
                    Layout.fillWidth: true
                    sourceComponent: inlineRowTail ? null
                        : kind === InspectorBackend.Boolean ? booleanControl
                        : kind === InspectorBackend.Number ? numberControl
                        : kind === InspectorBackend.Fraction ? fractionControl
                        : kind === InspectorBackend.Text ? textControl
                        : kind === InspectorBackend.MultilineText ? multilineControl
                        : kind === InspectorBackend.ReadOnly ? readOnlyControl
                        : kind === InspectorBackend.Color ? colorControl
                        : kind === InspectorBackend.LayeredColor ? layeredColorControl
                        : kind === InspectorBackend.LayeredText ? layeredTextControl
                        : kind === InspectorBackend.LayeredDrawing ? layeredDrawingControl
                        : kind === InspectorBackend.FontFamilies ? fontFamiliesControl
                        : kind === InspectorBackend.Selector
                            || kind === InspectorBackend.AudioCachePreset
                            || kind === InspectorBackend.VisualCacheQuality
                            ? selectorControl
                        : kind === InspectorBackend.Vector2 ? vector2Control
                        : kind === InspectorBackend.Vector3 ? vector3Control
                        : kind === InspectorBackend.LayeredNumber ? layeredNumberControl
                        : kind === InspectorBackend.LayeredVector2 ? layeredVector2Control
                        : kind === InspectorBackend.LayeredVector3 ? layeredVector3Control
                        : kind === InspectorBackend.ProjectSettings ? projectSettingsControl
                        : kind === InspectorBackend.Performance ? performanceControl
                        : kind === InspectorBackend.LayeredBoolean ? layeredBooleanControl
                        : kind === InspectorBackend.LayeredSelector ? layeredSelectorControl
                        : kind === InspectorBackend.AudioCache ? audioCacheControl
                        : kind === InspectorBackend.VisualCache ? visualCacheControl
                        : kind === InspectorBackend.Analysis ? analysisControl
                        : kind === InspectorBackend.ModifierMenu ? audioModifierMenuControl
                        : kind === InspectorBackend.TtsEditor ? ttsEditorControl
                        : kind === InspectorBackend.BeatDetection ? beatDetectionControl
                        : kind === InspectorBackend.InfoHeading ? infoHeadingControl
                        : kind === InspectorBackend.InfoArtwork ? infoArtworkControl
                        : kind === InspectorBackend.FileLocation ? fileLocationControl
                        : kind === InspectorBackend.InfoLoading ? infoLoadingControl
                        : actionControl

                    function component(componentIndex) {
                        const revision = kind === InspectorBackend.AudioCache
                            || kind === InspectorBackend.VisualCache
                            ? backend.documentRevision + backend.cacheRevision
                            : backend.revision
                        return revision >= 0 ? backend.controlComponent(
                            categoryIndex, itemIndex, index, componentIndex) : 0
                    }
                    function componentText(componentIndex) {
                        return root.refreshed(backend.controlComponentText(
                            categoryIndex, itemIndex, index, componentIndex))
                    }
                    function tooltip() {
                        const revision = kind === InspectorBackend.AudioCache
                            || kind === InspectorBackend.VisualCache
                            ? backend.documentRevision + backend.cacheRevision
                            : backend.revision
                        return revision >= 0 ? backend.controlTooltip(
                            categoryIndex, itemIndex, index) : ""
                    }
                    function minimum() {
                        return root.refreshed(backend.controlMinimum(
                            categoryIndex, itemIndex, index))
                    }
                    function maximum() {
                        return root.refreshed(backend.controlMaximum(
                            categoryIndex, itemIndex, index))
                    }
                    function dragStep() {
                        return root.refreshed(backend.controlDragStep(
                            categoryIndex, itemIndex, index))
                    }
                    function digits() {
                        return root.refreshed(backend.controlDigits(
                            categoryIndex, itemIndex, index))
                    }
                    function unit() {
                        return root.refreshed(backend.controlUnit(
                            categoryIndex, itemIndex, index))
                    }
                    function widthCharacters() {
                        return root.refreshed(backend.controlWidthCharacters(
                            categoryIndex, itemIndex, index))
                    }
                    function prefixIcon() {
                        return root.refreshed(backend.controlPrefixIcon(
                            categoryIndex, itemIndex, index))
                    }
                    function prefixIconRotates() {
                        return root.refreshed(backend.controlPrefixIconRotates(
                            categoryIndex, itemIndex, index))
                    }
                    function prefixIconRotationOffset() {
                        return root.refreshed(backend.controlPrefixIconRotationOffset(
                            categoryIndex, itemIndex, index))
                    }
                    function prefix(componentIndex) {
                        return root.refreshed(backend.controlPrefix(
                            categoryIndex, itemIndex, index, componentIndex))
                    }
                    function locked() {
                        return root.refreshed(backend.controlLock(
                            categoryIndex, itemIndex, index))
                    }
                    function edit(next) {
                        backend.setControlValue(categoryIndex, itemIndex, index, String(next))
                    }
                    function commit() {
                        backend.commitControl(categoryIndex, itemIndex, index)
                    }

                    Component {
                        id: booleanControl
                        SwitchRow {
                            label: controlLoader.label
                            subtitle: root.refreshed(backend.controlSubtitle(
                                controlLoader.categoryIndex,
                                controlLoader.itemIndex, controlLoader.index))
                            tooltip: controlLoader.tooltip()
                            active: controlLoader.value === "true"
                            onToggled: function(active) { controlLoader.edit(active) }
                        }
                    }
                    Component {
                        id: beatDetectionControl
                        SwitchRow {
                            id: beatRow
                            property int polls: 0
                            property bool observedBusy: false
                            label: controlLoader.label
                            subtitle: root.refreshed(backend.controlSubtitle(
                                controlLoader.categoryIndex,
                                controlLoader.itemIndex, controlLoader.index))
                            active: controlLoader.value === "true"
                            onActiveChanged: if (!active) {
                                busy = false
                                observedBusy = false
                                polls = 0
                            }
                            onToggled: function(active) {
                                polls = 0
                                observedBusy = false
                                controlLoader.edit(active)
                            }
                            Timer {
                                interval: 33
                                repeat: true
                                running: beatRow.active
                                    && (beatRow.busy || beatRow.polls < 60)
                                onTriggered: {
                                    const loading = backend.controlBusy(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index)
                                    beatRow.busy = loading
                                    beatRow.observedBusy = beatRow.observedBusy || loading
                                    beatRow.polls += 1
                                    if (beatRow.observedBusy && !loading)
                                        beatRow.polls = 60
                                }
                            }
                        }
                    }
                    Component {
                        id: ttsEditorControl
                        TtsEditor {
                            audioId: controlLoader.value
                            documentRevision: backend.documentRevision
                            onError: function(message) { root.error(message) }
                            onOpenPath: function(url) { Qt.openUrlExternally(url) }
                        }
                    }
                    Component {
                        id: numberControl
                        ControlRow {
                            label: controlLoader.label
                            NumberPicker {
                                value: Number(controlLoader.value)
                                minimum: controlLoader.minimum()
                                maximum: controlLoader.maximum()
                                dragStep: controlLoader.dragStep()
                                digits: controlLoader.digits()
                                unitName: controlLoader.unit()
                                widthCharacters: root.refreshed(
                                    backend.controlWidthCharacters(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex,
                                        controlLoader.index))
                                onEdited: function(value) { controlLoader.edit(value) }
                                onCommitted: function(value) { controlLoader.commit() }
                            }
                        }
                    }
                    Component {
                        id: fractionControl
                        ControlRow {
                            label: controlLoader.label
                            NumberPicker {
                                value: Number(controlLoader.value)
                                fractionNumerator: controlLoader.componentText(0)
                                fractionDenominator: controlLoader.componentText(1)
                                minimum: controlLoader.minimum()
                                maximum: controlLoader.maximum()
                                dragStep: controlLoader.dragStep()
                                digits: controlLoader.digits()
                                unitName: controlLoader.unit()
                                widthCharacters: root.refreshed(
                                    backend.controlWidthCharacters(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex,
                                        controlLoader.index))
                                onFractionEdited: function(numerator, denominator) {
                                    backend.setControlFraction(controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index,
                                        numerator, denominator)
                                }
                                onCommitted: function(value) { controlLoader.commit() }
                            }
                        }
                    }
                    Component {
                        id: textControl
                        ControlRow {
                            label: controlLoader.label
                            SingleLineTextInput {
                                value: controlLoader.value
                                onEdited: function(value) { controlLoader.edit(value) }
                                onCommitted: function(value) { controlLoader.commit() }
                            }
                        }
                    }
                    Component {
                        id: multilineControl
                        ControlRow {
                            label: controlLoader.label
                            HoverHandler { id: multilineHover }
                            ToolTip.visible: multilineHover.hovered
                                && controlLoader.tooltip().length > 0
                            ToolTip.text: controlLoader.tooltip()
                            MultilineTextInput {
                                value: controlLoader.value
                                onEdited: function(value) { controlLoader.edit(value) }
                                onCommitted: function(value) { controlLoader.commit() }
                            }
                        }
                    }
                    Component {
                        id: layeredTextControl
                        InspectorTextGraphProperty {
                            id: propertyGraph
                            expressionDiagnosticProvider: function(source) {
                                return root.expressionDiagnostic(source)
                            }
                            expressionDiagnosticDebounce: root.expressionDiagnosticDebounce
                            expressionCompletionDebounce: root.expressionCompletionDebounce
                            expressionCompletionProvider: function(source, cursor, automatic) {
                                return root.expressionCompletion(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, source, cursor,
                                    automatic)
                            }
                            readonly property int graphDocumentRevision: backend.documentRevision
                            readonly property int graphModelRevision: backend.graphRevision
                            readonly property int graphPlayheadRevision:
                                backend.playheadRevision
                            readonly property int textRevision:
                                backend.documentRevision + backend.graphRevision
                                    + backend.playheadRevision
                            readonly property var expressionResult: {
                                const documentRevision = backend.documentRevision
                                const expressionRevision = backend.expressionRevision
                                const playheadRevision = backend.playheadRevision
                                return propertyGraph.expression
                                        && documentRevision + expressionRevision
                                        + playheadRevision >= 0
                                    ? backend.controlExpressionResult(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index)
                                    : []
                            }
                            label: controlLoader.label
                            textValue: textRevision >= 0 ? backend.controlValue(
                                controlLoader.categoryIndex,
                                controlLoader.itemIndex, controlLoader.index) : ""
                            keyframes: root.refreshed(backend.controlKeyframes(
                                controlLoader.categoryIndex,
                                controlLoader.itemIndex, controlLoader.index))
                            expression: root.refreshed(backend.controlExpression(
                                controlLoader.categoryIndex,
                                controlLoader.itemIndex, controlLoader.index))
                            expressionValue: root.refreshed(
                                backend.controlExpressionSource(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index))
                            expressionOutput: expressionResult.length > 0
                                ? expressionResult[0] : ""
                            expressionError: expressionResult.length > 1
                                ? expressionResult[1] : ""
                            externalClipboardMarker: backend.keyframeClipboardMarker()
                            textInterpolationLabels: backend.textInterpolationLabels()
                            textInterpolationTooltips: backend.textInterpolationTooltips()
                            textInterpolationIndexForOwner: function(ownerId) {
                                return backend.controlGraphTextInterpolation(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    ownerId)
                            }

                            function updateGraphPlayhead() {
                                if (!propertyGraph.keyframes)
                                    return
                                const playhead = backend.controlGraphPlayhead(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                if (playhead.length === 2)
                                    propertyGraph.setGraphPlayhead(playhead[0], playhead[1])
                            }
                            function configureGraph() {
                                if (!propertyGraph.keyframes)
                                    return
                                const times = backend.controlGraphPointTimes(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                const segments = backend.controlGraphSegments(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                const timing = backend.controlGraphTiming(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                if (timing.length !== 6)
                                    return
                                propertyGraph.replaceSpeedGraphComponent(
                                    0, times, segments, 0)
                                propertyGraph.setGraphRange(
                                    timing[0], timing[1], timing[2], timing[3])
                                propertyGraph.setGraphFrameStep(timing[4], timing[5])
                                propertyGraph.setGraphSnapping(
                                    backend.keyframeSnappingEnabled(),
                                    backend.keyframeSnappingRadius())
                                propertyGraph.setGraphExternalClipboard(true)
                                propertyGraph.setTextInterpolation(true)
                                propertyGraph.updateGraphPlayhead()
                            }

                            Component.onCompleted: configureGraph()
                            onGraphLoaded: configureGraph()
                            onGraphDocumentRevisionChanged: configureGraph()
                            onGraphModelRevisionChanged: configureGraph()
                            onGraphPlayheadRevisionChanged: updateGraphPlayhead()
                            onTextEdited: function(value) { controlLoader.edit(value) }
                            onTextCommitted: function(value) { controlLoader.commit() }
                            onGraphPlaybackToggled: backend.toggleControlGraphPlayback()
                            onGraphPlayheadChanged: function(component, numerator, denominator) {
                                if (component === 0)
                                    backend.seekControlGraph(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index,
                                        numerator, denominator)
                            }
                            onGraphKeysMoved: function(component, oldTimes, times, values) {
                                if (component === 0)
                                    backend.moveControlGraphKeys(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index,
                                        oldTimes, times, values)
                            }
                            onGraphEditFinished: controlLoader.commit()
                            onGraphKeysDeleted: function(component, times) {
                                if (component === 0)
                                    backend.deleteControlGraphKeys(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index, times)
                            }
                            onGraphKeyAdded: function(component, numerator, denominator) {
                                if (component === 0)
                                    backend.addControlGraphKey(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index,
                                        numerator, denominator)
                            }
                            onGraphCopyRequested: function(component, times) {
                                if (component === 0 && backend.copyControlGraphKeys(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index, times)) {
                                    propertyGraph.copyExternalClipboardMarker()
                                }
                            }
                            onGraphPasteRequested: function(component, numerator, denominator) {
                                if (component === 0)
                                    backend.pasteControlGraphKeys(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index,
                                        numerator, denominator)
                            }
                            onGraphInterpolationChanged: function(component, ownerId, interpolation) {
                                if (component === 0)
                                    backend.setControlGraphInterpolation(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index,
                                        ownerId, interpolation)
                            }
                            onTextInterpolationChanged: function(ownerId, interpolation) {
                                backend.setControlGraphTextInterpolation(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    ownerId, interpolation)
                            }
                            onKeyframesToggled: function(enabled) {
                                backend.setControlKeyframes(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, enabled)
                            }
                            onExpressionToggled: function(enabled) {
                                backend.setControlExpression(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, enabled)
                            }
                            onExpressionEdited: function(value) {
                                backend.setControlExpressionSource(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, value)
                            }
                            onExpressionCommitted: controlLoader.commit()
                        }
                    }
                    Component {
                        id: readOnlyControl
                        ControlRow {
                            label: controlLoader.label
                            RowLayout {
                                BusyIndicator {
                                    visible: running
                                    readonly property int busyRevision:
                                        backend.revision + backend.analysisRevision
                                    running: busyRevision >= 0 && backend.controlBusy(
                                            controlLoader.categoryIndex,
                                            controlLoader.itemIndex, controlLoader.index)
                                    implicitWidth: 18
                                    implicitHeight: 18
                                }
                                ReadOnlyField { text: controlLoader.value }
                            }
                        }
                    }
                    Component {
                        id: fontFamiliesControl
                        ControlRow {
                            label: controlLoader.label
                            FontFamilyList {
                                value: controlLoader.value
                                browserBackend: backend
                                categoryIndex: controlLoader.categoryIndex
                                itemIndex: controlLoader.itemIndex
                                controlIndex: controlLoader.index
                                onEdited: function(value) { controlLoader.edit(value) }
                            }
                        }
                    }
                    Component {
                        id: infoHeadingControl
                        Label {
                            text: controlLoader.label
                            font.bold: true
                            Layout.fillWidth: true
                            Layout.topMargin: 8
                            wrapMode: Text.Wrap
                        }
                    }
                    Component {
                        id: infoArtworkControl
                        ColumnLayout {
                            spacing: 6
                            Label {
                                text: controlLoader.label
                                font.bold: true
                                Layout.fillWidth: true
                            }
                            Image {
                                source: controlLoader.value
                                asynchronous: true
                                cache: true
                                fillMode: Image.PreserveAspectFit
                                sourceSize.height: 220
                                Layout.fillWidth: true
                                Layout.preferredHeight: 220
                            }
                        }
                    }
                    Component {
                        id: fileLocationControl
                        ControlRow {
                            label: controlLoader.label
                            RowLayout {
                                spacing: 6
                                ReadOnlyField {
                                    text: controlLoader.value
                                    Layout.fillWidth: true
                                }
                                ToolButton {
                                    icon.name: "folder-open-symbolic"
                                    ToolTip.visible: hovered
                                    ToolTip.text: controlLoader.tooltip()
                                    onClicked: backend.showControlPath(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex,
                                        controlLoader.index)
                                }
                            }
                        }
                    }
                    Component {
                        id: infoLoadingControl
                        ControlRow {
                            label: controlLoader.label
                            RowLayout {
                                spacing: 6
                                BusyIndicator {
                                    running: true
                                    implicitWidth: 18
                                    implicitHeight: 18
                                }
                                Label { text: controlLoader.value }
                            }
                        }
                    }
                    Component {
                        id: colorControl
                        ControlRow {
                            label: controlLoader.label
                            ColorPicker {
                                title: controlLoader.tooltip().length > 0
                                    ? controlLoader.tooltip() : controlLoader.label
                                color: Qt.rgba(
                                    controlLoader.component(0) / 255,
                                    controlLoader.component(1) / 255,
                                    controlLoader.component(2) / 255,
                                    controlLoader.component(3) / 255)
                                withAlpha: root.refreshed(backend.controlWithAlpha(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex,
                                    controlLoader.index))
                                onSelected: function(color) {
                                    backend.setControlColor(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex,
                                        controlLoader.index,
                                        color.r, color.g, color.b, color.a)
                                }
                                onScreenColorFailed: function(message) { root.error(message) }
                            }
                        }
                    }
                    Component {
                        id: layeredColorControl
                        InspectorGraphProperty {
                            id: propertyGraph
                            expressionDiagnosticProvider: function(source) {
                                return root.expressionDiagnostic(source)
                            }
                            expressionDiagnosticDebounce: root.expressionDiagnosticDebounce
                            expressionCompletionDebounce: root.expressionCompletionDebounce
                            expressionCompletionProvider: function(source, cursor, automatic) {
                                return root.expressionCompletion(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, source, cursor,
                                    automatic)
                            }
                            readonly property int graphDocumentRevision: backend.documentRevision
                            readonly property int graphModelRevision: backend.graphRevision
                            readonly property int graphPlayheadRevision:
                                backend.playheadRevision
                            readonly property int colorRevision:
                                backend.documentRevision + backend.graphRevision
                                    + backend.playheadRevision
                            readonly property var colorChannels: {
                                const revision = propertyGraph.colorRevision
                                return revision >= 0 ? backend.controlColor(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index) : []
                            }
                            readonly property var expressionResult: {
                                const documentRevision = backend.documentRevision
                                const expressionRevision = backend.expressionRevision
                                const playheadRevision = backend.playheadRevision
                                return propertyGraph.expression
                                        && documentRevision + expressionRevision
                                        + playheadRevision >= 0
                                    ? backend.controlExpressionResult(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index)
                                    : []
                            }
                            label: controlLoader.label
                            graphValueDrivesEditor: false
                            initialGraphValue: 0
                            keyframes: root.refreshed(backend.controlKeyframes(
                                controlLoader.categoryIndex,
                                controlLoader.itemIndex, controlLoader.index))
                            expression: root.refreshed(backend.controlExpression(
                                controlLoader.categoryIndex,
                                controlLoader.itemIndex, controlLoader.index))
                            expressionValue: root.refreshed(
                                backend.controlExpressionSource(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index))
                            expressionOutput: expressionResult.length > 0
                                ? expressionResult[0] : ""
                            expressionError: expressionResult.length > 1
                                ? expressionResult[1] : ""
                            externalClipboardMarker: backend.keyframeClipboardMarker()

                            function updateGraphPlayhead() {
                                if (!propertyGraph.keyframes)
                                    return
                                const playhead = backend.controlGraphPlayhead(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                if (playhead.length === 2)
                                    propertyGraph.setGraphPlayhead(playhead[0], playhead[1])
                            }
                            function configureGraph() {
                                if (!propertyGraph.keyframes)
                                    return
                                const times = backend.controlGraphPointTimes(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                const segments = backend.controlGraphSegments(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                const timing = backend.controlGraphTiming(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                if (timing.length !== 6)
                                    return
                                propertyGraph.replaceSpeedGraphComponent(
                                    0, times, segments, 0)
                                propertyGraph.setGraphRange(
                                    timing[0], timing[1], timing[2], timing[3])
                                propertyGraph.setGraphFrameStep(timing[4], timing[5])
                                propertyGraph.setGraphSnapping(
                                    backend.keyframeSnappingEnabled(),
                                    backend.keyframeSnappingRadius())
                                propertyGraph.setGraphExternalClipboard(true)
                                propertyGraph.updateGraphPlayhead()
                            }

                            Component.onCompleted: configureGraph()
                            onGraphLoaded: configureGraph()
                            onGraphDocumentRevisionChanged: configureGraph()
                            onGraphModelRevisionChanged: configureGraph()
                            onGraphPlayheadRevisionChanged: updateGraphPlayhead()

                            RowLayout {
                                ColorPicker {
                                    color: propertyGraph.colorChannels.length === 4 ? Qt.rgba(
                                        Number(propertyGraph.colorChannels[0]) / 255,
                                        Number(propertyGraph.colorChannels[1]) / 255,
                                        Number(propertyGraph.colorChannels[2]) / 255,
                                        Number(propertyGraph.colorChannels[3]) / 255) : "transparent"
                                    title: controlLoader.label
                                    withAlpha: root.refreshed(backend.controlWithAlpha(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index))
                                    Layout.fillWidth: true
                                    onSelected: function(color) {
                                        backend.setControlColor(
                                            controlLoader.categoryIndex,
                                            controlLoader.itemIndex, controlLoader.index,
                                            color.r, color.g, color.b, color.a)
                                        controlLoader.commit()
                                    }
                                    onScreenColorFailed: function(message) { root.error(message) }
                                }
                                ToolButton {
                                    visible: root.refreshed(backend.controlHasAction(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index))
                                    enabled: root.refreshed(backend.controlActionSensitive(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index))
                                    flat: true
                                    icon.name: controlLoader.prefixIcon()
                                    display: AbstractButton.IconOnly
                                    ToolTip.visible: hovered && controlLoader.tooltip().length > 0
                                    ToolTip.text: controlLoader.tooltip()
                                    onClicked: backend.triggerControlAction(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index)
                                }
                            }

                            onGraphPlaybackToggled: backend.toggleControlGraphPlayback()
                            onGraphPlayheadChanged: function(component, numerator, denominator) {
                                if (component === 0)
                                    backend.seekControlGraph(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index,
                                        numerator, denominator)
                            }
                            onGraphKeysMoved: function(component, oldTimes, times, values) {
                                if (component === 0)
                                    backend.moveControlGraphKeys(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index,
                                        oldTimes, times, values)
                            }
                            onGraphEditFinished: controlLoader.commit()
                            onGraphKeysDeleted: function(component, times) {
                                if (component === 0)
                                    backend.deleteControlGraphKeys(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index, times)
                            }
                            onGraphKeyAdded: function(component, numerator, denominator) {
                                if (component === 0)
                                    backend.addControlGraphKey(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index,
                                        numerator, denominator)
                            }
                            onGraphCopyRequested: function(component, times) {
                                if (component === 0 && backend.copyControlGraphKeys(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index, times)) {
                                    propertyGraph.copyExternalClipboardMarker()
                                }
                            }
                            onGraphPasteRequested: function(component, numerator, denominator) {
                                if (component === 0)
                                    backend.pasteControlGraphKeys(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index,
                                        numerator, denominator)
                            }
                            onGraphInterpolationChanged: function(component, ownerId, interpolation) {
                                if (component === 0)
                                    backend.setControlGraphInterpolation(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index,
                                        ownerId, interpolation)
                            }
                            onKeyframesToggled: function(enabled) {
                                backend.setControlKeyframes(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, enabled)
                            }
                            onExpressionToggled: function(enabled) {
                                backend.setControlExpression(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, enabled)
                            }
                            onExpressionEdited: function(value) {
                                backend.setControlExpressionSource(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, value)
                            }
                            onExpressionCommitted: controlLoader.commit()
                        }
                    }
                    Component {
                        id: selectorControl
                        ControlRow {
                            id: selectorRow
                            readonly property var values: root.refreshed(
                                backend.controlChoiceValues(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex,
                                    controlLoader.index))
                            readonly property var labels: root.refreshed(
                                backend.controlChoiceLabels(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex,
                                    controlLoader.index))
                            readonly property var icons: root.refreshed(
                                backend.controlChoiceIcons(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex,
                                    controlLoader.index))
                            label: controlLoader.label
                            HoverHandler { id: selectorHover }
                            ToolTip.visible: selectorHover.hovered
                                && controlLoader.tooltip().length > 0
                            ToolTip.text: controlLoader.tooltip()
                            RowLayout {
                                spacing: 6
                                Loader {
                                    Layout.fillWidth: true
                                    sourceComponent: selectorRow.icons.length > 0
                                        ? buttonSelector : dropdownSelector
                                }
                                BusyIndicator {
                                    visible: running
                                    running: backend.controlBusy(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex,
                                        controlLoader.index)
                                    implicitWidth: 18
                                    implicitHeight: 18
                                }
                            }
                            Component {
                                id: dropdownSelector
                                Dropdown {
                                    enabled: controlLoader.editable && controlLoader.sensitive
                                    value: controlLoader.value
                                    values: selectorRow.values
                                    labels: selectorRow.labels
                                    onSelected: function(value) {
                                        controlLoader.edit(value)
                                        controlLoader.commit()
                                    }
                                }
                            }
                            Component {
                                id: buttonSelector
                                ButtonSelector {
                                    enabled: controlLoader.editable && controlLoader.sensitive
                                    value: controlLoader.value
                                    values: selectorRow.values
                                    labels: selectorRow.labels
                                    icons: selectorRow.icons
                                    onSelected: function(value) {
                                        controlLoader.edit(value)
                                        controlLoader.commit()
                                    }
                                }
                            }
                        }
                    }
                    Component {
                        id: vector2Control
                        ControlRow {
                            label: controlLoader.label
                            Number2Picker {
                                first: controlLoader.component(0)
                                second: controlLoader.component(1)
                                minimum: controlLoader.minimum()
                                maximum: controlLoader.maximum()
                                dragStep: controlLoader.dragStep()
                                digits: controlLoader.digits()
                                unitName: controlLoader.unit()
                                firstPrefix: controlLoader.prefix(0)
                                secondPrefix: controlLoader.prefix(1)
                                enableLock: controlLoader.locked()
                                onValuesEdited: function(first, second, component) {
                                    backend.setControlComponents(controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index,
                                        first, second, 0, component)
                                }
                                onFirstCommitted: function(value) { controlLoader.commit() }
                                onSecondCommitted: function(value) { controlLoader.commit() }
                            }
                        }
                    }
                    Component {
                        id: vector3Control
                        ControlRow {
                            label: controlLoader.label
                            Number3Picker {
                                first: controlLoader.component(0)
                                second: controlLoader.component(1)
                                third: controlLoader.component(2)
                                minimum: controlLoader.minimum()
                                maximum: controlLoader.maximum()
                                dragStep: controlLoader.dragStep()
                                digits: controlLoader.digits()
                                unitName: controlLoader.unit()
                                prefixes: [
                                    controlLoader.prefix(0),
                                    controlLoader.prefix(1),
                                    controlLoader.prefix(2)
                                ]
                                enableLock: controlLoader.locked()
                                onValuesEdited: function(first, second, third, component) {
                                    backend.setControlComponents(controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index,
                                        first, second, third, component)
                                }
                                onCommitted: function(axis, value) { controlLoader.commit() }
                            }
                        }
                    }
                    Component {
                        id: layeredNumberControl
                        InspectorGraphProperty {
                            id: propertyGraph
                            expressionDiagnosticProvider: function(source) {
                                return root.expressionDiagnostic(source)
                            }
                            expressionDiagnosticDebounce: root.expressionDiagnosticDebounce
                            expressionCompletionDebounce: root.expressionCompletionDebounce
                            expressionCompletionProvider: function(source, cursor, automatic) {
                                return root.expressionCompletion(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, source, cursor,
                                    automatic)
                            }
                            readonly property int graphDocumentRevision: backend.documentRevision
                            readonly property int graphModelRevision: backend.graphRevision
                            readonly property int graphPlayheadRevision:
                                backend.playheadRevision
                            readonly property int graphTransformRevision:
                                controlLoader.transformLive ? backend.transformRevision : 0
                            readonly property int scalarValueRevision:
                                backend.documentRevision + backend.playheadRevision
                                    + backend.graphRevision + graphTransformRevision
                            readonly property var expressionResult: {
                                const documentRevision = backend.documentRevision
                                const expressionRevision = backend.expressionRevision
                                const playheadRevision = backend.playheadRevision
                                return propertyGraph.expression
                                        && documentRevision + expressionRevision
                                        + playheadRevision
                                        + propertyGraph.graphTransformRevision >= 0
                                    ? backend.controlExpressionResult(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index)
                                    : []
                            }
                            label: controlLoader.label
                            initialGraphValue: scalarValueRevision >= 0
                                ? Number(backend.controlValue(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)) : 0
                            keyframes: root.refreshed(backend.controlKeyframes(
                                controlLoader.categoryIndex,
                                controlLoader.itemIndex, controlLoader.index))
                            expression: root.refreshed(backend.controlExpression(
                                controlLoader.categoryIndex,
                                controlLoader.itemIndex, controlLoader.index))
                            expressionValue: root.refreshed(
                                backend.controlExpressionSource(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index))
                            expressionOutput: expressionResult.length > 0
                                ? expressionResult[0] : ""
                            expressionError: expressionResult.length > 1
                                ? expressionResult[1] : ""
                            externalClipboardMarker: backend.keyframeClipboardMarker()
                            function updateGraphPlayhead() {
                                if (!propertyGraph.keyframes)
                                    return
                                const playhead = backend.controlGraphPlayhead(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                if (playhead.length === 2)
                                    propertyGraph.setGraphPlayhead(playhead[0], playhead[1])
                            }
                            function configureGraph() {
                                if (!propertyGraph.keyframes)
                                    return
                                const times = backend.controlGraphPointTimes(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                const values = backend.controlGraphPointValues(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                const segments = backend.controlGraphSegments(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                const timing = backend.controlGraphTiming(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                if (timing.length !== 6)
                                    return
                                propertyGraph.replaceRawGraphComponent(
                                    0, times, values, segments, Number(controlLoader.value))
                                propertyGraph.setGraphRange(
                                    timing[0], timing[1], timing[2], timing[3])
                                propertyGraph.setGraphFrameStep(timing[4], timing[5])
                                propertyGraph.setGraphSnapping(
                                    backend.keyframeSnappingEnabled(),
                                    backend.keyframeSnappingRadius())
                                propertyGraph.setGraphExternalClipboard(true)
                                propertyGraph.updateGraphPlayhead()
                            }
                            Component.onCompleted: configureGraph()
                            onGraphLoaded: configureGraph()
                            onGraphDocumentRevisionChanged: configureGraph()
                            onGraphModelRevisionChanged: configureGraph()
                            onGraphTransformRevisionChanged: configureGraph()
                            onGraphPlayheadRevisionChanged: updateGraphPlayhead()
                            NumberPicker {
                                value: propertyGraph.graphValue
                                minimum: controlLoader.minimum()
                                maximum: controlLoader.maximum()
                                dragStep: controlLoader.dragStep()
                                digits: controlLoader.digits()
                                unitName: controlLoader.unit()
                                widthCharacters: controlLoader.widthCharacters()
                                prefixIconSource: controlLoader.prefixIcon().length > 0
                                    ? "qrc:/qt/qml/dev/shrimply/components/icons/"
                                        + controlLoader.prefixIcon() : ""
                                prefixIconRotates: controlLoader.prefixIconRotates()
                                prefixIconRotationOffsetDegrees:
                                    controlLoader.prefixIconRotationOffset()
                                onEdited: function(value) { propertyGraph.editValue(value) }
                                onCommitted: function(value) { controlLoader.commit() }
                            }
                            onBaseValueEdited: function(value) { controlLoader.edit(value) }
                            onKeyframeValueEdited: function(value) { controlLoader.edit(value) }
                            onGraphPlaybackToggled: backend.toggleControlGraphPlayback()
                            onGraphPlayheadChanged: function(component, numerator, denominator) {
                                if (component !== 0)
                                    return
                                backend.seekControlGraph(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    numerator, denominator)
                            }
                            onGraphKeysMoved: function(component, oldTimes, times, values) {
                                if (component !== 0)
                                    return
                                backend.moveControlGraphKeys(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    oldTimes, times, values)
                            }
                            onGraphEditFinished: controlLoader.commit()
                            onGraphKeysDeleted: function(component, times) {
                                if (component !== 0)
                                    return
                                backend.deleteControlGraphKeys(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, times)
                            }
                            onGraphKeyAdded: function(component, numerator, denominator) {
                                if (component !== 0)
                                    return
                                backend.addControlGraphKey(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    numerator, denominator)
                            }
                            onGraphCopyRequested: function(component, times) {
                                if (component !== 0)
                                    return
                                if (backend.copyControlGraphKeys(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, times)) {
                                    propertyGraph.copyExternalClipboardMarker()
                                }
                            }
                            onGraphPasteRequested: function(component, numerator, denominator) {
                                if (component !== 0)
                                    return
                                backend.pasteControlGraphKeys(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    numerator, denominator)
                            }
                            onGraphInterpolationChanged: function(component, ownerId, interpolation) {
                                if (component !== 0)
                                    return
                                backend.setControlGraphInterpolation(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    ownerId, interpolation)
                            }
                            onKeyframesToggled: function(enabled) {
                                backend.setControlKeyframes(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, enabled)
                            }
                            onExpressionToggled: function(enabled) {
                                backend.setControlExpression(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, enabled)
                            }
                            onExpressionEdited: function(value) {
                                backend.setControlExpressionSource(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, value)
                            }
                            onExpressionCommitted: controlLoader.commit()
                        }
                    }
                    Component {
                        id: layeredDrawingControl
                        InspectorGraphProperty {
                            id: propertyGraph
                            expressionDiagnosticProvider: function(source) {
                                return root.expressionDiagnostic(source)
                            }
                            expressionDiagnosticDebounce: root.expressionDiagnosticDebounce
                            expressionCompletionDebounce: root.expressionCompletionDebounce
                            expressionCompletionProvider: function(source, cursor, automatic) {
                                return root.expressionCompletion(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, source, cursor,
                                    automatic)
                            }
                            readonly property int graphDocumentRevision: backend.documentRevision
                            readonly property int graphModelRevision: backend.graphRevision
                            readonly property int graphPlayheadRevision: backend.playheadRevision
                            readonly property var expressionResult: {
                                const revision = backend.documentRevision
                                    + backend.expressionRevision + backend.playheadRevision
                                return propertyGraph.expression && revision >= 0
                                    ? backend.controlExpressionResult(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index) : []
                            }
                            label: controlLoader.label
                            initialGraphValue: 0
                            keyframes: root.refreshed(backend.controlKeyframes(
                                controlLoader.categoryIndex,
                                controlLoader.itemIndex, controlLoader.index))
                            expression: root.refreshed(backend.controlExpression(
                                controlLoader.categoryIndex,
                                controlLoader.itemIndex, controlLoader.index))
                            expressionValue: root.refreshed(
                                backend.controlExpressionSource(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index))
                            expressionOutput: expressionResult.length > 0
                                ? expressionResult[0] : ""
                            expressionError: expressionResult.length > 1
                                ? expressionResult[1] : ""
                            externalClipboardMarker: backend.keyframeClipboardMarker()

                            Label {
                                text: controlLoader.value
                                color: palette.placeholderText
                                Layout.fillWidth: true
                            }

                            function updateGraphPlayhead() {
                                if (!propertyGraph.keyframes)
                                    return
                                const playhead = backend.controlGraphPlayhead(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                if (playhead.length === 2)
                                    propertyGraph.setGraphPlayhead(playhead[0], playhead[1])
                            }
                            function configureGraph() {
                                if (!propertyGraph.keyframes)
                                    return
                                const times = backend.controlGraphPointTimes(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                const segments = backend.controlGraphSegments(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                const timing = backend.controlGraphTiming(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                if (timing.length !== 6)
                                    return
                                propertyGraph.replaceSpeedGraphComponent(0, times, segments, 0)
                                propertyGraph.setGraphRange(
                                    timing[0], timing[1], timing[2], timing[3])
                                propertyGraph.setGraphFrameStep(timing[4], timing[5])
                                propertyGraph.setGraphSnapping(
                                    backend.keyframeSnappingEnabled(),
                                    backend.keyframeSnappingRadius())
                                propertyGraph.setGraphExternalClipboard(true)
                                propertyGraph.updateGraphPlayhead()
                            }
                            Component.onCompleted: configureGraph()
                            onGraphLoaded: configureGraph()
                            onGraphDocumentRevisionChanged: configureGraph()
                            onGraphModelRevisionChanged: configureGraph()
                            onGraphPlayheadRevisionChanged: updateGraphPlayhead()
                            onGraphPlaybackToggled: backend.toggleControlGraphPlayback()
                            onGraphPlayheadChanged: function(component, numerator, denominator) {
                                if (component === 0)
                                    backend.seekControlGraph(controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index,
                                        numerator, denominator)
                            }
                            onGraphKeysMoved: function(component, oldTimes, times, values) {
                                if (component === 0)
                                    backend.moveControlGraphKeys(controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index,
                                        oldTimes, times, values)
                            }
                            onGraphEditFinished: controlLoader.commit()
                            onGraphKeysDeleted: function(component, times) {
                                if (component === 0)
                                    backend.deleteControlGraphKeys(controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index, times)
                            }
                            onGraphKeyAdded: function(component, numerator, denominator) {
                                if (component === 0)
                                    backend.addControlGraphKey(controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index,
                                        numerator, denominator)
                            }
                            onGraphCopyRequested: function(component, times) {
                                if (component === 0 && backend.copyControlGraphKeys(
                                        controlLoader.categoryIndex, controlLoader.itemIndex,
                                        controlLoader.index, times))
                                    propertyGraph.copyExternalClipboardMarker()
                            }
                            onGraphPasteRequested: function(component, numerator, denominator) {
                                if (component === 0)
                                    backend.pasteControlGraphKeys(controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index,
                                        numerator, denominator)
                            }
                            onGraphInterpolationChanged: function(component, ownerId, interpolation) {
                                if (component === 0)
                                    backend.setControlGraphInterpolation(
                                        controlLoader.categoryIndex, controlLoader.itemIndex,
                                        controlLoader.index, ownerId, interpolation)
                            }
                            onKeyframesToggled: function(enabled) {
                                backend.setControlKeyframes(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, enabled)
                            }
                            onExpressionToggled: function(enabled) {
                                backend.setControlExpression(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, enabled)
                            }
                            onExpressionEdited: function(value) {
                                backend.setControlExpressionSource(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, value)
                            }
                            onExpressionCommitted: controlLoader.commit()
                        }
                    }
                    Component {
                        id: layeredBooleanControl
                        InspectorGraphProperty {
                            id: propertyGraph
                            expressionDiagnosticProvider: function(source) {
                                return root.expressionDiagnostic(source)
                            }
                            expressionDiagnosticDebounce: root.expressionDiagnosticDebounce
                            expressionCompletionDebounce: root.expressionCompletionDebounce
                            expressionCompletionProvider: function(source, cursor, automatic) {
                                return root.expressionCompletion(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, source, cursor,
                                    automatic)
                            }
                            readonly property int graphDocumentRevision: backend.documentRevision
                            readonly property int graphPlayheadRevision:
                                backend.playheadRevision
                            readonly property var expressionResult: {
                                const documentRevision = backend.documentRevision
                                const expressionRevision = backend.expressionRevision
                                const playheadRevision = backend.playheadRevision
                                return propertyGraph.expression
                                        && documentRevision + expressionRevision
                                        + playheadRevision >= 0
                                    ? backend.controlExpressionResult(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index)
                                    : []
                            }
                            label: controlLoader.label
                            initialGraphValue: controlLoader.value === "true" ? 1 : 0
                            keyframes: root.refreshed(backend.controlKeyframes(
                                controlLoader.categoryIndex,
                                controlLoader.itemIndex, controlLoader.index))
                            expression: root.refreshed(backend.controlExpression(
                                controlLoader.categoryIndex,
                                controlLoader.itemIndex, controlLoader.index))
                            expressionValue: root.refreshed(
                                backend.controlExpressionSource(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index))
                            expressionOutput: expressionResult.length > 0
                                ? expressionResult[0] : ""
                            expressionError: expressionResult.length > 1
                                ? expressionResult[1] : ""
                            externalClipboardMarker: backend.keyframeClipboardMarker()
                            function updateGraphPlayhead() {
                                if (!propertyGraph.keyframes)
                                    return
                                const playhead = backend.controlGraphPlayhead(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                if (playhead.length === 2)
                                    propertyGraph.setGraphPlayhead(playhead[0], playhead[1])
                            }
                            function configureGraph() {
                                if (!propertyGraph.keyframes)
                                    return
                                const times = backend.controlGraphPointTimes(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                const values = backend.controlGraphPointValues(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                const timing = backend.controlGraphTiming(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                if (timing.length !== 6)
                                    return
                                propertyGraph.replaceStepGraphComponent(0, times, values)
                                propertyGraph.setGraphRange(
                                    timing[0], timing[1], timing[2], timing[3])
                                propertyGraph.setGraphFrameStep(timing[4], timing[5])
                                propertyGraph.setGraphSnapping(
                                    backend.keyframeSnappingEnabled(),
                                    backend.keyframeSnappingRadius())
                                propertyGraph.setGraphExternalClipboard(true)
                                propertyGraph.updateGraphPlayhead()
                            }
                            Component.onCompleted: configureGraph()
                            onGraphLoaded: configureGraph()
                            onGraphDocumentRevisionChanged: configureGraph()
                            onGraphPlayheadRevisionChanged: updateGraphPlayhead()
                            SwitchRow {
                                label: ""
                                active: propertyGraph.graphValue >= 0.5
                                onToggled: function(active) {
                                    propertyGraph.editValue(active ? 1 : 0)
                                    controlLoader.commit()
                                }
                            }
                            onBaseValueEdited: function(value) {
                                controlLoader.edit(value >= 0.5)
                            }
                            onKeyframeValueEdited: function(value) {
                                controlLoader.edit(value >= 0.5)
                            }
                            onGraphPlaybackToggled: backend.toggleControlGraphPlayback()
                            onGraphPlayheadChanged: function(component, numerator, denominator) {
                                if (component !== 0)
                                    return
                                backend.seekControlGraph(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    numerator, denominator)
                            }
                            onGraphKeysMoved: function(component, oldTimes, times, values) {
                                if (component !== 0)
                                    return
                                if (times.length === 0)
                                    return
                                const canonicalTimes = backend.moveControlGraphKeys(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    oldTimes, times, values)
                                if (canonicalTimes.length === times.length)
                                    propertyGraph.reconcileStepGraphMoves(
                                        component, oldTimes, times, canonicalTimes)
                                else
                                    propertyGraph.rollbackStepGraphMoves(
                                        component, oldTimes, times)
                            }
                            onGraphEditFinished: controlLoader.commit()
                            onGraphKeysDeleted: function(component, times) {
                                if (component !== 0)
                                    return
                                backend.deleteControlGraphKeys(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, times)
                            }
                            onGraphKeyAdded: function(component, numerator, denominator) {
                                if (component !== 0)
                                    return
                                backend.addControlGraphKey(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    numerator, denominator)
                            }
                            onGraphCopyRequested: function(component, times) {
                                if (component !== 0)
                                    return
                                if (backend.copyControlGraphKeys(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, times)) {
                                    propertyGraph.copyExternalClipboardMarker()
                                }
                            }
                            onGraphPasteRequested: function(component, numerator, denominator) {
                                if (component !== 0)
                                    return
                                backend.pasteControlGraphKeys(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    numerator, denominator)
                            }
                            onKeyframesToggled: function(enabled) {
                                backend.setControlKeyframes(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, enabled)
                            }
                            onExpressionToggled: function(enabled) {
                                backend.setControlExpression(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, enabled)
                            }
                            onExpressionEdited: function(value) {
                                backend.setControlExpressionSource(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, value)
                            }
                            onExpressionCommitted: controlLoader.commit()
                        }
                    }
                    Component {
                        id: layeredSelectorControl
                        InspectorGraphProperty {
                            id: propertyGraph
                            expressionDiagnosticProvider: function(source) {
                                return root.expressionDiagnostic(source)
                            }
                            expressionDiagnosticDebounce: root.expressionDiagnosticDebounce
                            expressionCompletionDebounce: root.expressionCompletionDebounce
                            expressionCompletionProvider: function(source, cursor, automatic) {
                                return root.expressionCompletion(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, source, cursor,
                                    automatic)
                            }
                            readonly property int graphDocumentRevision: backend.documentRevision
                            readonly property int graphModelRevision: backend.graphRevision
                            readonly property int graphPlayheadRevision:
                                backend.playheadRevision
                            readonly property var expressionResult: {
                                const documentRevision = backend.documentRevision
                                const expressionRevision = backend.expressionRevision
                                const playheadRevision = backend.playheadRevision
                                return propertyGraph.expression
                                        && documentRevision + expressionRevision
                                        + playheadRevision >= 0
                                    ? backend.controlExpressionResult(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index)
                                    : []
                            }
                            readonly property var values: root.refreshed(
                                backend.controlChoiceValues(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index))
                            readonly property int selectedIndex: Math.max(0,
                                values.indexOf(controlLoader.value))
                            label: controlLoader.label
                            initialGraphValue: selectedIndex
                            keyframes: root.refreshed(backend.controlKeyframes(
                                controlLoader.categoryIndex,
                                controlLoader.itemIndex, controlLoader.index))
                            expression: root.refreshed(backend.controlExpression(
                                controlLoader.categoryIndex,
                                controlLoader.itemIndex, controlLoader.index))
                            expressionValue: root.refreshed(
                                backend.controlExpressionSource(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index))
                            expressionOutput: expressionResult.length > 0
                                ? expressionResult[0] : ""
                            expressionError: expressionResult.length > 1
                                ? expressionResult[1] : ""
                            externalClipboardMarker: backend.keyframeClipboardMarker()
                            function updateGraphPlayhead() {
                                if (!propertyGraph.keyframes)
                                    return
                                const playhead = backend.controlGraphPlayhead(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                if (playhead.length === 2)
                                    propertyGraph.setGraphPlayhead(playhead[0], playhead[1])
                            }
                            function configureGraph() {
                                if (!propertyGraph.keyframes)
                                    return
                                const times = backend.controlGraphPointTimes(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                const pointValues = backend.controlGraphPointValues(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                const timing = backend.controlGraphTiming(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                if (timing.length !== 6)
                                    return
                                propertyGraph.replaceStepGraphComponent(0, times, pointValues)
                                propertyGraph.setGraphRange(
                                    timing[0], timing[1], timing[2], timing[3])
                                propertyGraph.setGraphFrameStep(timing[4], timing[5])
                                propertyGraph.setGraphSnapping(
                                    backend.keyframeSnappingEnabled(),
                                    backend.keyframeSnappingRadius())
                                propertyGraph.setGraphExternalClipboard(true)
                                propertyGraph.updateGraphPlayhead()
                            }
                            Component.onCompleted: configureGraph()
                            onGraphLoaded: configureGraph()
                            onGraphDocumentRevisionChanged: configureGraph()
                            onGraphModelRevisionChanged: configureGraph()
                            onGraphPlayheadRevisionChanged: updateGraphPlayhead()
                            Dropdown {
                                value: propertyGraph.values[Math.max(0, Math.min(
                                    propertyGraph.values.length - 1,
                                    Math.round(propertyGraph.graphValue)))] || ""
                                values: propertyGraph.values
                                labels: root.refreshed(backend.controlChoiceLabels(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index))
                                onSelected: function(value) {
                                    propertyGraph.editValue(propertyGraph.values.indexOf(value))
                                    controlLoader.commit()
                                }
                            }
                            onBaseValueEdited: function(value) {
                                const index = Math.max(0, Math.min(values.length - 1,
                                    Math.round(value)))
                                if (values.length > 0)
                                    controlLoader.edit(values[index])
                            }
                            onKeyframeValueEdited: function(value) {
                                const index = Math.max(0, Math.min(values.length - 1,
                                    Math.round(value)))
                                if (values.length > 0)
                                    controlLoader.edit(values[index])
                            }
                            onGraphPlaybackToggled: backend.toggleControlGraphPlayback()
                            onGraphPlayheadChanged: function(component, numerator, denominator) {
                                if (component !== 0)
                                    return
                                backend.seekControlGraph(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    numerator, denominator)
                            }
                            onGraphKeysMoved: function(component, oldTimes, times, pointValues) {
                                if (component !== 0)
                                    return
                                if (times.length === 0)
                                    return
                                const canonicalTimes = backend.moveControlGraphKeys(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    oldTimes, times, pointValues)
                                if (canonicalTimes.length === times.length)
                                    propertyGraph.reconcileStepGraphMoves(
                                        component, oldTimes, times, canonicalTimes)
                                else
                                    propertyGraph.rollbackStepGraphMoves(
                                        component, oldTimes, times)
                            }
                            onGraphEditFinished: controlLoader.commit()
                            onGraphKeysDeleted: function(component, times) {
                                if (component !== 0)
                                    return
                                backend.deleteControlGraphKeys(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, times)
                            }
                            onGraphKeyAdded: function(component, numerator, denominator) {
                                if (component !== 0)
                                    return
                                backend.addControlGraphKey(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    numerator, denominator)
                            }
                            onGraphCopyRequested: function(component, times) {
                                if (component !== 0)
                                    return
                                if (backend.copyControlGraphKeys(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, times)) {
                                    propertyGraph.copyExternalClipboardMarker()
                                }
                            }
                            onGraphPasteRequested: function(component, numerator, denominator) {
                                if (component !== 0)
                                    return
                                backend.pasteControlGraphKeys(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    numerator, denominator)
                            }
                            onKeyframesToggled: function(enabled) {
                                backend.setControlKeyframes(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, enabled)
                            }
                            onExpressionToggled: function(enabled) {
                                backend.setControlExpression(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, enabled)
                            }
                            onExpressionEdited: function(value) {
                                backend.setControlExpressionSource(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, value)
                            }
                            onExpressionCommitted: controlLoader.commit()
                        }
                    }
                    Component {
                        id: layeredVector2Control
                        InspectorPairGraphProperty {
                            id: propertyGraph
                            expressionDiagnosticProvider: function(source) {
                                return root.expressionDiagnostic(source)
                            }
                            expressionDiagnosticDebounce: root.expressionDiagnosticDebounce
                            expressionCompletionDebounce: root.expressionCompletionDebounce
                            expressionCompletionProvider: function(source, cursor, automatic) {
                                return root.expressionCompletion(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, source, cursor,
                                    automatic)
                            }
                            readonly property int graphDocumentRevision: backend.documentRevision
                            readonly property int graphModelRevision: backend.graphRevision
                            readonly property int graphPlayheadRevision:
                                backend.playheadRevision
                            readonly property int graphTransformRevision:
                                controlLoader.transformLive ? backend.transformRevision : 0
                            readonly property int pairValueRevision:
                                backend.documentRevision + backend.playheadRevision
                                    + backend.graphRevision + graphTransformRevision
                            readonly property var expressionResult: {
                                const documentRevision = backend.documentRevision
                                const expressionRevision = backend.expressionRevision
                                const playheadRevision = backend.playheadRevision
                                return propertyGraph.expression
                                        && documentRevision + expressionRevision
                                        + playheadRevision
                                        + propertyGraph.graphTransformRevision >= 0
                                    ? backend.controlExpressionResult(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index)
                                    : []
                            }
                            label: controlLoader.label
                            graphValueDrivesEditor: false
                            initialGraphValue: pairValueRevision >= 0
                                ? backend.controlComponent(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, 0) : 0
                            initialSecondValue: pairValueRevision >= 0
                                ? backend.controlComponent(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, 1) : 0
                            keyframes: root.refreshed(backend.controlKeyframes(
                                controlLoader.categoryIndex,
                                controlLoader.itemIndex, controlLoader.index))
                            expression: root.refreshed(backend.controlExpression(
                                controlLoader.categoryIndex,
                                controlLoader.itemIndex, controlLoader.index))
                            expressionValue: root.refreshed(
                                backend.controlExpressionSource(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index))
                            expressionOutput: expressionResult.length > 0
                                ? expressionResult[0] : ""
                            expressionError: expressionResult.length > 1
                                ? expressionResult[1] : ""
                            externalClipboardMarker: backend.keyframeClipboardMarker()
                            minimum: controlLoader.minimum()
                            maximum: controlLoader.maximum()
                            dragStep: controlLoader.dragStep()
                            digits: controlLoader.digits()
                            widthCharacters: controlLoader.widthCharacters()
                            unitName: controlLoader.unit()
                            firstPrefix: controlLoader.prefix(0)
                            secondPrefix: controlLoader.prefix(1)
                            enableLock: controlLoader.locked()
                            trailing: Loader {
                                active: controlLoader.inlineRowPrimary
                                sourceComponent: RowLayout {
                                    id: inlineControlRow
                                    readonly property int selectorIndex:
                                        backend.controlRowMember(
                                            controlLoader.categoryIndex,
                                            controlLoader.itemIndex,
                                            controlLoader.index,
                                            InspectorBackend.Auxiliary)
                                    readonly property int removeIndex:
                                        backend.controlRowMember(
                                            controlLoader.categoryIndex,
                                            controlLoader.itemIndex,
                                            controlLoader.index,
                                            InspectorBackend.TrailingAction)
                                    Dropdown {
                                        Layout.fillWidth: true
                                        visible: inlineControlRow.selectorIndex >= 0
                                        enabled: backend.controlEditable(
                                            controlLoader.categoryIndex,
                                            controlLoader.itemIndex,
                                            inlineControlRow.selectorIndex)
                                            && backend.controlSensitive(
                                                controlLoader.categoryIndex,
                                                controlLoader.itemIndex,
                                                inlineControlRow.selectorIndex)
                                        value: root.refreshed(backend.controlValue(
                                            controlLoader.categoryIndex,
                                            controlLoader.itemIndex,
                                            inlineControlRow.selectorIndex))
                                        values: root.refreshed(backend.controlChoiceValues(
                                            controlLoader.categoryIndex,
                                            controlLoader.itemIndex,
                                            inlineControlRow.selectorIndex))
                                        labels: root.refreshed(backend.controlChoiceLabels(
                                            controlLoader.categoryIndex,
                                            controlLoader.itemIndex,
                                            inlineControlRow.selectorIndex))
                                        ToolTip.visible: hovered
                                        ToolTip.text: backend.controlLabel(
                                            controlLoader.categoryIndex,
                                            controlLoader.itemIndex,
                                            inlineControlRow.selectorIndex)
                                        onSelected: function(value) {
                                            backend.setControlValue(
                                                controlLoader.categoryIndex,
                                                controlLoader.itemIndex,
                                                inlineControlRow.selectorIndex,
                                                String(value))
                                            backend.commitControl(
                                                controlLoader.categoryIndex,
                                                controlLoader.itemIndex,
                                                inlineControlRow.selectorIndex)
                                        }
                                    }
                                    ToolButton {
                                        flat: true
                                        enabled: backend.controlSensitive(
                                            controlLoader.categoryIndex,
                                            controlLoader.itemIndex,
                                            inlineControlRow.removeIndex)
                                        icon.name: backend.controlPrefixIcon(
                                            controlLoader.categoryIndex,
                                            controlLoader.itemIndex,
                                            inlineControlRow.removeIndex)
                                        display: AbstractButton.IconOnly
                                        ToolTip.visible: hovered
                                        ToolTip.text: backend.controlTooltip(
                                            controlLoader.categoryIndex,
                                            controlLoader.itemIndex,
                                            inlineControlRow.removeIndex)
                                        onClicked: backend.triggerControlAction(
                                            controlLoader.categoryIndex,
                                            controlLoader.itemIndex,
                                            inlineControlRow.removeIndex)
                                    }
                                }
                            }
                            function updateGraphPlayhead() {
                                if (!propertyGraph.keyframes)
                                    return
                                const playhead = backend.controlGraphPlayhead(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                if (playhead.length === 2)
                                    propertyGraph.setGraphPlayhead(playhead[0], playhead[1])
                            }
                            function configureGraph() {
                                if (!propertyGraph.keyframes)
                                    return
                                const times = backend.controlGraphPointTimes(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                const segments = backend.controlGraphSegments(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                const timing = backend.controlGraphTiming(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                if (timing.length !== 6)
                                    return
                                propertyGraph.replaceSpeedGraphComponent(
                                    0, times, segments, 0)
                                propertyGraph.setGraphRange(
                                    timing[0], timing[1], timing[2], timing[3])
                                propertyGraph.setGraphFrameStep(timing[4], timing[5])
                                propertyGraph.setGraphSnapping(
                                    backend.keyframeSnappingEnabled(),
                                    backend.keyframeSnappingRadius())
                                propertyGraph.setGraphExternalClipboard(true)
                                propertyGraph.updateGraphPlayhead()
                            }
                            Component.onCompleted: configureGraph()
                            onGraphLoaded: configureGraph()
                            onGraphDocumentRevisionChanged: configureGraph()
                            onGraphModelRevisionChanged: configureGraph()
                            onGraphTransformRevisionChanged: configureGraph()
                            onGraphPlayheadRevisionChanged: updateGraphPlayhead()
                            onBasePairEdited: function(first, second) {
                                backend.setControlComponents(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    first, second, 0, 0)
                            }
                            onKeyframePairEdited: function(first, second, component) {
                                backend.setControlComponents(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    first, second, 0, component)
                            }
                            onPairCommitted: controlLoader.commit()
                            onGraphPlaybackToggled: backend.toggleControlGraphPlayback()
                            onGraphPlayheadChanged: function(component, numerator, denominator) {
                                if (component !== 0)
                                    return
                                backend.seekControlGraph(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    numerator, denominator)
                            }
                            onGraphKeysMoved: function(component, oldTimes, times, values) {
                                if (component !== 0)
                                    return
                                backend.moveControlGraphKeys(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    oldTimes, times, values)
                            }
                            onGraphEditFinished: controlLoader.commit()
                            onGraphKeysDeleted: function(component, times) {
                                if (component !== 0)
                                    return
                                backend.deleteControlGraphKeys(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, times)
                            }
                            onGraphKeyAdded: function(component, numerator, denominator) {
                                if (component !== 0)
                                    return
                                backend.addControlGraphKey(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    numerator, denominator)
                            }
                            onGraphCopyRequested: function(component, times) {
                                if (component !== 0)
                                    return
                                if (backend.copyControlGraphKeys(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, times)) {
                                    propertyGraph.copyExternalClipboardMarker()
                                }
                            }
                            onGraphPasteRequested: function(component, numerator, denominator) {
                                if (component !== 0)
                                    return
                                backend.pasteControlGraphKeys(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    numerator, denominator)
                            }
                            onGraphInterpolationChanged: function(component, ownerId, interpolation) {
                                if (component !== 0)
                                    return
                                backend.setControlGraphInterpolation(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    ownerId, interpolation)
                            }
                            onKeyframesToggled: function(enabled) {
                                backend.setControlKeyframes(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, enabled)
                            }
                            onExpressionToggled: function(enabled) {
                                backend.setControlExpression(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, enabled)
                            }
                            onExpressionEdited: function(value) {
                                backend.setControlExpressionSource(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, value)
                            }
                            onExpressionCommitted: controlLoader.commit()
                        }
                    }
                    Component {
                        id: actionControl
                        Loader {
                            Layout.fillWidth: true
                            readonly property bool iconOnlyRow:
                                controlLoader.label.length > 0
                                && controlLoader.prefixIcon().length > 0
                                && backend.controlHasAction(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex,
                                    controlLoader.index)
                                && backend.controlDragPayload(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex,
                                    controlLoader.index).length === 0
                            sourceComponent: iconOnlyRow ? labeledIconAction
                                : controlLoader.label.length === 0
                                    && controlLoader.prefixIcon().length > 0
                                ? flatActionButton : labeledActionButton
                            Component {
                                id: flatActionButton
                                ProgressButton {
                                    text: ComponentTranslations.text(controlLoader.value)
                                    flat: true
                                    icon.name: controlLoader.prefixIcon()
                                    ToolTip.visible: hovered
                                        && controlLoader.tooltip().length > 0
                                    ToolTip.text: controlLoader.tooltip()
                                    onClicked: backend.triggerControlAction(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index)
                                }
                            }
                            Component {
                                id: labeledIconAction
                                ControlRow {
                                    label: controlLoader.label
                                    RowLayout {
                                        Item { Layout.fillWidth: true }
                                        ToolButton {
                                            flat: true
                                            icon.name: controlLoader.prefixIcon()
                                            display: AbstractButton.IconOnly
                                            ToolTip.visible: hovered
                                                && controlLoader.tooltip().length > 0
                                            ToolTip.text: controlLoader.tooltip()
                                            onClicked: backend.triggerControlAction(
                                                controlLoader.categoryIndex,
                                                controlLoader.itemIndex,
                                                controlLoader.index)
                                        }
                                    }
                                }
                            }
                            Component {
                                id: labeledActionButton
                                ControlRow {
                                    label: controlLoader.label
                                    subtitle: root.refreshed(backend.controlSubtitle(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index))
                                    RowLayout {
                                        id: actionRow
                                        Layout.fillWidth: true
                                        readonly property string dragPayload: backend.controlDragPayload(
                                            controlLoader.categoryIndex,
                                            controlLoader.itemIndex, controlLoader.index)
                                        ProgressButton {
                                            id: actionButton
                                            text: ComponentTranslations.text(controlLoader.value)
                                            icon.name: controlLoader.prefixIcon()
                                            progressState: backend.controlBusy(
                                                controlLoader.categoryIndex,
                                                controlLoader.itemIndex,
                                                controlLoader.index)
                                                    ? ProgressButton.Indeterminate
                                                    : ProgressButton.Idle
                                            Layout.fillWidth: true
                                            Drag.active: actionDrag.active
                                            Drag.dragType: Drag.Automatic
                                            Drag.supportedActions: Qt.CopyAction
                                            Drag.mimeData: ({ "text/plain": actionRow.dragPayload })
                                            DragHandler {
                                                id: actionDrag
                                                enabled: actionRow.dragPayload.length > 0
                                                target: null
                                            }
                                            ToolTip.visible: hovered
                                                && controlLoader.tooltip().length > 0
                                            ToolTip.text: controlLoader.tooltip()
                                            onClicked: {
                                                if (actionRow.dragPayload.length === 0)
                                                    backend.triggerControlAction(
                                                        controlLoader.categoryIndex,
                                                        controlLoader.itemIndex,
                                                        controlLoader.index)
                                            }
                                        }
                                        ToolButton {
                                            readonly property string actionIcon:
                                                backend.controlActionIcon(
                                                    controlLoader.categoryIndex,
                                                    controlLoader.itemIndex,
                                                    controlLoader.index)
                                            visible: backend.controlHasAction(
                                                controlLoader.categoryIndex,
                                                controlLoader.itemIndex,
                                                controlLoader.index)
                                                && actionIcon.length > 0
                                            flat: true
                                            icon.name: actionIcon
                                            display: AbstractButton.IconOnly
                                            ToolTip.visible: hovered
                                            ToolTip.text: backend.controlActionTooltip(
                                                controlLoader.categoryIndex,
                                                controlLoader.itemIndex,
                                                controlLoader.index)
                                            onClicked: backend.triggerSecondaryControlAction(
                                                controlLoader.categoryIndex,
                                                controlLoader.itemIndex,
                                                controlLoader.index)
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Component {
                        id: audioCacheControl
                        RowLayout {
                            SystemPalette { id: audioCachePalette }
                            Layout.fillWidth: true
                            Item { Layout.fillWidth: true }
                            ProgressButton {
                                readonly property real cacheProgress: controlLoader.component(0)
                                readonly property bool baking:
                                    controlLoader.component(1) > 0
                                readonly property bool cancelHovered: baking && hovered
                                highlighted: !baking
                                text: cancelHovered ? qsTr("Cancel") : controlLoader.value
                                progressState: !baking ? ProgressButton.Idle
                                    : cacheProgress < 0 ? ProgressButton.Indeterminate
                                    : ProgressButton.Progress
                                progress: Math.max(0, cacheProgress)
                                palette.button: cancelHovered
                                    ? backend.destructiveBackground()
                                    : audioCachePalette.button
                                palette.buttonText: cancelHovered
                                    ? backend.destructiveForeground()
                                    : audioCachePalette.buttonText
                                ToolTip.visible: hovered && controlLoader.tooltip().length > 0
                                ToolTip.text: controlLoader.tooltip()
                                onClicked: backend.triggerControlAction(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                            }
                        }
                    }
                    Component {
                        id: audioModifierMenuControl
                        RowLayout {
                            Layout.fillWidth: true
                            Item { Layout.fillWidth: true }
                            ModifierMenuButton {
                                values: root.refreshed(backend.controlChoiceValues(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index))
                                labels: root.refreshed(backend.controlChoiceLabels(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index))
                                searchTerms: root.refreshed(
                                    backend.controlChoiceSearchTerms(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index))
                                onSelected: function(value) { controlLoader.edit(value) }
                            }
                            Button {
                                visible: controlLoader.value === "true"
                                flat: true
                                icon.name: "edit-paste-symbolic"
                                text: qsTr("Paste Modifier")
                                display: AbstractButton.IconOnly
                                ToolTip.visible: hovered
                                ToolTip.text: text
                                onClicked: controlLoader.edit("__paste__")
                            }
                            Item { Layout.fillWidth: true }
                        }
                    }
                    Component {
                        id: visualCacheControl
                        RowLayout {
                            SystemPalette { id: visualCachePalette }
                            Layout.fillWidth: true
                            Item { Layout.fillWidth: true }
                            ProgressButton {
                                readonly property real cacheProgress: controlLoader.component(0)
                                readonly property bool baking:
                                    controlLoader.component(1) > 0
                                readonly property bool cancelHovered: baking && hovered
                                highlighted: !baking
                                text: cancelHovered ? qsTr("Cancel") : controlLoader.value
                                progressState: !baking ? ProgressButton.Idle
                                    : cacheProgress < 0 ? ProgressButton.Indeterminate
                                    : ProgressButton.Progress
                                progress: Math.max(0, cacheProgress)
                                palette.button: cancelHovered
                                    ? backend.destructiveBackground()
                                    : visualCachePalette.button
                                palette.buttonText: cancelHovered
                                    ? backend.destructiveForeground()
                                    : visualCachePalette.buttonText
                                ToolTip.visible: hovered
                                    && controlLoader.tooltip().length > 0
                                ToolTip.text: controlLoader.tooltip()
                                onClicked: backend.triggerControlAction(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                            }
                        }
                    }
                    Component {
                        id: analysisControl
                        RowLayout {
                            SystemPalette { id: analysisPalette }
                            Layout.fillWidth: true
                            Item { Layout.fillWidth: true }
                            ProgressButton {
                                id: analysisButton
                                readonly property int statusRevision: backend.analysisRevision
                                readonly property real analysisProgress: {
                                    const revision = statusRevision
                                    return revision >= 0 ? backend.controlComponent(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index, 0) : -1
                                }
                                readonly property bool analysisRunning: {
                                    const revision = statusRevision
                                    return revision >= 0 && backend.controlComponent(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index, 1) > 0
                                }
                                readonly property bool analysisCancelling: {
                                    const revision = statusRevision
                                    return revision >= 0 && backend.controlComponent(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index, 2) > 0
                                }
                                readonly property bool suggested: {
                                    const revision = statusRevision
                                    return revision >= 0 && backend.controlComponent(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index, 3) > 0
                                }
                                readonly property string analysisLabel: {
                                    const revision = statusRevision
                                    return revision >= 0 ? ComponentTranslations.text(
                                        backend.controlValue(
                                            controlLoader.categoryIndex,
                                            controlLoader.itemIndex, controlLoader.index)) : ""
                                }
                                readonly property string analysisTooltip: {
                                    const revision = statusRevision
                                    return revision >= 0 ? backend.controlTooltip(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index) : ""
                                }
                                readonly property bool cancelHovered: analysisRunning && hovered
                                enabled: {
                                    const revision = statusRevision
                                    return revision >= 0 && backend.controlSensitive(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index)
                                }
                                highlighted: suggested
                                text: cancelHovered ? qsTr("Cancel") : analysisLabel
                                progressState: !analysisRunning && !analysisCancelling
                                    ? ProgressButton.Idle
                                    : analysisProgress < 0 ? ProgressButton.Indeterminate
                                    : ProgressButton.Progress
                                progress: Math.max(0, analysisProgress)
                                palette.button: cancelHovered
                                    ? backend.destructiveBackground() : analysisPalette.button
                                palette.buttonText: cancelHovered
                                    ? backend.destructiveForeground() : analysisPalette.buttonText
                                ToolTip.visible: hovered && analysisTooltip.length > 0
                                ToolTip.text: analysisTooltip
                                onClicked: backend.triggerControlAction(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                Timer {
                                    interval: 50
                                    repeat: true
                                    running: analysisButton.analysisRunning
                                        || analysisButton.analysisCancelling
                                    onTriggered: backend.pollAnalysisControl(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index)
                                }
                            }
                        }
                    }
                    Component {
                        id: layeredVector3Control
                        InspectorTripleGraphProperty {
                            id: propertyGraph
                            expressionDiagnosticProvider: function(source) {
                                return root.expressionDiagnostic(source)
                            }
                            expressionDiagnosticDebounce: root.expressionDiagnosticDebounce
                            expressionCompletionDebounce: root.expressionCompletionDebounce
                            expressionCompletionProvider: function(source, cursor, automatic) {
                                return root.expressionCompletion(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, source, cursor,
                                    automatic)
                            }
                            readonly property int graphDocumentRevision: backend.documentRevision
                            readonly property int graphModelRevision: backend.graphRevision
                            readonly property int graphPlayheadRevision:
                                backend.playheadRevision
                            readonly property int tripleValueRevision:
                                backend.documentRevision + backend.playheadRevision
                                    + backend.graphRevision
                            readonly property var expressionResult: {
                                const documentRevision = backend.documentRevision
                                const expressionRevision = backend.expressionRevision
                                const playheadRevision = backend.playheadRevision
                                return propertyGraph.expression
                                        && documentRevision + expressionRevision
                                        + playheadRevision >= 0
                                    ? backend.controlExpressionResult(
                                        controlLoader.categoryIndex,
                                        controlLoader.itemIndex, controlLoader.index)
                                    : []
                            }
                            label: controlLoader.label
                            graphValueDrivesEditor: false
                            initialGraphValue: tripleValueRevision >= 0
                                ? backend.controlComponent(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, 0) : 0
                            initialSecondValue: tripleValueRevision >= 0
                                ? backend.controlComponent(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, 1) : 0
                            initialThirdValue: tripleValueRevision >= 0
                                ? backend.controlComponent(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, 2) : 0
                            keyframes: root.refreshed(backend.controlKeyframes(
                                controlLoader.categoryIndex,
                                controlLoader.itemIndex, controlLoader.index))
                            expression: root.refreshed(backend.controlExpression(
                                controlLoader.categoryIndex,
                                controlLoader.itemIndex, controlLoader.index))
                            expressionValue: root.refreshed(
                                backend.controlExpressionSource(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index))
                            expressionOutput: expressionResult.length > 0
                                ? expressionResult[0] : ""
                            expressionError: expressionResult.length > 1
                                ? expressionResult[1] : ""
                            externalClipboardMarker: backend.keyframeClipboardMarker()
                            minimum: controlLoader.minimum()
                            maximum: controlLoader.maximum()
                            dragStep: controlLoader.dragStep()
                            digits: controlLoader.digits()
                            widthCharacters: controlLoader.widthCharacters()
                            unitName: controlLoader.unit()
                            prefixes: [controlLoader.prefix(0), controlLoader.prefix(1),
                                controlLoader.prefix(2)]
                            enableLock: controlLoader.locked()
                            function updateGraphPlayhead() {
                                if (!propertyGraph.keyframes)
                                    return
                                const playhead = backend.controlGraphPlayhead(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                if (playhead.length === 2)
                                    propertyGraph.setGraphPlayhead(playhead[0], playhead[1])
                            }
                            function configureGraph() {
                                if (!propertyGraph.keyframes)
                                    return
                                const times = backend.controlGraphPointTimes(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                const segments = backend.controlGraphSegments(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                const timing = backend.controlGraphTiming(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index)
                                if (timing.length !== 6)
                                    return
                                propertyGraph.replaceSpeedGraphComponent(
                                    0, times, segments, 0)
                                propertyGraph.setGraphRange(
                                    timing[0], timing[1], timing[2], timing[3])
                                propertyGraph.setGraphFrameStep(timing[4], timing[5])
                                propertyGraph.setGraphSnapping(
                                    backend.keyframeSnappingEnabled(),
                                    backend.keyframeSnappingRadius())
                                propertyGraph.setGraphExternalClipboard(true)
                                propertyGraph.updateGraphPlayhead()
                            }
                            Component.onCompleted: configureGraph()
                            onGraphLoaded: configureGraph()
                            onGraphDocumentRevisionChanged: configureGraph()
                            onGraphModelRevisionChanged: configureGraph()
                            onGraphPlayheadRevisionChanged: updateGraphPlayhead()
                            onBaseTripleEdited: function(first, second, third) {
                                backend.setControlComponents(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    first, second, third, 0)
                            }
                            onKeyframeTripleEdited: function(first, second, third, component) {
                                backend.setControlComponents(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    first, second, third, component)
                            }
                            onTripleCommitted: controlLoader.commit()
                            onGraphPlaybackToggled: backend.toggleControlGraphPlayback()
                            onGraphPlayheadChanged: function(component, numerator, denominator) {
                                if (component !== 0)
                                    return
                                backend.seekControlGraph(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    numerator, denominator)
                            }
                            onGraphKeysMoved: function(component, oldTimes, times, values) {
                                if (component !== 0)
                                    return
                                backend.moveControlGraphKeys(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    oldTimes, times, values)
                            }
                            onGraphEditFinished: controlLoader.commit()
                            onGraphKeysDeleted: function(component, times) {
                                if (component !== 0)
                                    return
                                backend.deleteControlGraphKeys(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, times)
                            }
                            onGraphKeyAdded: function(component, numerator, denominator) {
                                if (component !== 0)
                                    return
                                backend.addControlGraphKey(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    numerator, denominator)
                            }
                            onGraphCopyRequested: function(component, times) {
                                if (component !== 0)
                                    return
                                if (backend.copyControlGraphKeys(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, times)) {
                                    propertyGraph.copyExternalClipboardMarker()
                                }
                            }
                            onGraphPasteRequested: function(component, numerator, denominator) {
                                if (component !== 0)
                                    return
                                backend.pasteControlGraphKeys(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    numerator, denominator)
                            }
                            onGraphInterpolationChanged: function(component, ownerId, interpolation) {
                                if (component !== 0)
                                    return
                                backend.setControlGraphInterpolation(
                                    controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index,
                                    ownerId, interpolation)
                            }
                            onKeyframesToggled: function(enabled) {
                                backend.setControlKeyframes(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, enabled)
                            }
                            onExpressionToggled: function(enabled) {
                                backend.setControlExpression(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, enabled)
                            }
                            onExpressionEdited: function(value) {
                                backend.setControlExpressionSource(controlLoader.categoryIndex,
                                    controlLoader.itemIndex, controlLoader.index, value)
                            }
                            onExpressionCommitted: controlLoader.commit()
                        }
                    }
                    Component {
                        id: projectSettingsControl
                        ProjectSettingsSelector {
                            initialWidth: controlLoader.component(0)
                            initialHeight: controlLoader.component(1)
                            initialFpsNumerator: controlLoader.componentText(2)
                            initialFpsDenominator: controlLoader.componentText(3)
                            onApplyRequested: function(width, height, numerator, denominator) {
                                backend.applyProjectSettings(width, height, numerator, denominator)
                            }
                        }
                    }
                    Component {
                        id: performanceControl
                        LivePerformance { Layout.fillWidth: true }
                    }
                }
            }
        }
    }
}
