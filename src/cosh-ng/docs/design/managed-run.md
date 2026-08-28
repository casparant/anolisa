# Managed Run Projection

[中文版](managed-run_zh.md)

## Decision

Managed Run is a semantic view over the existing durable Gateway Task Plane,
not a second scheduler or a renamed process wrapper. `TaskId` remains the stable
identity of admitted user intent; each `RunId` remains one execution attempt.
The first implementation adds an incremental projection and CLI inspection path
without changing Task storage schema, event schema, or daemon API version.
The user-facing decision surface maps `start`, `answer`, `cancel`, `retry`, and
`resolve-approval` onto the existing authenticated coordinator commands; it
does not create a second mutation path.

This choice preserves the existing single-writer coordinator, immutable event
ledger, idempotency, retry fencing, Runtime containment, and restart recovery.
Adding a parallel Managed Run store would create two owners for the same
lifecycle and make recovery ambiguous.

## Semantic model

```text
Managed Run (TaskId)
├── immutable target
├── ordered attempts[]
│   ├── attempt 1 (RunId)
│   └── attempt 2 (RunId, after explicit retry)
└── completion
    ├── execution
    ├── verification
    └── workspace_disposition
```

The three completion axes have different authorities:

| Axis | Authority | Current evidence |
|---|---|---|
| Execution | Task Coordinator and Runtime bridge | Task/Run/execution events |
| Verification | A future verifier independent of Runtime self-report | Not recorded |
| Workspace disposition | A future checkpoint/workspace authority | Not recorded |

`TaskSucceeded` and `RunSucceeded` establish only execution success in this
projection. They produce `ManagedRunState::ExecutionCompleted`, not a claim that
the requested goal was independently verified or that workspace mutations were
committed.

## Projection path

```text
cosh agent managed-run inspect <TaskId>
        |
        v
authenticated local Gateway event pages (64 events/page)
        |
        v
TaskAggregate invariant validation
        |
        v
ManagedRunProjector (incremental, non-mutating on error)
        |
        v
ManagedRunView (human JSON or one JSONL event)
```

The CLI first reads an authorized Task projection revision, then fetches enough
pages to reach at least that snapshot. `ManagedRunProjector` processes pages
incrementally so event history does not need one unbounded client allocation.
Before committing each event to its local
view, it clones and advances both the canonical `TaskAggregate` and Managed Run
projection. A validation failure leaves the previous projection unchanged.

The CLI rejects non-Task identifiers, mismatched page identities, non-advancing
cursors, empty nonterminal pages, cursor/revision disagreement, revision gaps,
and illegal lifecycle transitions.

## Attempt reduction

- `TaskQueued` allocates attempt 1 with the selected Runtime.
- `RunRetryQueued` links the previous attempt to its replacement, appends the
  new attempt, and inherits the previous Runtime selector because the retry
  command does not select a different Runtime.
- Runtime, input, and approval events update only the active attempt.
- Governed execution events count planned, known-successful, known-failed, and
  uncertain side effects without storing raw arguments or results.
- `ExecutionUncertain` produces an uncertain attempt and a suspended Managed
  Run. It is never converted to success or automatic retry.
- Terminal Task facts close the overall execution lifecycle but retain every
  attempt and its failure or uncertainty evidence.

## Compatibility boundary

This slice is read-only over existing durable facts. It does not:

- add a verifier or infer verification from Runtime output;
- call `ws-ckpt`, create a checkpoint, commit changes, or roll them back;
- change Task event schema v1, Gateway API v1, SQLite schema, or scheduler
  settlement;
- turn the ungoverned `cosh agent run` ACP command into a Managed Run;
- expose raw prompt, model reasoning, file content, or tool arguments.

Future verification and workspace integration must add explicit, versioned
events written by the authority that owns each outcome. At that point the
projection can advance the two `not_recorded` axes. Extending the view without
durable authoritative facts would recreate the success-overclaim problem this
design prevents.

## Executable scenarios

The `managed_run_demo` example constructs valid immutable ledgers and exercises
three boundaries:

| Scenario | Expected projection |
|---|---|
| `success` | Execution succeeded; verification and disposition are not recorded |
| `retry` | One Task identity, failed attempt 1, successful attempt 2 |
| `uncertain` | One planned side effect is uncertain and the Managed Run is suspended |

These scenarios and unit tests are provider-independent and deterministic apart
from event envelope message IDs, which are not exposed by the projection.
