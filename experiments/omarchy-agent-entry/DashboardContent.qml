import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts

Rectangle {
    id: root
    color: "#111827"
    property var agents: []
    property string project: ""
    property string configPath: ""
    property string message: "选择一个 Agent 开始工作。首次登录在各自原生界面完成。"
    property bool busy: false
    signal actionRequested(var args)
    signal refreshRequested()
    signal closeRequested()
    function terminalAvailable() {
        for (var i = 0; i < agents.length; i++) if (agents[i].id === "cosh") return agents[i].available
        return false
    }
    Shortcut { sequence: "Escape"; enabled: !registerDialog.visible; onActivated: root.closeRequested() }

    ScrollView {
        anchors.fill: parent
        anchors.margins: 28
        contentWidth: availableWidth
        clip: true
        ColumnLayout {
            width: parent.width
            spacing: 20

            RowLayout {
                Layout.fillWidth: true
                ColumnLayout {
                    spacing: 5
                    Text { text: "ANOLISA  /  AGENT ENTRY"; color: "#8babf5"; font.pixelSize: 12; font.letterSpacing: 1.5 }
                    Text { text: "从这里，开始 Agent 工作"; color: "#f2f5ff"; font.pixelSize: 27; font.bold: true }
                    Text { text: "终端、原生 Agent 和 Web Console，在一个入口找到。"; color: "#9cabc4"; font.pixelSize: 13 }
                }
                Item { Layout.fillWidth: true }
                EntryButton { objectName: "addEntry"; text: "添加入口"; onClicked: registerDialog.open() }
                EntryButton { text: "刷新"; enabled: !root.busy; onClicked: root.refreshRequested() }
                EntryButton { text: "收起"; quiet: true; onClicked: root.closeRequested() }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 12
                Text { text: "项目目录"; color: "#b4c1d7"; font.pixelSize: 13 }
                EntryField {
                    id: projectField
                    Layout.fillWidth: true
                    text: root.project
                    placeholderText: "/home/you/Projects/example"
                    onAccepted: root.actionRequested(["project", text])
                }
                EntryButton { text: "保存目录"; enabled: !root.busy; onClicked: root.actionRequested(["project", projectField.text]) }
            }

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: hero.implicitHeight + 36
                color: "#1c2940"
                radius: 14
                border.color: "#3a527c"
                RowLayout {
                    id: hero
                    anchors.fill: parent
                    anchors.margins: 18
                    spacing: 20
                    Rectangle {
                        width: 64; height: 64; radius: 12; color: "#101b30"
                        Text { anchors.centerIn: parent; text: ">_"; font.pixelSize: 26; color: "#9abaff"; font.family: "monospace" }
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 7
                        Text { text: "真实终端 · cosh-ng"; font.pixelSize: 19; font.bold: true; color: "#edf2ff" }
                        Text {
                            Layout.fillWidth: true
                            wrapMode: Text.WordWrap
                            text: "打开独立终端，在当前项目中工作。已有 cosh-ng 可以直接启动；终端与面板可以并排使用。"
                            color: "#a9b9d3"; font.pixelSize: 13
                        }
                        Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; text: root.terminalAvailable() ? "已发现 cosh 启动命令 · 登录与会话在原生终端完成" : "尚未发现 cosh-ng：可添加自定义终端入口，或在配置中指定命令。"; color: "#8da8d4"; font.pixelSize: 12 }
                    }
                    ColumnLayout {
                        Layout.minimumWidth: 166
                        spacing: 9
                        EntryButton { Layout.fillWidth: true; text: "打开 cosh-ng"; accent: true; enabled: root.terminalAvailable() && !root.busy; onClicked: root.actionRequested(["launch", "cosh", "--project", projectField.text]) }
                        EntryButton { objectName: "openTerminal"; Layout.fillWidth: true; text: "打开 Shell Terminal"; enabled: !root.busy; onClicked: root.actionRequested(["launch", "terminal", "--project", projectField.text]) }
                    }
                }
            }

            RowLayout {
                Text { text: "选择你的 Agent"; color: "#e8eefc"; font.pixelSize: 18; font.bold: true }
                Item { Layout.fillWidth: true }
                Text { text: "图形客户端 / 终端 / Web"; color: "#8798b5"; font.pixelSize: 12 }
            }

            GridLayout {
                Layout.fillWidth: true
                columns: root.width >= 1080 ? 4 : 2
                columnSpacing: 14
                rowSpacing: 14
                Repeater {
                    model: root.agents.filter(function(agent) { return agent.id !== "cosh" })
                    delegate: Rectangle {
                        id: card
                        required property var modelData
                        Layout.fillWidth: true
                        Layout.minimumWidth: 210
                        implicitHeight: 182
                        radius: 12
                        color: "#1a2436"
                        border.color: "#303e56"
                        ColumnLayout {
                            anchors.fill: parent
                            anchors.margins: 16
                            spacing: 9
                            RowLayout {
                                Text { text: card.modelData.name; color: "#edf2ff"; font.pixelSize: 18; font.bold: true; Layout.fillWidth: true; elide: Text.ElideRight }
                                Rectangle { width: 7; height: 7; radius: 4; color: card.modelData.available ? "#89d6b0" : "#b5a174" }
                            }
                            Text { text: card.modelData.subtitle; color: "#9dacbf"; font.pixelSize: 12; Layout.fillWidth: true; elide: Text.ElideRight }
                            Text { text: card.modelData.status; color: card.modelData.available ? "#89d6b0" : "#c0a775"; font.pixelSize: 12 }
                            Item { Layout.fillHeight: true }
                            RowLayout {
                                spacing: 8
                                EntryButton {
                                    objectName: "agentAction-" + card.modelData.id
                                    Layout.fillWidth: true
                                    text: card.modelData.available ? "打开" : (["qoder", "qoder-cli", "codex", "qoderwake"].indexOf(card.modelData.install) >= 0 ? "安装…" : "查看说明")
                                    accent: card.modelData.available
                                    enabled: !root.busy && (card.modelData.available || card.modelData.install !== "none")
                                    onClicked: {
                                        var action = card.modelData.available ? "launch" : (["qoder", "qoder-cli", "codex", "qoderwake"].indexOf(card.modelData.install) >= 0 ? "install" : "docs")
                                        root.actionRequested([action, card.modelData.id, "--project", projectField.text])
                                    }
                                }
                                EntryButton {
                                    text: "说明"
                                    visible: !!card.modelData.docs && (card.modelData.available || card.modelData.install !== "guide")
                                    quiet: true
                                    enabled: !root.busy
                                    onClicked: root.actionRequested(["docs", card.modelData.id])
                                }
                            }
                        }
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: statusText.implicitHeight + 24
                radius: 9; color: "#172235"; border.color: "#2d3c55"
                Text {
                    id: statusText
                    anchors.fill: parent; anchors.margins: 12
                    text: root.message
                    textFormat: Text.PlainText
                    wrapMode: Text.WrapAnywhere
                    color: "#b7c8e5"; font.pixelSize: 12
                }
            }
            RowLayout {
                Layout.fillWidth: true
                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: "v0.1 · 不读取账号凭据，不自动执行任务。检测到命令不代表已经登录；Tokenless / Checkpoint 尚未接入。"
                    color: "#7f91ad"; font.pixelSize: 11
                }
                EntryButton { text: "编辑配置"; quiet: true; onClicked: root.actionRequested(["edit-config"]) }
            }
        }
    }

    Dialog {
        id: registerDialog
        objectName: "registerDialog"
        title: "添加 Agent 入口"
        modal: true
        anchors.centerIn: parent
        width: Math.min(root.width - 40, 570)
        standardButtons: Dialog.NoButton
        background: Rectangle { color: "#202c42"; radius: 12; border.color: "#526787" }
        header: Text { text: "添加 Agent 入口"; color: "#f0f4ff"; font.pixelSize: 20; padding: 20 }
        contentItem: ColumnLayout {
            spacing: 12
            EntryField { id: entryId; Layout.fillWidth: true; placeholderText: "唯一 ID，如 my-cosh（小写字母、数字、短横线）" }
            EntryField { id: entryName; Layout.fillWidth: true; placeholderText: "显示名称，如 我的 cosh" }
            ComboBox { id: entryKind; Layout.fillWidth: true; model: ["终端命令", "图形程序", "Web 页面"] }
            EntryField {
                id: entryValue
                Layout.fillWidth: true
                placeholderText: entryKind.currentIndex === 2 ? "https://example.com/console" : '命令参数 JSON，例如 ["/opt/anolisa/bin/cosh-ng"]'
            }
            Text {
                Layout.fillWidth: true; wrapMode: Text.WordWrap
                color: "#bdc9dc"; font.pixelSize: 12
                text: "只注册你信任的程序或页面。命令按参数数组执行，不拼接 Shell。登录由各自 Agent 完成。"
            }
            RowLayout {
                Item { Layout.fillWidth: true }
                EntryButton { text: "取消"; onClicked: registerDialog.close() }
                EntryButton {
                    text: "保存入口"; accent: true
                    enabled: !root.busy && entryId.text.length > 0 && entryName.text.length > 0 && entryValue.text.length > 0
                    onClicked: {
                        root.actionRequested(["register", entryId.text, entryName.text, ["terminal", "gui", "web"][entryKind.currentIndex], entryValue.text])
                        registerDialog.close()
                    }
                }
            }
        }
    }
}
