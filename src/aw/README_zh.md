# AW

[English](README.md)

AW 为 Agent 工作负载提供共享的系统能力，同时不把这些能力绑定到某一种
Agent 或某一种界面。本 workspace 承载 Agent Environment、IDE、工作流引擎和
具体 Provider 共同使用的公共 Contracts 与 Headless 运行机制。

当前实现刻意保持最小：

| Crate | 职责 |
| --- | --- |
| `aw-contracts` | 与传输无关的系统身份和版本化 Provider Contracts |
| `aw-provider-host` | Provider 发现、准入、能力投影、有界调用和诊断 |

COSH 是产品架构中的默认交互式 Agent Environment。当前 PoC 用
tokenless 验证了通用 Host，但尚未改变 COSH 的常规启动流程。ACP 当前随
COSH 交付，因为它是 COSH 接入 Agent 的协议之一；只有其他 Agent
Environment 也需要复用同一实现时，才需要通用接入面。

## 依赖方向

```text
Agent Environment / Headless 调用方
               │
               ▼
        aw-contracts
               ▲
               │
    aw-provider-host ──────► Provider 软件包
                                  providers/<id>/
```

`aw-contracts` 不包含传输、进程管理、持久化或具体 Provider 实现。
`aw-provider-host` 可以依赖 Contracts，但两个 crate 都不得依赖 COSH。

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
  --manifest /absolute/path/to/providers/tokenless/provider/provider.toml \
  --executable-root /absolute/path/to/tokenless/target/debug
```

`list` 输出 Runtime Capability Graph，`invoke` 接收版本化
`CapabilityInvocation` JSON。Host 只接收显式的绝对 manifest 根目录，不搜索环境
中的 `PATH`。

## 目标 Workspace 形态

长期 workspace 将 Contracts、Provider 托管、系统状态、客户端 API 和服务传输
分别放在独立 crate 中：

```text
src/aw/crates/
├── aw-contracts/       # 已有
├── aw-provider-host/   # 已有
├── aw-core/            # 权威身份与状态协调
├── aw-client/          # 公共客户端协议和 SDK 接口
└── aw-service/         # 服务传输与监管
```

标为目标的名称只表示架构归宿，不代表这些 crate 已经交付。
