# Managed Run 投影

[English](managed-run.md)

## 决策

Managed Run 是现有 durable Gateway Task Plane 之上的语义视图，不是第二套 scheduler，
也不是改名后的 process wrapper。`TaskId` 继续表示已接纳用户意图的稳定身份，每个
`RunId` 继续表示一次 execution attempt。首个实现增加 incremental projection 和 CLI
查看入口，不修改 Task storage schema、event schema 或 daemon API version。
面向用户的 decision surface 把 `start`、`answer`、`cancel`、`retry` 与
`resolve-approval` 映射到现有 authenticated coordinator command，不创建第二条 mutation path。

这个选择保留现有 single-writer coordinator、immutable event ledger、idempotency、retry
fencing、Runtime containment 和 restart recovery。如果再增加平行的 Managed Run store，
同一个 lifecycle 将出现两个 owner，recovery 也会变得含糊。

## 语义模型

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

三条 completion 状态轴分别由不同主体负责：

| 状态轴 | 负责主体 | 当前 evidence |
|---|---|---|
| Execution | Task Coordinator 与 Runtime bridge | Task/Run/execution event |
| Verification | 独立于 Runtime self-report 的未来 verifier | 尚未记录 |
| Workspace disposition | 未来 checkpoint/workspace authority | 尚未记录 |

在这个投影中，`TaskSucceeded` 与 `RunSucceeded` 只证明 execution success。它们生成
`ManagedRunState::ExecutionCompleted`，不会声称用户要求的目标已经独立验证，也不会声称
workspace mutation 已经提交。

## 投影路径

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

CLI 会先读取 authorized Task projection revision，再获取足够的分页以至少到达这个 snapshot。
`ManagedRunProjector` 以 incremental 方式处理分页，因此客户端不需要为全部 event history
进行一次无界内存分配。每条 event 写入本地视图前，它会同时 clone 并推进 canonical
`TaskAggregate` 与 Managed Run projection；校验失败时，之前的投影保持不变。

CLI 会拒绝非 Task identifier、分页 Task identity 不匹配、cursor 不前进、空的非终止分页、
cursor/revision 不一致、revision gap 和非法 lifecycle transition。

## Attempt 归约

- `TaskQueued` 使用选定 Runtime 分配 attempt 1。
- `RunRetryQueued` 把前一次 attempt 链接到 replacement，追加新的 attempt，并继承上一次
  Runtime selector，因为 retry command 不会选择另一个 Runtime。
- Runtime、input 和 approval event 只更新 active attempt。
- Governed execution event 分别统计 planned、known-successful、known-failed 与 uncertain
  side effect，不存储 raw argument 或 result。
- `ExecutionUncertain` 会得到 uncertain attempt 和 suspended Managed Run，绝不会转换成
  success 或 automatic retry。
- Terminal Task fact 结束整体 execution lifecycle，但保留每一次 attempt 及其 failure 或
  uncertainty evidence。

## 兼容边界

这个实现只读取现有 durable fact。它不会：

- 增加 verifier，也不会从 Runtime output 推断 verification；
- 调用 `ws-ckpt`、创建 checkpoint、提交或回滚变更；
- 修改 Task event schema v1、Gateway API v1、SQLite schema 或 scheduler settlement；
- 把非托管的 `cosh agent run` ACP command 变成 Managed Run；
- 暴露 raw prompt、模型推理、文件内容或 tool argument。

未来的 verification 与 workspace 集成必须由各 outcome 的 owner 写入显式、带版本的 event。
届时投影才能推进两个 `not_recorded` 状态轴。如果没有 authoritative durable fact 就扩展
视图，会重新引入本设计希望避免的 success overclaim 问题。

## 可执行场景

`managed_run_demo` example 构造合法的 immutable ledger，覆盖三条边界：

| 场景 | 预期投影 |
|---|---|
| `success` | Execution 成功；verification 与 disposition 未记录 |
| `retry` | 一个 Task identity、失败的 attempt 1、成功的 attempt 2 |
| `uncertain` | 一个 planned side effect 结果不确定，Managed Run suspended |

这些场景与 unit test 不依赖 provider，并具有确定性；只有 event envelope message ID 是随机的，
而投影不会暴露它。
