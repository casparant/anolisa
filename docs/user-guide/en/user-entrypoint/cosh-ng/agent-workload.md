# Inspect Agent Workloads

[中文版](../../../zh/user-entrypoint/cosh-ng/agent-workload.md)

Agent Workload shows what the COSH Host admitted, executed, and governed without
depending on one Agent provider. Use it to correlate a provider or ACP Session
with the host attempt, policy decisions, side effects, and incomplete outcome
evidence.

Agent Workload is not a hosted Agent product. Claude Managed Agents, Codex,
Claude Code, or another harness owns its model loop and provider Session; COSH
owns the local execution envelope represented by this view.

## Identity model

| Field | Owner and meaning |
|---|---|
| `task_id` | Stable COSH identity for the admitted workload |
| `run_id` | One COSH execution attempt; retry allocates another one |
| `attempt` | One-based attempt order under the stable Task |
| `runtime_binding.agent_session_id` | COSH logical Agent Session |
| `runtime_binding.runtime_instance_id` | Supervised Runtime process instance |
| `runtime_binding.external_session` | Opaque provider or ACP Session reference |
| `revision` | Latest immutable Task event included in the view |

An external Session is correlation metadata, not the Agent Workload identity.
Changing providers or retrying a failed attempt does not overwrite prior host
evidence.

## Start and inspect a workload

Start the packaged Gateway as described in the [cosh-ng user guide](README.md),
then admit a workload:

```bash
gateway_socket="/run/cosh-gateway-$USER/gateway.sock"
printf '%s\n' 'inspect the failed service' | \
  cosh agent workload --socket "$gateway_socket" start \
    --idempotency-key '<stable-start-key>'
cosh agent workload --socket "$gateway_socket" inspect '<tsk_UUID>'
```

Use the bare binary after a source or unified build:

```bash
cosh-gateway workload --socket "$gateway_socket" inspect '<tsk_UUID>'
```

For one machine-readable JSONL event, put the output option before `inspect`:

```bash
cosh agent workload --socket "$gateway_socket" --output jsonl inspect '<tsk_UUID>'
```

The client reads the authorized event ledger in bounded pages, validates every
identity, revision, and lifecycle transition, and builds the view locally. It
does not request raw prompts, file contents, or model reasoning.

Lifecycle decisions reuse the authenticated and idempotent Task coordinator:

```bash
printf '%s\n' 'answer to the question' | \
  cosh agent workload --socket "$gateway_socket" answer '<tsk_UUID>' \
    --input-request-id '<inp_UUID>' --idempotency-key '<stable-answer-key>'
cosh agent workload --socket "$gateway_socket" cancel '<tsk_UUID>' \
  --run-id '<run_UUID>' --idempotency-key '<stable-cancel-key>'
cosh agent workload --socket "$gateway_socket" retry '<tsk_UUID>' \
  --previous-run-id '<run_UUID>' --idempotency-key '<stable-retry-key>'
```

`resolve-approval` is also available for profiles that can produce approvals.
The packaged task-only profile currently has no approvable side effect.

## Read provider correlation

When an adapter emits an authenticated `RuntimeBound` event, the active attempt
contains its latest binding:

```json
{
  "task_id": "tsk_...",
  "attempts": [{
    "run_id": "run_...",
    "runtime_binding": {
      "agent_session_id": "ags_...",
      "runtime_instance_id": "rti_...",
      "external_session": {
        "kind": "provider_session",
        "authority": "anthropic-managed-agents",
        "value": "provider-session-42"
      }
    }
  }]
}
```

The generic contract can represent a Claude Managed Agents Session, but this
prototype does not ship a Claude Managed Agents adapter. The direct Codex and
Claude Code path uses installed ACP profiles; the packaged task-only Gateway
does not yet admit those profiles as durable workloads.

## Read completion evidence

The `completion` object has three independent axes:

| Axis | Question answered |
|---|---|
| `execution` | What happened while the Agent Runtime executed? |
| `verification` | Did an independent verifier establish the requested outcome? |
| `workspace_disposition` | Were workspace mutations retained, committed, rolled back, or otherwise settled? |

The current event contract records execution facts but not verifier or
workspace-disposition facts. A Runtime success is therefore presented as:

```json
{
  "state": "execution_completed",
  "completion": {
    "execution": "succeeded",
    "verification": "not_recorded",
    "workspace_disposition": "not_recorded"
  }
}
```

`execution_completed` does not prove the provider's higher-level task outcome.
An uncertain governed side effect reports `state: suspended` and
`completion.execution: uncertain`; do not automatically retry an unknown side
effect.

## Run deterministic scenarios

Contributors can exercise the projection without a daemon or model provider:

```bash
cd src/cosh-ng
cargo run -p cosh-gateway --example agent_workload_demo --locked -- success
cargo run -p cosh-gateway --example agent_workload_demo --locked -- retry
cargo run -p cosh-gateway --example agent_workload_demo --locked -- uncertain
cargo run -p cosh-gateway --example agent_workload_demo --locked -- provider-session
```

- `success` shows execution success without invented verification.
- `retry` shows one `task_id` with two ordered `run_id` attempts.
- `uncertain` shows a planned side effect whose result cannot be proven.
- `provider-session` shows an external Session attached to the host attempt
  without becoming its identity; it is a contract demo, not a live provider
  integration.

See the [Agent Workload design](../../../../../src/cosh-ng/docs/design/agent-workload.md)
for projection invariants and ownership boundaries.
