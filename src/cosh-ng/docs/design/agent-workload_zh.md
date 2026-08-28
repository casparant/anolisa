# Agent Workload 投影

[English](agent-workload.md)

## 决策

Agent Workload 是 COSH Host 对已接纳到受治理执行环境中的工作所提供的视图。它不是托管
Agent 服务、Agent harness 或 provider-owned Session。Claude Managed Agents、Codex、
Claude Code 与本地 harness 都位于这条边界之上，并可以提供实际执行工作的 Runtime。

这个实现投影现有 durable Gateway Task ledger，不增加第二套 scheduler 或 store。`TaskId`
是稳定的 COSH identity，`RunId` 表示一次 Host execution attempt，`RuntimeBindingRef` 把
attempt 与 COSH logical Session 及 opaque external Session 关联起来。External Session 不会
替换 `TaskId`，也不会成为另一套 lifecycle owner。

`workload` CLI 把 `start`、`answer`、`cancel`、`retry` 与 `resolve-approval` 映射到
现有 authenticated Task command；`inspect` 增加 incremental evidence projection。这个
实现不修改 Task storage schema、event schema 或 daemon API version。

## 分层边界

```text
Agent control plane 与 harness
  Claude Managed Agents | Codex | Claude Code | local harness
  负责：model loop、Agent definition、provider Session、orchestration
                         |
                         | RuntimeBindingRef.external_session
                         v
COSH Agent Host
  负责：admission、attempt identity、policy、cancel、execution ledger
                         |
                         | RuntimeInstanceId + governed target
                         v
Linux process / sandbox / workspace
```

因此，COSH 可以承载不同 Agent 系统发起的工作，而不假装自己就是这些系统。未来的 Claude
Managed Agents adapter 可以通过现有 opaque external reference 绑定 provider Session；这个
prototype 不实现、也不仿造 Claude Managed Agents API。

## Identity 与 outcome 模型

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

三条 completion 状态轴分别由不同主体负责：

| 状态轴 | 负责主体 | 当前 evidence |
|---|---|---|
| Execution | Task Coordinator 与 Runtime bridge | Task、Run 与 governed-execution event |
| Verification | 独立于 Runtime self-report 的 verifier | 尚未记录 |
| Workspace disposition | checkpoint 或 workspace authority | 尚未记录 |

`TaskSucceeded` 与 `RunSucceeded` 只证明 Host 观察到 execution success。它们生成
`AgentWorkloadState::ExecutionCompleted`，不会声称 provider 的高层 task outcome 已经验证，
也不会声称 workspace mutation 已提交。

## 投影路径

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

CLI 会先读取 authorized Task projection revision，再获取足够的有界分页以到达这个 snapshot。
每条 event 写入本地视图前，projector 会同时 clone 并推进 canonical `TaskAggregate` 与
Agent Workload projection；校验失败时，之前的视图保持不变。

CLI 会拒绝非 Task identifier、分页 Task identity 不匹配、cursor 不前进、空的非终止分页、
cursor/revision 不一致、revision gap 和非法 lifecycle transition。

## Attempt 归约

- `TaskQueued` 使用选定 Runtime 追加一次 attempt。
- `RuntimeBound` 记录最新 fenced Runtime binding；adapter 提供 provider-owned external
  Session reference 时也会一并记录。
- `RunRetryQueued` 把前一次 attempt 链接到 replacement，追加新的 attempt，但不改变
  `TaskId`。
- Runtime、input、approval 与 governed-execution event 只更新 active attempt。
- `ExecutionUncertain` 会产生 uncertain attempt 与 suspended workload；绝不会转成 success
  或 automatic retry。
- Terminal Task fact 结束 Host execution，同时保留每次 attempt 及其 failure 或 uncertainty
  evidence。

## 兼容边界

这个实现不会：

- 实现 Claude Managed Agents 或其他 harness 的 Agent definition、model loop、provider
  Session API、scheduling 或 multi-Agent orchestration；
- 从 Runtime success 推断 provider 的业务级 outcome；
- 增加 verifier，或从 Runtime output 推断 verification；
- 调用 `ws-ckpt`、创建 checkpoint、提交或回滚变更；
- 修改 Task event schema v1、Gateway API v1、SQLite schema 或 scheduler settlement；
- 把直接执行的 `cosh agent run` ACP command 变成 durable workload；
- 暴露 raw prompt、模型推理、文件内容或 tool argument。

未来的 verification 与 workspace 集成必须由各 outcome 的 owner 写入显式、带版本的 event。
真实 provider adapter 同样必须写入 authenticated `RuntimeBound` fact；仅由 caller 提供 tag
不能成为 Session evidence。

## 可执行场景

`agent_workload_demo` example 构造合法的 immutable ledger：

| 场景 | 预期投影 |
|---|---|
| `success` | Host execution 成功；verification 与 disposition 未记录 |
| `retry` | 一个 Task identity、失败的 attempt 1、成功的 attempt 2 |
| `uncertain` | 一个 planned side effect 结果不确定，workload suspended |
| `provider-session` | provider Session 通过 Runtime binding 关联，但不替换 Task identity |

这些场景除 event-envelope message ID 外均具有确定性，而投影不会暴露该 ID。
`provider-session` 是 contract demonstration，不代表已经交付 Claude Managed Agents adapter。
