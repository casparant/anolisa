# AW Hook Correlation

[中文版](aw-hook-correlation_zh.md)

## Purpose

COSH remains the owner of its Agent protocol and Hook lifecycle. AW needs
stable system identities around the same execution so Core can route a Tool
Result to a Provider without treating a Provider-native call ID as a global
identity. The `execution_scope` object is that correlation boundary.

This is a correlation Contract for the current source PoC. Its values are not
authentication credentials and must not authorize privileged work.

## Implemented boundary

For the built-in COSH Agent, one successful Tool Call follows this path:

```text
provider-native Tool Call
        |
        | COSH executes the tool
        v
COSH PostToolUse
        |-- tool_use_id       native ID, unchanged
        |-- tool_response     display and model slots
        |-- execution_scope       system correlation
        v
aw-cosh-hook
        |
        `-- submits only tool_response.llmContent to AW Core
```

COSH keeps the native `tool_use_id` because other Hook consumers may depend on
it. `execution_scope.tool_use_id` is a separate typed AW identity derived from
the Agent Session, Turn, and native call ID. With the canonical COSH Session
and Turn UUIDs used by the built-in Agent, re-observing the same logical call
produces the same `tol_...` value; a different native call produces a different
value.

## Wire shape

The relevant part of a `PostToolUse` input is:

```json
{
  "hook_event_name": "PostToolUse",
  "tool_use_id": "provider-call-42",
  "tool_name": "shell",
  "tool_response": {
    "llmContent": "model-visible result",
    "returnDisplay": "operator-visible result"
  },
  "tool_response_is_error": false,
  "execution_scope": {
    "environment_id": "env_...",
    "execution_context_id": "ctx_...",
    "actor_id": "act_...",
    "agent_session_id": "ags_...",
    "turn_id": "trn_...",
    "tool_use_id": "tol_..."
  }
}
```

| Field | Current source and lifetime |
| --- | --- |
| `environment_id` | Allocated once for one `CoshCore` instance |
| `execution_context_id` | Allocated once for that instance's execution context |
| `actor_id` | Opaque process-local caller correlation; not an authenticated principal |
| `agent_session_id` | Canonical COSH Session UUID with the `ags_` type prefix; otherwise an ID stable for one `CoshCore` instance |
| `turn_id` | Canonical COSH Turn UUID with the `trn_` type prefix; otherwise a generated ID for that observation |
| `tool_use_id` | Deterministic UUIDv8 derived from Agent Session, Turn, and the native Tool Call ID |

Core derives the source Artifact identity from this scope plus the source
content digest. When COSH supplies canonical Session and Turn UUIDs, retrying
the same Tool Result can consequently reuse the same Artifact and Provider
idempotency identity.

## Responses and bypasses

When AW returns a valid non-empty lossless candidate, the adapter emits:

```json
{
  "suppressOutput": true,
  "systemMessage": "AW · tokenless · estimated context 359→110 tokens · saved 69%",
  "hookSpecificOutput": {
    "hookEventName": "PostToolUse",
    "updatedToolResponse": "smaller model-visible representation"
  }
}
```

The original operator display is not submitted to AW. Failed tool results
and empty `llmContent` bypass Provider discovery and return `{}`. A Provider
failure keeps the original model-visible result.

COSH aggregates PostToolUse Hooks in configuration order, and the last valid
replacement wins. An AW Provider receipt therefore proves that a candidate
was produced, not that those bytes were finally sent to the model. Final
adoption evidence requires a COSH callback after Hook aggregation and output
redaction.

## Manual source wiring

The Hook is not installed by default. A developer can wire a source build into
a trusted COSH config using absolute paths:

```toml
[hooks]
enabled = true

[[hooks.PostToolUse]]
name = "aw-context-projection"
command = "/absolute/path/to/aw-cosh-hook --manifest /absolute/path/to/providers/tokenless/provider.toml --executable-root /absolute/path/to/src/tokenless/target/debug --target-id local-source-poc --allow-unenforced-provider --receipt-log /absolute/path/to/aw-receipts.jsonl"
timeout = 5000
sequential = true
```

`--allow-unenforced-provider` is a conspicuous PoC opt-in. Provider permission
declarations are validated, but the current Host does not enforce them with an
OS sandbox. In the current PostToolUse implementation, a Hook execution failure
preserves the original result; the `fail_open` setting only affects PreToolUse
failure decisions and is therefore omitted here.

## Coverage

The implemented path covers the built-in COSH Agent's normal successful
PostToolUse route. It does not intercept arbitrary external Agents merely
launched inside `cosh-shell`, `ShellEvidence`, failed tools, IDE Agents, or
workflow engines. Those Environments need an equivalent adapter boundary; they
must not be emulated by inventing COSH sessions.

See [Tool-result Context Projection](../../../aw/docs/design/context-projection.md)
for the Core and Provider side of this flow.
