# Tool-result Context Projection

[中文版](context-projection_zh.md)

## Purpose

An Agent can return a large tool result to its model even when the operator
only needs the original result on screen and the model can work from a smaller,
recoverable representation. This PoC gives AW a concrete system boundary
for that problem: the Agent Environment offers the model-visible tool result,
Core binds it to a stable Tool Call, and a Provider may propose a replacement.

Tokenless is the first implementation of the
`context.projection.prepare/v1` Capability. Core and Provider Host contain no
tokenless-specific branch. Another Provider can qualify by declaring the same
Capability, authority, scope, and exact input and output Contracts.

## Implemented path

```text
COSH built-in Agent
  executes one tool
        |
        | original provider-native result
        | + stable execution_scope
        v
PostToolUse Hook process: aw-cosh-hook
        |
        | ToolResultSubmission
        v
AW Core
  owns scope, artifact identity, routing, deadline, and budget
        |
        | canonical context.projection.prepare/v1 invocation
        v
Provider Host
  admits manifest, validates exact Contracts, maps JSON, bounds process
        |
        | native tokenless compression request / response
        v
tokenless binary
        |
        | lossless candidate + meters
        v
COSH adapter emits updatedToolResponse
        |
        `--> COSH may place the replacement in the next model history
```

The adapter extracts only `tool_response.llmContent`. It does not submit
`returnDisplay` as model context. A tool result marked as an error, or an empty
model-visible result, bypasses Provider discovery and returns `{}`.

## Responsibility by crate

| Owner | Implemented responsibility | Must not own |
| --- | --- | --- |
| `aw-contracts` | Typed IDs, `ToolResultSubmission`, `ContextProjectionCandidate`, schema identities and digests | I/O, process execution, COSH fields, tokenless protocol |
| `aw-core` | Stable execution context, source artifact and digest, exact Provider route, policy revision, deadline, output budget, candidate validation | COSH Hook JSON, Provider-native JSON, user presentation |
| `aw-provider-host` | Explicit discovery, admission, Runtime Capability Graph, `json-map/v1`, bounded `exec-json/v1`, content-free receipt | Capability policy, final candidate adoption, Agent history |
| `aw-cosh-hook` | COSH input/output translation, model-visible slot selection, lossless-only replacement request, user notification | Provider-specific algorithms or routing by Provider ID |
| `tokenless` | Native compression and the Provider package mapping its protocol to the canonical Capability | AW execution identity or COSH Hook semantics |

This dependency direction keeps a Provider replaceable without making Context
Management optional. The Capability and its policy belong to AW; tokenless
is the current first-party mechanism.

## Core objects

### Agent execution context

`SessionContextSpec` establishes one `AgentExecutionContext` containing:

| Identity | Meaning in this PoC |
| --- | --- |
| `target` | Governed host or remote target asserted by the adapter |
| `environment_id` | Agent Environment serving the execution |
| `execution_context_id` | Stable AW correlation across Hook calls in one execution |
| `actor_id` | Caller-asserted actor correlation; not an authorization credential |
| `agent_session_id` | Logical Agent session |
| `work_id`, `attempt_id` | Optional future Managed Work correlation; absent in the COSH PoC |

An Attempt is rejected without its Work identity. A tool-result projection is
rejected without an Agent Session, Turn, and Tool Use identity.

### Source artifact

Core treats the original model-visible tool result as an immutable Context
Artifact. It computes the SHA-256 source digest and a deterministic `art_...`
identity from execution context, turn, tool call, and source digest. The
Provider receives that artifact plus media type, origin, optional tool name,
and whether text re-encoding is allowed.

### Projection candidate

A Provider with `Advise` authority returns a proposal, not a mutation. A valid
candidate identifies the exact source artifact and digest, carries the proposed
model-visible content and media type, declares its transformation chain, and
labels reversibility as `lossless`, `retrievable`, or `unrecoverable`.

The current COSH adapter adopts only a non-empty `lossless` candidate. Core can
parse the other Contract variants, but this PoC deliberately does not request
their adoption.

## Routing and invocation

Core selects a Provider only when all of these facts match:

- Capability `context.projection.prepare/v1`;
- `Advise` authority and `ToolCall` scope;
- `Ready` health;
- the exact content-addressed input and output Contract identities and digests;
- the current enforcement policy.

No match is an explicit unavailable error. More than one match is an ambiguity
error unless the caller supplies `--preferred-provider`. Core does not silently
choose by registration order.

For each accepted preparation, Core supplies a policy revision, wall-time and
output-byte budget, deadline, canonical input digest, and a stable idempotency
key derived from the Tool Use identity and input digest. Provider Host maps the
canonical request to the Provider's native protocol. In the tokenless package,
that mapping is declared in `providers/tokenless/provider/provider.toml`; it is
not compiled into Core or Host.

## What the operator and model receive

The two surfaces are intentionally different in the current COSH path:

| Surface | Current behavior |
| --- | --- |
| Operator display | COSH keeps the provider-native tool output already emitted by the tool path. The adapter adds a short `systemMessage`, for example `AW · tokenless · estimated context 359→110 tokens · saved 69%`. |
| Next model request | The adapter returns `hookSpecificOutput.updatedToolResponse`; COSH uses the winning PostToolUse replacement when it appends the tool result to conversation history. |

The PoC therefore optimizes model context without pretending that it rewrites
screen output that has already been rendered. The response carries
`suppressOutput` for Hook wire compatibility; current cosh-ng does not assign
that field an independent display behavior, and it does not erase the original
tool display.

## Outcomes and records

Provider Host uses typed dispositions:

| Disposition | Adapter behavior |
| --- | --- |
| `Produced` with valid non-empty `lossless` candidate | Request `updatedToolResponse` and display the savings notification |
| `Bypassed` or `EffectApplied` | Return no replacement and no notification |
| `Denied`, `Failed`, or `Uncertain` | Keep the original result and display a short failure notification |
| Tool execution result is already an error | Do not discover or invoke a Provider; return `{}` |

`Produced` means only that the Provider created a candidate. It is not proof
that COSH finally delivered those bytes to the model.

The optional `--receipt-log PATH` appends mode-`0600` JSONL records containing:

- `replacement_requested`, which says this adapter emitted a replacement;
- a content-free `ProviderReceipt` containing identities, scope, disposition,
  output digest and size, meters, and bounded diagnostic facts.

It does not store the original tool content or candidate content. It is also
not final adoption evidence: another Hook may replace the candidate later, or
a blocking Hook decision may prevent it from reaching history.

## Trust and enforcement

The tokenless manifest declares no network access, no inherited environment,
no filesystem access, and no retention. Admission verifies these declarations,
schema resources, executable resolution, and package identity. The one-shot
Host also clears inherited environment and bounds input, output, and time.

The current Host does **not** create an OS sandbox that enforces declared
network and filesystem policy. Core therefore rejects a Provider whose graph
guarantee is `declared_not_enforced` by default. The source PoC requires the
explicit `--allow-unenforced-provider` opt-in; it means "trust this Provider for
this PoC," not "the declarations are enforced."

Correlation fields are also not credentials. Authorization must come from a
separate authenticated boundary before these IDs can govern privileged work.

## Source PoC

Run from the repository root:

```bash
cargo build --manifest-path providers/tokenless/Cargo.toml --bin tokenless
cargo build --manifest-path src/aw/Cargo.toml -p aw-cosh-hook

AW_REPO_ROOT="$(pwd)"
"$AW_REPO_ROOT/src/aw/target/debug/aw-cosh-hook" \
  --manifest "$AW_REPO_ROOT/providers/tokenless/provider/provider.toml" \
  --executable-root "$AW_REPO_ROOT/providers/tokenless/target/debug" \
  --target-id local-source-poc \
  --allow-unenforced-provider \
  --receipt-log /tmp/aw-context-projection-receipts.jsonl \
  < "$AW_REPO_ROOT/src/aw/crates/aw-cosh-hook/fixtures/post-tool-use.json"
```

The current fixture produces a response shaped like this (candidate content is
abbreviated):

```json
{
  "suppressOutput": true,
  "systemMessage": "AW · tokenless · estimated context 359→110 tokens · saved 69%",
  "hookSpecificOutput": {
    "hookEventName": "PostToolUse",
    "updatedToolResponse": "builds[6]{id,project,status,duration_ms,owner}: ..."
  }
}
```

The standalone `aw-cosh-hook` command is a source-level PoC and diagnostic
surface. It is not yet a packaged public CLI or a stable command Contract.

## Current limitations

- The Hook is not installed or wired into default COSH configuration.
- The path covers the COSH **built-in Agent** only. It does not intercept an
  arbitrary external Agent running inside `cosh-shell`, an IDE Agent, or a
  workflow engine until that Environment supplies an equivalent boundary.
- It does not cover `cosh-shell` command evidence or a future `ShellEvidence`
  path.
- COSH aggregates multiple PostToolUse Hooks in configuration order and the
  last valid replacement wins. AW has no callback proving which replacement
  finally entered model history.
- `replacement_requested` records adapter intent, not final adoption.
- Provider declarations are admitted but not enforced by an OS sandbox.
- Core state and receipts are not yet provided by a durable AW service.

See [Headless Provider Host](provider-host.md) for lower-level invocation
semantics and [COSH AW Correlation](../../../cosh-ng/docs/design/aw-hook-correlation.md)
for the Environment-side identity mapping.
