# anolisa CLI

The `anolisa` CLI installs complete ANOLISA product releases and manages
individual components. Complete Linux releases use AW package records and
versioned program directories. The existing component commands resolve raw/RPM
sources, maintain scoped records, and diagnose or repair drift.

---

## Installation

### Option A: Install script (recommended)

```bash
curl -fsSL https://get.agentic-os.sh | bash
```

On GNU/Linux x86_64 or aarch64 this installs the complete product in user scope
and requires Python 3.6 or newer for download and extraction. On macOS it installs the
standalone CLI. Use `--cli-only` on Linux to select the standalone CLI channel.
See the [installation guide](installation.md) for version selection, archive
installation, file locations, existing-entry conflicts, and legacy overrides.

### Option B: YUM (Alinux)

```bash
sudo yum install anolisa
```

Verify installation:

```bash
anolisa --version
```

---

## Complete product release commands

These commands use a verified, already unpacked Linux product archive. `--bundle`
must name its absolute real directory; use the exact binary shipped in that
archive for initial installation:

```bash
./packages/anolisa-cli/payload/bin/anolisa release inspect --bundle "$PWD" --json
./packages/anolisa-cli/payload/bin/anolisa release install --bundle "$PWD" --scope user --dry-run --json
./packages/anolisa-cli/payload/bin/anolisa release install --bundle "$PWD" --scope user --json
./packages/anolisa-cli/payload/bin/anolisa release status --bundle "$PWD" --scope user --json
```

`release inspect` verifies the bundle's descriptors and payloads without invoking
AW or writing installation state. `release install --dry-run` previews the
selected release and checks for conflicts without installing packages or
changing public commands. `release status` reads durable product progress and
the actual current-version reference; it does not report service health.

Product `--scope` defaults to `user` even when run as root. System installation
requires `--scope system` and root; it never invokes `sudo` automatically.
Do not combine release commands with legacy `--install-mode` or `--prefix`.
The product's versioned program directories and external configuration/data
roots are listed in the [installation guide](installation.md#program-configuration-and-package-database-locations).

Installation records each AW package commit and verifies all packages before
selecting the new release. If it stops, keep the bundle and reported product
record; repeating installation for the identical bundle and scope resumes the
operation. Partial package commits remain recorded rather than being presented
as a release-wide rollback. Existing unrecognized command entries and edited
configuration are preserved and reported as conflicts. Old standalone CLI
records and component data are not automatically migrated.

Installation prepares disabled Provider templates with fixed program/resource
paths. Render their user-directory arguments for the actual invoking user;
these templates are not executable AW transport definitions and AW does not
expand shell variables in argv. System installation does not bind user data or
session sockets to the root installer's environment. Enabling a Provider,
connecting a native Agent, creating service users, applying capabilities, and
starting services require separate configuration. `anolisa status`,
`anolisa update all`, and the other component commands below do not manage the
AW product release lifecycle.

## Existing component scope and visibility

`--install-mode user` writes under the current user's roots, while
`--install-mode system` writes system state and normally requires root. When
the option is omitted, root defaults to system mode and a regular user defaults
to user mode.

Read-only commands use a user-plus-system view. A regular user can therefore
see and diagnose a system installation, and adapter discovery can use its
published contract. Mutating commands still write only the explicitly selected
scope. In particular, a user installation may coexist with a system
installation of the same component.

---

## Existing component commands

### install

Install one component through the configured raw or RPM backend, or plan every
component in the index:

```bash
anolisa --install-mode user install <component>
sudo anolisa --install-mode system install <component>
anolisa install --all
```

An installation in the other scope does not make the selected scope
"already installed." Reinstalling or changing an existing record is handled
by lifecycle planning rather than silently overwriting it.

RPM installs support Yum 3 and DNF 4. ANOLISA prefers yum and uses dnf when
yum is absent. Use `sudo` to check dependencies and conflicts before installing:

```bash
sudo anolisa --install-mode system --dry-run install cosh-ng --backend rpm
```

This check does not install or change packages. Resolve any reported conflicts
before retrying; a successful check does not guarantee the later installation
will succeed. If a requested version conflicts with system restrictions such
as a version lock, the command fails without substituting another version.

If ANOLISA cannot identify the system's package manager, follow the reported
instructions to install missing dependencies manually, then retry.

### uninstall

Remove one installation from the selected scope:

```bash
anolisa uninstall <component>
anolisa uninstall <component> --purge
sudo anolisa --install-mode system uninstall <component> --remove-system-package
```

ANOLISA-owned files and managed RPM packages are removed by their owning
backend. Adopted or observed system RPMs are left installed by default; use
`--remove-system-package` only when native package removal is intended.

### update

Update one component, every recorded component, the CLI binary, or run the
read-only RPM update report:

```bash
anolisa update <component>
anolisa update all
anolisa update self
anolisa update --check
```

`update all` does not update the CLI binary. Delegated members are merged into
one native transaction where possible; each component keeps its own recovery
journal and record.

`update self` refuses to directly replace an AW-managed product CLI. Install a
verified product release to replace that CLI. A standalone CLI retains its
self-update flow; an RPM-owned CLI delegates to the native package manager.

### list and status

Inspect the effective user-plus-system view:

```bash
anolisa list
anolisa list --installed
anolisa status
anolisa status <component>
```

In a user view with records in both scopes, the user record is active and the
system record remains visible as shadowed state. A system-mode view reads only
the system root; it does not enumerate other users' state.

Untracked RPM observations use the same package resolver as installation.
An index mapping takes precedence over historical package aliases and
capabilities: an old cosh-ng RPM providing `anolisa-component(cosh)` is shown
under cosh-ng. `action=install`
identifies a lifecycle action; it is not a native dependency check. Use the
RPM install dry-run to check whether the packages can coexist.

### doctor

Run read-only health, dependency, service, state, and recovery-journal checks:

```bash
anolisa doctor
anolisa doctor <component>
anolisa --dry-run doctor <component>
```

`doctor` scans every root in the current visibility view: user mode includes
the user root and a readable system root, while system mode includes only the
system root. It qualifies system repair suggestions with
`sudo anolisa --install-mode system` when the current invocation cannot mutate
that root. `--fix` is reserved in this release; follow the reported `fix_plan`
explicitly.

For raw installations, `status` and `doctor` allow content edits to files
declared as `type = "config"`. Missing files, unsafe paths, unexpected symlinks,
and permission or capability drift still fail checks; ordinary data and
executable files still undergo SHA-256 verification. Older installation records
recover unambiguous config declarations from their saved component manifest in
memory. If overlapping directory declarations mix config and immutable kinds,
legacy files retain digest checking because their source mapping is unknown.
Back up edited configs and reinstall the component to record accurate kinds.
Diagnostics do not rewrite state.

### restart

Restart services recorded for an installation in the selected scope:

```bash
anolisa --dry-run --install-mode user restart <component>
anolisa --install-mode user restart <component>
anolisa --dry-run --install-mode system restart <component>
sudo anolisa --install-mode system restart <component>
```

`--dry-run` lists the units that would restart and does not run
`systemctl daemon-reload` or `systemctl restart`. System-mode preview
reads recorded state without taking the exclusive install lock, so it
does not need write access to the state root.

### upgrade

Plan or apply the system/RPM image upgrade. Raw-managed components are reported
as skipped rather than migrated to another backend:

```bash
anolisa --install-mode system --dry-run upgrade
sudo anolisa --install-mode system upgrade
sudo anolisa --install-mode system upgrade --target <profile>
```

### adopt, repair, and forget

Manage state without confusing package ownership:

```bash
sudo anolisa --install-mode system adopt <component>
sudo anolisa --install-mode system repair <component>
anolisa --install-mode user forget <component>
sudo anolisa --install-mode system forget <component>
```

`adopt` records an existing system RPM as delegated-adopted without claiming
native removal authority. `repair` reconciles a scoped record with rpmdb or an
interrupted journal. `forget` removes only the record in the selected scope and
never performs package or owned-file removal; a user-scoped forget cannot
delete a visible system record.

If an operation is pending, `forget` directs you to `repair`, even when an
install failed before creating an installation record. Repair reconciles the
journal with rpmdb; forgetting must not discard evidence of a transaction
that may already have changed the system.

### adapter

Manage component adapters:

```bash
anolisa adapter scan
anolisa adapter enable <component> [framework]
anolisa adapter disable <component> [framework]
anolisa adapter status [component]
```

For OpenClaw plugins, `adapter enable` accepts the plugin's declared capabilities
by default. ANOLISA adds `--accept-capabilities` to installation and activation
only when the corresponding command's help advertises it. To withhold these
automatic grants, use:

```bash
anolisa adapter enable tokenless openclaw --no-accept-capabilities
anolisa --dry-run adapter enable tokenless openclaw --no-accept-capabilities
```

The dry-run plan shows both commands without the consent flag. This option is
only valid for an OpenClaw plugin adapter; other frameworks and skill-only
adapters reject it. It does not revoke capabilities already authorized in
OpenClaw, and hosts without the consent flag continue to apply their own policy.
Environment settings such as `ANOLISA_ACCEPT_CAPABILITIES` or
`AGENT_MEMORY_ACCEPT_CAPABILITIES` do not configure `adapter enable`; use the
CLI option for this entry point.

If OpenClaw requires a new grant, its refusal remains an execution failure
(`EXECUTION_FAILED`, exit code 1) and the adapter receipt is kept for cleanup
with `anolisa adapter disable tokenless openclaw`. Review the reported
capabilities before authorizing them separately. Capability consent does not
authorize `--allow-unsafe-plugin-install`; a consent rejection is reported
separately from a plugin-safety rejection.

For OpenCode, manage an installed Tokenless plugin with:

```bash
anolisa adapter enable tokenless opencode
anolisa adapter status tokenless
anolisa adapter disable tokenless opencode
```

The driver finds `opencode` on PATH, or uses `OPENCODE_BIN`. It registers the
manifest's `.js` or `.ts` entry as `plugins/<plugin_id>.<extension>` under
`OPENCODE_CONFIG_DIR`, otherwise `XDG_CONFIG_HOME/opencode`, otherwise
`~/.config/opencode`. Custom directory values must be absolute. Keep the same
directory configuration for later status and disable commands; if it changes,
restore the original environment before retrying cleanup. Project-local plugin
installation and npm plugin management are outside this driver's scope.

Enable adopts an existing symlink to the same plugin source, including one
created by Tokenless's standalone installer. Relative links must resolve lexically to the
recorded source path; directory aliases are rejected so cleanup remains possible after
package removal. After adoption, disable removes
that link. A conflicting file, directory, or different symlink is preserved;
failed cleanup keeps the receipt so it can be retried after resolving the conflict.
Same-path upgrades replace the entry atomically. If enable or disable is interrupted,
retry the command with the same configuration to recover pending changes. If an error
reports a preserved entry, keep its recovery directory, resolve the conflicting public
path, and retry; recovery also restores displaced directories. Complete ANOLISA disable and
recovery before switching back to a standalone installer, which does not process these journals.
The configuration directory's filesystem must support atomic exchange and non-overwriting
rename for replacement and cleanup. If it does not, the operation fails; resolving a pathname
conflict alone will not add the missing filesystem support.
If the old installer used `TOKENLESS_OPENCODE_CONFIG_DIR`, set
`OPENCODE_CONFIG_DIR` to that same directory before enabling through ANOLISA.

Restart OpenCode after enabling or disabling. Status verifies the link and
package source and preserves `cleanup_failed` when enable was interrupted or cleanup still
needs a retry, even if the active link matches. Retry enable or complete disable to resolve it. Runtime loading is reported as `unknown`; an existing link
does not prove a running OpenCode process loaded it. `--dry-run` previews the
operation without modifying plugin files or receipts.

#### QwenWorkCN (macOS)

```bash
anolisa --dry-run adapter enable tokenless qwenwork
anolisa adapter enable tokenless qwenwork
anolisa adapter status tokenless
anolisa adapter disable tokenless qwenwork
```

The driver sideloads receipt-owned files and never changes desktop activation.
Restart QwenWorkCN after enable/disable and open a new session. Status reports
loading and activation as `unknown`; file presence alone is not proof of activation.
Keep the same `TOKENLESS_QWENWORK_HOME` for subsequent operations;
`TOKENLESS_QWENWORK_APP` overrides the detected application bundle. Existing
script installations must be removed with their original uninstaller before CLI
enable. Modified and unrelated files are preserved. See [QwenWork integration](../tokenless/framework-integration.md#qwenwork-macos)
for dependencies, migration, and the 1.2.5 native Bash / 1.3.2 workspace Shell baselines.

### logs and bug reports

Inspect component logs or generate a diagnostic bundle:

```bash
anolisa logs <component>
anolisa logs <component> --limit 50
anolisa logs <component> --severity warn
anolisa logs <component> --since '2026-10-01T00:30:00Z'
anolisa bug
```

`--level` is an alias for `--severity`.

`--since` accepts an inclusive RFC3339 lower bound. Time-zone offsets and
fractional seconds are compared as instants: `2026-10-01T08:30:00+08:00` and
`2026-10-01T00:30:00Z` select the same time window. `--limit` keeps the most recent
matching entries in append order.

An invalid `--since` value returns `INVALID_ARGUMENT`. Records whose `started_at`
cannot be parsed as RFC3339 do not match a time filter; omit `--since` to inspect
them. Malformed JSON remains a query error. These queries do not modify the log.

With `--component cosh-ng`, `anolisa bug` also asks the installed
`cosh-shell` binary to export its sanitized diagnostic bundle to a fresh
private path (`0600`, never overwritten) and summarizes the bundle's health
finding IDs and manifest in the report. The binary is resolved from the
cosh-ng installation's private libexec locations (the raw contract's
directory and the RPM's `/usr/libexec` path), with `COSH_SHELL_BIN` as an
override and PATH as a development fallback; the printed manual and
reproduction commands use the resolved absolute path so they stay runnable.
The bundle is written below the calling user's own state root —
even when diagnosing a system-scope installation — and is never uploaded;
review it locally before attaching it to an issue. If no bundle can be
produced, the report says so explicitly and prints the manual
`cosh-shell diagnostics export` command instead.

---

## Recovery Behavior

Install, uninstall, update, adopt, and repair write recovery intent in the
selected state root before their lifecycle side effects. Native package
operations are forward-only: if dnf may have committed but the ANOLISA record
did not, the journal remains pending and `anolisa repair <component>`
re-observes rpmdb. Owned-file operations keep verified backups and compensate
in reverse order on failure. `forget` is an atomic record-only state update; it
does not perform package/file side effects or create a recovery journal.

`upgrade` remains a compatibility orchestrator rather than a planner/journal
consumer. It refuses existing pending recovery and re-observes rpmdb after a
transaction failure, but it does not create a per-component recovery journal.
After an interrupted `upgrade`, run `anolisa doctor` and reconcile any reported
component drift before starting another lifecycle mutation.

Do not delete a pending journal merely to unblock a command. Run `doctor` to
identify its scope and subject, then run the qualified `repair` command. A
malformed or ambiguous journal is intentionally left pending for manual
inspection.

---

## Global Options

| Option | Description |
|--------|-------------|
| `--install-mode user\|system` | Select the existing component command's mutation scope |
| `--prefix <PATH>` | Override the existing component command's install prefix |
| `--dry-run` | Print the plan without executing it |
| `--json` | Emit machine-readable JSON |
| `-v, --verbose` | Increase verbosity |
| `-q, --quiet` | Suppress non-error output |
| `--no-color` | Disable colored output |
| `--version` | Show the CLI version |
| `--help` | Show command help |

---

## Existing component workflow example

With your Agent already installed, scan for its adapter and use the reported
name in place of `<framework>`:

```bash
curl -fsSL https://get.agentic-os.sh | bash -s -- --cli-only
anolisa env
anolisa install tokenless
anolisa adapter scan
anolisa adapter enable tokenless <framework>
anolisa doctor
anolisa status
```

---

## Configuration

Backend selection and endpoints are read from `/etc/anolisa/repo.toml` in
system mode or `~/.config/anolisa/repo.toml` in user mode:

```toml
schema_version = 1
default_backend = "raw"

[backends.raw]
base_url = "https://repo.example.com/anolisa/v1/"
```

The raw backend re-fetches the distribution index on every run: there is no
index cache and no stale-index fallback, so an unreachable repository fails the
command instead of resolving against older data. `cache_ttl_secs` and
`offline_fallback` were never wired up; configs that still set them keep
parsing, but the values are ignored.

For RPM repositories that require a proxy or a private CA, set `proxy`,
`proxy_username`, `proxy_password`, or `sslcacert` directly in `[main]` of
`/etc/yum.conf` (or `/etc/dnf/dnf.conf` when the former is absent).
Without explicit settings, ANOLISA uses environment proxies (`http_proxy`,
`https_proxy`, `all_proxy`, `no_proxy`) and the system's trusted certificates.

CLI flags override the operation being run; there is no `[install] mode`
setting.

---

## See Also

- [Installation Guide](installation.md)
- [Troubleshooting](troubleshooting.md)
