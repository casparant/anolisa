# Tokenless 作为 AW Provider

[English](README.md)

Tokenless Provider 包声明 AW 如何使用 Tokenless 完成 Context
Projection。它把 Tokenless 现有的一次性压缩协议作为 AW 的首个
`context.projection.prepare/v1` Provider 暴露出来。这个包只包含
manifest、schema、fixture 和文档；独立的 Tokenless 实现仍位于
[`src/tokenless/`](../../src/tokenless/)。

## 这个 Capability 做什么

`context.projection.prepare/v1` 接受一份模型可见的 Tool 结果或其他上下文
Artifact，返回一个更小的候选内容，以及候选如何产生的事实。它的
Authority 是 `advise`：可以提出候选，但不能决定把候选发送给模型。

PoC 把系统 Contract 与实现自己的协议分开：

| 边界 | 输入 | 输出 |
| --- | --- | --- |
| AW Capability | Context Artifact、调用边界和约束 | Context Projection 候选 |
| Tokenless 原生协议 | `CompressionRequest` v1 | `CompressionResponse` v1 |

机器可读的 JSON Schema 位于 [`schemas/`](schemas/)。manifest 使用通用的
`json-map/v1` codec，把标准输入映射为 `CompressionRequest`，执行
`tokenless compress`，再把 `CompressionResponse` 映射为标准输出和 Meter。
Host 只解释映射声明，没有 Tokenless 特判，也不会把完整的 Core invocation
交给 binary。

这样既保留了现有 Adapter 使用的稳定原生协议，也允许另一个实现用不同的
原生协议提供同一项 AW Capability。

## 为什么 `applied` 映射为 `produced`

在 Tokenless 原生 Adapter 协议中，`applied` 表示返回的 `output` 是一个
有效的替换候选。但在 AW Core 边界上，Provider 只是准备好了候选；
Core 尚未选择它，也没有把它放进模型请求。因此，manifest 把原生
`applied` 映射为 Core Receipt 的 `produced`。

后续由 Core 记录候选是否真正交付。这样，Agent Environment 忽略候选、
换用其他结果，或者最终没有发出对应模型请求时，系统不会把“生成过候选”
误计成“已经节省上下文”。

其他原生 disposition 的映射如下：

| Tokenless disposition | Core Provider disposition | 含义 |
| --- | --- | --- |
| `applied` | `produced` | 已产生更小的候选，但尚未证明已交付 |
| `dry_run`、`passthrough`、`no_savings`、`reversibility_unavailable` | `bypassed` | 不应使用候选替换权威输入 |
| `timeout`、`error` | `failed` | Provider 已进入明确的终态失败 |

进程超时、非零退出、非法 JSON 和超大输出属于 Driver 失败，由通用
`exec-json/v1` Driver 处理，不需要 Gateway 编写 Tokenless 特判。

## 仓库与安装目录

仓库把实现源码与 Provider 声明放在两个职责明确的根目录：

```text
src/tokenless/                 # 实现、测试、Adapter 与打包逻辑
providers/tokenless/           # 声明式 Provider 包
├── provider.toml
├── schemas/
├── fixtures/
├── README.md
└── README_zh.md
```

这种分离让 Tokenless 可以继续作为普通组件演进，Provider 包则保持为
一份小型、可独立评审的 AW 调用声明。发行打包会组合这两个根目录：
`make install`、RPM 和 raw package 把 binary 安装到 binary prefix，并把
Provider 声明安装到 AW 的标准 data 目录：

```text
/usr/bin/tokenless
/usr/share/aw/providers/tokenless/provider.toml
/usr/share/aw/providers/tokenless/schemas/...
```

npm 发行形态仍只交付 binary 和 Agent Adapter，不写入 Host 级的 Provider
发现目录。因此，只安装 npm 包不会把 Tokenless 注册到 AW
Provider Host。

上层系统向 Provider Host 提供显式可信根目录。Host 只在指定的 executable
root 下解析 manifest 中的 `tokenless` 命令；manifest 不能继承调用方的
`PATH` 或任意环境变量。Schema resource 必须位于 Provider 包内，并匹配
manifest 固定的 SHA-256 digest。

manifest 在 Provider 调用中关闭 Stats 导出和 SLS，让
`ProviderReceipt` 成为调用事实的唯一标准记录，避免形成第二条计量路径。
它同时关闭 retrieval publication，因此这项 `advise` Capability 不会打开
Stash 数据库，也不会保留输入内容。manifest 相应声明不需要文件写入、
网络、遥测或保留权限。

## 端到端 PoC 证明什么

仓库中的标准 fixture 会作为 `CapabilityInvocation` 提交给 Headless
Provider Host。Host 对 manifest 和 schema digest 做准入，映射请求，调用
真实 Tokenless binary，并返回两个相互分离的对象：

- 只交给当前调用方的 Context Projection 内容；
- 不含内容的 `ProviderReceipt`，记录身份、disposition、digest、字节数、
  时间和 token 估算。

对于这个 fixture，Tokenless 原生结果是 `applied`；Host 返回 Core
`produced`，且 prepared token 估算低于 source token。原始 Tool 内容不会
进入可持久化 Receipt。Provider crash、超时、输出超限或非法 JSON 时，
Host 会在调用已接受后产生有界的失败事实，而不会把原生输出泄漏到事件账本。

首版 manifest 只暴露候选准备能力。`context.projection.commit` 和
`context.retrieve` 需要 Core 拥有交付、Lease 和授权语义，因此不在这个
PoC 中声明。
