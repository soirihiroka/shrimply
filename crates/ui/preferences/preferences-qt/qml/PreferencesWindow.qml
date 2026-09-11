import QtCore
import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts

ApplicationWindow {
    id: root

    required property var backend
    required property var owner
    property int trustedLocationCount: 0
    property bool loading: false
    property int currentPage: 0
    property bool blenderBusy: false
    property bool serverBusy: false
    property string serverError: ""

    title: backend.translate("Preferences — Shrimply")
    transientParent: owner
    modality: Qt.WindowModal
    flags: Qt.Dialog
    width: 980
    height: 680
    minimumWidth: 760
    minimumHeight: 520

    function numberValue(key) { return Number(backend.preferenceValue(key)) }
    function configure(spin, key) {
        spin.from = backend.preferenceMinimum(key)
        spin.to = backend.preferenceMaximum(key)
        spin.stepSize = backend.preferenceStep(key)
        if ("valueScale" in spin)
            spin.valueScale = backend.preferenceScale(key)
        spin.value = numberValue(key)
    }
    function reloadServers() {
        serverModel.clear()
        for (let index = 0; index < backend.preferenceServerCount(); ++index)
            serverModel.append({ url: backend.preferenceServerUrl(index) })
        serverChoice.currentIndex = backend.preferenceSelectedServer()
        serverUrl.text = serverChoice.currentIndex >= 0
            ? serverModel.get(serverChoice.currentIndex).url : ""
        serverError = ""
        serverBusy = true
        clearServerStatus()
        backend.refreshPreferenceServerStatus()
    }
    function clearServerStatus() {
        serverDeviceModel.clear()
        serverDevice.currentIndex = -1
        serverVersion.text = ""
        serverProtocol.text = ""
        serverTorch.text = ""
        serverCuda.text = ""
        serverJobs.text = ""
        serverReservations.text = ""
        serverWorkers.text = ""
        serverFeatures.text = ""
    }
    function reloadServerStatus() {
        serverDeviceModel.clear()
        for (let index = 0; index < backend.preferenceServerDeviceCount(); ++index)
            serverDeviceModel.append({ label: backend.preferenceServerDeviceLabel(index) })
        serverDevice.currentIndex = backend.preferenceServerSelectedDevice()
        serverVersion.text = backend.preferenceServerDetail("version")
        serverProtocol.text = backend.preferenceServerDetail("protocol")
        serverTorch.text = backend.preferenceServerDetail("torch")
        serverCuda.text = backend.preferenceServerDetail("cuda")
        serverJobs.text = backend.preferenceServerDetail("jobs")
        serverReservations.text = backend.preferenceServerDetail("reservations")
        serverWorkers.text = backend.preferenceServerDetail("workers")
        serverFeatures.text = backend.preferenceServerDetail("features")
    }
    function showConnectorError(message) {
        if (message.length === 0)
            return false
        settingsErrorText.text = message
        settingsError.open()
        return true
    }
    function openPreferences() {
        trustedLocationCount = backend.refreshTrustedLocations()
        loading = true
        configure(captionSize, "caption-font-size")
        captionColor.text = backend.preferenceValue("caption-background-color")
        fontFamily.text = backend.preferenceValue("default-text-font-family")
        configure(visualDuration, "default-visual-duration")
        configure(snapRadius, "timeline-snap-radius")
        previewZoomPan.checked = backend.preferenceValue("preview-zoom-pan") === "true"
        configure(previewPadding, "preview-padding")
        configure(previewShadow, "preview-shadow-size")
        previewUpsample.currentIndex = numberValue("preview-upsample-method")
        previewDownsample.currentIndex = numberValue("preview-downsample-method")
        configure(decoderPool, "temporal-decoder-pool-size")
        configure(gpuMemory, "gpu-host-memory")
        blenderPath.text = backend.preferenceValue("blender-binary")
        reloadServers()
        loading = false
        show()
        raise()
        requestActivate()
    }

    Shortcut { sequence: StandardKey.Cancel; onActivated: root.close() }

    Connections {
        target: backend
        function onPreferenceBlenderFinished(error) {
            root.blenderBusy = false
            if (!root.showConnectorError(error))
                blenderPath.text = backend.preferenceValue("blender-binary")
        }
        function onPreferenceServerStatusChanged(error) {
            root.serverBusy = false
            root.serverError = error
            if (error.length === 0)
                root.reloadServerStatus()
            else
                root.clearServerStatus()
        }
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        Pane {
            Layout.fillHeight: true
            Layout.preferredWidth: 210
            Layout.minimumWidth: 210
            padding: 10

            ColumnLayout {
                anchors.fill: parent
                spacing: 4

                ItemDelegate {
                    Layout.fillWidth: true
                    text: backend.translate("Appearance")
                    icon.name: "preferences-desktop-theme"
                    highlighted: root.currentPage === 0
                    onClicked: root.currentPage = 0
                }
                ItemDelegate {
                    Layout.fillWidth: true
                    text: backend.translate("Performance")
                    icon.name: "speedometer"
                    highlighted: root.currentPage === 1
                    onClicked: root.currentPage = 1
                }
                ItemDelegate {
                    Layout.fillWidth: true
                    text: backend.translate("External")
                    icon.name: "application-x-executable"
                    highlighted: root.currentPage === 2
                    onClicked: root.currentPage = 2
                }
                ItemDelegate {
                    Layout.fillWidth: true
                    text: backend.translate("Security")
                    icon.name: "security-high-symbolic"
                    highlighted: root.currentPage === 3
                    onClicked: root.currentPage = 3
                }
                Item { Layout.fillHeight: true }
            }
        }

        Rectangle { Layout.fillHeight: true; Layout.preferredWidth: 1; color: palette.mid }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            StackLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                currentIndex: root.currentPage

                ScrollView {
                    id: appearanceScroll
                    clip: true
                    padding: 18
                    contentWidth: availableWidth
                    ColumnLayout {
                        width: appearanceScroll.availableWidth
                        spacing: 16

                        GroupBox {
                            title: backend.translate("Captions")
                            Layout.fillWidth: true
                            GridLayout {
                                anchors.fill: parent
                                columns: 2
                                columnSpacing: 24
                                rowSpacing: 12
                                Label { text: backend.translate("Font Size") }
                                SpinBox {
                                    id: captionSize
                                    Layout.fillWidth: true
                                    editable: true
                                    onValueModified: if (!root.loading)
                                        backend.setPreferenceValue("caption-font-size", String(value))
                                }
                                Label { text: backend.translate("Background Color") }
                                RowLayout {
                                    Layout.fillWidth: true
                                    TextField { id: captionColor; Layout.fillWidth: true; readOnly: true }
                                    Button { text: backend.translate("Choose…"); onClicked: captionColorDialog.open() }
                                }
                            }
                        }

                        GroupBox {
                            title: backend.translate("Text")
                            Layout.fillWidth: true
                            GridLayout {
                                anchors.fill: parent
                                columns: 2
                                columnSpacing: 24
                                Label { text: backend.translate("Default Text Font") }
                                RowLayout {
                                    Layout.fillWidth: true
                                    TextField {
                                        id: fontFamily
                                        Layout.fillWidth: true
                                        onEditingFinished: if (!root.loading && text.length > 0)
                                            backend.setPreferenceValue("default-text-font-family", text)
                                    }
                                    Button { text: backend.translate("Choose…"); onClicked: fontDialog.open() }
                                }
                            }
                        }

                        GroupBox {
                            title: backend.translate("Timeline")
                            Layout.fillWidth: true
                            GridLayout {
                                anchors.fill: parent
                                columns: 2
                                columnSpacing: 24
                                rowSpacing: 12
                                Label { text: backend.translate("Default Visual Duration") }
                                SpinBox {
                                    id: visualDuration
                                    property int valueScale: 1
                                    Layout.fillWidth: true
                                    editable: true
                                    textFromValue: (value, locale) => Number(value / valueScale).toLocaleString(locale, "f", 1) + backend.translate(" s")
                                    valueFromText: (text, locale) => Math.round(Number.fromLocaleString(locale, text.replace(/[^0-9.,-]/g, "")) * valueScale)
                                    onValueModified: if (!root.loading)
                                        backend.setPreferenceValue("default-visual-duration", String(value))
                                }
                                Label { text: backend.translate("Snap Attraction Radius") }
                                SpinBox {
                                    id: snapRadius
                                    Layout.fillWidth: true
                                    editable: true
                                    onValueModified: if (!root.loading)
                                        backend.setPreferenceValue("timeline-snap-radius", String(value))
                                }
                            }
                        }

                        GroupBox {
                            title: backend.translate("Preview")
                            Layout.fillWidth: true
                            GridLayout {
                                anchors.fill: parent
                                columns: 2
                                columnSpacing: 24
                                rowSpacing: 12
                                Label { text: backend.translate("Preview zoom and pan") }
                                Switch {
                                    id: previewZoomPan
                                    ToolTip.text: backend.translate("Scroll to zoom; middle-drag to pan; middle-click to fit.")
                                    ToolTip.visible: hovered
                                    onToggled: if (!root.loading)
                                        backend.setPreferenceValue("preview-zoom-pan", String(checked))
                                }
                                Label { text: backend.translate("Padding") }
                                SpinBox {
                                    id: previewPadding
                                    Layout.fillWidth: true
                                    editable: true
                                    onValueModified: if (!root.loading)
                                        backend.setPreferenceValue("preview-padding", String(value))
                                }
                                Label { text: backend.translate("Shadow Size") }
                                SpinBox {
                                    id: previewShadow
                                    Layout.fillWidth: true
                                    editable: true
                                    onValueModified: if (!root.loading)
                                        backend.setPreferenceValue("preview-shadow-size", String(value))
                                }
                                Label { text: backend.translate("Upsample Method") }
                                ComboBox {
                                    id: previewUpsample
                                    Layout.fillWidth: true
                                    model: [backend.translate("Nearest"), backend.translate("Bilinear")]
                                    onActivated: if (!root.loading)
                                        backend.setPreferenceValue("preview-upsample-method", String(currentIndex))
                                }
                                Label { text: backend.translate("Downsample Method") }
                                ComboBox {
                                    id: previewDownsample
                                    Layout.fillWidth: true
                                    model: [backend.translate("Nearest"), backend.translate("Bilinear"), backend.translate("Trilinear")]
                                    onActivated: if (!root.loading)
                                        backend.setPreferenceValue("preview-downsample-method", String(currentIndex))
                                }
                            }
                        }
                        Item { Layout.fillHeight: true }
                    }
                }

                ScrollView {
                    id: performanceScroll
                    clip: true
                    padding: 18
                    contentWidth: availableWidth
                    ColumnLayout {
                        width: performanceScroll.availableWidth
                        spacing: 16
                        GroupBox {
                            title: backend.translate("Playback and Rendering")
                            Layout.fillWidth: true
                            GridLayout {
                                anchors.fill: parent
                                columns: 2
                                columnSpacing: 24
                                rowSpacing: 12
                                Label { text: backend.translate("Temporal Decoder Pool Size") }
                                SpinBox {
                                    id: decoderPool
                                    Layout.fillWidth: true
                                    editable: true
                                    onValueModified: if (!root.loading)
                                        backend.setPreferenceValue("temporal-decoder-pool-size", String(value))
                                }
                                Label { text: backend.translate("GPU Host Memory Budget") }
                                SpinBox {
                                    id: gpuMemory
                                    property int valueScale: 1
                                    Layout.fillWidth: true
                                    editable: true
                                    textFromValue: (value, locale) => Number(value / valueScale).toLocaleString(locale, "f", 2) + backend.translate(" GiB")
                                    valueFromText: (text, locale) => Math.round(Number.fromLocaleString(locale, text.replace(/[^0-9.,-]/g, "")) * valueScale)
                                    onValueModified: if (!root.loading)
                                        backend.setPreferenceValue("gpu-host-memory", String(value))
                                }
                            }
                        }
                        Item { Layout.fillHeight: true }
                    }
                }

                ScrollView {
                    id: servicesScroll
                    clip: true
                    padding: 18
                    contentWidth: availableWidth
                    ColumnLayout {
                        width: servicesScroll.availableWidth
                        spacing: 16
                        GroupBox {
                            title: backend.translate("Blender")
                            Layout.fillWidth: true
                            RowLayout {
                                anchors.fill: parent
                                TextField { id: blenderPath; Layout.fillWidth: true; readOnly: true; placeholderText: backend.translate("Not configured") }
                                Button {
                                    text: backend.translate("Choose…")
                                    enabled: !root.blenderBusy
                                    onClicked: root.blenderBusy = backend.choosePreferenceBlenderBinary()
                                }
                                Button {
                                    text: backend.translate("Clear")
                                    enabled: !root.blenderBusy && blenderPath.text.length > 0
                                    onClicked: { backend.clearPreferenceBlenderBinary(); blenderPath.clear() }
                                }
                            }
                        }
                        GroupBox {
                            title: backend.translate("Compute Server")
                            Layout.fillWidth: true
                            ColumnLayout {
                                anchors.fill: parent
                                ComboBox {
                                    id: serverChoice
                                    Layout.fillWidth: true
                                    model: ListModel { id: serverModel }
                                    textRole: "url"
                                    onActivated: {
                                        backend.selectPreferenceServer(currentIndex)
                                        serverUrl.text = currentText
                                        root.serverBusy = true
                                        root.serverError = ""
                                        root.clearServerStatus()
                                        backend.refreshPreferenceServerStatus()
                                    }
                                }
                                TextField { id: serverUrl; Layout.fillWidth: true; placeholderText: "https://server.example" }
                                RowLayout {
                                    Button {
                                        text: backend.translate("Add")
                                        onClicked: if (!root.showConnectorError(backend.addPreferenceServer(serverUrl.text)))
                                            root.reloadServers()
                                    }
                                    Button {
                                        text: backend.translate("Save")
                                        enabled: serverChoice.currentIndex >= 0
                                        onClicked: if (!root.showConnectorError(backend.editPreferenceServer(serverChoice.currentIndex, serverUrl.text)))
                                            root.reloadServers()
                                    }
                                    Button {
                                        text: backend.translate("Remove")
                                        enabled: serverModel.count > 1 && serverChoice.currentIndex >= 0
                                        onClicked: {
                                            backend.removePreferenceServer(serverChoice.currentIndex)
                                            root.reloadServers()
                                        }
                                    }
                                    Item { Layout.fillWidth: true }
                                }
                            }
                        }
                        GroupBox {
                            title: backend.translate("Selected Server")
                            Layout.fillWidth: true
                            GridLayout {
                                anchors.fill: parent
                                columns: 2
                                columnSpacing: 24
                                rowSpacing: 10
                                Label { text: backend.translate("Status") }
                                RowLayout {
                                    Layout.fillWidth: true
                                    BusyIndicator { running: root.serverBusy; visible: running; implicitWidth: 22; implicitHeight: 22 }
                                    Label {
                                        Layout.fillWidth: true
                                        text: root.serverBusy ? backend.translate("Checking…")
                                            : root.serverError.length > 0 ? root.serverError : backend.translate("Available")
                                        wrapMode: Text.Wrap
                                    }
                                }
                                Label { text: backend.translate("Version") }
                                Label { id: serverVersion; Layout.fillWidth: true }
                                Label { text: backend.translate("Protocol") }
                                Label { id: serverProtocol; Layout.fillWidth: true }
                                Label { text: backend.translate("Torch") }
                                Label { id: serverTorch; Layout.fillWidth: true }
                                Label { text: backend.translate("CUDA") }
                                Label { id: serverCuda; Layout.fillWidth: true }
                                Label { text: backend.translate("Device") }
                                ComboBox {
                                    id: serverDevice
                                    Layout.fillWidth: true
                                    enabled: !root.serverBusy && currentIndex >= 0
                                    model: ListModel { id: serverDeviceModel }
                                    textRole: "label"
                                    onActivated: {
                                        root.serverBusy = true
                                        backend.selectPreferenceServerDevice(currentIndex)
                                    }
                                }
                                Label { text: backend.translate("Jobs") }
                                Label { id: serverJobs; Layout.fillWidth: true }
                                Label { text: backend.translate("Reserved memory") }
                                Label { id: serverReservations; Layout.fillWidth: true }
                                Label { text: backend.translate("Workers") }
                                Label { id: serverWorkers; Layout.fillWidth: true; wrapMode: Text.Wrap }
                                Label { text: backend.translate("Available") }
                                Label { id: serverFeatures; Layout.fillWidth: true; wrapMode: Text.Wrap }
                            }
                        }
                        Item { Layout.fillHeight: true }
                    }
                }

                ScrollView {
                    id: securityScroll
                    clip: true
                    padding: 18
                    contentWidth: availableWidth
                    ColumnLayout {
                        width: securityScroll.availableWidth
                        spacing: 16
                        GroupBox {
                            title: backend.translate("Trusted executable sources")
                            Layout.fillWidth: true
                            ColumnLayout {
                                anchors.fill: parent
                                Label { text: backend.translate("Removing trust stops affected workers. Folder entries include subfolders."); wrapMode: Text.WordWrap; Layout.fillWidth: true }
                                Repeater {
                                    model: root.trustedLocationCount
                                    RowLayout {
                                        required property int index
                                        Layout.fillWidth: true
                                        Label { text: backend.trustedLocation(index); textFormat: Text.PlainText; wrapMode: Text.WrapAnywhere; Layout.fillWidth: true }
                                        Button { text: backend.translate("Remove"); onClicked: { const count = backend.revokeTrust(index); root.trustedLocationCount = 0; root.trustedLocationCount = count } }
                                    }
                                }
                                Label { visible: root.trustedLocationCount === 0; text: backend.translate("No trusted locations") }
                            }
                        }
                        Item { Layout.fillHeight: true }
                    }
                }
            }
        }
    }

    ColorDialog {
        id: captionColorDialog
        title: backend.translate("Caption Background Color")
        options: ColorDialog.ShowAlphaChannel
        onAccepted: {
            captionColor.text = String(selectedColor)
            backend.setPreferenceValue("caption-background-color", captionColor.text)
        }
    }
    FontDialog {
        id: fontDialog
        title: backend.translate("Default Text Font")
        onAccepted: {
            fontFamily.text = selectedFont.family
            backend.setPreferenceValue("default-text-font-family", fontFamily.text)
        }
    }
    Dialog {
        id: settingsError
        anchors.centerIn: parent
        modal: true
        title: backend.translate("Could Not Save Preference")
        standardButtons: Dialog.Ok
        Label { id: settingsErrorText; wrapMode: Text.Wrap; width: 420 }
    }
}
