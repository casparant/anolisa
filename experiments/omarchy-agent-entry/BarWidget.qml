import QtQuick
import Quickshell

Item {
    id: root
    property QtObject bar: null
    property string moduleName: "anolisa.agent-entry"
    property var settings: ({})
    readonly property bool vertical: bar ? bar.vertical : false
    implicitWidth: vertical ? 28 : 104
    implicitHeight: 28

    Rectangle {
        anchors.fill: parent
        radius: 7
        color: hover.containsMouse ? "#344364" : "#25334d"
        border.color: "#667bba"
        Text {
            anchors.centerIn: parent
            text: root.vertical ? "A" : "✦  Agent 入口"
            color: "#e9efff"
            font.pixelSize: 12
            font.bold: true
        }
        MouseArea {
            id: hover
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: Quickshell.execDetached(["omarchy-shell", "shell", "toggle", root.moduleName, "{}"])
        }
    }
}
