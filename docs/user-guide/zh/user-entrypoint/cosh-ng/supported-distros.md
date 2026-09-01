# 支持的平台与Linux发行版

[English](../../../en/user-entrypoint/cosh-ng/supported-distros.md)

cosh-ng 的交互式终端可在 Linux 和 macOS 上运行。Linux 提供完整运行环境；
macOS 支持交互体验，但存在下述限制。

| 平台 | 交互式 Shell | 支持级别 |
|---|---|---|
| Linux | Bash 或 zsh | 完整的 cosh-ng 功能 |
| macOS arm64 | Bash 或 zsh | 功能受限；依赖 Linux 的能力不可用 |

## Linux发行版

Alibaba Cloud Linux 4 提供推荐的 RPM 安装路径。其他 Linux 发行版可以从
源码构建 cosh-ng，但当前发布的 raw 包还不是覆盖所有发行版的可移植契约。
支持的安装路径见[快速开始](QUICKSTART.md)。

## 安装前

在目标主机上运行 `anolisa env`，查看检测到的平台和可用 backend。安装后，
先运行 `command -v cosh` 验证公开入口，再进入工作空间启动 `cosh`。
由 ANOLISA 管理安装时，`anolisa status cosh-ng` 会报告组件版本。
