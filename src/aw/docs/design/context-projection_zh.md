# Tool Result Context Projection

[English](context-projection.md)

## 用途

Agent 可能把很大的 Tool Result 返回给模型，但操作者只需要在屏幕上保留原始
结果，模型则可以使用更小、可恢复的表达。这个 PoC 为该问题建立了一个具体的
AW 系统边界：Agent Environment 提交模型可见的 Tool Result，Core 将其绑定到
稳定的 Tool Call，Provider 则可以提出替代表达。

Tokenless 是 `context.projection.prepare/v1` Capability 的第一个实现。Core 和
Provider Host 中没有 tokenless 特判。另一个 Provider 只要声明相同的 Capability、
authority、scope 以及完全一致的输入输出 Contracts，就可以参与路由。

## 已实现路径

```text
COSH 内置 Agent
  执行一次 Tool Call
        |
        | 原始 Provider 协议的 Tool Result
        | + 稳定 execution_scope
        v
PostToolUse Hook 进程：aw-cosh-hook
        |
        | ToolResultSubmission
        v
AW Core
  拥有 scope、artifact identity、路由、deadline 和 budget
        |
        | 规范 context.projection.prepare/v1 invocation
        v
Provider Host
  准入 manifest、校验精确 Contracts、映射 JSON、约束进程
        |
        | tokenless 原生压缩请求 / 响应
        v
tokenless binary
        |
        | lossless 候选结果 + meters
        v
COSH adapter 输出 updatedToolResponse
        |
        `--> COSH 可将替代结果放入下一轮模型 History
```

Adapter 只提取 `tool_response.llmContent`，不会把 `returnDisplay` 当成模型
Context 提交。如果 Tool Result 已被标记为错误，或者模型可见内容为空，则跳过
Provider 发现并返回 `{}`。

## 各 crate 职责

| 归属 | 已实现职责 | 不应拥有 |
| --- | --- | --- |
| `aw-contracts` | 强类型 ID、`ToolResultSubmission`、`ContextProjectionCandidate`、schema identity 和 digest | I/O、进程执行、COSH 字段、tokenless 协议 |
| `aw-core` | 稳定 Execution Context、源 Artifact 及 digest、精确 Provider 路由、policy revision、deadline、输出 budget、候选结果校验 | COSH Hook JSON、Provider 原生 JSON、用户呈现 |
| `aw-provider-host` | 显式发现、准入、Runtime Capability Graph、`json-map/v1`、有界 `exec-json/v1`、无正文 Receipt | Capability 策略、最终候选采纳、Agent History |
| `aw-cosh-hook` | COSH 输入输出翻译、选择模型可见槽位、仅请求采纳 lossless 候选、用户通知 | Provider 特定算法，或按 Provider ID 路由 |
| `tokenless` | 原生压缩，以及将其协议映射到规范 Capability 的 Provider 软件包 | AW Execution identity 或 COSH Hook 语义 |

这种依赖方向让 Provider 可替换，但不会让 Context Management 变成可有可无的能力。
Capability 及其策略属于 AW；tokenless 是当前的第一方机制实现。

## Core 对象

### Agent Execution Context

`SessionContextSpec` 建立一个 `AgentExecutionContext`，包含：

| Identity | 在当前 PoC 中的含义 |
| --- | --- |
| `target` | Adapter 声明的受管本机或远程目标 |
| `environment_id` | 承载该 Execution 的 Agent Environment |
| `execution_context_id` | 同一 Execution 的多次 Hook 调用之间稳定的 AW 关联 |
| `actor_id` | 由调用方声明的 Actor 关联；它不是授权凭证 |
| `agent_session_id` | 逻辑 Agent Session |
| `work_id`, `attempt_id` | 未来 Managed Work 的可选关联；COSH PoC 中为空 |

没有 Work identity 的 Attempt 会被拒绝。没有 Agent Session、Turn 和 Tool Use identity 的
Tool Result Projection 也会被拒绝。

### 源 Artifact

Core 把原始的模型可见 Tool Result 当成不可变的 Context Artifact。它计算 SHA-256
源 digest，并根据 Execution Context、Turn、Tool Call 和源 digest 确定性地生成
`art_...` identity。Provider 收到该 Artifact，以及 media type、origin、可选 Tool Name
和是否允许重编码为文本。

### Projection 候选结果

具有 `Advise` authority 的 Provider 返回的是建议，而不是直接修改。有效候选结果必须
指向精确的源 Artifact 和 digest，包含建议给模型的内容和 media type，声明变换链，
并把 reversibility 标记为 `lossless`、`retrievable` 或 `unrecoverable`。

当前 COSH adapter 只采纳非空的 `lossless` 候选。Core 能解析 Contract 中的其他取值，
但这个 PoC 不会请求采纳它们。

## 路由与调用

Core 只在下列事实全部匹配时才选择 Provider：

- Capability 为 `context.projection.prepare/v1`；
- authority 为 `Advise`，scope 包含 `ToolCall`；
- health 为 `Ready`；
- 内容寻址的输入输出 Contract identity 和 digest 完全匹配；
- 满足当前 enforcement policy。

没有匹配项会显式返回 unavailable 错误。如果有多个匹配项，但调用方未给出
`--preferred-provider`，则返回 ambiguity 错误。Core 不按注册顺序默默挑选。

对每次受理的 Preparation，Core 提供 policy revision、墙钟时间与输出字节 budget、
deadline、规范输入 digest，以及由 Tool Use identity 与输入 digest 派生的稳定
idempotency key。Provider Host 将规范请求映射到 Provider 原生协议。在 tokenless
软件包中，这个映射声明在 `providers/tokenless/provider/provider.toml`，而不是编译在
Core 或 Host 里。

## 操作者和模型分别收到什么

在当前 COSH 路径中，两个界面有意保持不同：

| 界面 | 当前行为 |
| --- | --- |
| 操作者显示 | COSH 保留 Tool 路径已经输出的 Provider 原生 Tool Result。Adapter 另外增加简短 `systemMessage`，例如 `AW · tokenless · estimated context 359→110 tokens · saved 69%`。 |
| 下一次模型请求 | Adapter 返回 `hookSpecificOutput.updatedToolResponse`；COSH 在向 Conversation History 追加 Tool Result 时使用 PostToolUse 最终胜出的替代项。 |

因此，PoC 优化的是模型 Context，不会声称改写已经渲染到屏幕上的内容。响应携带
`suppressOutput` 是为了兼容 Hook wire；当前 cosh-ng 没有赋予该字段独立的显示行为，
它也不会删除原始 Tool 显示。

## Outcome 与记录

Provider Host 使用强类型 disposition：

| Disposition | Adapter 行为 |
| --- | --- |
| `Produced` 且存在有效、非空的 `lossless` 候选 | 请求 `updatedToolResponse`，并显示节省通知 |
| `Bypassed` 或 `EffectApplied` | 不返回替代项，也不显示通知 |
| `Denied`、`Failed` 或 `Uncertain` | 保留原始结果，并显示简短失败通知 |
| Tool 执行结果本身已是错误 | 不发现、不调用 Provider，直接返回 `{}` |

`Produced` 只表示 Provider 创建了候选结果，不证明 COSH 最终把这些字节交付给了模型。

可选参数 `--receipt-log PATH` 会以 mode `0600` 追加 JSONL 记录，其中包含：

- `replacement_requested`：表示这个 adapter 已经输出了替代请求；
- 无正文 `ProviderReceipt`：包含 identity、scope、disposition、输出 digest 和大小、
  meters 与有界诊断事实。

它不保存原始 Tool 内容或候选内容，也不是最终采纳证据：后续 Hook 仍可能替换该候选，
阻断型 Hook decision 也可能阻止它进入 History。

## 信任与强制

Tokenless manifest 声明不访问网络、不继承环境变量、不访问文件系统、不保留数据。
准入流程会校验这些声明、schema 资源、executable 解析和软件包 identity。One-shot
Host 也会清理继承环境，并限制输入、输出和时间。

当前 Host **不会**创建 OS sandbox 来强制已声明的网络与文件系统策略。因此 Core
默认拒绝 graph guarantee 为 `declared_not_enforced` 的 Provider。源码 PoC 必须显式给出
`--allow-unenforced-provider`；它的含义是“在这个 PoC 中信任该 Provider”，而不是“声明已被强制”。

关联字段同样不是凭证。在这些 ID 能够管治特权工作之前，授权必须来自独立的身份验证边界。

## 源码 PoC

在仓库根目录执行：

```bash
cargo build --manifest-path providers/tokenless/Cargo.toml --bin tokenless
cargo build --manifest-path src/aw/Cargo.toml -p aw-cosh-hook

AW_REPO_ROOT="$(pwd)"
"$AW_REPO_ROOT/src/aw/target/debug/aw-cosh-hook" \
  --manifest "$AW_REPO_ROOT/providers/tokenless/provider/provider.toml" \
  --executable-root "$AW_REPO_ROOT/providers/tokenless/target/debug" \
  --target-id local-source-poc \
  --allow-unenforced-provider \
  --receipt-log /tmp/aw-context-projection-receipts.jsonl \
  < "$AW_REPO_ROOT/src/aw/crates/aw-cosh-hook/fixtures/post-tool-use.json"
```

当前 fixture 会生成形如下面的响应（候选内容已缩写）：

```json
{
  "suppressOutput": true,
  "systemMessage": "AW · tokenless · estimated context 359→110 tokens · saved 69%",
  "hookSpecificOutput": {
    "hookEventName": "PostToolUse",
    "updatedToolResponse": "builds[6]{id,project,status,duration_ms,owner}: ..."
  }
}
```

独立的 `aw-cosh-hook` 命令是源码级 PoC 和诊断面，尚不是已打包的公开 CLI，
其命令语法也不是稳定 Contract。

## 当前限制

- Hook 尚未安装或接入 COSH 默认配置。
- 当前路径只覆盖 COSH **内置 Agent**。在其他 Environment 提供等价边界之前，它不会
  拦截 `cosh-shell` 中运行的任意外部 Agent、IDE Agent 或工作流引擎。
- 它不覆盖 `cosh-shell` 命令 Evidence，也没有未来 `ShellEvidence` 路径。
- COSH 按配置顺序聚合多个 PostToolUse Hook，最后一个有效替代项胜出。AW 尚没有
  callback 证明哪个替代项最终进入模型 History。
- `replacement_requested` 记录的是 adapter 意图，不是最终采纳。
- Provider 声明会被准入校验，但尚未由 OS sandbox 强制。
- Core 状态和 Receipt 尚未由持久化 AW service 提供。

底层调用语义见 [Headless Provider Host](provider-host_zh.md)，Environment 侧的 identity
映射见 [COSH AW 关联](../../../cosh-ng/docs/design/aw-hook-correlation_zh.md)。
