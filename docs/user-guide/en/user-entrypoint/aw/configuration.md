# Configuration and Agent launch

[中文版](../../../zh/user-entrypoint/aw/configuration.md)

`aw.yaml` contains Agent entrypoints, enabled Providers and runtime data locations.
Model accounts remain in the Agent's own configuration. Complete [installation](installation.md) first.

## Generate configuration

Reuse the installation variables. `config init` does not overwrite files; see the next section to edit one.

```bash
aw config init --prefix "$AW_PREFIX" \
  --config "$AW_HOME/aw.yaml" --state-dir "$AW_HOME/state" \
  --qoder "$AW_QODER"
```

| Capability | Add to the generation command |
| --- | --- |
| Launch the Agent only | No Provider arguments |
| Compress tool output | `--provider tokenless` |
| Check tool safety | `--provider sec-core --socket /run/agent-sec-core/daemon.sock` |
| Safety checks and compression | `--provider sec-core+tokenless --socket /run/agent-sec-core/daemon.sock` |

Install the selected Providers first; the generator supplies executable paths and integrity metadata.

## Edit existing configuration

Exit Agents using the file and stop the service with the original configuration before editing.
Configuration is not hot-reloaded.

```bash
aw stop --config "$AW_HOME/aw.yaml"
cp "$AW_HOME/aw.yaml" "$AW_HOME/aw.yaml.bak"
aw config show --config "$AW_HOME/aw.yaml"
```

Edit YAML directly or use `aw config` to add and remove Providers and Hooks.
The file and its parent must belong to the current user and must not be writable by others.
The editor requires an absolute path and rejects symlinks. `show` includes private configuration
values; remove sensitive data before sharing its output.

```bash
aw config validate --config "$AW_HOME/aw.yaml"
aw run --config "$AW_HOME/aw.yaml" --agent qoder
```

Successful validation prints `configuration valid`. It checks fields and references;
program availability is checked at launch. Write actual absolute paths in YAML;
AW does not expand variables such as `$HOME`.
For all editor commands and a custom rule example, see the
[configuration tool reference](../../../../developer-guide/en/aw/packages.md#edit-existing-provider-and-hook-configuration).

## Edit Provider declarations

Use these shortcuts to add or remove one declaration in an existing configuration:

```bash
aw provider add custom --config "$AW_HOME/aw.yaml" --definition ./provider.json
aw provider remove custom --config "$AW_HOME/aw.yaml"
```

`provider.json` must contain the complete Provider definition as a YAML/JSON
object, not an entire AW configuration or an executable download. Adding the
same definition again leaves the file unchanged; a different definition under
the same name is rejected. Remove refuses Providers still referenced by any
Hook, including disabled Hooks; remove those references first.

| Shortcut | Existing command, still supported |
| --- | --- |
| `aw provider add NAME --config ABS_FILE --definition FILE` | `aw config add-provider --config ABS_FILE --name NAME --definition FILE` |
| `aw provider remove NAME --config ABS_FILE` | `aw config remove-provider --config ABS_FILE --name NAME` |

Use `--name NAME` instead of the positional name when it starts with `--`; do
not combine the two forms. These commands share the existing configuration
editor and ownership checks. They do not install/uninstall software, create
Hook references, enable a Provider or reload a service. Changed files are
rewritten as YAML without preserving comments or formatting; keep the backup
from the editing workflow above. Validate and restart separately to apply edits.

## Compare policy edits

Prepare a separate candidate file and compare it with the existing configuration:

```bash
aw validate --config next.yaml --against current.yaml
```

Both files are validated. The comparison permits changes only to Providers,
events and the default event budget. Agent entrypoints, native scheduling,
service paths, audit, observation and metadata must remain identical.
Success prints `policy update scope valid; runtime activation is not checked`.
It does not run Providers, modify files, check native readiness or update a
running Agent. Apply edits using the existing stop/edit/run workflow above.
Plain `aw validate --config FILE` still prints `configuration valid`.

`aw config validate` is equivalent to `aw validate`, including `--against` and
relative file paths; it does not enforce the editor's ownership checks. The old
`aw-package config validate` retains its stricter editor checks and original output.
Other legacy `aw-package config` commands and `aw-package configure` remain compatible.

## Register native Hooks

For Agents with persistent native registration, use
`aw agent connect --config FILE --agent TARGET` and any required native options,
such as Hermes `--native-profile`. It uses the existing Adapter registration and
support checks; it does not install the Agent or software packages.
The old `aw install --config FILE --agent TARGET` syntax remains compatible.
Combining native-registration options with `--prefix` or `--bundle` is rejected.

## Choose an Agent

A configuration can contain multiple targets. `--agent` selects a name under `spec.agents`.
The generator currently supports Qoder and OpenClaw. For QwenPaw, Hermes and SDK applications,
use the [native integration reference](../../../../developer-guide/en/aw/native-agents.md).

| Agent | Current requirements | Entrypoint |
| --- | --- | --- |
| Qoder | CLI 1.1.64 or later with native Hooks available | Interactive or print |
| OpenClaw | 2026.9.6 with initialized native configuration and state | A newly launched Gateway |
| QwenPaw | 2.2.2b4 with AgentScope 2.0.9 | App/API |
| Hermes | Follow the [compatible version and setup](../../../../developer-guide/en/aw/native-agents.md#start-hermes) | Local chat |
| AgentScope, Pi, Claude SDK | Explicit application wiring required | [SDK integration reference](../../../../developer-guide/en/aw/native-agents.md) |

### Qoder

Launch from the intended working directory. Arguments after `--` go to Qoder.

```bash
aw run --config "$AW_HOME/aw.yaml" --agent qoder
aw run --config "$AW_HOME/aw.yaml" --agent qoder -- -p 'Read the current directory with a tool.'
```

For an additional settings file, use AW's `--native-settings /absolute/qoder.settings.json`.
AW preserves existing settings and adds callbacks for this run.

### OpenClaw

Stop any Gateway using the same native profile first. Replace these variables with absolute
paths to your existing installation and profile.

```bash
export AW_NODE=/absolute/path/to/node
export AW_OPENCLAW=/absolute/path/to/openclaw.mjs
export AW_OPENCLAW_CONFIG=/absolute/path/to/openclaw.json
export AW_OPENCLAW_STATE=/absolute/path/to/openclaw-state
aw config init --prefix "$AW_PREFIX" \
  --config "$AW_HOME/openclaw.yaml" --state-dir "$AW_HOME/openclaw-aw-state" \
  --node "$AW_NODE" --openclaw "$AW_OPENCLAW"
aw run --config "$AW_HOME/openclaw.yaml" --agent openclaw \
  --native-settings "$AW_OPENCLAW_CONFIG" --native-state-dir "$AW_OPENCLAW_STATE"
```

Wait for `AW openclaw native hooks ready`, then use a client connected to that Gateway.
Add the Provider arguments above when generating configuration for compression or safety checks.
`agent --local` bypasses this Gateway.

## Add your own rules

A simple rule reads a tool event from stdin and writes JSON `true` or `false` to stdout.
Configuration chooses observation or blocking when the rule matches. `on_error` controls execution
failures; safety checks typically use `block`. Follow the
[boolean rule tutorial](../../../../developer-guide/en/aw/integration.md) to prepare a script and verify
both allowed and blocked calls.
