#!/usr/bin/env python3
"""Install this local plugin without changing system packages or overwriting user data."""

import argparse
import os
import shlex
import shutil
import subprocess
import sys
import time
from pathlib import Path

ID = "anolisa.agent-entry"
SOURCE = Path(__file__).resolve().parent
FILES = [
    "manifest.json",
    "BarWidget.qml",
    "Dashboard.qml",
    "DashboardContent.qml",
    "EntryButton.qml",
    "EntryField.qml",
    "agent_entry.py",
]


def run(command: list[str]) -> None:
    subprocess.run(command, check=True, timeout=15)


def desktop_quote(value: str) -> str:
    if any(c in value for c in "\n\r\0"):
        raise ValueError("非法路径")
    return (
        '"'
        + value.replace("\\", "\\\\").replace('"', '\\"').replace("`", "\\`").replace("$", "\\$")
        + '"'
    )


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--autostart", action="store_true", help="每次登录后显示 Dashboard")
    p.add_argument("--replace", action="store_true", help="备份已有同名插件后升级")
    p.add_argument("--uninstall", action="store_true", help="禁用并移至备份目录；保留 Agent 与配置")
    args = p.parse_args()
    if sys.platform != "linux":
        p.error("安装器仅在目标 Linux 桌面上运行；macOS 可运行测试。")
    if os.geteuid() == 0:
        p.error("请使用登录桌面的普通用户，不要 sudo。")
    for tool in ("omarchy", "omarchy-shell", "python3"):
        if not shutil.which(tool):
            p.error("未找到 " + tool + "；请在 Omarchy 图形会话中运行。")
    config = Path.home() / ".config"
    target = config / "omarchy/plugins" / ID
    backups = config / "anolisa/plugin-backups"
    launcher = Path.home() / ".local/bin/anolisa-agent-entry"
    desktop = Path.home() / ".local/share/applications/anolisa-agent-entry.desktop"
    autostart = config / "omarchy/hooks/post-boot.d/anolisa-agent-entry.sh"
    if args.uninstall:
        run(["omarchy", "plugin", "disable", ID])
        paths = [target, launcher, desktop, autostart]
        backup = backups / ("removed-" + str(time.time_ns()))
        backup.mkdir(parents=True)
        for path in paths:
            if path.exists() or path.is_symlink():
                # These are exact, plugin-owned paths; never remove Agent software.
                shutil.move(str(path), str(backup / (path.parent.name + "-" + path.name)))
        run(["omarchy-shell", "shell", "rescanPlugins"])
        print("插件文件已移至可恢复备份：", backup)
        print("Agent 安装、账号与 ~/.config/anolisa/agent-entry.json 均保留。")
        return
    run(["omarchy", "plugin", "validate", str(SOURCE)])
    owned_paths = [target, launcher, desktop] + ([autostart] if args.autostart else [])
    existing = [path for path in owned_paths if path.exists() or path.is_symlink()]
    if existing and not args.replace:
        p.error("已有同名安装。确认升级请加 --replace，旧文件会备份。")
    if existing:
        backup = backups / ("upgrade-" + str(time.time_ns()))
        backup.mkdir(parents=True)
        for path in existing:
            shutil.move(str(path), str(backup / (path.parent.name + "-" + path.name)))
        print("升级前备份：", backup)
    target.mkdir(parents=True, exist_ok=False)
    for name in FILES:
        shutil.copy2(SOURCE / name, target / name)
    helper = target / "agent_entry.py"
    helper.chmod(0o755)
    launcher.parent.mkdir(parents=True, exist_ok=True)
    launcher.symlink_to(helper)
    desktop.parent.mkdir(parents=True, exist_ok=True)
    desktop_text = (
        "[Desktop Entry]\nType=Application\nName=ANOLISA Agent 入口\n"
        "Comment=启动原生 Agent 与终端\nExec=" + desktop_quote(str(launcher)) + " open\n"
        "Icon=utilities-terminal\nTerminal=false\nCategories=Development;Utility;\n"
    )
    desktop.write_text(desktop_text, encoding="utf-8")
    if args.autostart:
        autostart.parent.mkdir(parents=True, exist_ok=True)
        autostart.write_text(
            "#!/bin/bash\n# ANOLISA Agent Entry — opt-in startup hook\n"
            + shlex.quote(str(launcher))
            + " open >/dev/null 2>&1 &\n",
            encoding="utf-8",
        )
    run(["omarchy-shell", "shell", "rescanPlugins"])
    # Discovery is asynchronous. Retry enable; do not edit shell.json directly.
    for attempt in range(15):
        outcome = subprocess.run(["omarchy", "plugin", "enable", ID], timeout=10)
        if outcome.returncode == 0:
            break
        time.sleep(0.3)
    else:
        raise RuntimeError("插件已复制，但启用失败。请查看 omarchy-shell 日志，不要重复覆盖配置。")
    run(["omarchy-shell", "shell", "summon", ID, "{}"])
    print("已安装。顶栏点击 Agent 入口；也可运行 ~/.local/bin/anolisa-agent-entry open")
    if args.autostart:
        print("已写入 Omarchy post-boot.d 独立启动钩子；不修改 Hyprland 或现有钩子文件。")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print("安装未完成：" + str(error), file=sys.stderr)
        sys.exit(1)
