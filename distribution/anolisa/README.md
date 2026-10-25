# anolisa

[中文版](README_zh.md)

Unified CLI gateway for ANOLISA product installation and component management.
The CLI installs verified complete Linux releases directly through the shared
local-package library and retains existing
raw/RPM component and adapter workflows. It is the primary user entry point of
the [ANOLISA](../../README.md) project.

## Quick Start

```bash
curl -fsSL https://get.agentic-os.sh | bash
export PATH="$HOME/.local/bin:$PATH"
anolisa --version
```

GNU/Linux x86_64 and aarch64 download the complete product archive in user scope;
the bootstrap requires Python 3.6 or newer. macOS installs the standalone CLI, which can
also be selected on Linux with `--cli-only`. For a downloaded complete archive,
run its `./install.sh --scope user` or, as root, `./install.sh --scope system`.
New bundles default to `complete` in both scopes, installing every delivered
platform package. Reduced profiles require explicit `--profile` selection.
After ordinary installation, the installer runs `anolisa setup`: piped use prints availability,
while a terminal can guide explicit user AW configuration. Linux `--rpm` is an
explicit alternative that runs the complete published component set through the
native RPM backend in system mode. Installation does not enable Providers or
services. Existing standalone command
entries are preserved and must be explicitly resolved before product activation.

System tarball installation places public commands in `/usr/local/bin`, private
programs in `/usr/local/lib/anolisa`, and static resources in
`/usr/local/share/anolisa`. User installation separates private programs from
XDG resources. ANOLISA records both locations. Disabled Provider templates retain
versioned program paths; runtime data and sockets must be resolved for the user
who will run them, independently of the software installation scope.

See the [installation guide](../../docs/user-guide/en/getting-started/installation.md)
for `--release-version`, `--release-url`, layout, compatibility, and offline use.
The [CLI reference](../../docs/user-guide/en/getting-started/anolisa-cli.md#product-release-commands)
covers `release inspect`, `release install`, `release status`, package
`release list` / `release doctor`, inactive-version cleanup through
`release uninstall --bundle ABS_DIRECTORY --yes`, and explicit
`release recover --scope user|system` for the new ANOLISA package database.
Existing AW-owned product versions retain their original database and use
`aw recover`; ownership is not automatically transferred.

## Command Overview

### Complete product releases

`anolisa release install --bundle ABS_DIRECTORY --scope user|system` installs
or resumes a verified unpacked release; add `--dry-run --json` to preview it.
`release status` reads its product progress. Product scope defaults to user,
even as root, and is separate from the legacy `--install-mode` option below.

`anolisa install --bundle ABS_DIRECTORY --profile standard-user --scope user`
uses the same product installer. Component selection uses
`release install --bundle ABS_DIRECTORY --component tokenless`; dependencies
are included. Profiles select installed files, while `preset plan/apply` and
`serve --preset container` separately manage services. `migrate plan/apply`
provides explicit, recorded transfer for supported legacy raw installations.
See [distribution maintenance](../README.md) for optional product recipes and component-owned RPM splits.

### Tier 1 — Existing component lifecycle

| Command | Description |
|---------|-------------|
| `list` | List available components (alias: `ls`) |
| `install` | Install a component from a configured raw or RPM backend |
| `uninstall` | Uninstall a component |
| `update` | Update a component, CLI itself (`self`), or all |
| `upgrade` | Apply an RPM/system-image upgrade plan (system scope) |
| `status` | Show component health |
| `doctor` | Diagnose issues and suggest fixes |
| `logs` | Query component logs (`--severity`, alias `--level`) |
| `restart` | Restart a component service (`--dry-run` lists units without dispatching) |
| `repair` | Reconcile state after manual RPM changes |
| `adopt` | Record an existing system RPM as adopted without default removal authority |
| `forget` | Drop state record without package operations |

`status` and `doctor` allow content edits to raw-installed `type = "config"`
files while continuing to check their presence, permissions, and path safety.

`logs --since <RFC3339>` applies an inclusive timestamp filter, accounting for
offsets and fractional seconds. Records with unparseable timestamps are excluded
only when `--since` is set.

### Tier 2 — Management

| Command | Description |
|---------|-------------|
| `overview` | Show recorded component versions, qualified availability and optional local candidate comparison |
| `setup` | Guide explicit user AW configuration and preview service policy |
| `adapter` | Manage component-to-framework adapters (scan / enable / disable / status) |
| `preset` | Review and explicitly apply native service activation policy |
| `serve` | Supervise allowlisted foreground services without systemd |
| `migrate` | Review and apply a fixed legacy raw-to-product migration |
| `osbase kernel` | Kernel modules and eBPF management |
| `osbase sandbox` | Sandbox runtime management (runc, gvisor, firecracker, etc.) |
| `osbase security` | Security overlay management (loongshield, seccomp-profiles) |
| `system` | System helper daemon lifecycle (setup / serve / teardown / status) |
| `register` | Join / leave Agentic OS Co-Build Program |
| `env` | Show environment detection results |
| `bug` | Generate a bug report (`--component cosh-ng` also summarizes the cosh-shell diagnostic bundle; `COSH_SHELL_BIN` overrides the binary) |

By default, `anolisa adapter enable <component> openclaw` accepts the plugin's
declared capabilities when each host command supports capability consent.
Add `--no-accept-capabilities` to withhold automatic consent during both
installation and activation. This does not revoke existing host authorization
or authorize an unsafe-install bypass.

Enable the Tokenless local plugin with `anolisa adapter enable tokenless opencode`.
ANOLISA manages its plugin link; restart OpenCode after enabling or disabling it.
See the [adapter reference](../../docs/user-guide/en/getting-started/anolisa-cli.md#adapter)
for custom configuration directories and existing-installation ownership.

Enable Tokenless in QwenWorkCN native Bash and macOS workspace Shell sessions with
`anolisa adapter enable tokenless qwenwork`. Restart the desktop application and
check activation there; CLI status verifies files but reports activation as
`unknown`. Existing script installations must be uninstalled before CLI enable.

## Existing Component Install Modes

| Mode | Prefix | When |
|------|--------|------|
| `system` | `/usr/local` (or custom `--prefix`) | Running as root (default) |
| `user` | `~/.local` | Running as non-root (default) |

Override with `--install-mode user|system`.
These modes apply to existing component commands. Complete product releases use
`--scope` and versioned directories instead. `update all` does not update a complete
product release; `update self` refuses direct replacement of its product-managed CLI.
RPM-owned CLI self-update continues through the native package manager.

Read-only discovery is broader than mutation scope: a user invocation can
list, inspect, diagnose, and attach adapters to a visible system installation.
Lifecycle mutations still target only the selected scope. Therefore
`anolisa --install-mode user install <component>` may create a separate user
installation even when the same component is already installed system-wide.

## Global Options

| Flag | Effect |
|------|--------|
| `--dry-run` | Print plan without executing |
| `--json` | Machine-readable JSON output |
| `-v, --verbose` | Increase verbosity |
| `-q, --quiet` | Suppress non-error output |
| `--no-color` | Disable colored output |

See the [full CLI guide](../../docs/user-guide/en/getting-started/anolisa-cli.md)
for command forms, scope behavior, and recovery workflows.

RPM installs support Yum 3 and DNF 4. Use `sudo` with RPM `--dry-run`
to check dependencies and conflicts before installing.

## Architecture

Five-crate Cargo workspace:

| Crate | Responsibility |
|-------|---------------|
| `anolisa-cli` | Command parsing, dispatch, terminal UI |
| `anolisa-core` | Component resolution, adapter management, osbase install logic |
| `anolisa-env` | Environment detection (distro, arch, capabilities) |
| `anolisa-build` | Build-time codegen and asset embedding |
| `anolisa-platform` | Filesystem layout, native package queries/transactions, systemd, IPC, privilege helpers |

Supports dual backends: **raw** (OSS tar.gz) and **RPM** (yum/dnf transactions).
The lifecycle planner separates ANOLISA-owned files from native-package
authority and records crash-recovery intent before side effects. Component
metadata is declared through `component.toml`.

The raw backend re-fetches the distribution index on every resolve and fails
when the repository is unreachable. Legacy `cache_ttl_secs` and
`offline_fallback` keys in `repo.toml` are still accepted for compatibility,
but the current raw backend ignores their values.

## Requirements

- Linux (x86_64 / aarch64) or macOS 11+ (arm64 / x86_64, limited)

Intel macOS prebuilt and npm support requires a release containing the
`darwin-x64` platform package. The website installer accepts both ARM and
Intel Macs running macOS 11 or later.
- Rust ≥ 1.93 (for source build)

## License

Apache License 2.0 — see [LICENSE](../../LICENSE).
