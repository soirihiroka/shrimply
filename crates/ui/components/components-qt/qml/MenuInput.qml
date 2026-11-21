import QtQuick

TapHandler {
    id: root
    property bool touch: false
    acceptedDevices: PointerDevice.TouchScreen | PointerDevice.Stylus
    onPressedChanged: if (pressed) touch = true
    onCanceled: touch = false

    function takeTouch() {
        const source = touch
        touch = false
        return source
    }
}
