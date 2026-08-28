# Inspect Managed Runs

[中文版](../../../zh/user-entrypoint/cosh-ng/managed-run.md)

A Managed Run gives one durable Agent task a stable identity across retries and
makes incomplete evidence visible. Use it when a Runtime exit code is not enough
to answer whether the requested goal was verified or what happened to workspace
changes.

## Identity model

| Field | Meaning |
|---|---|
| `task_id` | Stable identity of the admitted user intent and its Managed Run |
| `run_id` | Identity of one execution attempt under that Task |
| `attempt` | One-based order of the Run within the Task |
| `revision` | Latest immutable Task event included in the projection |

Retrying appends another attempt with a new `run_id`; it does not replace the
`task_id` or erase the failed attempt.

## Inspect a live Task

Start the packaged Gateway as described in the [cosh-ng user guide](README.md),
then admit an intent through the Managed Run decision surface:

```bash
gateway_socket="/run/cosh-gateway-$USER/gateway.sock"
printf '%s\n' 'inspect the failed service' | \
  cosh agent managed-run --socket "$gateway_socket" start \
    --idempotency-key '<stable-start-key>'
cosh agent managed-run --socket "$gateway_socket" inspect '<tsk_UUID>'
```

Use the bare binary after a source or unified build:

```bash
cosh-gateway managed-run --socket "$gateway_socket" inspect '<tsk_UUID>'
```

For a single machine-readable event, put the output option before `inspect`:

```bash
cosh agent managed-run --socket "$gateway_socket" --output jsonl inspect '<tsk_UUID>'
```

The client reads the authorized event ledger in bounded pages, validates every
identity, revision, and lifecycle transition, and builds the projection locally.
It does not request raw prompts, file contents, or model reasoning.

Lifecycle decisions reuse the same authenticated, idempotent Task coordinator
contracts as the lower-level `task` namespace:

```bash
printf '%s\n' 'answer to the question' | \
  cosh agent managed-run --socket "$gateway_socket" answer '<tsk_UUID>' \
    --input-request-id '<inp_UUID>' --idempotency-key '<stable-answer-key>'
cosh agent managed-run --socket "$gateway_socket" cancel '<tsk_UUID>' \
  --run-id '<run_UUID>' --idempotency-key '<stable-cancel-key>'
cosh agent managed-run --socket "$gateway_socket" retry '<tsk_UUID>' \
  --previous-run-id '<run_UUID>' --idempotency-key '<stable-retry-key>'
```

`resolve-approval` is also available for profiles that can produce approvals.
The packaged task-only profile currently has no approvable side effect.

## Read the result

The `completion` object has three independent axes:

| Axis | Question answered |
|---|---|
| `execution` | What happened while the Agent Runtime executed? |
| `verification` | Did an independent verifier establish the requested outcome? |
| `workspace_disposition` | Were workspace mutations retained, committed, rolled back, or otherwise settled? |

The current Task event contract records execution facts but does not yet record
verifier or workspace-disposition facts. Those two fields therefore report
`not_recorded`. A Runtime success is presented as:

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

`execution_completed` deliberately does not mean that the user's goal is proven
complete. Later verifier and checkpoint integration must add explicit durable
facts; clients must not infer them from a successful process exit.

An uncertain governed side effect reports `state: suspended` and
`completion.execution: uncertain`. Inspect the attempt's `uncertainty_reason`
before choosing reconciliation or operator action. Do not automatically retry an
unknown side effect.

## Run the deterministic scenarios

Contributors can exercise the projection without a daemon or model provider:

```bash
cd src/cosh-ng
cargo run -p cosh-gateway --example managed_run_demo --locked -- success
cargo run -p cosh-gateway --example managed_run_demo --locked -- retry
cargo run -p cosh-gateway --example managed_run_demo --locked -- uncertain
```

- `success` shows execution success without invented verification.
- `retry` shows one `task_id` with two ordered `run_id` attempts.
- `uncertain` shows a planned side effect whose result cannot be proven.

See the [Managed Run design](../../../../../src/cosh-ng/docs/design/managed-run.md)
for projection invariants and extension boundaries.
