# Installation Guide

[中文版](../../zh/getting-started/installation.md)

Install the complete ANOLISA product on GNU/Linux, then configure the Agent
integrations and services you need. macOS and existing component workflows use
the standalone CLI channel described below.

## Install the complete product on Linux

On GNU/Linux x86_64 or aarch64, the default installer downloads the complete
product archive, verifies its declared byte size and SHA-256, and runs its local
installer. Python 3.6 or newer is required for this download and safe extraction step.
The complete product does not support macOS or musl Linux.

New archives declare package profiles. Both user and system scope default to
`complete`: all packages delivered for the platform, including backends, AW,
implemented Providers, skills and native integrations. Installation does not
start every service or activate Agent plugins. A program that needs root,
kernel features or further configuration is not made ready merely by installing
its files. Narrower profiles require an explicit choice. Legacy schema-1
archives retain their original full install.

```bash
curl -fsSL https://get.agentic-os.sh | bash
export PATH="$HOME/.local/bin:$PATH"
anolisa --version
```

After ordinary installation, `install.sh` runs `anolisa setup`; `--json`,
`--quiet`, `--dry-run` and help skip that follow-up. A piped invocation prints
installed versions, availability and next steps. In a terminal, setup can guide
the invoking user through creating a new AW configuration. Run `anolisa setup`
again after installing an Agent, or use `anolisa overview --check-services` to
review local installation and service evidence.

For a Linux system with a configured RPM backend, explicitly select native
installation of the complete published component set:

```bash
curl -fsSL https://get.agentic-os.sh | bash -s -- --rpm
```

This prepares the standalone CLI, runs `anolisa install --all --backend rpm`
in system mode, and uses `sudo` for that action when needed. It prints a
non-interactive system overview; run `anolisa setup` as the ordinary user who
will use the Agent. This route does not automatically select RPM based on the
host distribution. `--rpm` cannot be mixed with product profile/version options
or individual-component options.

For the default tarball route, scope is `user`, including when the invoking user
is root. Select an available product version or an independent release index with:

```bash
curl -fsSL https://get.agentic-os.sh | bash -s -- --release-version 1.2.3 --scope user
curl -fsSL https://get.agentic-os.sh | bash -s -- --scope user --profile standard-user
curl -fsSL https://get.agentic-os.sh | bash -s -- --release-version 1.2.3 --release-url https://example.org/anolisa/release-index.json --scope user
```

Replace `1.2.3` with an available product version. `--release-version` accepts
`stable` or a SemVer version. `ANOLISA_RELEASE_VERSION` and
`ANOLISA_RELEASE_URL` provide the same settings to the shell running the
installer. The default stable index is
`https://anolisa.oss-cn-hangzhou.aliyuncs.com/anolisa-releases/anolisa/v1/product/release-index.json`.
A concrete version uses `v1/product/releases/<version>/release-index.json`
below the same repository root unless `--release-url` is supplied. An explicit
version must match `product_version` in the selected JSON index. Product failure
stops installation; it does not switch to the CLI channel.

System tarball installation requires explicit `--scope system` and an already
privileged process. The product archive installer never invokes `sudo` for you.
Download and inspect the script or archive first, then run it with the
privileges you choose.

### Install a downloaded archive

Download the matching `anolisa-VERSION-linux-ARCH-gnu.tar.gz` and its `.sha256`
file from the release publisher. Verify it, unpack it, and run its installer.
The archive contains one directory named `anolisa-RELEASE_ID-linux-ARCH`;
use the actual names from the release.

```bash
sha256sum -c anolisa-VERSION-linux-x86_64-gnu.tar.gz.sha256
tar -xzf anolisa-VERSION-linux-x86_64-gnu.tar.gz
cd anolisa-RELEASE_ID-linux-x86_64
./install.sh --scope user
```

For system installation, run `./install.sh --scope system` from a root shell.
Use `--profile standard-user` for the supported user component collection,
`--profile minimal` for AW and CLI only, or `--profile native-openclaw` with
system scope for the available native OpenClaw integration packages.
`ANOLISA_RELEASE_PROFILE` selects the same profile in the download script.
The profile list and exact package composition are included in
`anolisa release inspect --bundle "$PWD" --json`.

Local installation needs no Python or network downloads. It invokes the exact
CLI in the archive. Preview, inspect, or read progress with that binary, using
an absolute path to the unpacked directory:

```bash
./packages/anolisa-cli/payload/bin/anolisa release inspect --bundle "$PWD" --json
./packages/anolisa-cli/payload/bin/anolisa release install --bundle "$PWD" --scope user --dry-run --json
./packages/anolisa-cli/payload/bin/anolisa release status --bundle "$PWD" --scope user --json
```

Keep the archive available if installation stops. Repeating the same release
installation with the same scope verifies installed packages and resumes its
recorded progress. A failure can leave packages committed in the package database; preserve the
reported record and recovery information. The new release is selected only
after its packages pass verification. Installation does not enable Providers,
connect native Agents, start daemons, or activate systemd units.

If a new product install reports an unfinished package transaction, explicitly
run `anolisa release recover --scope user --json` (or, as root,
`anolisa release recover --scope system --json`) before retrying. Recovery does
not activate a product version. Existing AW-owned product versions keep using
the original AW database and `aw recover`; they are not automatically transferred.

### Program, configuration, and package database locations

`ARCH` is `x86_64` or `aarch64`. `RELEASE_ID` identifies the immutable release,
including its source revision, and can differ from the product version.

| Content | User scope | System scope |
|---------|------------|--------------|
| Private programs | `~/.local/lib/ARCH-linux-gnu/anolisa/releases/RELEASE_ID/` | `/usr/local/lib/anolisa/releases/RELEASE_ID/` |
| Static resources | `${XDG_DATA_HOME:-~/.local/share}/anolisa/resources/releases/ARCH-linux-gnu/RELEASE_ID/` | `/usr/local/share/anolisa/releases/RELEASE_ID/` |
| Current-version reference | `~/.local/lib/ARCH-linux-gnu/anolisa/current` | `/usr/local/lib/anolisa/current` |
| Public commands | `~/.local/bin/` | `/usr/local/bin/` |
| Product configuration templates | `${XDG_CONFIG_HOME:-~/.config}/anolisa/` | `/usr/local/etc/anolisa/` |
| Product progress and entry records | `${XDG_STATE_HOME:-~/.local/state}/anolisa/` | `/var/lib/anolisa/` |
| Product download cache | `${XDG_CACHE_HOME:-~/.cache}/anolisa/` | `/var/cache/anolisa/` |
| New ANOLISA raw package database | `${XDG_STATE_HOME:-~/.local/state}/anolisa/packages/` | `/var/lib/anolisa/packages/` |

ANOLISA records both the private program prefix and its independent resource root.
The private prefix's controlled `share` link points to those same versioned
resources so component launchers retain their resource lookup paths. Programs
and resources may be on different supported filesystems. Do not copy or rename
registered directories or change this link by hand.

The system product state directory `/var/lib/anolisa` remains private to
administrators. System product inventory, file verification and recovery require
root. Ordinary users can still execute installed programs, read static resources,
and use `anolisa overview` to inspect a public product summary. That summary is
installation evidence rather than file verification. AW runtime file leases do
not read the private product database.

Existing product versions recorded in the AW database keep that original
ownership. The installer resumes one only when its product record and installed
files match; it does not automatically move the database or register those files
again. The historical `.aw-packages/aw-core.json` filename remains as a locator,
with an explicit manager marker for new records. AW package queries and writes
reject an ANOLISA-owned locator. Use `anolisa release status` for product progress;
AW runtime file locks remain compatible without granting package ownership.

Installing software for the system does not choose a shared runtime identity.
Each ordinary user still uses their own XDG configuration, data, state, cache
and session runtime directories. Root's HOME or XDG values are never embedded
in another user's Provider invocation. The disabled Provider templates require
explicit rendering with the invoking user's directories before use in an AW
configuration. A separately configured system service uses its own service
identity and `/var/lib`, `/var/cache` and `/run` directories.

The tarball uses the administrator's `/usr/local` hierarchy. Native RPM/DEB
packaging has separate `/usr` and `/etc` rules; the tarball installer does not
create native package records or take over native-owned commands.

An existing unrecognized command is preserved. Use the bundled CLI's
`migrate plan --bundle "$PWD" --scope user --output "$HOME/anolisa-migration.json"`
to inspect supported legacy raw installations, then review the plan before
`migrate apply --plan "$HOME/anolisa-migration.json"`. A historical standalone
CLI has no ownership receipt: explicitly accept its independently verified
binary checksum with `--legacy-cli-sha256 SHA256` when creating the plan.

Migration requires the same scope, inactive daemons and disabled native
adapters. It preserves configuration, data and rollback files, installs the new
release separately, and retires only the selected old raw records after success.
Repeat the same saved apply after interruption. RPM-owned installations remain
with their native manager; changing native Agent integration to AW is a separate
explicit configuration operation. Details are in the
[CLI reference](anolisa-cli.md).

## Existing CLI and component installation

On macOS the default script continues to install only the CLI. On Linux select
that channel with `--cli-only`. It does not require Python 3.

`--component`, `--cosh-ng`, `--backend`, `--upgrade`, `--uninstall`, and
`--install-mode` retain their existing meanings and select this channel.
The same applies to `ANOLISA_VERSION` (CLI version), `ANOLISA_UPDATE_URL`
(CLI TOML index), `ANOLISA_INSTALL_DIR` (CLI directory), and `ANOLISA_MIRROR`
(repository base), including explicitly empty legacy environment variables.
Do not mix these settings or `--cli-only` with product release settings or
`--scope`: the script rejects that combination before downloading.

The remaining component, adapter, service, status, and update examples describe
this existing component channel. They do not manage a complete AW product release.

For preinstalled systems, cloud environments, or a container base image, see
[Images and cloud services](images.md).

---

## Step 1: Install the standalone ANOLISA CLI

The `anolisa` CLI is the unified entry point for managing all ANOLISA components.

### Option A: Install script (recommended)

```bash
curl -fsSL https://get.agentic-os.sh | bash -s -- --cli-only
```

The same installer can manage one published component immediately after it
prepares the CLI. Pass the registry name, backend, and scope. Raw is the
default backend. On Alibaba Cloud Linux 4, install cosh-ng through the RPM
backend so dnf selects the matching system libraries:

```bash
curl -fsSL https://get.agentic-os.sh | bash -s -- --component cosh-ng --backend rpm --install-mode system
export PATH="$HOME/.local/bin:$PATH"
```

`--cosh-ng` is shorthand for `--component cosh-ng`. Component, backend, and
platform checks remain owned by the CLI, so the installer does not maintain a
second component allowlist. In explicit system mode, the script uses `sudo`
only for the component action; the CLI remains in the current user directory.
Without `--install-mode`, scope follows the normal ANOLISA euid default.

This workflow refuses to overwrite a product-managed or native-package-owned CLI.
It installs the CLI to `~/.local/bin` by default, but the shell runs
the first `anolisa` found in `PATH`. If an older npm or Homebrew installation
comes first, the installer reports both paths and versions and exits without
printing `done`. It never removes another channel's files. Put the standalone
installation first and refresh the current shell:

```bash
export PATH="$HOME/.local/bin:$PATH"
hash -r
```

Persist that `export` in the profile read by Bash or Zsh. Fish users can run
`fish_add_path --move "$HOME/.local/bin"`. To remove the older installation instead,
use its owner: `npm uninstall -g @anolisa/cli` or `brew uninstall anolisa`.

### Option B: YUM (Alinux)

```bash
sudo yum install anolisa
```

After installation, verify:

```bash
anolisa --version
```

Before installing a component that uses the current raw package contract,
update an older CLI through the tool that owns it:

```bash
# CLI installed by get.agentic-os.sh
anolisa update self

# RPM-owned CLI
sudo anolisa update self
```

AgentSecCore requires `anolisa` 0.2.17 or later. The CLI reports an update hint
and stops before changing the host when it cannot safely read a package
contract.

A product-managed CLI rejects `anolisa update self` to protect the installed
release. Use a verified product release installer instead. RPM-owned CLI
self-update continues to use the native package manager.

---

## Step 2: Environment Detection

Run the environment check to identify your system capabilities:

```bash
anolisa env
```

This displays:
- OS and architecture
- Available filesystems (btrfs for ws-ckpt)
- FUSE availability (for skillfs)
- Installed Agent runtimes (cosh-ng, OpenClaw, Hermes)
- Kernel features (eBPF for agentsight)

---

## Step 3: Install Components

Install components individually based on your needs:

```bash
anolisa install <component>
```

### Available Components

| Component | Description | Supported modes |
|-----------|-------------|-----------------|
| `cosh-ng` | AI-native terminal and deterministic Agent runtime (experimental) | system (Linux), user or system (macOS 11+ (arm64 / x86_64)) |
| `os-skills` | System management and DevOps skills | user, system |
| `tokenless` | Token optimization (compression) | user, system |
| `ws-ckpt` | Workspace checkpoint/rollback | **system** |
| `skillfs` | FUSE virtual skill filesystem | **system** |
| `agent-memory` | MCP-based persistent memory | user, system |
| `agentsight` | eBPF tracing and dashboard | **system** |
| `sec-core` | Local security runtime, scanners, and adapters | **system** |
| `ktuner` | Deterministic kernel-tuning engine for agents | **system** (RPM, Linux x86_64) |

> **Note**: System-only components require `sudo` and an explicit system scope:
> ```bash
> sudo anolisa --install-mode system install agentsight
> ```

On Alibaba Cloud Linux 4, install cosh-ng from the RPM backend in system mode,
then start the terminal with `cosh`:

```bash
sudo anolisa --install-mode system install cosh-ng --backend rpm
cosh
```

The public installer combines CLI bootstrap and component installation:

```bash
curl -fsSL https://get.agentic-os.sh | bash -s -- --component cosh-ng --backend rpm --install-mode system
export PATH="$HOME/.local/bin:$PATH"
```

On macOS 11+ (arm64 / x86_64), the cosh-ng raw package has a separate user-scope contract:

```bash
curl -fsSL https://get.agentic-os.sh | bash -s -- --component cosh-ng --backend raw --install-mode user
export PATH="$HOME/.local/bin:$PATH"
```

Use the component name `sec-core` with the ANOLISA CLI. The Alinux RPM keeps
its package name `agent-sec-core`:

```bash
sudo anolisa --install-mode system install sec-core

# If you install the RPM directly, let ANOLISA track it before adapter setup
sudo yum install anolisa agent-sec-core
sudo anolisa --install-mode system adopt sec-core
```

Continue to [Step 4](#step-4-adapter-setup), then run
`anolisa adapter enable sec-core <framework>` as the user who owns the target
Agent configuration.

### Install All Components

```bash
anolisa install --all
```

### YUM Alternative (Alinux)

For each component, you can also use YUM. Install the system CLI in the same
transaction so sudo can find it without relying on a user-local PATH. A direct
RPM installation does not create an ANOLISA state record, so adopt it before
using lifecycle or adapter commands:

```bash
sudo yum install anolisa <rpm-package>
sudo anolisa --install-mode system adopt <component>
```

---

## Step 4: Adapter Setup

Adapters bridge components to specific Agent frameworks. Enable an adapter after installing the component:

```bash
anolisa adapter scan
anolisa adapter enable <component> [framework]
```

### Examples

```bash
# Tokenless hook for cosh
/usr/share/tokenless/scripts/install.sh --cosh

# Tokenless plugin for OpenClaw
/usr/share/tokenless/scripts/install.sh --openclaw

# ws-ckpt plugin for OpenClaw (requires OpenClaw >= 2026.2.13)
ws-ckpt plugin install --runtime openclaw

# ws-ckpt plugin for Hermes
ws-ckpt plugin install --runtime hermes

# AgentSecCore plugin for OpenClaw
anolisa adapter enable sec-core openclaw
```

The system installation owns the component files. Adapter enablement runs as
the user who owns the target Agent configuration, so it does not need `sudo`
for a normal user-scoped framework installation.

---

## Step 5: Start Long-running Services

Installing a component and starting its resident service are separate actions.
AgentSight installs `agentsight.service` and its enforcer dependency without
enabling either unit. Start the main unit when the machine is ready to collect
events:

```bash
sudo systemctl enable --now agentsight.service
sudo systemctl status agentsight.service
```

The main unit runs eBPF tracing and the Dashboard together as root and keeps its
data private under `/var/log/sysak/.agentsight`. Use `sudo` for commands that
query service data. For foreground troubleshooting, stop the unit first, then
run the tracer and server as root in separate terminals:

```bash
sudo systemctl stop agentsight.service

# Terminal 1
sudo agentsight trace

# Terminal 2
sudo agentsight serve
```

---

## Step 6: Verify Installation

Check the status of all installed components:

```bash
anolisa status
```

Run the built-in diagnostic:

```bash
anolisa doctor
```

---

## Uninstallation

Remove a specific component:

```bash
anolisa uninstall <component>
```

The public installer also accepts an installed component name for removal. It
refreshes the stable CLI first, then delegates to `anolisa uninstall`:

```bash
curl -fsSL https://get.agentic-os.sh | bash -s -- --component cosh-ng --install-mode system --uninstall
```

There is no batch uninstall command. List the installed records, then remove
each intended component explicitly so its authority and package-removal policy
are reviewed independently:

```bash
anolisa list --installed
anolisa uninstall <component>
```

---

## Upgrade

Update a specific component:

```bash
anolisa update <component>
```

Update a selected component and the script-installed stable CLI together:

```bash
curl -fsSL https://get.agentic-os.sh | bash -s -- --component cosh-ng --install-mode system --upgrade
```

Update all installed components:

```bash
anolisa update all
```

`update all` updates recorded components but not the CLI binary. Use
`anolisa update self` for a script-installed CLI or `sudo anolisa update self`
for an RPM-owned CLI.

---

## Next Steps

- [anolisa CLI Reference](anolisa-cli.md)
- [cosh-ng Quick Start](../cosh-ng/index.md)
- [Troubleshooting](troubleshooting.md)
