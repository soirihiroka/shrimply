import QtQuick
import dev.shrimply.components
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import QtQuick.Window

Item {
    id: root
    property color color: "#000000"
    property string title: "Select color"
    property bool withAlpha: true
    signal selected(color color)
    signal screenColorFailed(string message)
    implicitWidth: button.implicitWidth
    implicitHeight: button.implicitHeight

    SystemPalette { id: systemPalette }

    ColorPickerBackend {
        id: backend
        onSelected: function(value) { Qt.callLater(function() { root.selected(value) }) }
        onScreenColorFailed: function(message) { root.screenColorFailed(message) }
    }
    Component.onCompleted: backend.configure(root.color, root.withAlpha)
    onColorChanged: backend.configure(root.color, root.withAlpha)
    onWithAlphaChanged: backend.configure(root.color, root.withAlpha)

    Button {
        id: button
        anchors.fill: parent
        onClicked: {
            picker.show()
            picker.requestActivate()
        }
        contentItem: Item {
            implicitWidth: buttonContent.implicitWidth
            implicitHeight: buttonContent.implicitHeight
            RowLayout {
                id: buttonContent
                anchors.centerIn: parent
                spacing: 8
                TransparentColorPreview {
                    implicitWidth: 22
                    implicitHeight: 22
                    radius: 11
                    color: backend.color
                }
                Label { text: backend.hex; font.family: "monospace" }
            }
        }
    }

    Window {
        id: picker
        width: 895
        height: 480
        minimumWidth: 895
        minimumHeight: 480
        visible: false
        flags: Qt.Dialog
        modality: Qt.NonModal
        transientParent: root.Window.window
        title: ComponentTranslations.text(root.title)
        color: systemPalette.window
        onVisibleChanged: if (visible) backend.configure(root.color, root.withAlpha)

        ColumnLayout {
            anchors.fill: parent
            anchors.topMargin: 18
            anchors.bottomMargin: 18
            anchors.leftMargin: 24
            anchors.rightMargin: 24
            spacing: 10

            RowLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 24

                ColumnLayout {
                    Layout.alignment: Qt.AlignTop
                    Layout.preferredWidth: 334
                    spacing: 10

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 8
                        ToolButton {
                            icon.source: "qrc:/shrimply/icons/color-select-symbolic.svg"
                            icon.color: palette.buttonText
                            display: AbstractButton.IconOnly
                            enabled: !backend.screenPicking
                            Accessible.name: ComponentTranslations.text("Pick a color from the screen")
                            onClicked: backend.pickScreenColor()
                        }
                        Button {
                            flat: true
                            implicitWidth: 34
                            implicitHeight: 34
                            padding: 6
                            Accessible.name: ComponentTranslations.text("Open the system color picker")
                            onClicked: nativeDialog.open()
                            contentItem: TransparentColorPreview {
                                implicitWidth: 22
                                implicitHeight: 22
                                radius: 11
                                color: backend.draft
                            }
                        }
                        TextField {
                            id: hex
                            Layout.fillWidth: true
                            font.family: "monospace"
                            Accessible.name: ComponentTranslations.text("Hexadecimal color")
                            onAccepted: if (!backend.applyHex(text)) selectAll()
                            onActiveFocusChanged: if (!activeFocus) backend.applyHex(text)
                            Binding {
                                target: hex
                                property: "text"
                                value: backend.hex
                                when: !hex.activeFocus
                            }
                        }
                    }

                    RowLayout {
                        spacing: 10
                        Item {
                            id: hue
                            readonly property real thumbRadius: 10
                            readonly property real trackWidth: 16
                            Layout.preferredHeight: colorPlane.height
                            Layout.preferredWidth: 24
                            Accessible.role: Accessible.Slider
                            Accessible.name: ComponentTranslations.text("Hue")

                            Rectangle {
                                x: (parent.width - width) / 2
                                y: hue.thumbRadius
                                width: hue.trackWidth
                                height: parent.height - hue.thumbRadius * 2
                                radius: width / 2
                                antialiasing: true
                                layer.enabled: true
                                layer.samples: 4
                                gradient: Gradient {
                                    GradientStop { position: 0; color: "#ff0000" }
                                    GradientStop { position: 1 / 6; color: "#ffff00" }
                                    GradientStop { position: 2 / 6; color: "#00ff00" }
                                    GradientStop { position: 3 / 6; color: "#00ffff" }
                                    GradientStop { position: 4 / 6; color: "#0000ff" }
                                    GradientStop { position: 5 / 6; color: "#ff00ff" }
                                    GradientStop { position: 1; color: "#ff0000" }
                                }
                            }
                            ColorSliderThumb {
                                x: (parent.width - width) / 2
                                y: hue.thumbRadius + backend.hue / 360
                                   * (parent.height - hue.thumbRadius * 2) - height / 2
                                radius: hue.thumbRadius
                            }
                            MouseArea {
                                anchors.fill: parent
                                cursorShape: Qt.PointingHandCursor
                                onPressed: function(mouse) { update(mouse.y) }
                                onPositionChanged: function(mouse) {
                                    if (pressed)
                                        update(mouse.y)
                                }
                                function update(y) {
                                    backend.setHsva(Math.max(0, Math.min(1,
                                        (y - hue.thumbRadius)
                                        / (height - hue.thumbRadius * 2))) * 360,
                                        backend.saturation, backend.brightness, backend.alpha)
                                }
                            }
                        }

                        Rectangle {
                            id: colorPlane
                            Layout.preferredWidth: 300
                            Layout.preferredHeight: 300
                            color: Qt.hsva(backend.hue / 360, 1, 1, 1)
                            antialiasing: true
                            Accessible.name: ComponentTranslations.text("Saturation and value")
                            Rectangle {
                                anchors.fill: parent
                                gradient: Gradient {
                                    GradientStop { position: 0; color: "transparent" }
                                    GradientStop { position: 1; color: "white" }
                                }
                            }
                            Rectangle {
                                anchors.fill: parent
                                gradient: Gradient {
                                    orientation: Gradient.Horizontal
                                    GradientStop { position: 0; color: "black" }
                                    GradientStop { position: 1; color: "transparent" }
                                }
                            }
                            Rectangle {
                                x: Math.max(0, Math.min(parent.width - 1,
                                    backend.brightness * (parent.width - 1)))
                                width: 1
                                height: parent.height
                                color: "white"
                            }
                            Rectangle {
                                y: Math.max(0, Math.min(parent.height - 1,
                                    (1 - backend.saturation) * (parent.height - 1)))
                                width: parent.width
                                height: 1
                                color: "white"
                            }
                            MouseArea {
                                anchors.fill: parent
                                cursorShape: Qt.CrossCursor
                                onPressed: function(mouse) { update(mouse) }
                                onPositionChanged: function(mouse) { if (pressed) update(mouse) }
                                function update(mouse) {
                                    backend.setHsva(backend.hue,
                                        1 - Math.max(0, Math.min(1,
                                            mouse.y / (height - 1))),
                                        Math.max(0, Math.min(1,
                                            mouse.x / (width - 1))),
                                        backend.alpha)
                                }
                            }
                        }
                    }

                    Item {
                        visible: root.withAlpha
                        Layout.preferredWidth: colorPlane.width
                        Layout.preferredHeight: visible ? 24 : 0
                        Layout.leftMargin: 34

                        Slider {
                            id: alpha
                            readonly property real thumbRadius: 10
                            anchors.fill: parent
                            leftPadding: 0
                            rightPadding: 0
                            topPadding: 0
                            bottomPadding: 0
                            from: 0
                            to: 1
                            value: backend.alpha
                            Accessible.name: ComponentTranslations.text("Alpha")
                            background: TransparentColorPreview {
                                x: alpha.thumbRadius
                                y: (alpha.height - height) / 2
                                width: alpha.width - alpha.thumbRadius * 2
                                height: 16
                                radius: 8
                                checkerboard: true
                                checkerOriginX: alpha.thumbRadius
                                checkerOriginY: 4
                                fillGradient: Gradient {
                                    orientation: Gradient.Horizontal
                                    GradientStop {
                                        position: 0
                                        color: Qt.rgba(backend.draft.r,
                                            backend.draft.g, backend.draft.b, 0)
                                    }
                                    GradientStop {
                                        position: 1
                                        color: Qt.rgba(backend.draft.r,
                                            backend.draft.g, backend.draft.b, 1)
                                    }
                                }
                            }
                            handle: ColorSliderThumb {
                                x: alpha.position * (alpha.width - width)
                                y: (alpha.height - height) / 2
                                radius: alpha.thumbRadius
                            }
                            onMoved: backend.setHsva(backend.hue, backend.saturation,
                                backend.brightness, value)
                        }
                    }
                }

                Rectangle {
                    Layout.fillHeight: true
                    Layout.preferredWidth: 1
                    color: systemPalette.mid
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    spacing: 6

                    GridLayout {
                        columns: 9
                        rows: 5
                        rowSpacing: 2
                        columnSpacing: 4
                        Repeater {
                            model: backend.paletteCount
                            PaletteSwatch {
                                required property int index
                                Layout.column: Math.floor(index / 5)
                                Layout.row: index % 5
                                color: backend.paletteColor(index)
                                label: backend.paletteLabel(index)
                                selected: color === backend.draft
                                familyPosition: index % 5
                                onActivated: backend.chooseColor(color)
                            }
                        }
                    }

                    Label {
                        text: ComponentTranslations.text("Recent")
                    }
                    RowLayout {
                        spacing: 6
                        Repeater {
                            model: backend.recentCount
                            PaletteSwatch {
                                required property int index
                                color: backend.recentColor(index)
                                label: backend.recentLabel(index)
                                selected: color === backend.draft
                                onActivated: backend.chooseColor(color)
                            }
                        }
                    }
                    Item { Layout.fillHeight: true }
                }
            }

            Button {
                Layout.alignment: Qt.AlignRight
                text: ComponentTranslations.text("Select")
                highlighted: true
                onClicked: {
                    backend.confirm()
                    picker.close()
                }
            }
        }
    }

    ColorDialog {
        id: nativeDialog
        parentWindow: picker
        title: ComponentTranslations.text(root.title)
        selectedColor: backend.draft
        options: root.withAlpha ? ColorDialog.ShowAlphaChannel : ColorDialog.NoOption
        onAccepted: backend.chooseColor(selectedColor)
    }
}
