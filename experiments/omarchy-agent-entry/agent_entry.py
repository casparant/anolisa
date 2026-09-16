#!/usr/bin/env python3
"""Local Agent entry. No daemon, credential store or shell eval."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent
PLUGIN_ID = "anolisa.agent-entry"


def config_path() -> Path:
    return (
        Path(os.environ.get("XDG_CONFIG_HOME", str(Path.home() / ".config")))
        / "anolisa/agent-entry.json"
    )


def env() -> dict[str, str]:
    result = os.environ.copy()
    paths = [
        str(Path.home() / p)
        for p in (
            ".local/bin",
            ".local/share/anolisa/agent-entry/npm/bin",
            ".local/share/mise/shims",
        )
    ]
    result["PATH"] = os.pathsep.join(paths + [result.get("PATH", "")])
    return result


def which(command: str) -> str | None:
    return shutil.which(os.path.expanduser(command), path=env()["PATH"])


def catalog() -> list[dict[str, Any]]:
    return [
        dict(
            id="cosh",
            name="cosh-ng",
            kind="terminal",
            candidates=["cosh-ng", "cosh"],
            subtitle="ANOLISA 交互终端",
            docs="https://github.com/agentic-os-org/ANOLISA",
            install="guide",
        ),
        dict(
            id="qoder",
            name="Qoder",
            kind="gui",
            candidates=["qoder-desktop", "Qoder"],
            subtitle="图形客户端 · 官方 DEB 转 Arch 包",
            docs="https://qoder.com/download",
            install="qoder",
        ),
        dict(
            id="qoder-cli",
            name="Qoder CLI",
            kind="terminal",
            candidates=["qodercli", "qoder"],
            subtitle="独立终端入口",
            docs="https://docs.qoder.com/cli/installation",
            install="qoder-cli",
        ),
        dict(
            id="codex",
            name="Codex CLI",
            kind="terminal",
            candidates=["codex"],
            subtitle="终端版 · 非桌面客户端",
            docs="https://github.com/openai/codex",
            install="codex",
        ),
        dict(
            id="qoderwake",
            name="QoderWake",
            kind="wake",
            candidates=["qoderwake"],
            subtitle="Linux 服务 + Web Console",
            docs="https://docs.qoder.com/qoderwake/cli-reference",
            install="qoderwake",
        ),
    ]


def valid_argv(value: object) -> list[str]:
    if not isinstance(value, list) or not value or len(value) > 64:
        raise ValueError('启动命令必须是非空 JSON 字符串数组，例如 ["cosh-ng"]')
    if any(not isinstance(v, str) or not v or "\0" in v or "\n" in v or "\r" in v for v in value):
        raise ValueError("命令参数不能包含空值、换行或 NUL")
    return value


def valid_url(value: str) -> str:
    parsed = urllib.parse.urlsplit(value)
    if (
        parsed.scheme not in ("https", "http")
        or not parsed.hostname
        or parsed.username
        or parsed.password
    ):
        raise ValueError("仅接受不含用户名/密码的 http(s) URL")
    if any(c in value for c in ("\n", "\r", "\0")):
        raise ValueError("URL 包含非法字符")
    return value


def load_config() -> dict[str, Any]:
    path = config_path()
    if not path.exists():
        return {"version": 1, "project": str(Path.home()), "agents": []}
    data = json.loads(path.read_text(encoding="utf-8"))
    if (
        not isinstance(data, dict)
        or data.get("version") != 1
        or not isinstance(data.get("agents", []), list)
    ):
        raise ValueError("配置格式错误；请检查 " + str(path))
    return data


def save_config(data: dict[str, Any]) -> None:
    path = config_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temp = tempfile.mkstemp(prefix=".agent-entry-", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(data, handle, ensure_ascii=False, indent=2)
            handle.write("\n")
        os.replace(temp, path)
    finally:
        if os.path.exists(temp):
            os.unlink(temp)


def project_dir(value: str | None) -> Path:
    path = Path(value or load_config().get("project", str(Path.home()))).expanduser().resolve()
    if not path.is_dir():
        raise ValueError("项目目录不存在：" + str(path))
    return path


def agents() -> list[dict[str, Any]]:
    items = catalog()
    seen = {a["id"] for a in items}
    for item in load_config().get("agents", []):
        if not isinstance(item, dict) or not re.fullmatch(
            r"local\.[a-z0-9_-]+", item.get("id", "")
        ):
            raise ValueError("自定义入口 id 必须使用 local. 前缀")
        if item["id"] in seen:
            raise ValueError("重复的入口 id：" + item["id"])
        if item.get("kind") not in ("terminal", "gui", "web"):
            raise ValueError("未知入口类型")
        if item["kind"] == "web":
            valid_url(item.get("url", ""))
        else:
            valid_argv(item.get("argv"))
        items.append(
            dict(item, subtitle=item.get("subtitle", "自定义入口"), install="none", docs="")
        )
        seen.add(item["id"])
    overrides = load_config().get("commands", {})
    if not isinstance(overrides, dict):
        raise ValueError("commands 必须是对象")
    for item in items:
        if item["id"] in overrides:
            item["argv"] = valid_argv(overrides[item["id"]])
    return items


def get_agent(agent_id: str) -> dict[str, Any]:
    for item in agents():
        if item["id"] == agent_id:
            return item
    raise ValueError("未知 Agent：" + agent_id)


def command_for(item: dict[str, Any]) -> list[str] | None:
    if "argv" in item:
        argv = valid_argv(item["argv"])
        executable = which(argv[0])
        return [executable] + argv[1:] if executable else None
    # GUI/CLI names can collide: use the desktop file to launch Qoder GUI.
    if item["id"] == "qoder" and which("gtk-launch"):
        roots = [Path.home() / ".local/share/applications", Path("/usr/share/applications")]
        for directory in roots:
            for file in sorted(directory.glob("*.desktop")):
                if file.stem.lower() in ("qoder", "qoder-ide", "qoder-desktop", "qoderide"):
                    return [which("gtk-launch"), file.stem]
    for candidate in item.get("candidates", []):
        executable = which(candidate)
        if executable:
            return [executable]
    return None


def snapshot() -> dict[str, Any]:
    output = []
    for source in agents():
        item = copy.deepcopy(source)
        found = command_for(item)
        ready = item["kind"] == "web" or bool(found)
        item.update(
            available=ready,
            status=(
                "Web 入口已配置"
                if item["kind"] == "web"
                else "启动入口已发现" if ready else "尚未发现启动入口"
            ),
        )
        # Presence can be a mise lazy launcher; it is not proof of auth or installation.
        output.append(item)
    return {
        "agents": output,
        "project": str(project_dir(None)),
        "config": str(config_path()),
        "note": "检测只确认启动入口，不读取凭据，也不代表已经登录或任务正在运行。",
    }


def detach(argv: list[str], cwd: Path | None = None) -> None:
    subprocess.Popen(
        argv,
        cwd=cwd,
        env=env(),
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
        close_fds=True,
    )


def terminal_argv(payload: list[str], cwd: Path) -> list[str]:
    for name in ("foot", "kitty", "alacritty", "xterm"):
        binary = which(name)
        if not binary:
            continue
        if name == "foot":
            return [
                binary,
                "--app-id=anolisa-agent-terminal",
                "--title=ANOLISA · Terminal",
                "--working-directory=" + str(cwd),
                "-e",
            ] + payload
        if name == "kitty":
            return [
                binary,
                "--class=anolisa-agent-terminal",
                "--title=ANOLISA · Terminal",
                "--directory=" + str(cwd),
            ] + payload
        if name == "alacritty":
            return [binary, "--working-directory", str(cwd), "-e"] + payload
        return [binary, "-title", "ANOLISA · Terminal", "-e"] + payload
    raise ValueError("没有找到 foot / kitty / alacritty / xterm 终端")


def open_terminal(payload: list[str], cwd: Path) -> None:
    detach(terminal_argv(payload, cwd), cwd)


def helper_payload(*args: str) -> list[str]:
    return [sys.executable, str(ROOT / "agent_entry.py")] + list(args)


def open_url(url: str) -> None:
    valid_url(url)
    opener = which("xdg-open")
    if not opener:
        raise ValueError("没有找到 xdg-open")
    detach([opener, url])


def launch(agent_id: str, directory: str | None = None) -> str:
    cwd = project_dir(directory)
    if agent_id == "terminal":
        shell = os.environ.get("SHELL", "/bin/bash")
        if not Path(shell).is_file():
            shell = "/bin/bash"
        open_terminal([shell], cwd)
        return "已请求打开终端"
    item = get_agent(agent_id)
    if item["kind"] == "web":
        open_url(item["url"])
        return "已请求打开 Web 入口"
    argv = command_for(item)
    if not argv:
        raise ValueError(item["name"] + " 尚未发现启动入口，请安装或注册命令。")
    if item["kind"] in ("terminal", "wake"):
        open_terminal(helper_payload("session", agent_id, "--project", str(cwd)), cwd)
    else:
        detach(argv, cwd)
    return "已发起启动请求；登录与实际运行请在原生界面确认。"


def session(agent_id: str, directory: str | None) -> None:
    item = get_agent(agent_id)
    argv = command_for(item)
    if not argv:
        raise ValueError("启动命令不存在")
    cwd = project_dir(directory)
    print("ANOLISA · " + item["name"] + "\n工作目录：" + str(cwd), flush=True)
    if item["kind"] == "wake":
        # Explicit action; no public listener, no login token reads, no stored credentials.
        if subprocess.run(argv + ["whoami"], cwd=cwd, env=env()).returncode != 0:
            if subprocess.run(argv + ["login"], cwd=cwd, env=env()).returncode != 0:
                raise ValueError("QoderWake 登录未完成")
        subprocess.run(
            argv + ["start", "--host", "127.0.0.1", "--open"], cwd=cwd, env=env(), check=True
        )
    else:
        # Do not append unattended / YOLO / bypass-permissions options.
        subprocess.run(argv, cwd=cwd, env=env(), check=True)


def confirm(message: str) -> None:
    print(message, flush=True)
    if not sys.stdin.isatty() or input("输入 INSTALL 确认，其他输入取消：").strip() != "INSTALL":
        raise ValueError("已取消，未执行安装")


def install_agent(agent_id: str) -> None:
    if agent_id == "qoder":
        subprocess.run([sys.executable, str(ROOT / "qoder/install.py")], env=env(), check=True)
        return
    elif agent_id == "qoder-cli":
        url = "https://qoder.com/install"
    elif agent_id == "qoderwake":
        url = "https://download.qoder.com/qoderwake/install.sh"
    elif agent_id == "codex":
        npm = which("npm")
        if not npm:
            raise ValueError("需要 npm。请先用 Omarchy 的开发工具安装 Node.js/npm，再重试。")
        prefix = Path.home() / ".local/share/anolisa/agent-entry/npm"
        command = [npm, "install", "--global", "--prefix", str(prefix), "@openai/codex"]
        confirm("从 npm 官方包 @openai/codex 安装到用户目录，不使用 sudo：\n" + shlex.join(command))
        subprocess.run(command, env=env(), check=True)
        return
    else:
        raise ValueError("此入口没有自动安装器，请使用官方说明或注册已安装的命令。")
    print("下载安装脚本供确认：" + url, flush=True)
    with urllib.request.urlopen(url, timeout=30) as response:
        if urllib.parse.urlsplit(response.url).scheme != "https":
            raise ValueError("拒绝非 HTTPS 重定向")
        body = response.read(4 * 1024 * 1024 + 1)
    if len(body) > 4 * 1024 * 1024 or not body:
        raise ValueError("安装脚本大小异常")
    cache = (
        Path(os.environ.get("XDG_CACHE_HOME", str(Path.home() / ".cache"))) / "anolisa-agent-entry"
    )
    cache.mkdir(parents=True, exist_ok=True)
    fd, filename = tempfile.mkstemp(prefix=agent_id + "-", suffix=".sh", dir=cache)
    with os.fdopen(fd, "wb") as handle:
        handle.write(body)
    digest = hashlib.sha256(body).hexdigest()
    confirm(
        "已下载："
        + filename
        + "\nSHA256（用于记录，不是官方签名）："
        + digest
        + "\n可以先在另一终端查看脚本。继续将以当前用户权限执行官方安装器。"
    )
    subprocess.run(["bash", filename], env=env(), check=True)


def register(args: argparse.Namespace) -> str:
    if not re.fullmatch(r"[a-z0-9_-]{1,48}", args.id):
        raise ValueError("id 只允许小写字母、数字、下划线和短横线")
    name = args.name.strip()
    if not name or len(name) > 80 or any(ord(c) < 32 for c in name):
        raise ValueError("名称不能为空，最多 80 字符")
    item = {"id": "local." + args.id, "name": name, "kind": args.kind}
    if args.kind == "web":
        item["url"] = valid_url(args.value)
    else:
        item["argv"] = valid_argv(json.loads(args.value))
    data = load_config()
    if any(x.get("id") == item["id"] for x in data.get("agents", [])):
        raise ValueError("入口 id 已存在，请使用其他 id 或编辑配置")
    data.setdefault("agents", []).append(item)
    save_config(data)
    return "入口已注册；启动时使用原生 Agent，不保存登录凭据。"


def wait_for_shell(action: str = "summon") -> str:
    binary = which("omarchy-shell")
    if not binary:
        raise ValueError("未找到 omarchy-shell")
    for _ in range(30):
        result = subprocess.run(
            [binary, "shell", "ping"], capture_output=True, text=True, timeout=3
        )
        if result.returncode == 0 and "ok" in result.stdout:
            subprocess.run([binary, "shell", action, PLUGIN_ID, "{}"], check=True, timeout=5)
            return "入口已打开"
        time.sleep(1)
    raise ValueError("Omarchy Shell 尚未就绪，稍后运行 agent-entry open 重试")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    sub.add_parser("status")
    sub.add_parser("open")
    sub.add_parser("toggle")
    sub.add_parser("edit-config")
    for action in ("launch", "session", "install", "install-run", "docs"):
        p = sub.add_parser(action)
        p.add_argument("id")
        p.add_argument("--project")
    p = sub.add_parser("project")
    p.add_argument("directory")
    p = sub.add_parser("register")
    p.add_argument("id")
    p.add_argument("name")
    p.add_argument("kind", choices=("terminal", "gui", "web"))
    p.add_argument("value", help='JSON argv, e.g. ["cosh-ng"], or a web URL')
    args = parser.parse_args()
    in_terminal = args.action in ("session", "install-run")
    try:
        if args.action == "status":
            result = snapshot()
        elif args.action in ("open", "toggle"):
            result = {"message": wait_for_shell("summon" if args.action == "open" else "toggle")}
        elif args.action == "launch":
            result = {"message": launch(args.id, args.project)}
        elif args.action == "session":
            session(args.id, args.project)
            result = {"message": "会话已结束"}
        elif args.action == "install-run":
            install_agent(args.id)
            result = {"message": "安装器已完成；请刷新入口并运行 Agent 完成登录。"}
        elif args.action == "install":
            item = get_agent(args.id)
            if item["install"] not in ("qoder", "qoder-cli", "codex", "qoderwake"):
                raise ValueError("没有适用的自动安装器，请查看官方说明")
            open_terminal(helper_payload("install-run", args.id), project_dir(args.project))
            result = {"message": "安装确认窗口已打开，请在终端查看并确认。"}
        elif args.action == "docs":
            open_url(get_agent(args.id)["docs"])
            result = {"message": "已打开官方说明"}
        elif args.action == "project":
            data = load_config()
            data["project"] = str(project_dir(args.directory))
            save_config(data)
            result = {"message": "项目目录已保存", "project": data["project"]}
        elif args.action == "register":
            result = {"message": register(args)}
        else:
            data = load_config()
            if not config_path().exists():
                save_config(data)
            editor = shlex.split(os.environ.get("EDITOR", "nano"))
            valid_argv(editor)
            open_terminal(editor + [str(config_path())], project_dir(None))
            result = {"message": "已打开配置编辑器"}
        print(json.dumps(result, ensure_ascii=False), flush=True)
        code = 0
    except (ValueError, OSError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        print(json.dumps({"error": str(error)}, ensure_ascii=False), flush=True)
        code = 1
    if in_terminal and sys.stdin.isatty():
        try:
            input("\n按 Enter 关闭此终端。")
        except EOFError:
            pass
    return code


if __name__ == "__main__":
    sys.exit(main())
