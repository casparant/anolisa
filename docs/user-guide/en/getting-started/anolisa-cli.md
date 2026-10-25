# anolisa CLI

The `anolisa` CLI installs complete ANOLISA product releases and manages
individual components. Complete Linux releases use ANOLISA package records and
versioned program directories. The existing component commands resolve raw/RPM
sources, maintain scoped records, and diagnose or repair drift.

---

## Installation

### Option A: Install script (recommended)

```bash
curl -fsSL https://get.agentic-os.sh | bash
```

On GNU/Linux x86_64 or aarch64 this installs the product's complete user-scope profile
and requires Python 3.6 or newer for download and extraction. Use the explicit
`--rpm` installer option for the complete published native RPM component set
in system scope. On macOS it installs the
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

## Local overview and user setup

`anolisa overview` combines actual recorded component versions from product,
legacy component and native RPM records with local executable availability.
It does not download an index or execute component binaries to guess versions.
Unavailable evidence is reported separately. An ordinary user can inspect public
system product summaries without reading the administrator-private database.

```bash
anolisa overview
anolisa overview --scope system --check-services --json
anolisa overview --candidate-bundle /absolute/unpacked-candidate --json
anolisa setup
anolisa setup --non-interactive
```

`--scope user|system|all` filters visible installation evidence; it does not grant
write permissions. `--check-services` queries systemd state without starting
services or treating an active unit as an endpoint health check. Candidate
comparison verifies a local bundle and reports package version differences;
remote latest versions remain unchecked. Review `install --bundle DIR --dry-run`
for a product update, or `migrate plan --help` for supported legacy migration.

`setup` prints the same overview. On a terminal it can ask for an installed AW
Adapter, an Agent executable and a new user configuration destination. Piped,
JSON and `--non-interactive` invocations do not prompt. For unattended creation,
choose a name reported by `aw agent list --json` and an actual Agent executable:

```bash
aw agent list --json
anolisa setup --write-config "$HOME/.config/anolisa/aw.yaml" \
  --agent qoder --agent-bin /absolute/path/to/qoder --dry-run --json
anolisa setup --write-config "$HOME/.config/anolisa/aw.yaml" \
  --agent qoder --agent-bin /absolute/path/to/qoder --yes
```

Add repeated `--agent-arg VALUE` for literal Agent arguments. Creation uses the
invoking user's identity and XDG state, preserves existing files, and starts with
empty Provider and event policy. `--provider NAME` with an explicit
`--provider-template ABS_JSON` renders selected definitions into a separate
disabled file for review; it does not enable them.
`setup --preset host --service-scope system` previews a service plan. Applying that plan remains an
explicit `preset apply` operation. System software installation and ordinary-user
Agent configuration are separate steps.

## Product release commands

A Linux product archive contains the available components; a product profile
chooses which packages to install. `--bundle` names its absolute, unpacked real
directory. For the first installation, run the CLI inside that archive:

```bash
./packages/anolisa-cli/payload/bin/anolisa release inspect --bundle "$PWD" --json
./packages/anolisa-cli/payload/bin/anolisa install --bundle "$PWD" --scope user --dry-run --json
./packages/anolisa-cli/payload/bin/anolisa install --bundle "$PWD" --scope user --json
./packages/anolisa-cli/payload/bin/anolisa release status --bundle "$PWD" --scope user --json
```

`release inspect` verifies every delivered package and lists the bundle's product
choices without starting AW or writing installation state. Selection happens
after full verification, so a damaged unselected package also fails verification.
`install --bundle` and `release install --bundle` use the same installation
implementation. `--dry-run` checks the selection and existing-entry conflicts
without installing packages or publishing commands. `release status` reads
product progress and the current-version reference, not service health.

New schema-2 bundles declare these profiles. The exact package inventory and
available native profiles come from `release inspect --json`.

| Profile | Scope | Installed files |
| --- | --- | --- |
| `complete` | user or system | Every delivered platform package, including native integrations |
| `minimal` | user or system | ANOLISA CLI and AW; explicit reduced selection |
| `standard-user` | user | User-capable cores, supported Providers, and skills |
| `standard-system` | system | Platform-supported backend cores, supported Providers, and skills |
| `native-<agent>` | system | Standard backends plus native integration bundles needed by that Agent |

For schema-2 bundles, `complete` is the default in both scopes. It installs
all delivered packages, including native plugins and any declared runtime
dependencies. Narrower profiles such as `minimal` and `standard-user` remain
explicit choices. Profiles choose files; they do not enable services, switch an
Agent to AW or satisfy privileged runtime requirements. Legacy schema-1
bundles retain their complete-package behavior and do not accept profile or
component selection.

A native integration bundle can contain resources for several Agents. Installing
it does not activate those Agents' plugins; activation remains explicit.

Select independent components from the same archive, including their native
integration resources and package dependencies:

```bash
anolisa release install --bundle /absolute/unpacked-release --component tokenless --component agent-memory --scope user
anolisa release status --bundle /absolute/unpacked-release --component tokenless --component agent-memory --scope user
```

`--profile` and `--component` are mutually exclusive. Every component selection
also includes the ANOLISA CLI. AW is included only when selected by the profile
or a declared package dependency; non-AW component selections do not require it.
The shorter form
`anolisa install tokenless --bundle /absolute/unpacked-release --scope user`
selects one archive component; `anolisa install tokenless` continues to use the
configured independent component backend. `install --all` still enumerates the
component repository and does not select a product profile.

Product scope defaults to `user`, including when run as root. System installation
requires root and explicit `--scope system`; it never invokes `sudo` automatically.
Do not combine product commands with legacy `--install-mode` or `--prefix`.
See the [installation guide](installation.md#program-configuration-and-package-database-locations)
for program, resource, configuration, and package-database locations.

Installation records each local package commit and verifies the selected packages
before activating the new release. Keep the bundle and reported product record
after an interruption. Repeat the same bundle, scope, profile or component set
to resume. Different selections receive different installation identities and
cannot resume each other's partial operation. Partial package commits remain
recorded; an interruption is not a release-wide rollback. Existing unrecognized
commands and edited configuration are preserved and reported as conflicts.
Use the migration command below for supported historical raw installations.

Activating a smaller selection removes unchanged product command launchers that
the new selection no longer provides. Administrator-edited launchers block that
switch and remain intact. Earlier version payloads remain available for rollback.

Installation prepares disabled Provider templates with fixed program and
resource paths. Render user-directory arguments for the actual invoking user;
these templates are not executable AW transport definitions, and AW does not
expand shell variables in argv. System installation does not bind user data or
session sockets to the root installer's environment. Enabling Providers,
connecting native Agents, preparing service users, capabilities, configuration,
and units are separate deployment actions. Existing component `status`,
`update all`, and `uninstall` commands do not manage product-release records.

### Query product package records and verify files

```bash
anolisa release list --scope user --json
anolisa release doctor --bundle /absolute/unpacked-release --scope user --json
sudo anolisa release list --scope system --json
sudo anolisa release doctor --bundle /absolute/unpacked-release --scope system --json
```

`release list` reads the selected ANOLISA package database without checking
payload files or requiring a bundle. It does not enumerate older AW-owned
versions from a second database. `release doctor` verifies the selected release's
recorded files without repairing them; supply the same `--profile` or
`--component` selection used for installation. A verified existing product
record may still refer to its original AW database. `release status` instead
reports product installation progress. Queries do not start AW, Providers or
component services, create a database or transfer ownership.

System product records remain under the administrator-private
`/var/lib/anolisa`, so inventory and verification use root. Ordinary users can
still run the installed programs and read static resources without access to
these package records.

### Recover product package transactions

```bash
anolisa release recover --scope user --json
sudo anolisa release recover --scope system --json
```

Choose the original installation scope. This explicitly recovers unfinished
transactions in the new ANOLISA package database; no bundle is required.
Recovery reports the database path and committed, rolled-back and conflicting
operations. It does not switch the active product release or start services.
Resolve reported conflicting files before retrying the same installation.
`--dry-run` is rejected because recovery changes files. Existing AW-owned
product versions retain their original database and use `aw recover` in that
scope; this command does not move their ownership to ANOLISA.

### Remove an inactive product version

After activating a different release, inspect and remove the previous version
with its original bundle and package selection:

```bash
anolisa release uninstall --bundle /absolute/old-unpacked-release --scope user --dry-run --json
anolisa release uninstall --bundle /absolute/old-unpacked-release --scope user --yes --json
```

Add the original `--profile NAME` or repeated `--component ID` when applicable.
System cleanup requires root and `--scope system`. Execution requires `--yes`;
preview does not. The command refuses the active version, changed product
controls and any recorded or pending service-unit publication for that version.
It removes verified owned payloads and the recorded resource alias, preserving
configuration, data, unknown files, current-version controls and empty package
registration metadata. Product progress records `removing` and then `removed`.
After an interrupted package transaction, run `release recover` and retry the
same cleanup. Historical AW-owned versions retain their original AW removal
authority; this command does not adopt them.

### Service presets and foreground operation

After installing a system raw product and preparing component configuration,
publish its selected system units from the same bundle and profile:

```bash
sudo anolisa release services --bundle /absolute/unpacked-release --profile standard-system --scope system --service ws-ckpt --service agent-sec-core --service agentsight --json
sudo anolisa release services --bundle /absolute/unpacked-release --profile standard-system --scope system --service ws-ckpt --service agent-sec-core --service agentsight --apply --json
```

The first command reviews destinations and template digests. `--apply` registers
the verified units and reloads systemd, including required unit dependencies.
It preserves unknown native units and administrator overrides rather than
overwriting them. Publication does not enable or start services. RPM packages
already install their own units through the native package manager.

`preset` uses the installed service catalog and the explicitly selected systemd
manager. Review its actions before applying them:

```bash
sudo anolisa preset plan --preset host --scope system --initialize --json
sudo anolisa preset apply --preset host --scope system --initialize --json
```

`--initialize` lets an unrecorded unit obtain its initial enable state through
systemd preset policy. Administrator overrides and previously recorded choices
are preserved. Enabling a unit and starting it are separate actions; a preset
may request an immediate start. Units, configuration, users, and host requirements
must already be prepared. Installing packages does not apply the `host` preset.
User-manager operations run as the actual user without `sudo`.

For containers and hosts without systemd, run the allowed foreground selection:

```bash
anolisa serve --preset container --dry-run --json
anolisa serve --preset container
```

The shipped `container` preset selects AW in the invoking user's identity. It
requires that user's AW configuration, runtime directory, and other preflight
conditions. It does not start root backends or an agent-memory MCP stdio session.
Services inherit stdout/stderr; `--json` is supported only with `--dry-run`.
Use a container init or runtime `--init` for PID 1 child reaping. The supervisor
waits for readiness, forwards termination signals, and stops the group when a
required process fails.

Both commands discover the service catalog from the active product resources or
the installed native policy package. Use `--config /absolute/services.json` for
an explicitly reviewed catalog. A root backend still needs its own host service
or explicitly configured privileged deployment.

### Migrate a historical raw installation

Migration supports same-scope ANOLISA raw installations with a trustworthy
`installed.toml` file inventory. It creates a fixed plan before changing anything:

```bash
./packages/anolisa-cli/payload/bin/anolisa migrate plan --bundle "$PWD" --profile standard-user --scope user --output "$HOME/anolisa-migration.json" --json
./packages/anolisa-cli/payload/bin/anolisa migrate apply --plan "$HOME/anolisa-migration.json" --json
```

Plan inspection does not install packages, stop services, or rewrite old state;
`--output` only publishes the requested plan file. Resolve every reported blocker
and generate a new plan before applying it. Native adapters must be disabled and
recorded daemon units must be provably inactive. RPM-managed records, missing
ownership inventories, unrecognized command files, and cross-scope transfers
block automatic takeover. A system source can specify `--source-prefix`; user
sources follow the actual invoking user's HOME/XDG roots.

A historical standalone `anolisa` executable has no component ownership receipt.
After reviewing its actual file and SHA-256, add
`--legacy-cli-sha256 <reviewed-sha256>` to `migrate plan` to accept those exact CLI
bytes. The option does not adopt other unknown commands.
An installation containing only this standalone CLI can migrate even when it
has no `installed.toml`; the plan records that absence and does not invent old
component records. If state appears after planning, apply requires a new plan.

Apply binds the saved plan to its caller, bundle digest, package selection, old
state and commands, and destination. It saves the original state and replaced
commands under the reported migration directory, then runs the normal product
installer. Old raw records are retired only after successful activation; old
component files, configuration, and data remain for rollback. Failed installation
keeps the old records and durable progress. Retry `migrate apply` with the same
plan; modified source evidence or changed choices require a new plan. Migration
does not convert native Agent configuration to AW or transfer RPM ownership.

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

`update self` refuses to directly replace a product-managed CLI. Install a
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
