# agent-sec-core AW Provider Package

[中文版](README_zh.md)

AW Provider onboarding surface for agent-sec-core security inspection: one
Package, one executable, three Capabilities across two Authorities.

| Capability | Authority | Boundary | Backing scanner |
| --- | --- | --- | --- |
| `security.content.inspect/v1` | Observe | PostToolUse | PII and credential regex detection |
| `security.code.inspect/v1` | Observe | PostToolUse | Dangerous-construct rules for shell and Python |
| `security.command.inspect/v1` | Mediate | PreToolUse | Same rules, returning a Tool Call gate verdict |

Component source stays in `src/agent-sec-core/`. This directory holds only what
the Provider Host reads: the manifest, the schemas admitted by digest, and
canonical input fixtures.

## Why one executable serves three Capabilities

The `providers.agentic-os.sh/v1` manifest declares a single top-level
`[executable]`, so every Capability in a Package shares one command line. Each
Capability's `json-map/v1` codec therefore injects its own `operation` constant
into the native request, and the entrypoint dispatches on that field. This is
the single-Endpoint shape of the v1 API; an explicit `endpoints[]` model is a
target Contract that this Host does not yet accept.

## Install layout

```text
/usr/bin/agent-sec-cli
/usr/share/agent-workload/providers/agent-sec-core/provider.toml
/usr/share/agent-workload/providers/agent-sec-core/schemas/...
```

`command` is the bare name `agent-sec-cli`, which the Host resolves only inside
an executable root the operator approved explicitly. `$PATH` is never searched.

## What the Provider path guarantees

The `aw-provider` entrypoint deliberately bypasses the agent-sec-core security
middleware lifecycle. That path writes a SecurityEvent to JSONL and SQLite and
emits telemetry on every call, which would make this manifest's `writes = []`,
`retention = "none"` and `telemetry = "disabled"` declarations false. PII custom
rules are disabled for the same reason: loading them reads a user configuration
file this manifest does not declare.

An end-to-end test asserts the guarantee rather than trusting it: it snapshots a
private `HOME`, runs the entrypoint under a cleared environment, and requires
the snapshot to be byte-identical afterwards.

Findings carry no matched content. Only a rule identity, its classification and
a count cross the boundary. Rule identities are normalized to `[a-z0-9._-]` with
a 64-byte cap, which is narrower than a general bounded name specifically so a
label cannot smuggle the value it matched.

## What this Package deliberately does not expose

| agent-sec-core surface | Why it is absent |
| --- | --- |
| `prompt_scan` in `standard`, `strict`, `multi_turn` mode | Requires a local model service over HTTP; this manifest declares `network = "none"` |
| `code_scan --mode llm` | Same model-service dependency |
| `agent-sec-daemon` and everything behind it | A long-lived Unix-socket service needs a `local-service/v1` Driver, which the Host does not implement |
| `linux-sandbox` | An execution wrapper that `execvp`s the target command, not a JSON function, and it needs privileges no Capability grants |
| Unified policy decision | No such capability exists in agent-sec-core today; allow/deny/ask currently lives as duplicated environment-variable logic inside each Agent adapter |

`prompt_scan --mode fast` is offline and would fit `exec-json/v1`, but it costs
roughly 200-400 ms of regex compilation per process on top of interpreter
start-up, so it is left out until that cost is measured against a real budget.

## Enforcement gap

`network` and `filesystem` permissions in this manifest are **declared, not
enforced**. The Host has no OS sandbox, so the Runtime Capability Graph reports
`declared_not_enforced` and Core refuses the Package unless an operator opts in
explicitly. Treat the declarations as an author's promise that conformance
tests check, not as an isolation guarantee.

## Verify locally

```bash
cd src/aw
cargo build -p aw-provider-host --bin aw-provider-host

# static admission only; does not run the executable
./target/debug/aw-provider-host doctor \
  --manifest "$PWD/../../providers/agent-sec-core/provider.toml" \
  --executable-root /usr/bin

# both Packages in one graph
./target/debug/aw-provider-host list \
  --manifest-dir "$PWD/../../providers" \
  --executable-root /usr/bin
```

`doctor` proves static admission only. It never runs the binary, so it cannot
replace an end-to-end invocation.
