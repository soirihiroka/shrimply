import QtQuick
import dev.shrimply.components
import QtQuick.Controls
import QtQuick.Layouts

RowLayout {
    id: root
    property string text: ""
    property string secondaryText: ""
    signal triggered()
    signal secondaryTriggered()
    spacing: 0

    Button {
        Layout.fillWidth: true
        text: ComponentTranslations.text(root.text)
        onClicked: root.triggered()
    }
    ToolButton {
        id: secondary
        text: "▾"
        MenuInput { id: menuInput }
        onClicked: menu.presentForInput(secondary, menuInput.takeTouch())
        AdaptiveMenu {
            id: menu
            MenuItem {
                text: ComponentTranslations.text(root.secondaryText)
                onTriggered: {
                    menu.close()
                    root.secondaryTriggered()
                }
            }
        }
    }
}
