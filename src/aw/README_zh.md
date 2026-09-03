# AW

[English](README.md)

AW 为 Agent 工作负载提供共享的系统能力，同时不把这些能力绑定到某一种
Agent 或某一种界面。本 workspace 承载 Agent Environment、IDE、工作流引擎和
具体 Provider 共同使用的公共 Contracts、Core 策略与 Headless Provider 运行机制。

当前实现包含五个 crate：

| Crate | 职责 |
| --- | --- |
| `aw-contracts` | 与传输无关的系统身份和版本化 Capability / Provider Contracts |
| `aw-provider-host` | Provider 发现、准入、codec 映射、有界调用和诊断 |
| `aw-core` | Execution Context 所有权、精确 Capability 路由、调用策略和候选结果校验 |
| `aw-ledger` | 追加式 Ledger 准入、哈希链、SQLite 存储、有界查询与链验证 |
| `aw-cosh-hook` | COSH `PreToolUse` / `PostToolUse` adapter，以及过渡期的 hook 侧 Ledger 写入方 |

COSH 是产品架构中的默认交互式 Agent Environment。当前 PoC 已把内置 Agent 的
两个 Tool Call 边界都接到 AW Core 和通用 Provider Host：`PreToolUse` 承载
Mediate 闸门，`PostToolUse` 承载 Observe 与 Advise。tokenless 和 agent-sec-core
是前两个真实 Provider。它尚未默认安装这些 Hook，也尚未覆盖 `cosh-shell` 里启动的任意 Agent。

## 依赖方向

```text
COSH / 其他 Agent Environment
               |  稳定的 Execution / Tool Call scope
               v
          aw-core
               |  规范 CapabilityInvocation
               v
    aw-provider-host ------> Provider 软件包
               |                  providers/<id>/
               v
        候选结果 + 无正文 Receipt

三个运行时 crate 都依赖叶子 crate aw-contracts。
```

`aw-contracts` 不包含传输、进程管理、持久化或具体 Provider 实现。
`aw-core` 依赖 Contracts 和 Provider Host，但不包含 COSH 专用 wire format。
COSH adapter 依赖 Core；AW Core 和 Provider Host 不反向依赖 COSH。

## 构建与测试

```bash
cd src/aw
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo doc --workspace --no-deps
```

## 检查一个 Provider

当前 binary 是开发与诊断面，尚不作为公开 CLI 打包或安装，其命令语法
也不是兼容性 Contract：

```bash
cargo run -p aw-provider-host -- \
  doctor \
  --manifest /absolute/path/to/providers/tokenless/provider.toml \
  --executable-root /absolute/path/to/src/tokenless/target/debug
```

`list` 输出 Runtime Capability Graph，`invoke` 接收版本化
`CapabilityInvocation` JSON。Host 只接收显式的绝对 manifest 根目录，不搜索环境
中的 `PATH`。

## 运行源码 PoC

当前端到端源码 PoC 的路径是：

```text
COSH PostToolUse -> aw-cosh-hook -> AW Core -> Provider Host -> tokenless
```

在仓库根目录构建 tokenless 和 adapter，然后把仓库中经过校验的 COSH Hook
fixture 输入 adapter：

```bash
cargo build --manifest-path src/tokenless/Cargo.toml --bin tokenless
cargo build --manifest-path src/aw/Cargo.toml -p aw-cosh-hook

AW_REPO_ROOT="$(pwd)"
"$AW_REPO_ROOT/src/aw/target/debug/aw-cosh-hook" \
  --manifest "$AW_REPO_ROOT/providers/tokenless/provider.toml" \
  --executable-root "$AW_REPO_ROOT/src/tokenless/target/debug" \
  --target-id local-source-poc \
  --allow-unenforced-provider \
  < "$AW_REPO_ROOT/src/aw/crates/aw-cosh-hook/fixtures/post-tool-use.json"
```

对当前 fixture，响应会请求把一个 lossless 表达放入模型的下一轮 Context，
并报告估算 Token 从 359 降到 110。`--allow-unenforced-provider` 必须显式给出：
PoC 会校验 Provider 声明，但还没有用 OS sandbox 强制这些声明。

详见 [Context Projection](docs/design/context-projection_zh.md)，其中说明了数据模型、
用户可见行为、Receipt 语义和当前限制。COSH Hook 的精确 wire 边界见
[COSH AW 关联](../cosh-ng/docs/design/aw-hook-correlation_zh.md)。

## 记录一次边界裁决

两个 adapter 都可以把边界的裁决持久化到追加式、哈希链式的 Ledger 中：

```bash
"$AW_REPO_ROOT/src/aw/target/debug/aw-cosh-hook" \
  --event PreToolUse \
  --manifest-dir "$AW_REPO_ROOT/providers" \
  --executable-root /absolute/path/to/provider/bin \
  --target-id local-source-poc \
  --allow-unenforced-provider \
  --ledger /absolute/path/to/ledger-dir \
  --ledger-mode required \
  < pre-tool-use.json
```

`--ledger-mode` 声明调用方对这次 append 的要求。`correlated`（默认）让边界照常
工作，只是不宣称这条事实；`required` 则让边界失败 —— 在 `PreToolUse` 上，这正是
让 COSH 失败关闭的机制。写入方是 hook 进程本身，所以这是过渡安排，见
[AW Ledger](docs/design/ledger_zh.md#过渡期的-hook-侧写入方)。

用 inspector 读回 Ledger。它链接自带的 bundled SQLite —— 这一点很关键，因为存储
使用 STRICT 表，`sqlite3` 低于 3.37 的主机根本打不开这个 schema：

```bash
cargo build --manifest-path src/aw/Cargo.toml -p aw-ledger --bin aw-ledger

src/aw/target/debug/aw-ledger --ledger /absolute/path/to/ledger-dir verify
src/aw/target/debug/aw-ledger --ledger /absolute/path/to/ledger-dir list
src/aw/target/debug/aw-ledger --ledger /absolute/path/to/ledger-dir body evt_<uuid>
```

`verify` 会重算每一个 body 摘要、record 摘要和 parent 链接，而不是读取已存的值。
`body` 打印的恰好是被存进去的字节，因此内容自由可以被审计，而不是被声称。

记录模型、哈希链与当前限制见 [AW Ledger](docs/design/ledger_zh.md)；三个 Capability
跨两个 Provider 在两个边界上的端到端记录见
[多 Provider 案例](docs/design/multi-provider-case_zh.md)。

## 目标 Workspace 形态

长期 workspace 将 Contracts、Provider 托管、系统状态、客户端 API 和服务传输
分别放在独立 crate 中：

```text
src/aw/crates/
|-- aw-contracts/       # 已有
|-- aw-provider-host/   # 已有
|-- aw-core/            # 已有：Context 与 Provider 策略协调
|-- aw-ledger/          # 已有：追加式边界记录与哈希链
|-- aw-cosh-hook/       # 已有：COSH 专用边界 adapter
|-- aw-client/          # 公共客户端协议和 SDK 接口
`-- aw-service/         # 服务传输与监管
```

标为目标的名称只表示架构归宿，不代表这些 crate 已经交付。
