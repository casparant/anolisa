# ANOLISA Agent Entry · Omarchy prototype

[中文版](README_zh.md)

An experimental Omarchy plugin that opens a terminal and native Agents from a
desktop Dashboard and a top-bar button. QML provides the UI; one Python script
handles actions and can also be called directly. No C++ compilation, custom
daemon, Agent implementation, account service, or package manager is required.

This experiment is **not part of ANOLISA's default installation, unified build,
or release inventory**. Its current UI is in Chinese.

![QtQuick layout preview, not an Omarchy deployment screenshot](assets/preview.png)

## Features and limits

- Toggle the Dashboard from the bar; optionally show it after desktop login.
- Open a real Shell Terminal or an existing cosh-ng in the project directory.
  The terminal is a separate window and can be tiled alongside the Dashboard.
- Launch Qoder GUI, Qoder CLI, Codex CLI, or QoderWake through their native
  entry points.
- Install Qoder CLI, Codex CLI, or QoderWake after reviewing the source in a
  terminal and explicitly typing INSTALL.
- Register a custom terminal command, GUI program, or Web URL.
- Authenticate in each Agent's own UI. This plugin does not collect credentials.

**Automatic Qoder GUI installation is deferred.** Its button opens the official
download page; it does not convert RPM/DEB packages to Arch packages. The inspected
official RPM contains `/opt/Qoder/qoder`, a desktop entry, and `qoder:` /
`qoder-app:` URL handlers. Dependencies, Electron sandbox behavior, and login
callbacks still need target-machine testing. An existing installation can be
detected through its desktop file or configured with an explicit command.
QoderWork is not included.

This is not an embedded terminal, unified chat client, or task-monitoring
Dashboard. Tokenless and Checkpoint data are not connected or simulated.
Hiding the Dashboard does not terminate launched Agents or terminals. Persistence
across desktop logout or machine shutdown is outside this prototype's scope.

## Deploy

Target: **Omarchy 4.0.1 with its Quickshell plugin system**. Run as the normal
desktop user inside an Omarchy terminal, not with sudo. The desktop must provide
`omarchy`, `omarchy-shell`, `qs`, `python3`, `jq`, `xdg-open`, and a supported
terminal (foot, kitty, alacritty, or xterm). Installing Codex CLI also requires npm.

From the root of a checkout of this branch:

```bash
cd experiments/omarchy-agent-entry
python3 install.py --autostart
```

No compilation is needed. The installer validates the manifest, copies the
plugin, enables it, and opens the Dashboard. It does not install Agents, change
system package repositories, or overwrite Hyprland configuration.

Omit `--autostart` to keep the Dashboard closed after login. This option creates
an independent Omarchy `post-boot.d` hook; it does not start Agents or tasks.

| Item | Location |
|---|---|
| Plugin | `~/.config/omarchy/plugins/anolisa.agent-entry/` |
| CLI launcher | `~/.local/bin/anolisa-agent-entry` |
| Application-menu entry | `~/.local/share/applications/anolisa-agent-entry.desktop` |
| Optional startup hook | `~/.config/omarchy/hooks/post-boot.d/anolisa-agent-entry.sh` |
| Project and custom entries | `~/.config/anolisa/agent-entry.json` |

## First use

1. Open Shell Terminal. An existing cosh-ng can be launched separately; this
   prototype does not include its binary.
2. Choose Install for Qoder CLI, Codex CLI, or QoderWake. Review the source and
   type INSTALL in the terminal. Network access and accounts are supplied by
   the target machine.
3. Refresh after installation, then Open and complete native authentication.
4. For Qoder GUI, use the download-page button or configure an existing binary.
5. QoderWake starts only after an explicit click. The helper calls `whoami`,
   then native `login` if needed, then
   `start --host 127.0.0.1 --open`. It does not expose a public listener.

### Configure commands

Use Add Entry, or Edit Configuration. Merge these fields into your existing
configuration rather than discarding other entries; replace paths with real
target-machine paths:

```json
{
  "version": 1,
  "project": "/home/yourname/Projects",
  "commands": {
    "cosh": ["/opt/anolisa/bin/cosh-ng"],
    "qoder": ["/opt/Qoder/qoder"]
  },
  "agents": []
}
```

The CLI offers the same registration operation:

```bash
~/.local/bin/anolisa-agent-entry register my-cosh "我的 cosh" terminal '["/opt/anolisa/bin/cosh-ng"]'
~/.local/bin/anolisa-agent-entry register console "我的 Web Console" web 'http://127.0.0.1:19820'
```

Registration stores a launch entry, not an MCP service or an Agent account.
Commands are argument arrays, not interpolated Shell strings. If a program needs
environment setup, register a wrapper script that you maintain.

Qoder GUI and CLI are detected separately; the ambiguous `qoder` command is not
used to infer GUI availability. When launched through `gtk-launch`, the GUI
decides whether to restore its previous project. The project directory supplies
the terminal working directory, not a universal GUI project-opening protocol.

## Diagnose, update, and remove

```bash
~/.local/bin/anolisa-agent-entry status
~/.local/bin/anolisa-agent-entry open
~/.local/bin/anolisa-agent-entry launch terminal
omarchy-shell shell listPlugins
omarchy-shell shell rescanPlugins
```

If the bar entry is absent, check that the plugin is enabled:

```bash
omarchy plugin enable anolisa.agent-entry
omarchy-shell shell summon anolisa.agent-entry '{}'
```

For QML load failures, use `qs log --help` to find the installed version's log
options. The plugin belongs to the existing Omarchy Shell process; do not start
a second full Shell just to debug it. If a launched app immediately exits, run
its actual command in a terminal to inspect the error. Command detection is not
an application-health or authentication check.

Upgrade:

```bash
python3 install.py --replace --autostart
```

Uninstall:

```bash
python3 install.py --uninstall
```

Old plugin files move to recoverable backups under
`~/.config/anolisa/plugin-backups/`. Agent installations, accounts, and entry
configuration remain untouched. To disable automatic display only, move the
plugin's startup hook out of `post-boot.d/`. There is no online updater.

## Validation

The manifest was checked using the official Omarchy **v4.0.1** validator.
Python tests cover launch arguments, configuration, installation confirmation,
backup, and uninstall. Real QtQuick rendering tests check the terminal button's
action signal, the registration dialog, Escape handling, and resizing.

**Linux/Hyprland/Omarchy integration has not been exercised on a target
machine.** Development validation ran on macOS. Bar loading, startup hooks,
Agent installation, authentication, and actual execution still require target
acceptance. The preview is a render of this QML, not a deployment screenshot.

```bash
python3 -m unittest discover -s tests -v
python3 -m pytest tests/test_*.py
omarchy plugin validate .
```

Optional UI check (requires PySide6 in the development environment, not at
deployment time):

```bash
QT_QPA_PLATFORM=offscreen python3 tests/render_qml.py /tmp/agent-entry-preview.png
```

Target acceptance sequence: toggle from the bar → launch a terminal → register
a real command → install and authenticate one CLI Agent → hide the Dashboard
and confirm the terminal remains → log in again to check automatic display.

## Files

- `DashboardContent.qml`: UI, buttons, and registration dialog.
- `Dashboard.qml`: Omarchy window and short-lived helper invocations.
- `BarWidget.qml`: top-bar button.
- `agent_entry.py`: launch, installation, and configuration; independently callable.
- `install.py`: local plugin installation and enablement.

The general UI uses QtQuick. Manifest discovery, bar placement, and Shell IPC
use Omarchy's plugin contract. Other Quickshell environments may reuse parts of
this code, but they are not currently supported targets.

## Sources checked on 2026-09-16

- [Omarchy v4.0.1 Shell plugin contract](https://github.com/omacom/omarchy/blob/v4.0.1/shell/README.md),
  commit `13f18b2cb7286fb54f87daf571a031aa6af3d8f0`.
- [Plugin development](https://plugins.omarchy.org/develop.html) and
  [Shell Plugins manual](https://omarchy.org/manual/shell-plugins/).
- [Quickshell FloatingWindow](https://quickshell.org/docs/v0.3.0/types/Quickshell/FloatingWindow/)
  and [Process](https://quickshell.org/docs/v0.3.0/types/Quickshell.Io/Process/).
- [Qoder downloads](https://qoder.com/download),
  [CLI installation](https://docs.qoder.com/cli/installation), and
  [QoderWake CLI](https://docs.qoder.com/qoderwake/cli-reference).
- [Codex CLI repository](https://github.com/openai/codex).

Only this plugin's source is distributed, under the repository's Apache-2.0
license. No third-party Agent binaries are bundled. The experiment is not
registered with the ANOLISA installer; production-component installation commands
do not apply until target validation and component onboarding are completed.
