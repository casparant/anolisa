import QtQuick
import QtQuick.Controls.Basic

TextField {
    id: control
    implicitHeight: 40
    color: "#e8eefc"
    placeholderTextColor: "#8796b1"
    selectionColor: "#45689d"
    selectedTextColor: "white"
    font.pixelSize: 13
    leftPadding: 12
    rightPadding: 12
    background: Rectangle {
        radius: 8
        color: "#101827"
        border.color: control.activeFocus ? "#8babf5" : "#3b4a64"
    }
}
