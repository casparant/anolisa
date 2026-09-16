import QtQuick
import QtQuick.Controls.Basic

Button {
    id: control
    property bool accent: false
    property bool quiet: false
    implicitHeight: 38
    leftPadding: 14
    rightPadding: 14
    contentItem: Text {
        text: control.text
        color: control.enabled ? (control.accent ? "#101827" : "#e3eaff") : "#71809b"
        font.pixelSize: 13
        font.bold: control.accent
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
    }
    background: Rectangle {
        radius: 8
        color: !control.enabled ? "#202a3b" : control.accent ? (control.hovered ? "#b3caff" : "#8babf5") : (control.hovered ? "#33435f" : (control.quiet ? "transparent" : "#28364e"))
        border.width: control.activeFocus ? 2 : 1
        border.color: control.activeFocus ? "#c6d6ff" : (control.accent ? "#8babf5" : "#3b4a64")
    }
}
