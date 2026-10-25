# anolisa

[English](README.md)

ANOLISA 产品安装和组件管理的统一 CLI 网关。CLI 直接调用共享 local-package 库
安装经过校验的完整
Linux 发行，同时保留已有 raw/RPM 组件和 adapter 流程。
它是 [ANOLISA](../../README_zh.md) 项目的主要用户入口。

## 快速开始

```bash
curl -fsSL https://get.agentic-os.sh | bash
export PATH="$HOME/.local/bin:$PATH"
anolisa --version
```

GNU/Linux x86_64 和 aarch64 默认下载完整产品归档并选择 user scope，下载引导需要
Python 3.6 或更新版本。macOS 安装独立 CLI；Linux 也可用 `--cli-only` 选择独立 CLI。
已下载完整归档时，执行其中的 `./install.sh --scope user`，或以 root 执行
`./install.sh --scope system`。普通安装后调用 `anolisa setup`：管道执行显示可用状态，
终端执行可引导明确的用户 AW 配置。Linux 的显式 `--rpm` 流程则通过原生 RPM backend
在 system 模式安装全部已发布组件。安装不会启用 Provider 或服务。已有独立命令
入口会被保留，需要明确处理冲突后才能激活产品。

新归档在两个 scope 都默认安装 `complete`，包含本平台全部交付包；
精简组合通过 `--profile` 显式选择。
使用 `--profile` 可以选择归档声明的其他包组合。

系统 tarball 安装将公共命令放入 `/usr/local/bin`，私有程序放入
`/usr/local/lib/anolisa`，静态资源放入 `/usr/local/share/anolisa`。用户安装
将私有程序与 XDG 资源分开，ANOLISA 同时登记两处位置。禁用的 Provider 模板固定
同版程序路径；运行数据与 socket 必须按实际运行用户解析，与软件安装范围分别决定。

`--release-version`、`--release-url`、目录、兼容性和离线安装说明见
[安装指南](../../docs/user-guide/zh/getting-started/installation.md)。
`release inspect`、`release install` 和 `release status` 见
[CLI 参考](../../docs/user-guide/zh/getting-started/anolisa-cli.md#产品发行命令)。

## 命令概览

产品包记录使用 `anolisa release list --scope user|system` 查询，
`release doctor --bundle ABS_DIRECTORY --scope user|system` 检查指定发行的文件；
系统产品记录与检查需要 root。
已切换到另一版本后，可用 `release uninstall --bundle ABS_DIRECTORY --yes`
清理不再活动的版本；`--dry-run` 预览不需要确认。
新 ANOLISA 包数据库的未完成事务使用
`anolisa release recover --scope user|system` 显式恢复，不需要 bundle。
已有 AW 所有的产品版本继续使用原数据库和 `aw recover`，不自动转移。

### 完整产品发行

`anolisa release install --bundle ABS_DIRECTORY --scope user|system` 安装或
继续已校验的解包发行，添加 `--dry-run --json` 可预览。
`release status` 读取其产品进度。产品 scope 默认 user，即使以 root 执行
也一样；它与下文旧的 `--install-mode` 选项分开。

`anolisa install --bundle ABS_DIRECTORY --profile standard-user --scope user`
调用同一产品安装器。按组件选择使用
`release install --bundle ABS_DIRECTORY --component tokenless`，同时包含依赖。
Profile 决定交付文件，`preset plan/apply` 和 `serve --preset container`
分别控制服务。`migrate plan/apply` 提供有记录的受支持旧 raw 安装迁移。
可选产品配方与组件自有 RPM 分包维护见 [distribution 说明](../README_zh.md)。

### 一级命令 — 已有组件生命周期

| 命令 | 说明 |
|------|------|
| `list` | 列出可用组件（别名：`ls`） |
| `install` | 从配置的后端安装组件（raw / rpm） |
| `uninstall` | 卸载组件 |
| `update` | 更新组件、CLI 自身（`self`）或全部（`all`） |
| `upgrade` | 应用 RPM/system image 升级计划（system scope） |
| `status` | 显示组件健康状态 |
| `doctor` | 诊断问题并给出修复建议 |
| `logs` | 查询组件日志（`--severity`，别名 `--level`） |
| `restart` | 重启组件服务（`--dry-run` 仅列出 unit，不执行重启） |
| `repair` | 手工修改 RPM 后协调 ANOLISA 状态与 rpmdb |
| `adopt` | 将已有 system RPM 记录为已采纳安装 |
| `forget` | 仅删除状态记录，不执行包操作 |

`status` 和 `doctor` 允许修改 raw 安装中 `type = "config"` 文件的内容，
同时继续检查文件是否存在、权限和路径安全。

`logs --since <RFC3339>` 按包含边界的时间戳筛选，正确比较时区偏移和小数秒。
只有设置 `--since` 时，无法解析时间戳的记录才会被排除。

### 二级命令 — 管理

| 命令 | 说明 |
|------|------|
| `overview` | 显示实际记录的组件版本、可用状态及可选本地候选比较 |
| `setup` | 引导明确的用户 AW 配置并预览服务策略 |
| `adapter` | 管理组件到框架的适配器 |
| `preset` | 查看并显式应用原生服务启用策略 |
| `serve` | 在没有 systemd 的环境管理允许的前台服务 |
| `migrate` | 查看并执行固定的旧 raw 到产品发行迁移 |
| `osbase kernel` | 内核模块与 eBPF 管理 |
| `osbase sandbox` | 沙箱运行时管理（runc、gvisor、firecracker 等） |
| `osbase security` | 安全覆盖层管理 |
| `system` | 系统辅助守护进程生命周期 |
| `register` | 加入/离开 Agentic OS Co-Build Program |
| `env` | 显示环境检测结果 |
| `bug` | 生成 bug 报告（`--component cosh-ng` 时附加 cosh-shell 诊断包摘要；`COSH_SHELL_BIN` 可覆盖二进制路径） |

默认情况下，`anolisa adapter enable <component> openclaw` 会在宿主的各条命令
支持 capability consent 时同意插件声明的能力。添加 `--no-accept-capabilities`
可在安装和激活两个阶段拒绝自动授予同意。此选项不撤销宿主已有的授权，也不授予
unsafe-install 覆盖权限。

通过 `anolisa adapter enable tokenless opencode` 启用 Tokenless 本地插件。
ANOLISA 管理插件链接；启用或禁用后需重启 OpenCode。
自定义配置目录及已有安装的接管规则见
[adapter 参考](../../docs/user-guide/zh/getting-started/anolisa-cli.md#adapter)。

千问办公 QwenWorkCN 的原生 Bash 和 macOS 工作区 Shell 会话使用
`anolisa adapter enable tokenless qwenwork` 接入 Tokenless。完成后重启应用并在应用内
检查激活状态；CLI 验证文件，但激活状态为 `unknown`。已有脚本安装需先卸载再通过 CLI 启用。

## 已有组件安装模式

| 模式 | 前缀 | 使用场景 |
|------|------|----------|
| `system` | `/usr/local`（或自定义 `--prefix`） | root 运行时的默认值 |
| `user` | `~/.local` | 普通用户运行时的默认值 |

可通过 `--install-mode user|system` 显式选择。
这些模式用于已有组件命令，完整产品发行使用 `--scope` 和按版本分开的目录。
`update all` 不更新完整产品发行；`update self` 拒绝直接替换产品管理的
CLI。RPM 管理的 CLI 自更新仍由原生包管理器完成。

只读发现范围比修改范围更广：普通用户可以列出、检查和诊断可见的
system 安装，也可以让 adapter 识别它；生命周期修改仍然只作用于所选
scope。因此，即使 system scope 已安装同名组件，
`anolisa --install-mode user install <component>` 仍可创建独立的 user 安装。

## 全局选项

| 参数 | 作用 |
|------|------|
| `--dry-run` | 输出计划但不执行 |
| `--json` | 输出机器可读的 JSON |
| `-v, --verbose` | 增加详细程度 |
| `-q, --quiet` | 隐藏非错误输出 |
| `--no-color` | 禁用彩色输出 |

完整的命令形式、scope 行为与恢复流程见
[CLI 用户指南](../../docs/user-guide/zh/getting-started/anolisa-cli.md)。

RPM 安装支持 Yum 3 和 DNF 4。使用 `sudo` 执行 RPM `--dry-run`，
可在安装前检查依赖和冲突。

## 架构

五 crate Cargo workspace：

| Crate | 职责 |
|-------|------|
| `anolisa-cli` | 命令解析、分发、终端 UI |
| `anolisa-core` | 组件解析、适配器管理、osbase 安装逻辑 |
| `anolisa-env` | 环境检测（发行版、架构、能力） |
| `anolisa-build` | 构建时代码生成与资源嵌入 |
| `anolisa-platform` | 文件系统布局、原生包查询与事务、systemd、IPC、权限辅助 |

支持双后端：**raw**（OSS tar.gz）和 **RPM**（yum/dnf 事务）。生命周期
planner 区分 ANOLISA 自有文件与 native package authority，并在副作用前
记录崩溃恢复意图。组件元数据通过 `component.toml` 声明。

raw backend 每次解析都会重新拉取 distribution index，仓库不可达时命令直接
失败。`repo.toml` 中旧的 `cache_ttl_secs` 和 `offline_fallback` 字段仍可正常
解析以保持向后兼容，但当前 raw backend 不会使用这些值。

## 环境要求

- Linux（x86_64 / aarch64）或 macOS 11+（arm64 / x86_64，功能受限）

Intel macOS 的预构建包和 npm 支持需要使用包含 `darwin-x64` 平台包的新版本。
官网安装脚本已同时支持 ARM Mac 与 macOS 11 及更高版本的 Intel Mac。
- 从源码构建需要 Rust ≥ 1.93

## 许可证

Apache License 2.0 — 详见 [LICENSE](../../LICENSE)。
