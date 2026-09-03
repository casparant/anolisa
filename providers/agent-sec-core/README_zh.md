# agent-sec-core AW Provider Package

[English](README.md)

agent-sec-core 安全检查的 AW Provider 接入面：一个 Package、一个可执行文件、
跨两种 Authority 的三个 Capability。

| Capability | Authority | 边界 | 后端扫描器 |
| --- | --- | --- | --- |
| `security.content.inspect/v1` | Observe | PostToolUse | PII 与凭据正则检测 |
| `security.code.inspect/v1` | Observe | PostToolUse | shell 与 Python 的危险构造规则 |
| `security.command.inspect/v1` | Mediate | PreToolUse | 同一套规则，返回 Tool Call 门禁裁决 |

组件源码仍在 `src/agent-sec-core/`。本目录只放 Provider Host 会读取的内容：
manifest、按 digest 准入的 schema，以及 canonical 输入 fixture。

## 为什么一个可执行文件服务三个 Capability

`providers.agentic-os.sh/v1` manifest 只有一份顶层 `[executable]`，因此一个
Package 内所有 Capability 共用同一条命令行。每个 Capability 的 `json-map/v1`
codec 各自向 native request 注入自己的 `operation` 常量，入口按该字段分发。
这是 v1 API 的单 Endpoint 形态；显式 `endpoints[]` 是目标 Contract，当前 Host
不接受。

## 安装布局

```text
/usr/bin/agent-sec-cli
/usr/share/agent-workload/providers/agent-sec-core/provider.toml
/usr/share/agent-workload/providers/agent-sec-core/schemas/...
```

`command` 是裸名 `agent-sec-cli`，Host 只在 Operator 显式批准的 executable root
中解析它，绝不搜索 `$PATH`。

## Provider 路径保证了什么

`aw-provider` 入口刻意绕过 agent-sec-core 的 security middleware lifecycle。
那条路径每次调用都会向 JSONL 与 SQLite 写 SecurityEvent 并发送 telemetry，
会使本 manifest 的 `writes = []`、`retention = "none"`、`telemetry = "disabled"`
声明失真。PII 自定义规则因同一原因被关闭：加载它需要读取本 manifest 未声明的
用户配置文件。

这项保证由端到端测试断言，而不是靠信任：测试对私有 `HOME` 取快照，在清空环境
下运行入口，并要求运行后快照逐字节一致。

Finding 不携带命中内容。跨越边界的只有规则身份、其分类和命中次数。规则身份被
归一到 `[a-z0-9._-]` 且上限 64 字节——比通用 bounded name 更窄，正是为了让标签
无法把命中值带出去。

## 本 Package 刻意不暴露什么

| agent-sec-core 执行面 | 缺席原因 |
| --- | --- |
| `prompt_scan` 的 `standard`/`strict`/`multi_turn` 模式 | 需要通过 HTTP 访问本地模型服务；本 manifest 声明 `network = "none"` |
| `code_scan --mode llm` | 同一模型服务依赖 |
| `agent-sec-daemon` 及其背后全部能力 | 长运行 Unix socket 服务需要 `local-service/v1` Driver，Host 尚未实现 |
| `linux-sandbox` | 它 `execvp` 目标命令，是执行包装器而非 JSON 函数，且需要任何 Capability 都不授予的特权 |
| 统一策略裁决 | agent-sec-core 今天不存在这样的能力；allow/deny/ask 目前是散落在各 Agent adapter 里的重复环境变量逻辑 |

`prompt_scan --mode fast` 离线可用、形态上适配 `exec-json/v1`，但每个进程要在
解释器启动之外额外付约 200–400 ms 的正则编译成本，因此在这项成本对照真实预算
实测之前先不纳入。

## 强制缺口

本 manifest 的 `network` 与 `filesystem` 权限是**声明，而非强制**。Host 没有
OS sandbox，因此 Runtime Capability Graph 会报告 `declared_not_enforced`，
且 Core 在 Operator 未显式选择信任前拒绝该 Package。请把这些声明当作作者承诺
（由 conformance 测试核对），而不是隔离保证。

## 本地验证

```bash
cd src/aw
cargo build -p aw-provider-host --bin aw-provider-host

# 仅静态准入，不运行可执行文件
./target/debug/aw-provider-host doctor \
  --manifest "$PWD/../../providers/agent-sec-core/provider.toml" \
  --executable-root /usr/bin

# 两个 Package 出现在同一张 graph 里
./target/debug/aw-provider-host list \
  --manifest-dir "$PWD/../../providers" \
  --executable-root /usr/bin
```

`doctor` 只证明静态准入。它从不运行 binary，因此不能替代端到端调用。
