# AW Hook 关联

[English](aw-hook-correlation.md)

## 用途

COSH 仍然拥有自己的 Agent 协议和 Hook 生命周期。AW 需要为同一次执行建立
稳定的系统 Identity，使 Core 能把 Tool Result 路由到 Provider，同时不把 Provider
原生 Call ID 误当成全局 Identity。`execution_scope` 就是这个关联边界。

这是当前源码 PoC 的关联 Contract。其中的值不是身份验证凭证，不能单独用来授权
特权操作。

## 已实现边界

对 COSH 内置 Agent，一次成功的 Tool Call 经过以下路径：

```text
Provider 原生 Tool Call
        |
        | COSH 执行 Tool
        v
COSH PostToolUse
        |-- tool_use_id       原生 ID，保持不变
        |-- tool_response     操作者与模型两个槽位
        |-- execution_scope       系统关联
        v
aw-cosh-hook
        |
        `-- 只把 tool_response.llmContent 提交给 AW Core
```

COSH 保留原生 `tool_use_id`，因为其他 Hook 消费者可能依赖它。
`execution_scope.tool_use_id` 是独立的 AW 强类型 Identity，由 Agent Session、Turn
和原生 Call ID 派生。使用内置 Agent 的规范 COSH Session / Turn UUID 时，同一逻辑
调用再次被观察会得到相同的 `tol_...`；不同原生调用则得到不同值。

## Wire 格式

`PostToolUse` 输入中与本链路有关的部分如下：

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

| 字段 | 当前来源与生命周期 |
| --- | --- |
| `environment_id` | 每个 `CoshCore` 实例分配一次 |
| `execution_context_id` | 为该实例的 Execution Context 分配一次 |
| `actor_id` | 进程内的不透明调用方关联，不代表经过认证的 Principal |
| `agent_session_id` | 规范 COSH Session UUID 加 `ags_` 类型前缀；否则使用在单个 `CoshCore` 实例内稳定的生成 ID |
| `turn_id` | 规范 COSH Turn UUID 加 `trn_` 类型前缀；否则为本次观察生成 ID |
| `tool_use_id` | 根据 Agent Session、Turn 和原生 Tool Call ID 确定性派生的 UUIDv8 |

Core 再根据这些 Scope 字段和源内容 digest 派生源 Artifact Identity。因此，当 COSH
提供规范 Session / Turn UUID 时，同一 Tool Result 重试可以复用相同 Artifact 和
Provider idempotency identity。

## 响应与绕过

当 AW 返回有效、非空的 lossless 候选结果时，Adapter 输出：

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

操作者看到的原始结果不会被提交给 AW。失败的 Tool Result 和空 `llmContent` 会
跳过 Provider 发现并返回 `{}`。Provider 失败时保留原始模型可见结果。

COSH 按配置顺序聚合 PostToolUse Hooks，最后一个有效替代项胜出。因此，AW
Provider Receipt 只能证明候选结果已经产生，不能证明这些字节最终已经发送给模型。
最终采纳证据需要 COSH 在 Hook 聚合和输出脱敏之后提供 callback。

## 手动接入源码构建

当前不会默认安装这个 Hook。开发者可以用绝对路径把源码构建接入受信任的 COSH
配置：

```toml
[hooks]
enabled = true

[[hooks.PostToolUse]]
name = "aw-context-projection"
command = "/absolute/path/to/aw-cosh-hook --manifest /absolute/path/to/providers/tokenless/provider.toml --executable-root /absolute/path/to/src/tokenless/target/debug --target-id local-source-poc --allow-unenforced-provider --receipt-log /absolute/path/to/aw-receipts.jsonl"
timeout = 5000
sequential = true
```

`--allow-unenforced-provider` 是醒目的 PoC 选择。系统会校验 Provider 权限声明，但
当前 Host 尚未通过 OS sandbox 强制这些声明。在当前 PostToolUse 实现中，Hook 执行
失败会保留原始结果；`fail_open` 只影响 PreToolUse 的失败决策，因此这里不配置它。

## 覆盖范围

当前实现覆盖 COSH 内置 Agent 正常成功的 PostToolUse 路径。它不会拦截仅仅运行在
`cosh-shell` 内的任意外部 Agent，也不覆盖 `ShellEvidence`、失败 Tool、IDE Agent
或工作流引擎。这些 Environment 需要提供等价的 Adapter 边界，不能靠伪造 COSH
Session 来模拟。

Core 和 Provider 侧流程见
[Tool Result Context Projection](../../../aw/docs/design/context-projection_zh.md)。
