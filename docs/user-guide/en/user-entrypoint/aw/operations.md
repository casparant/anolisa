# Operations

[中文版](../../../zh/user-entrypoint/aw/operations.md)

Use the configuration path from installation. Exit all Agents sharing that configuration before
stopping its AW service.

## Check status and stop

```bash
aw status --config "$HOME/aw/aw.yaml"
aw inspect --config "$HOME/aw/aw.yaml" --json
aw stop --config "$HOME/aw/aw.yaml"
```

`inspect` includes Provider summaries. The AW service can outlive the Agent; use `stop`
to shut it down and `status` to confirm. Manage sec-core and AgentSight separately.

The grouped commands preserve the same arguments and JSON responses:

```bash
aw service status --config "$HOME/aw/aw.yaml"
aw service stop --config "$HOME/aw/aw.yaml"
aw service serve --config "$HOME/aw/aw.yaml" --state-dir "$HOME/aw/state"
aw service --help
aw --version
```

`serve` stays in the foreground until stopped or interrupted. It does not install
systemd units or start an Agent. The original `aw serve/status/stop` commands remain
compatible. `--version` reports the CLI crate version, not an installed bundle's
Preview release identifier or native Agent version.

## List installation records

```bash
aw list --installed --prefix "$AW_PREFIX"
aw list --installed --prefix "$AW_PREFIX" --json
```

Read the selected prefix's AW receipts to see component versions, platforms,
source commits, file counts and dependencies. This does not scan the system,
query RPM/DEB, verify payload files or run installed programs. The states are
`absent` (no prefix), `unmanaged` (no AW receipts) and `installed` (recorded).
Missing records are not evidence that no software exists; malformed receipts
produce an error rather than an empty list. JSON uses
`scope: "installation_records"`, `payload_verified: false` and `runtime_checked: false`.

## Check installation integrity

```bash
aw doctor --prefix "$AW_PREFIX"
aw doctor --prefix "$AW_PREFIX" --json
```

Check mandatory Preview file/dependency declarations, recorded payload hashes,
permissions, safe file paths, same-build
dependencies and conflicting file ownership. Results distinguish `checked` from
`unhealthy` components and identify individual findings. This read-only snapshot
does not download, execute or repair software, verify publisher identity, or
check service/Agent/Provider readiness. Files not recorded by AW are outside its scope.

Exit code is `0` only when an installation exists and all checks finish without
findings. Absent or unmanaged prefixes, incomplete checks and integrity problems
return `1`. JSON uses `scope: "installation_integrity"`; malformed metadata can
fail before a report is produced. Use `aw service status` and actual Agent tool
behavior separately to verify runtime operation.

## Inspect configured Agents and Providers

```bash
aw agent list --config "$HOME/aw/aw.yaml"
aw provider list --config "$HOME/aw/aw.yaml" --json
aw provider show compression --config "$HOME/aw/aw.yaml"
```

Use a Provider name from your configuration for `show`. These commands validate and
read the selected file without starting an Agent, Provider or service. They show
names, protocols, transport kinds and event/step references; private configuration,
environment variables and executable arguments are omitted. `referenced` includes
disabled references, whose enabled flags are shown separately. A configured or
referenced instance is not proof of runtime activation. JSON marks this boundary
with `scope: "configuration"` and `runtime_checked: false`.

## Upgrade and uninstall

Before upgrading, exit Agents, stop the service using the old AW binary and original configuration,
and back up configuration and state. Install the new package into a new prefix and generate
configuration with its paths. Keep the old package and backups for rollback. Preview has no
cross-version data migration, so use a new state directory when upgrading.

Uninstall with the prefix used during installation. Use the command below for a system install;
omit `sudo` for a personal install.

```bash
sudo "$AW_PREFIX/bin/aw" uninstall --prefix "$AW_PREFIX"
```

Uninstall preserves external configuration, audit history and native Agent data. Modified or missing
package files cause an error first. Follow the diagnostic instead of deleting a directory still in use.

## Keep data

AW stores invocation metadata in `state_dir/journal`, without tool content. Components such as
Tokenless have separate data directories and retention settings; see [Tokenless data](tokenless.md#data-and-privacy).

Set an explicit state directory to preserve records across restarts. It must belong to the runtime
user with mode `0700`. Containers also need persistent storage for this directory, the framework
profile and the workspace. Automatic state paths may use temporary storage. See
[containers and persistence](../../../../developer-guide/en/aw/native-agents.md#containers-and-persistent-data)
for deployment details.

## Troubleshooting

| Symptom | Action |
| --- | --- |
| Validation passes but launch fails | Check the reported Agent version, executable paths and Provider configuration |
| Edits do not change behavior | Stop the old service using the original configuration or socket, then launch the edited file |
| Directory or configuration permissions rejected | Use a private directory owned by the runtime user; avoid symlinks when editing configuration |
| Service protocol mismatch | Stop the old service with the old binary, then consistently use the new installation |
| OpenClaw reports result replacement unavailable | Check the supported framework version and middleware API; safety checks and completion observation can still work |
| Socket remains after a forced exit | Confirm the old service has exited, then remove only its stale `aw.sock`; preserve the journal |
| Long-running session reports `native_event_limit` | Restart the AW-managed Agent; a new native session does not reset the instance counter |
| No compression statistics | Check that a supported tool ran and review Tokenless recording settings |

Each Agent instance currently holds up to 1,024 associated native events. A completed tool call
usually uses two entries when both before and after are enabled. Plan instance restarts for
long-running Gateways and Apps.

Verify policies using both AW's result and actual tool behavior. A model declining to call a tool,
or a native permission denial, does not prove that an AW rule ran.
