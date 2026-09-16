import QtQuick
import Quickshell
import Quickshell.Io

Item {
    id: root
    property var shell: null
    property var manifest: null
    readonly property bool opened: window.visible
    readonly property string helper: decodeURIComponent(Qt.resolvedUrl("agent_entry.py").toString().replace(/^file:\/\//, ""))
    property string statusError: ""
    property string actionError: ""

    function open(payload) {
        window.visible = true
        refresh()
    }
    function close() { window.visible = false }
    function refresh() {
        if (statusProcess.running) return
        statusError = ""
        statusProcess.command = ["python3", helper, "status"]
        statusProcess.running = true
    }
    function act(args) {
        if (actionProcess.running) return
        actionError = ""
        content.message = "正在处理…"
        actionProcess.command = ["python3", helper].concat(args)
        actionProcess.running = true
    }

    Process {
        id: statusProcess
        stdout: StdioCollector {
            onStreamFinished: {
                try {
                    var result = JSON.parse(text)
                    if (result.error) content.message = result.error
                    else {
                        content.agents = result.agents
                        content.project = result.project
                        content.configPath = result.config
                    }
                } catch (e) { content.message = "状态读取失败：" + String(e) }
            }
        }
        stderr: StdioCollector { onStreamFinished: root.statusError = text }
        onExited: function(code, status) {
            if (code !== 0 && root.statusError) content.message = root.statusError
        }
    }
    Process {
        id: actionProcess
        stdout: StdioCollector {
            onStreamFinished: {
                try {
                    var result = JSON.parse(text)
                    content.message = result.error || result.message || "完成"
                    if (result.project) content.project = result.project
                } catch (e) { content.message = "操作返回格式异常：" + String(e) }
            }
        }
        stderr: StdioCollector { onStreamFinished: root.actionError = text }
        onExited: function(code, status) {
            if (code !== 0 && root.actionError) content.message = root.actionError
            refreshTimer.restart()
        }
    }
    Timer { id: refreshTimer; interval: 200; onTriggered: root.refresh() }

    FloatingWindow {
        id: window
        title: "ANOLISA · Agent 入口"
        visible: false
        implicitWidth: 1160
        implicitHeight: 740
        minimumSize: Qt.size(800, 560)
        color: "#111827"
        DashboardContent {
            id: content
            anchors.fill: parent
            busy: statusProcess.running || actionProcess.running
            onActionRequested: function(args) { root.act(args) }
            onRefreshRequested: root.refresh()
            onCloseRequested: root.close()
        }
    }
}
