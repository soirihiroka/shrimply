import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.shrimply.components
import dev.shrimply.export

Item {
    id: root

    property var owner
    readonly property bool busy: exportBackend.busy

    function openVideo() {
        if (exportBackend.busy)
            return
        exportBackend.resetVideo()
        videoWindow.show()
        videoWindow.raise()
        videoWindow.requestActivate()
    }

    function openCaptions() {
        if (exportBackend.busy)
            return
        mergeCaptions.checked = true
        captionsWindow.show()
        captionsWindow.raise()
        captionsWindow.requestActivate()
    }

    function exportJson() {
        if (exportBackend.startJson())
            progressWindow.show()
    }

    ExportBackend {
        id: exportBackend

        onSucceeded: function(title) {
            progressWindow.close()
            successWindow.message = title
            successWindow.show()
        }
        onFailed: function(heading, body) {
            progressWindow.close()
            errorWindow.heading = heading
            errorWindow.message = body
            errorWindow.show()
        }
        onCanceled: progressWindow.close()
        onOpenPath: function(url) { Qt.openUrlExternally(url) }
    }

    Timer {
        interval: 100
        repeat: true
        running: exportBackend.busy
        onTriggered: exportBackend.poll()
    }

    ApplicationWindow {
        id: videoWindow
        title: exportBackend.translate("Export Video")
        transientParent: root.owner
        modality: Qt.WindowModal
        flags: Qt.Dialog
        palette: root.owner.palette
        width: 760
        height: 820
        minimumWidth: 600
        minimumHeight: 560

        ColumnLayout {
            anchors.fill: parent
            spacing: 0

            ScrollView {
                id: videoScroll
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                padding: 18
                contentWidth: availableWidth

                ColumnLayout {
                    width: videoScroll.availableWidth
                    spacing: 16

                    GroupBox {
                        title: exportBackend.translate("Video format")
                        Layout.fillWidth: true

                        ColumnLayout {
                            anchors.fill: parent
                            spacing: 12

                            ControlRow {
                                label: exportBackend.translate("Export format")
                                ComboBox {
                                    model: ["H.264", "H.265", "GIF"]
                                    currentIndex: exportBackend.videoCodec
                                    onActivated: exportBackend.videoCodec = currentIndex
                                }
                            }
                            ControlRow {
                                label: exportBackend.translate("Container")
                                visible: exportBackend.videoCodec !== 2
                                ComboBox {
                                    model: ["MP4", "MKV"]
                                    currentIndex: exportBackend.container
                                    onActivated: exportBackend.container = currentIndex
                                }
                            }
                        }
                    }

                    GroupBox {
                        title: exportBackend.translate("Encoder settings")
                        Layout.fillWidth: true
                        visible: exportBackend.videoCodec !== 2

                        ColumnLayout {
                            anchors.fill: parent
                            spacing: 12

                            ControlRow {
                                label: exportBackend.translate("Rate Control")
                                ComboBox {
                                    model: [
                                        exportBackend.translate("Constant QP"),
                                        exportBackend.translate("Constant Bitrate"),
                                        exportBackend.translate("Variable Bitrate"),
                                        exportBackend.translate("Variable Bitrate with Target Quality"),
                                        exportBackend.translate("Lossless")
                                    ]
                                    currentIndex: exportBackend.rateControl
                                    onActivated: exportBackend.rateControl = currentIndex
                                }
                            }
                            ControlRow {
                                label: exportBackend.translate("Video Bitrate")
                                visible: exportBackend.rateControl >= 1 && exportBackend.rateControl <= 3
                                NumberPicker {
                                    minimum: 50
                                    maximum: 250000
                                    dragStep: 50
                                    digits: 0
                                    value: exportBackend.bitrateKbps
                                    onEdited: function(value) { exportBackend.bitrateKbps = value }
                                }
                            }
                            ControlRow {
                                label: exportBackend.translate("Max Video Bitrate")
                                visible: exportBackend.rateControl === 2 || exportBackend.rateControl === 3
                                NumberPicker {
                                    minimum: 50
                                    maximum: 250000
                                    dragStep: 50
                                    digits: 0
                                    value: exportBackend.maxBitrateKbps
                                    onEdited: function(value) { exportBackend.maxBitrateKbps = value }
                                }
                            }
                            ControlRow {
                                label: exportBackend.translate("Target Quality")
                                visible: exportBackend.rateControl === 3
                                NumberPicker {
                                    minimum: 1
                                    maximum: 51
                                    digits: 0
                                    value: exportBackend.targetQuality
                                    onEdited: function(value) { exportBackend.targetQuality = value }
                                }
                            }
                            ControlRow {
                                label: exportBackend.translate("Constant QP")
                                visible: exportBackend.rateControl === 0
                                NumberPicker {
                                    minimum: 0
                                    maximum: 51
                                    digits: 0
                                    value: exportBackend.constantQp
                                    onEdited: function(value) { exportBackend.constantQp = value }
                                }
                            }
                            ControlRow {
                                label: exportBackend.translate("Keyframe Interval")
                                NumberPicker {
                                    minimum: 0
                                    maximum: 10
                                    digits: 0
                                    unitName: "s"
                                    value: exportBackend.keyframeIntervalSeconds
                                    onEdited: function(value) { exportBackend.keyframeIntervalSeconds = value }
                                }
                            }
                            ControlRow {
                                label: exportBackend.translate("Preset")
                                ComboBox {
                                    model: [
                                        exportBackend.translate("P1: Fastest (Lowest Quality)"),
                                        exportBackend.translate("P2: Faster (Lower Quality)"),
                                        exportBackend.translate("P3: Fast (Low Quality)"),
                                        exportBackend.translate("P4: Medium (Medium Quality)"),
                                        exportBackend.translate("P5: Slow (Good Quality)"),
                                        exportBackend.translate("P6: Slower (Better Quality)"),
                                        exportBackend.translate("P7: Slowest (Best Quality)")
                                    ]
                                    currentIndex: exportBackend.preset
                                    onActivated: exportBackend.preset = currentIndex
                                }
                            }
                            ControlRow {
                                label: exportBackend.translate("Tuning")
                                visible: exportBackend.rateControl !== 4
                                ComboBox {
                                    model: [
                                        exportBackend.translate("Ultra High Quality"),
                                        exportBackend.translate("High Quality"),
                                        exportBackend.translate("Low Latency"),
                                        exportBackend.translate("Ultra Low Latency")
                                    ]
                                    currentIndex: exportBackend.tuning
                                    onActivated: exportBackend.tuning = currentIndex
                                }
                            }
                            ControlRow {
                                label: exportBackend.translate("Multi Pass")
                                ComboBox {
                                    model: [
                                        exportBackend.translate("Single Pass"),
                                        exportBackend.translate("Two Passes (Quarter Resolution)"),
                                        exportBackend.translate("Two Passes (Full Resolution)")
                                    ]
                                    currentIndex: exportBackend.multipass
                                    onActivated: exportBackend.multipass = currentIndex
                                }
                            }
                            ControlRow {
                                label: exportBackend.translate("Profile")
                                ComboBox {
                                    model: ["Main", "Main10"]
                                    currentIndex: exportBackend.profile
                                    onActivated: exportBackend.profile = currentIndex
                                }
                            }
                            SwitchRow {
                                label: exportBackend.translate("Look-ahead")
                                active: exportBackend.lookAhead
                                onToggled: function(active) { exportBackend.lookAhead = active }
                            }
                            SwitchRow {
                                label: exportBackend.translate("Adaptive Quantization")
                                active: exportBackend.adaptiveQuantization
                                onToggled: function(active) { exportBackend.adaptiveQuantization = active }
                            }
                            ControlRow {
                                label: exportBackend.translate("B Frames")
                                NumberPicker {
                                    minimum: 0
                                    maximum: 16
                                    digits: 0
                                    value: exportBackend.bFrames
                                    onEdited: function(value) { exportBackend.bFrames = value }
                                }
                            }
                            SwitchRow {
                                label: exportBackend.translate("B Frame as Reference")
                                active: exportBackend.bFrameAsReference
                                onToggled: function(active) { exportBackend.bFrameAsReference = active }
                            }
                        }
                    }

                    GroupBox {
                        title: exportBackend.translate("Output")
                        Layout.fillWidth: true

                        ColumnLayout {
                            anchors.fill: parent
                            spacing: 12

                            ControlRow {
                                label: exportBackend.translate("Frame rate")
                                ComboBox {
                                    model: exportBackend.frameRateLabels
                                    currentIndex: exportBackend.frameRateIndex
                                    onActivated: exportBackend.frameRateIndex = currentIndex
                                }
                            }
                            ControlRow {
                                label: exportBackend.translate("Background Alpha")
                                visible: exportBackend.videoCodec === 2
                                NumberPicker {
                                    minimum: 0
                                    maximum: 255
                                    digits: 0
                                    value: exportBackend.backgroundAlpha
                                    onEdited: function(value) { exportBackend.backgroundAlpha = value }
                                }
                            }
                        }
                    }

                    GroupBox {
                        title: exportBackend.translate("Audio")
                        Layout.fillWidth: true
                        visible: exportBackend.videoCodec !== 2

                        ColumnLayout {
                            anchors.fill: parent
                            spacing: 12

                            ControlRow {
                                label: exportBackend.translate("Audio Encoder")
                                ComboBox {
                                    model: ["AAC", "Opus"]
                                    currentIndex: exportBackend.audioEncoder
                                    onActivated: exportBackend.audioEncoder = currentIndex
                                }
                            }
                            ControlRow {
                                label: exportBackend.translate("Audio Sample Rate")
                                ComboBox {
                                    model: ["44100", "48000", "96000"]
                                    currentIndex: exportBackend.audioSampleRate
                                    onActivated: exportBackend.audioSampleRate = currentIndex
                                }
                            }
                            ControlRow {
                                label: exportBackend.translate("Audio Bitrate")
                                NumberPicker {
                                    minimum: 32
                                    maximum: 512
                                    dragStep: 8
                                    digits: 0
                                    unitName: "kbps"
                                    value: exportBackend.audioBitrateKbps
                                    onEdited: function(value) { exportBackend.audioBitrateKbps = value }
                                }
                            }
                        }
                    }
                }
            }

            Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: palette.mid }
            RowLayout {
                Layout.fillWidth: true
                Layout.margins: 12
                Item { Layout.fillWidth: true }
                Button { text: exportBackend.translate("Cancel"); onClicked: videoWindow.close() }
                Button {
                    text: exportBackend.translate("Export")
                    highlighted: true
                    onClicked: {
                        if (exportBackend.startVideo()) {
                            videoWindow.close()
                            progressWindow.show()
                        }
                    }
                }
            }
        }
    }

    ApplicationWindow {
        id: captionsWindow
        title: exportBackend.translate("Export Captions")
        transientParent: root.owner
        modality: Qt.WindowModal
        flags: Qt.Dialog
        palette: root.owner.palette
        width: 440
        height: 230

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 18
            spacing: 12

            RadioButton {
                id: mergeCaptions
                text: exportBackend.translate("Merge into one file")
                checked: true
            }
            RadioButton {
                text: exportBackend.translate("Export each track separately")
            }
            Item { Layout.fillHeight: true }
            RowLayout {
                Layout.fillWidth: true
                Item { Layout.fillWidth: true }
                Button { text: exportBackend.translate("Cancel"); onClicked: captionsWindow.close() }
                Button {
                    text: exportBackend.translate("Export YTT")
                    highlighted: true
                    onClicked: {
                        if (exportBackend.startCaptions(!mergeCaptions.checked)) {
                            captionsWindow.close()
                            progressWindow.show()
                        }
                    }
                }
            }
        }
    }

    ApplicationWindow {
        id: progressWindow
        title: exportBackend.translate("Exporting")
        transientParent: root.owner
        modality: Qt.WindowModal
        flags: Qt.Dialog
        palette: root.owner.palette
        width: 520
        height: 190
        onClosing: function(close) {
            if (exportBackend.busy)
                exportBackend.cancel()
        }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 24
            spacing: 16

            Label {
                Layout.fillWidth: true
                text: exportBackend.status
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
            }
            ProgressBar {
                Layout.fillWidth: true
                from: 0
                to: 1
                value: exportBackend.progress
                indeterminate: !exportBackend.progressDeterminate
            }
            Label {
                Layout.fillWidth: true
                text: exportBackend.progressText
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
            }
            Button {
                Layout.alignment: Qt.AlignRight
                text: exportBackend.translate("Cancel")
                enabled: exportBackend.busy
                onClicked: exportBackend.cancel()
            }
        }
    }

    ApplicationWindow {
        id: successWindow
        property string message
        title: exportBackend.translate("Export Complete")
        transientParent: root.owner
        modality: Qt.WindowModal
        flags: Qt.Dialog
        palette: root.owner.palette
        width: 480
        height: 180

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 18
            Label { Layout.fillWidth: true; text: successWindow.message; wrapMode: Text.Wrap }
            Item { Layout.fillHeight: true }
            RowLayout {
                Layout.fillWidth: true
                Item { Layout.fillWidth: true }
                Button { text: exportBackend.translate("Close"); onClicked: successWindow.close() }
                Button {
                    text: exportBackend.translate("Show in Files")
                    highlighted: true
                    onClicked: {
                        successWindow.close()
                        exportBackend.revealLastOutput()
                    }
                }
            }
        }
    }

    ApplicationWindow {
        id: errorWindow
        property string heading
        property string message
        title: heading
        transientParent: root.owner
        modality: Qt.WindowModal
        flags: Qt.Dialog
        palette: root.owner.palette
        width: 520
        height: 220

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 18
            Label { Layout.fillWidth: true; text: errorWindow.message; wrapMode: Text.Wrap }
            Item { Layout.fillHeight: true }
            Button {
                Layout.alignment: Qt.AlignRight
                text: exportBackend.translate("Close")
                onClicked: errorWindow.close()
            }
        }
    }
}
