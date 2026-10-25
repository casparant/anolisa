# 从源码构建 AW

[English](../../en/aw/building.md)

在 Linux x86_64 或 aarch64 上编译并打包 AW。现成包见[安装 AW](../../../docs/user-guide/zh/aw/installation.md)。

## 构建环境

安装 [rustup](https://www.rust-lang.org/tools/install)。Ubuntu / Debian 系统依赖如下。

```bash
sudo apt-get update
sudo apt-get install -y git build-essential pkg-config libssl-dev
```

构建需要联网下载 Rust、Cargo 依赖及可选的 RTK，尚不支持全离线构建。
目标机器的架构和 glibc 须与构建环境兼容；面向旧系统时，在对应系统上构建。

## 构建安装包

使用干净的源码工作树和空输出目录。

```bash
git clone https://github.com/agentic-os-org/ANOLISA.git anolisa
cd anolisa
cargo run --locked --release --manifest-path distribution/tooling/aw-preview/Cargo.toml \
  --bin anolisa-aw-preview -- \
  --version 0.1.0-preview.1 --component core \
  --output "$PWD/target/aw-preview-packages"
cd target/aw-preview-packages
sha256sum -c SHA256SUMS
```

校验应全部为 `OK`。产物为 `aw-core-0.1.0-preview.1-linux-<arch>.tar.gz`
和 `SHA256SUMS`。`--version` 是打包标识，与源码 tag 无关。

| `--component` | 产物 |
| --- | --- |
| `core` | AW 启动与配置工具 |
| `all` | core、两个 Provider 扩展包及 all-in-one |
| `sec-core` / `tokenless` | 对应扩展包，须搭配同一次构建的 core |

需要安全检查和压缩时选 `all`，并换一个空输出目录。包内提供 RTK 和 TOON 编码，
无需客户另行安装。Agent 本身、模型凭据和 AgentSight 需单独准备。

Preview 构建器位于 `distribution/tooling/aw-preview`，具体后端组合和 Provider
配置模板在这里维护。历史 `aw-build` Cargo binary 在源码树中转调该工具，不是
第二个已安装包管理器。Provider 包携带 `templates/<provider>.json`，all-in-one
携带 Tokenless、sec-core 及组合模板；core-only 不含后端模板。

完整 ANOLISA 产品发行使用[distribution 入口](../../../distribution/README_zh.md)。
产品 raw 直接调用共享库，使用 ANOLISA 包记录；独立 Preview 流程保留原 AW 管理权。

## 交付与验收

交付选定的压缩包及其校验条目，按[安装与首次运行](../../../docs/user-guide/zh/aw/installation.md#安装)
完成安装、配置和启动。使用 all-in-one 时，设 `AW_BUNDLE_KIND=aw-all-in-one`。

验收应确认 Agent 能调用工具、`aw status` 能查询状态、`aw stop` 能停止服务。
启用可选能力后，再按 [Tokenless](../../../docs/user-guide/zh/aw/tokenless.md)
或[安全检查](../../../docs/user-guide/zh/aw/security.md)验证效果。

其他 Agentic OS 组件的编译见[仓库构建指南](../../../docs/user-guide/zh/getting-started/building.md)。
