# 查看 Managed Run

[English](../../../en/user-entrypoint/cosh-ng/managed-run.md)

Managed Run 为一项 durable Agent task 提供跨 retry 的稳定身份，并显式展示缺失的
evidence。当 Runtime exit code 不足以判断目标是否经过验证、workspace change 最终如何
处置时，可以使用这个视图。

## 身份模型

| 字段 | 含义 |
|---|---|
| `task_id` | 已接纳用户意图及其 Managed Run 的稳定身份 |
| `run_id` | 该 Task 下单次 execution attempt 的身份 |
| `attempt` | Run 在 Task 内从 1 开始的顺序 |
| `revision` | 投影所包含的最后一条 immutable Task event |

Retry 会用新的 `run_id` 追加一次 attempt，不会替换 `task_id`，也不会抹掉失败的 attempt。

## 查看正在运行的 Task

按照 [cosh-ng 用户手册](README.md)启动 package Gateway，然后通过 Managed Run decision
surface 接纳一项 intent：

```bash
gateway_socket="/run/cosh-gateway-$USER/gateway.sock"
printf '%s\n' 'inspect the failed service' | \
  cosh agent managed-run --socket "$gateway_socket" start \
    --idempotency-key '<stable-start-key>'
cosh agent managed-run --socket "$gateway_socket" inspect '<tsk_UUID>'
```

源码构建或 unified build 后使用 bare binary：

```bash
cosh-gateway managed-run --socket "$gateway_socket" inspect '<tsk_UUID>'
```

如果需要单条机器可读 event，把 output 参数放在 `inspect` 之前：

```bash
cosh agent managed-run --socket "$gateway_socket" --output jsonl inspect '<tsk_UUID>'
```

客户端以有界分页读取经过授权的 event ledger，校验每个 identity、revision 和 lifecycle
transition，再在本地生成投影。它不会请求 raw prompt、文件内容或模型推理过程。

Lifecycle decision 会复用低层 `task` namespace 相同的 authenticated、idempotent Task
coordinator contract：

```bash
printf '%s\n' 'answer to the question' | \
  cosh agent managed-run --socket "$gateway_socket" answer '<tsk_UUID>' \
    --input-request-id '<inp_UUID>' --idempotency-key '<stable-answer-key>'
cosh agent managed-run --socket "$gateway_socket" cancel '<tsk_UUID>' \
  --run-id '<run_UUID>' --idempotency-key '<stable-cancel-key>'
cosh agent managed-run --socket "$gateway_socket" retry '<tsk_UUID>' \
  --previous-run-id '<run_UUID>' --idempotency-key '<stable-retry-key>'
```

能够产生 approval 的 profile 还可以使用 `resolve-approval`。当前 package task-only
profile 没有需要 approval 的 side effect。

## 理解输出

`completion` 对象包含三条互相独立的状态轴：

| 状态轴 | 回答的问题 |
|---|---|
| `execution` | Agent Runtime 执行期间发生了什么？ |
| `verification` | 独立 verifier 是否确认了用户要求的结果？ |
| `workspace_disposition` | Workspace mutation 是保留、提交、回滚，还是通过其他方式完成处置？ |

当前 Task event contract 记录 execution fact，但尚未记录 verifier 或
workspace-disposition fact。因此后两项会显示 `not_recorded`。Runtime 成功会表示为：

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

`execution_completed` 有意不表示用户目标已经被证明完成。后续 verifier 与 checkpoint
集成必须写入显式 durable fact；客户端不能从成功的 process exit 推断这些结果。

如果 governed side effect 的结果不确定，输出会是 `state: suspended` 和
`completion.execution: uncertain`。在选择 reconciliation 或人工处理前，先查看该
attempt 的 `uncertainty_reason`；不要自动重试结果未知的 side effect。

## 运行确定性场景

贡献者无需 daemon 或 model provider 即可运行投影：

```bash
cd src/cosh-ng
cargo run -p cosh-gateway --example managed_run_demo --locked -- success
cargo run -p cosh-gateway --example managed_run_demo --locked -- retry
cargo run -p cosh-gateway --example managed_run_demo --locked -- uncertain
```

- `success` 展示 execution 成功，但不虚构 verification。
- `retry` 展示同一个 `task_id` 下两个有序的 `run_id` attempt。
- `uncertain` 展示已规划 side effect 的结果无法得到证明。

投影 invariant 与扩展边界请参阅 [Managed Run 设计](../../../../../src/cosh-ng/docs/design/managed-run_zh.md)。
