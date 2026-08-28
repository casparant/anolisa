# 查看 Agent Workload

[English](../../../en/user-entrypoint/cosh-ng/agent-workload.md)

Agent Workload 展示 COSH Host 接纳、执行和治理了什么，不依赖某一个 Agent provider。它把
provider 或 ACP Session 与 Host attempt、policy decision、side effect 及尚不完整的 outcome
evidence 关联起来。

Agent Workload 不是托管 Agent 产品。Claude Managed Agents、Codex、Claude Code 或其他
harness 负责自己的 model loop 与 provider Session；COSH 负责这个视图所表达的本地执行边界。

## Identity 模型

| 字段 | Owner 与含义 |
|---|---|
| `task_id` | 已接纳 workload 的稳定 COSH identity |
| `run_id` | 一次 COSH execution attempt；retry 会分配新的 ID |
| `attempt` | 稳定 Task 下从 1 开始的 attempt 顺序 |
| `runtime_binding.agent_session_id` | COSH logical Agent Session |
| `runtime_binding.runtime_instance_id` | 受监督的 Runtime process instance |
| `runtime_binding.external_session` | opaque provider 或 ACP Session reference |
| `revision` | 视图包含的最后一条 immutable Task event |

External Session 是 correlation metadata，不是 Agent Workload identity。切换 provider 或 retry
失败的 attempt 都不会覆盖之前的 Host evidence。

## 启动并查看 workload

按照 [cosh-ng 用户手册](README.md)启动 package Gateway，然后接纳一个 workload：

```bash
gateway_socket="/run/cosh-gateway-$USER/gateway.sock"
printf '%s\n' 'inspect the failed service' | \
  cosh agent workload --socket "$gateway_socket" start \
    --idempotency-key '<stable-start-key>'
cosh agent workload --socket "$gateway_socket" inspect '<tsk_UUID>'
```

源码构建或 unified build 后使用 bare binary：

```bash
cosh-gateway workload --socket "$gateway_socket" inspect '<tsk_UUID>'
```

如需单条机器可读 JSONL event，把 output 参数放在 `inspect` 前：

```bash
cosh agent workload --socket "$gateway_socket" --output jsonl inspect '<tsk_UUID>'
```

客户端以有界分页读取经过授权的 event ledger，校验每个 identity、revision 与 lifecycle
transition，再在本地生成视图。它不会请求 raw prompt、文件内容或模型推理过程。

Lifecycle decision 复用 authenticated、idempotent Task coordinator：

```bash
printf '%s\n' 'answer to the question' | \
  cosh agent workload --socket "$gateway_socket" answer '<tsk_UUID>' \
    --input-request-id '<inp_UUID>' --idempotency-key '<stable-answer-key>'
cosh agent workload --socket "$gateway_socket" cancel '<tsk_UUID>' \
  --run-id '<run_UUID>' --idempotency-key '<stable-cancel-key>'
cosh agent workload --socket "$gateway_socket" retry '<tsk_UUID>' \
  --previous-run-id '<run_UUID>' --idempotency-key '<stable-retry-key>'
```

能够产生 approval 的 profile 还可以使用 `resolve-approval`。当前 package task-only profile
没有需要 approval 的 side effect。

## 查看 provider correlation

Adapter 写入 authenticated `RuntimeBound` event 后，active attempt 会包含最新 binding：

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

通用 contract 可以表达 Claude Managed Agents Session，但这个 prototype 没有交付 Claude
Managed Agents adapter。Direct Codex 与 Claude Code path 使用已安装的 ACP profile；package
task-only Gateway 尚不把这些 profile 接纳为 durable workload。

## 理解 completion evidence

`completion` 对象包含三条互相独立的状态轴：

| 状态轴 | 回答的问题 |
|---|---|
| `execution` | Agent Runtime 执行期间发生了什么？ |
| `verification` | 独立 verifier 是否确认了用户要求的结果？ |
| `workspace_disposition` | Workspace mutation 是保留、提交、回滚，还是通过其他方式处置？ |

当前 event contract 记录 execution fact，但不记录 verifier 或 workspace-disposition fact。
Runtime 成功因此表示为：

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

`execution_completed` 不证明 provider 的高层 task outcome。Governed side effect 结果不确定时，
输出为 `state: suspended` 与 `completion.execution: uncertain`；不要自动 retry 结果未知的
side effect。

## 运行确定性场景

贡献者无需 daemon 或 model provider 即可运行投影：

```bash
cd src/cosh-ng
cargo run -p cosh-gateway --example agent_workload_demo --locked -- success
cargo run -p cosh-gateway --example agent_workload_demo --locked -- retry
cargo run -p cosh-gateway --example agent_workload_demo --locked -- uncertain
cargo run -p cosh-gateway --example agent_workload_demo --locked -- provider-session
```

- `success` 展示 execution 成功，但不虚构 verification。
- `retry` 展示同一个 `task_id` 下两个有序的 `run_id` attempt。
- `uncertain` 展示已规划 side effect 的结果无法得到证明。
- `provider-session` 展示 external Session 关联到 Host attempt，但不成为它的 identity；这是
  contract demo，不是 live provider integration。

投影 invariant 与 ownership boundary 参阅
[Agent Workload 设计](../../../../../src/cosh-ng/docs/design/agent-workload_zh.md)。
