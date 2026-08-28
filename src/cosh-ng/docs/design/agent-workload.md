# Agent Workload Projection

[中文版](agent-workload_zh.md)

## Decision

Agent Workload is the COSH Host view of work admitted to a governed execution
environment. It is not a hosted Agent service, an Agent harness, or a
provider-owned session. Systems such as Claude Managed Agents, Codex, Claude
Code, and local harnesses remain above this boundary and may supply the Runtime
that performs the work.

The implementation projects the existing durable Gateway Task ledger instead
of adding a second scheduler or store. `TaskId` is the stable COSH identity,
`RunId` identifies one host execution attempt, and `RuntimeBindingRef` correlates
that attempt with a COSH logical session and an opaque external session. The
external session never replaces `TaskId` or becomes another lifecycle owner.

The `workload` CLI maps `start`, `answer`, `cancel`, `retry`, and
`resolve-approval` onto existing authenticated Task commands. `inspect` adds an
incremental evidence projection. No Task storage schema, event schema, or daemon
API version changes in this slice.

## Layer boundary

```text
Agent control planes and harnesses
  Claude Managed Agents | Codex | Claude Code | local harness
  own: model loop, Agent definition, provider session, orchestration
                         |
                         | RuntimeBindingRef.external_session
                         v
COSH Agent Host
  owns: admission, attempt identity, policy, cancellation, execution ledger
                         |
                         | RuntimeInstanceId + governed target
                         v
Linux process / sandbox / workspace
```

COSH can therefore host work initiated by different Agent systems without
pretending to be those systems. A future Claude Managed Agents adapter could
bind its provider session through the existing opaque external reference; this
prototype does not implement or emulate the Claude Managed Agents API.

## Identity and outcome model

```text
Agent Workload (TaskId)
├── immutable host target
├── ordered attempts[]
│   ├── attempt 1 (RunId)
│   │   └── latest RuntimeBindingRef
│   │       ├── AgentSessionId (COSH)
│   │       ├── RuntimeInstanceId (supervised process)
│   │       └── external_session (provider-owned, opaque)
│   └── attempt 2 (RunId, after explicit retry)
└── completion
    ├── execution
    ├── verification
    └── workspace_disposition
```

The three completion axes have different authorities:

| Axis | Authority | Current evidence |
|---|---|---|
| Execution | Task Coordinator and Runtime bridge | Task, Run, and governed-execution events |
| Verification | A verifier independent of Runtime self-report | Not recorded |
| Workspace disposition | A checkpoint or workspace authority | Not recorded |

`TaskSucceeded` and `RunSucceeded` establish only host-observed execution
success. They produce `AgentWorkloadState::ExecutionCompleted`; they do not
claim that a provider's higher-level task outcome was verified or that workspace
mutations were committed.

## Projection path

```text
cosh agent workload inspect <TaskId>
        |
        v
authenticated local Gateway event pages (64 events/page)
        |
        v
TaskAggregate invariant validation
        |
        v
AgentWorkloadProjector (incremental, non-mutating on error)
        |
        v
AgentWorkloadView (human JSON or one JSONL event)
```

The CLI first reads an authorized Task projection revision, then fetches enough
bounded pages to reach that snapshot. Before committing an event to its local
view, the projector clones and advances both the canonical `TaskAggregate` and
the Agent Workload projection. Validation failure leaves the prior view intact.

The CLI rejects non-Task identifiers, mismatched page identities,
non-advancing cursors, empty nonterminal pages, cursor/revision disagreement,
revision gaps, and illegal lifecycle transitions.

## Attempt reduction

- `TaskQueued` appends an attempt with the selected Runtime.
- `RuntimeBound` records the latest fenced Runtime binding, including the
  provider-owned external session reference when the adapter supplies one.
- `RunRetryQueued` links the previous attempt to its replacement and appends a
  new attempt without changing `TaskId`.
- Runtime, input, approval, and governed-execution events update only the
  active attempt.
- `ExecutionUncertain` produces an uncertain attempt and a suspended workload;
  it never becomes success or an automatic retry.
- Terminal Task facts close host execution while preserving every attempt and
  its failure or uncertainty evidence.

## Compatibility boundary

This slice does not:

- implement Agent definitions, model loops, provider Session APIs, scheduling,
  or multi-Agent orchestration from Claude Managed Agents or another harness;
- infer a provider's business-level outcome from Runtime success;
- add a verifier or infer verification from Runtime output;
- call `ws-ckpt`, create a checkpoint, commit changes, or roll them back;
- change Task event schema v1, Gateway API v1, SQLite schema, or scheduler
  settlement;
- turn the direct `cosh agent run` ACP command into a durable workload;
- expose raw prompts, model reasoning, file content, or tool arguments.

Future verification and workspace integration must add explicit, versioned
events written by the authority that owns each outcome. A real provider adapter
must likewise write an authenticated `RuntimeBound` fact; a caller-provided tag
alone is not accepted as session evidence.

## Executable scenarios

The `agent_workload_demo` example constructs valid immutable ledgers:

| Scenario | Expected projection |
|---|---|
| `success` | Host execution succeeded; verification and disposition are not recorded |
| `retry` | One Task identity, failed attempt 1, successful attempt 2 |
| `uncertain` | One planned side effect is uncertain and the workload is suspended |
| `provider-session` | A provider Session is correlated through a Runtime binding without replacing Task identity |

The scenarios are deterministic apart from event-envelope message IDs, which
the projection does not expose. `provider-session` is a contract demonstration,
not evidence of a shipped Claude Managed Agents adapter.
