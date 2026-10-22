# 运行维护

[English](../../../en/user-entrypoint/aw/operations.md)

以下命令使用安装时的配置路径。所有共享同一份配置的 Agent 退出后，再停止对应 AW 服务。

## 查看状态与停止

```bash
aw status --config "$HOME/aw/aw.yaml"
aw inspect --config "$HOME/aw/aw.yaml" --json
aw stop --config "$HOME/aw/aw.yaml"
```

`inspect` 包含 Provider 摘要。退出 Agent 后 AW 服务仍可运行，用 `stop` 停止，
再用 `status` 确认。sec-core 和 AgentSight 服务需分别管理。

分组命令保留相同的参数与 JSON 响应：

```bash
aw service status --config "$HOME/aw/aw.yaml"
aw service stop --config "$HOME/aw/aw.yaml"
aw service serve --config "$HOME/aw/aw.yaml" --state-dir "$HOME/aw/state"
aw service --help
aw --version
```

`serve` 持续在前台运行，直到被停止或中断。它不安装 systemd 单元，也不启动 Agent。
原有 `aw serve/status/stop` 命令保持兼容。`--version` 显示 CLI crate 版本，
不是已安装包的 Preview 发布标识或原生 Agent 版本。

## 查看安装记录

```bash
aw list --installed --prefix "$AW_PREFIX"
aw list --installed --prefix "$AW_PREFIX" --json
```

读取指定目录的 AW 安装记录，展示包版本、平台和文件数量。format 1 包含源码提交，
format 2 增加 `tool`/`provider` 类型及 user/raw 安装身份。`--json` 提供结构化依赖。
它不扫描整台机器、不查询 RPM/DEB、不校验文件内容，也不运行已安装的程序。
状态分为 `absent`（目录不存在）、`unmanaged`（没有 AW 安装记录）和
`installed`（已有记录）。没有记录不代表不存在软件；记录损坏时会报错，不伪装成
空列表。JSON 使用 `scope: "installation_records"`、`payload_verified: false`
和 `runtime_checked: false`。
format 2 还输出 `receipt_format: 2`、`installation_scope`、`backend` 和
`publisher_verified: false`。读取安装记录不会使其来源变为受信。

## 检查安装完整性

```bash
aw doctor --prefix "$AW_PREFIX"
aw doctor --prefix "$AW_PREFIX" --json
```

两种格式都会检查记录中的文件摘要、权限与路径安全性。format 1 还检查 Preview
必备文件和依赖声明、同构建依赖及文件归属冲突；format 2 校验记录中的 bundle 内
依赖闭包与精确文件长度。
结果区分 `checked` 与 `unhealthy` 组件，并列出具体问题。这是只读快照，
不下载、执行或修复软件，不认证发布者身份，也不检查服务、Agent 或 Provider
是否就绪。AW 未记录的文件不在检查范围内。

仅在安装存在、全部检查完成且没有问题时以 `0` 退出。目录不存在、没有安装记录、
检查未完成或发现完整性问题均返回 `1`。JSON 使用 `scope: "installation_integrity"`；
元数据损坏可能在生成报告前报错。运行时是否工作，仍需单独结合
`aw service status` 和 Agent 的实际工具行为判断。

## 查看已配置的 Agent 与 Provider

```bash
aw agent list --config "$HOME/aw/aw.yaml"
aw provider list --config "$HOME/aw/aw.yaml" --json
aw provider show compression --config "$HOME/aw/aw.yaml"
```

`show` 使用配置中实际存在的 Provider 名称。这些命令只校验并读取指定文件，不启动
Agent、Provider 或服务。输出名称、协议、传输类型及 event/step 引用，不显示私有配置、
环境变量或执行参数。`referenced` 也包含已禁用的引用，其启用标志单独展示。
已配置或已引用不代表运行时已经生效。JSON 以 `scope: "configuration"` 和
`runtime_checked: false` 标明这一边界。

## 升级与卸载

升级前退出 Agent，用旧版 AW 和原配置停止服务，备份配置与状态目录。
新包安装到新目录，再用新路径生成配置。保留旧包和备份，便于回退；
Preview 尚无跨版本数据迁移，升级时使用新的状态目录。

卸载使用安装时的前缀。系统安装执行下列命令，个人安装去掉 `sudo`。

```bash
sudo "$AW_PREFIX/bin/aw" uninstall --prefix "$AW_PREFIX"
```

卸载保留外部配置、审计和原生 Agent 数据。包文件被修改或缺失时会先报错，
按提示处理，不要直接删除仍在使用的目录。

对于开放的 format 2 安装，使用其所属的非 root 用户调用已有 AW CLI，
不要假设工具安装目录内也有 AW 二进制：

```bash
aw uninstall --prefix "$AW_DEMO_PREFIX"
```

同一命令识别 `.aw-installation.json`，移除其中记录的整个 bundle，而不是指定的单个包。
删除前检查所有权和已记录文件的完整性，未知文件会保留，因此安装目录可能继续存在。
它不移除原始源包，也不改写引用该目录的配置。不检查运行中的引用：卸载前应自行
停止使用者并更新外部配置。format 2 当前没有原地更新或迁移，应将替代版本安装到
另一个新目录。

原型会回滚进程内捕获到的失败，但尚无支持崩溃后续跑的卸载日志。
若错误提示保留了 `.aw-uninstall-*` 恢复目录，应保留并恢复提示中的文件后再重试，
不要将它当作临时垃圾删除。取消在文件之间检查，当前有界文件读取可能先完成。

## 保存数据

AW 的 `state_dir/journal` 保存调用元数据，不保存工具正文。
Tokenless 等组件有自己的数据目录和留存设置，参见[Tokenless 数据说明](tokenless.md#数据与隐私)。

需要跨重启保留记录时，在配置中使用明确的状态目录。该目录须归运行用户所有且权限为
`0700`。容器中还需要持久化该目录，以及框架自己的 profile 和工作区。
默认的自动状态目录可能位于临时存储。详细部署步骤见
[容器与持久化](../../../../developer-guide/zh/aw/native-agents.md#容器与持久化数据)。

## 常见问题

| 现象 | 处理方式 |
| --- | --- |
| 配置校验成功，启动仍失败 | 按错误检查 Agent 版本、可执行路径和 Provider 配置 |
| 修改配置后仍使用旧行为 | 用原配置或原 socket 停止旧服务，再启动修改后的配置 |
| 目录或配置权限不符合要求 | 使用当前用户拥有的私有目录，配置编辑时避免符号链接 |
| 服务协议不匹配 | 用旧二进制停止旧服务，再统一使用新安装的 AW |
| OpenClaw 显示结果替换不可用 | 检查支持的框架版本及 middleware API；此时安全检查和完成观察仍可工作 |
| 强制退出后留下 socket | 先确认原服务已退出，再移除该服务遗留的 `aw.sock`，保留 journal |
| 长时间运行出现 `native_event_limit` | 退出并重新启动 AW 管理的 Agent；新建原生会话不会重置实例计数 |
| 压缩统计为空 | 确认发生了受支持的工具调用，并检查 Tokenless 的录制设置 |

当前每个 Agent 实例最多关联 1,024 个原生事件；同时使用 before 和 after 时，一次工具
调用通常占两个条目。长期运行的 Gateway/App 需要安排实例重启。

检查策略是否生效时，应同时观察 AW 的处理结果和实际工具行为。
模型没有调用工具或被原生权限拒绝，都不能证明 AW 的规则已经执行。
