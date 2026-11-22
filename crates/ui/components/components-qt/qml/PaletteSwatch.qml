import QtQuick
import QtQuick.Controls
import QtQuick.Effects

FocusScope {
    id: root
    required property color color
    required property string label
    property bool selected: false
    property int familyPosition: -1
    signal activated

    implicitWidth: 48
    implicitHeight: 32
    activeFocusOnTab: true
    Accessible.role: Accessible.RadioButton
    Accessible.name: label
    Accessible.checked: selected
    Keys.onSpacePressed: activated()
    Keys.onReturnPressed: activated()
    Keys.onEnterPressed: activated()

    TransparentColorPreview {
        anchors.fill: parent
        visible: root.familyPosition < 0
        color: root.color
        radius: 6
        checkerCellSize: 10
        borderColor: root.activeFocus ? palette.highlight : "transparent"
    }

    Rectangle {
        anchors.fill: parent
        visible: root.familyPosition >= 0
        color: root.color
        topLeftRadius: root.familyPosition === 0 || root.familyPosition < 0 ? 6 : 0
        topRightRadius: topLeftRadius
        bottomLeftRadius: root.familyPosition === 4 || root.familyPosition < 0 ? 6 : 0
        bottomRightRadius: bottomLeftRadius
        border.width: root.activeFocus ? 2 : 0
        border.color: palette.highlight
        antialiasing: true
    }

    Image {
        id: selectedIcon
        anchors.centerIn: parent
        width: 16
        height: 16
        sourceSize.width: width
        sourceSize.height: height
        source: "qrc:/shrimply/icons/object-select-symbolic.svg"
        visible: false
    }
    MultiEffect {
        anchors.fill: selectedIcon
        source: selectedIcon
        visible: root.selected
        colorization: 1
        colorizationColor: root.color.r * 0.30 + root.color.g * 0.59
                           + root.color.b * 0.11 > 0.5 ? "#2e3436" : "white"
    }

    TapHandler { onTapped: root.activated() }
}
