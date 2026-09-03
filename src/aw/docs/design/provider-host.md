# Headless Provider Host

[中文版](provider-host_zh.md)

The Provider Host PoC exercises one versioned system Capability through a
headless, bounded invocation. The first Driver is `exec-json/v1`: one admitted
process handles one invocation.

## Boundary

Core exchanges `CapabilityInvocation` and `ProviderInvocationResult` with the
Host. Existing Provider binaries keep their native JSON protocols. The Host
uses the Capability's `json-map/v1` codec to build native stdin from canonical
input and to map native stdout back into a canonical result. No mapping selects
logic by `provider_id`.

```text
canonical CapabilityInvocation
        |
        | validate identity, scope, digest, deadline, and budget
        v
json-map/v1 request fields ---> native JSON ---> exec-json Provider
                                                    |
canonical transient output <--- json-map/v1 <--- native JSON
content-free ProviderReceipt <--- disposition, meters, evidence facts
```

The input and output contracts are content-addressed JSON resources. Admission
requires strict relative regular-file paths, matching SHA-256 digests, and
valid JSON. Full JSON Schema instance validation is not part of this PoC.

Payload digests use **Agent Workload canonical JSON v1**: recursively sort object keys,
retain array order, and emit compact UTF-8 JSON. The same encoding is written to
`exec-json/v1` stdin.

## Result semantics

- `Produced` means an Advise Provider returned a candidate. It does not mean
  Core delivered or adopted that candidate.
- Only `Produced` carries transient output. The durable receipt records only
  its schema, digest, and encoded byte count.
- Timeout, non-zero exit, oversized output, malformed JSON, and mapping failure
  after acceptance return a content-free `Failed` receipt.
- Manifest, schema, scope, digest, deadline, budget, and state-root path
  validation failures happen before acceptance and return an admission error.
  Failure to materialize an admitted state directory returns a content-free
  `Failed` receipt.
- This one-shot PoC admits Observe, Advise, and Mediate Capabilities from
  ExecutionContext through ToolCall scope. It rejects Enforce, Host, and User
  declarations until effect reconciliation and those invocation scopes exist.

## Package declarations

The manifest declares network, environment, filesystem, and data handling
requirements. The Runtime Capability Graph exposes those declarations and the
current guarantee `declared_not_enforced`. The process Host clears inherited
environment variables and bounds I/O and time, but this PoC does not claim an
OS sandbox enforces network or filesystem declarations.

## Headless commands

These commands are a development and diagnostic surface. The binary is not
packaged or installed as a public CLI, and its syntax is not a compatibility
contract.

All discovery roots and files are explicit absolute paths. A manifest directory
uses `<root>/<provider-id>/provider.toml`; the package directory and manifest
identities must match. Bare executable names are resolved only below an explicit
`--executable-root`; the Host never scans `PATH`.

```console
$ aw-provider-host --output jsonl list \
    --manifest /opt/agent-workload/providers/tokenless/provider.toml \
    --executable-root /opt/agent-workload/bin

$ aw-provider-host --output jsonl doctor \
    --manifest-dir /opt/agent-workload/providers \
    --executable-root /opt/agent-workload/bin

$ aw-provider-host --output jsonl invoke \
    --manifest /opt/agent-workload/providers/tokenless/provider.toml \
    --executable-root /opt/agent-workload/bin \
    --invocation-file /tmp/context-projection-invocation.json
```

`--state-root` is required only when a manifest references
`{provider_state_dir}`; stateless Providers do not receive or create one.

The invoke response contains a transient `outcome` next to a content-free
`receipt`. Generic ledgers may retain the receipt; they must not retain the
outcome by implication.

## Non-goals

This PoC does not create Agent Work, schedule runtimes, persist receipts,
reconcile external effects, attest or pin executable bytes, replace a
Provider's native protocol, discover arbitrary executables, or claim full
permission enforcement. It carries caller-owned idempotency and policy metadata
but does not deduplicate calls or evaluate policy. Those remain separate Core
and sandbox concerns.
