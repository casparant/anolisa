# anolisa CLI

`anolisa` CLI 用于安装完整 ANOLISA 产品发行及管理独立组件。完整 Linux
发行使用 ANOLISA 包记录和按版本分开的程序目录。已有组件命令解析 raw/RPM 来源，
维护按 scope 隔离的记录，并诊断或修复状态漂移。

---

## 安装

### 方式 A：安装脚本（推荐）

```bash
curl -fsSL https://get.agentic-os.sh | bash
```

在 GNU/Linux x86_64 或 aarch64 上，此命令安装产品的 user-scope 完整组合，
下载和解包需要 Python 3.6 或更新版本。显式使用安装脚本的 `--rpm` 参数，
可在 system scope 安装全部已发布的原生 RPM 组件。在 macOS 上，它安装独立 CLI。Linux 用户可以通过
`--cli-only` 选择独立 CLI 渠道。版本选择、归档安装、文件位置、已有入口冲突和
旧参数说明见[安装指南](installation.md)。

### 方式 B：YUM（Alinux）

```bash
sudo yum install anolisa
```

验证安装：

```bash
anolisa --version
```

---

## 本地概览与用户配置

`anolisa overview` 汇总产品记录、旧组件记录与本地 RPM 记录中的实际组件版本，
并检查本地可执行入口。它不下载索引，也不执行组件程序猜版本；无法读取的证据会
单独说明。普通用户可以读取公开的系统产品摘要，无需访问管理员私有的包数据库。

```bash
anolisa overview
anolisa overview --scope system --check-services --json
anolisa overview --candidate-bundle /absolute/unpacked-candidate --json
anolisa setup
anolisa setup --non-interactive
```

`--scope user|system|all` 只筛选安装证据的可见范围，不授予写权限。
`--check-services` 查询 systemd 状态，不启动服务，也不把 unit active 当作接口健康。
候选比较会验证本地 bundle 并列出包版本差异，不查询远端最新版。产品更新先查看
`install --bundle DIR --dry-run`；受支持的历史迁移入口见 `migrate plan --help`。

`setup` 先显示相同概览。在终端中，它可以询问已装 AW Adapter、Agent 可执行文件和
新的用户配置位置；管道、JSON 或 `--non-interactive` 执行不提问。
无人值守创建时，从 `aw agent list --json` 选择实际名称，并指定真实 Agent 程序：

```bash
aw agent list --json
anolisa setup --write-config "$HOME/.config/anolisa/aw.yaml" \
  --agent qoder --agent-bin /absolute/path/to/qoder --dry-run --json
anolisa setup --write-config "$HOME/.config/anolisa/aw.yaml" \
  --agent qoder --agent-bin /absolute/path/to/qoder --yes
```

Agent 参数使用可重复的 `--agent-arg VALUE`，按字面值传递。创建使用当前调用用户的
身份与 XDG state，保留已有文件，初始 Provider 和事件策略为空。
`--provider NAME` 配合显式 `--provider-template ABS_JSON`，会把选中的定义渲染到
单独的禁用文件供检查，不自动启用。`setup --preset host --service-scope system`
仅预览服务计划；应用仍需显式执行 `preset apply`。
系统软件安装与普通用户的 Agent 配置分别完成。

## 产品发行命令

Linux 产品归档包含可交付的组件，产品 profile 决定安装哪些包。`--bundle`
指向已解包真实目录的绝对路径。首次安装使用该归档内的 CLI：

```bash
./packages/anolisa-cli/payload/bin/anolisa release inspect --bundle "$PWD" --json
./packages/anolisa-cli/payload/bin/anolisa install --bundle "$PWD" --scope user --dry-run --json
./packages/anolisa-cli/payload/bin/anolisa install --bundle "$PWD" --scope user --json
./packages/anolisa-cli/payload/bin/anolisa release status --bundle "$PWD" --scope user --json
```

`release inspect` 校验全部交付包，列出归档声明的产品组合，不调用 AW 或写入
安装状态。选择包之前先完成整套校验，因此未选包损坏也会导致验证失败。
`install --bundle` 与 `release install --bundle` 使用同一安装实现。
`--dry-run` 检查选择与已有入口冲突，不安装包或发布命令。
`release status` 读取产品进度和当前版本引用，不报告服务健康状态。

新的 schema-2 归档声明下列 profile。准确的包集合和可用原生组合见
`release inspect --json`。

| Profile | Scope | 安装内容 |
| --- | --- | --- |
| `complete` | user 或 system | 本平台交付的全部包，包括原生集成 |
| `minimal` | user 或 system | ANOLISA CLI 与 AW，需显式选择的精简组合 |
| `standard-user` | user | 用户可运行的核心程序、已支持的 Provider 和技能 |
| `standard-system` | system | 本平台支持的后端核心程序、已支持的 Provider 和技能 |
| `native-<agent>` | system | 标准后端及该 Agent 需要的原生接入包 |

schema-2 归档在两个 scope 都默认选择 `complete`，安装全部交付包，包括原生
插件与已声明的运行依赖。`minimal`、`standard-user` 等精简组合仍可显式选择。
Profile 决定文件，不会启用服务、将 Agent 切换到 AW 或满足特权运行条件。旧 schema-1 归档保留全包安装行为，不接受
profile 或组件选择。

一个原生接入包可以包含多个 Agent 的资源。安装该包不会激活这些 Agent
的插件，启用仍需显式执行。

也可以从同一归档选择独立组件，带上其原生接入资源和依赖包：

```bash
anolisa release install --bundle /absolute/unpacked-release --component tokenless --component agent-memory --scope user
anolisa release status --bundle /absolute/unpacked-release --component tokenless --component agent-memory --scope user
```

`--profile` 与 `--component` 互斥。组件选择始终包含 ANOLISA CLI；AW 仅在 profile 或明确的包依赖要求时包含，
不使用 AW 的组件选择不必安装 AW。
简写 `anolisa install tokenless --bundle /absolute/unpacked-release --scope user`
选择归档里的单个组件；`anolisa install tokenless` 继续使用配置的独立组件
backend。`install --all` 仍枚举组件仓库，不表示产品 profile。

产品 scope 始终默认 `user`，包括 root 执行时。system 安装必须以 root 执行并
显式选择 `--scope system`，不会自动调用 `sudo`。不要将产品命令与旧
`--install-mode` 或 `--prefix` 混用。程序、资源、配置和包数据库的位置见
[安装指南](installation.md#程序配置与包数据库的位置)。

安装会记录每个本地包的提交，选定的包全部通过验证后才激活新发行。中断后保留
归档和输出中的产品记录，以相同归档、scope、profile 或组件集合重试。不同选择
使用不同安装身份，不能接续对方的部分操作。已提交的包仍有记录，中断不代表
整次发行已经回滚。无法确认归属的已有命令和编辑过的配置会保留并报告冲突。
已支持的历史 raw 安装可使用下文迁移命令。

激活较小组合时，会移除新组合不再提供且未被修改的产品命令入口。管理员修改过
的入口会保留并阻止这次切换。较早版本的程序文件仍保留，可用于回退。

安装会准备禁用的 Provider 模板，固定程序和资源路径。必须按实际调用用户
渲染用户目录参数；模板不是可直接执行的 AW transport 定义，AW 不在 argv
中展开 shell 变量。system 安装不会把用户数据或会话 socket 绑定到 root
安装者的环境。启用 Provider、连接原生 Agent、准备服务用户、capability、
配置和 unit 都是单独的部署动作。已有组件的 `status`、`update all`、
`uninstall` 命令不管理 产品发行记录。

### 查询产品包记录与检查文件

```bash
anolisa release list --scope user --json
anolisa release doctor --bundle /absolute/unpacked-release --scope user --json
sudo anolisa release list --scope system --json
sudo anolisa release doctor --bundle /absolute/unpacked-release --scope system --json
```

`release list` 只读所选 ANOLISA 包数据库，不检查 payload 文件，也不需要 bundle，
不会跨到另一数据库列出旧 AW 所有的版本。`release doctor` 检查所选发行的已登记
文件，不修复；应提供安装时相同的 `--profile` 或 `--component` 选择。
已有且核验匹配的产品记录仍可引用原 AW 数据库。`release status` 则显示产品安装
进度。这些查询不启动 AW、Provider 或组件服务，不建库、不转移所有权。

系统产品记录位于管理员私有的 `/var/lib/anolisa`，清单与文件检查使用 root。
普通用户仍可运行已安装的程序、读取静态资源，不需要访问这些包记录。

### 恢复产品包事务

```bash
anolisa release recover --scope user --json
sudo anolisa release recover --scope system --json
```

选择原安装的 scope。此命令显式恢复新 ANOLISA 包数据库中未完成的事务，无需
提供 bundle。结果报告数据库位置、已提交、已回滚和冲突操作；不切换活动产品
版本，也不启动服务。先处理报告的冲突文件，再重试相同安装。
恢复会修改文件，因此拒绝 `--dry-run`。已有 AW 所有的产品版本继续使用原数据库
及同 scope 的 `aw recover`，此命令不会将其管理权转移到 ANOLISA。

### 清理不再使用的产品版本

先激活另一发行，再用旧版本原来的 bundle 和包选择预览、清理：

```bash
anolisa release uninstall --bundle /absolute/old-unpacked-release --scope user --dry-run --json
anolisa release uninstall --bundle /absolute/old-unpacked-release --scope user --yes --json
```

原安装如使用 `--profile NAME` 或重复的 `--component ID`，清理时保持相同选择。
系统清理需要 root 和 `--scope system`。实际执行需 `--yes`，预览不需要。
命令拒绝活动版本、被修改的产品控制文件，以及该版本已有或待完成的服务单元发布
记录；仅删除核验过的自有 payload 和已登记资源别名，保留配置、数据、未知文件、
当前版本控制文件和空包登记元数据。产品进度先记录 `removing`，完成后记录
`removed`。包事务中断后先执行 `release recover`，再重试同一清理命令。
历史 AW 所有的版本保留原 AW 卸载权限，此命令不会接管。

### 服务预设与前台运行

安装 system raw 产品并准备好组件配置后，使用相同归档和 profile 发布选定的
system unit：

```bash
sudo anolisa release services --bundle /absolute/unpacked-release --profile standard-system --scope system --service ws-ckpt --service agent-sec-core --service agentsight --json
sudo anolisa release services --bundle /absolute/unpacked-release --profile standard-system --scope system --service ws-ckpt --service agent-sec-core --service agentsight --apply --json
```

第一条命令检查目标路径和模板摘要，`--apply` 登记已验证的 unit 并 reload
systemd，包含所需的 unit 依赖。未知原生 unit 和管理员覆盖会被保留并拒绝
覆盖。发布不会启用或启动服务。RPM 包的 unit 已由原生包管理器安装。

`preset` 使用已安装的服务目录和显式选择的 systemd manager。先预览，再应用：

```bash
sudo anolisa preset plan --preset host --scope system --initialize --json
sudo anolisa preset apply --preset host --scope system --initialize --json
```

`--initialize` 允许尚无部署记录的 unit 通过 systemd preset policy 初始化
启用状态，保留管理员覆盖与已有记录中的选择。启用和立即启动是独立动作，
预设可以请求立即启动。unit、配置、用户和宿主运行条件必须事先准备完成。
安装包不会应用 `host` 预设。用户 manager 操作应由实际用户执行，不使用
`sudo`。

容器或没有 systemd 的环境可以运行允许的前台组合：

```bash
anolisa serve --preset container --dry-run --json
anolisa serve --preset container
```

内置 `container` 预设以调用用户身份运行 AW，要求该用户的 AW 配置、runtime
目录及其他预检条件就绪。它不会启动 root 后端或 agent-memory MCP stdio
会话。服务继承 stdout/stderr；只有配合 `--dry-run` 才支持 `--json`。
容器应使用 init 或运行时的 `--init` 回收 PID 1 子进程。管理器等待服务就绪，
转发终止信号，并在必要进程失败时停止整组。

两类命令均从当前产品资源或已安装的原生策略包发现服务目录。可通过
`--config /absolute/services.json` 显式指定已审阅的目录。root 后端仍需要
独立的宿主服务或明确配置的特权部署。

### 迁移历史 raw 安装

迁移支持同一 scope 内、具有可信 `installed.toml` 文件清单的 ANOLISA raw
安装。先生成固定计划，再执行修改：

```bash
./packages/anolisa-cli/payload/bin/anolisa migrate plan --bundle "$PWD" --profile standard-user --scope user --output "$HOME/anolisa-migration.json" --json
./packages/anolisa-cli/payload/bin/anolisa migrate apply --plan "$HOME/anolisa-migration.json" --json
```

计划检查不安装包、不停止服务、不改写旧状态；`--output` 只发布指定的计划
文件。先处理所有 blocker，再重新生成计划并执行。原生 adapter 必须禁用，
已记录的 daemon unit 必须能确认处于 inactive。RPM 管理记录、缺少归属清单、
未知命令文件以及跨 scope 转移均阻止自动接管。system 来源可指定
`--source-prefix`；user 来源使用实际调用用户的 HOME/XDG 目录。

旧独立 `anolisa` 可执行文件没有组件归属记录。审阅实际文件及其 SHA-256 后，
可在 `migrate plan` 增加 `--legacy-cli-sha256 <reviewed-sha256>`，接受这些
准确的 CLI 字节。该选项不会接管其他未知命令。
只有这个独立 CLI、没有 `installed.toml` 的安装也可以迁移。计划记录状态
文件缺失这一事实，不虚构旧组件记录；计划生成后若出现状态文件，需重新规划。

执行绑定计划中的调用者、归档摘要、包选择、旧状态与命令以及目标目录。
原状态和被替换命令会备份到报告的迁移目录，再调用正常产品安装器。只有成功
激活后才退役旧 raw 记录；旧组件文件、配置和数据保留用于回退。安装失败
保留旧记录与持久化进度，使用同一计划重试 `migrate apply`。来源证据或选择
变化时需生成新计划。迁移不会将原生 Agent 配置转换成 AW，也不会转移 RPM
所有权。

## 已有组件的 scope 与可见性

`--install-mode user` 写入当前用户目录，`--install-mode system` 写入
system state，通常需要 root。未指定时，root 默认使用 system mode，普通
用户默认使用 user mode。

只读命令使用 user-plus-system 视图。因此普通用户可以看到和诊断 system
安装，adapter discovery 也可以使用其发布的 contract。修改命令仍然只写入
显式选择的 scope；同一组件的 user 安装和 system 安装可以共存。

---

## 已有组件命令

### install

通过配置的 raw 或 RPM backend 安装单个组件，或规划 index 中的全部组件：

```bash
anolisa --install-mode user install <component>
sudo anolisa --install-mode system install <component>
anolisa install --all
```

另一个 scope 中已有安装，不会让当前 scope 变成“already installed”。已有
记录的重装或变更由 lifecycle planner 处理，不会被静默覆盖。

RPM 安装支持 Yum 3 和 DNF 4。ANOLISA 优先使用 yum，不存在时使用 dnf。
安装前可使用 `sudo` 检查依赖和冲突：

```bash
sudo anolisa --install-mode system --dry-run install cosh-ng --backend rpm
```

此检查不会安装或更改软件包。请先解决报告的冲突再重试；检查通过不保证
后续安装一定成功。指定版本与系统版本锁等限制冲突时，命令会失败，不会
替换为其他版本。

如果 ANOLISA 无法识别系统包管理器，请按提示手工安装缺失依赖后重试。

### uninstall

从所选 scope 移除一个安装：

```bash
anolisa uninstall <component>
anolisa uninstall <component> --purge
sudo anolisa --install-mode system uninstall <component> --remove-system-package
```

ANOLISA-owned 文件和 managed RPM package 由各自的 owner backend 移除。
adopted 或 observed system RPM 默认保留；只有确实要移除 native package 时
才使用 `--remove-system-package`。

### update

更新一个组件、全部已记录组件、CLI 本身，或运行只读 RPM 更新检查：

```bash
anolisa update <component>
anolisa update all
anolisa update self
anolisa update --check
```

`update all` 不更新 CLI 本身。delegated 成员会在可行时合并成一次 native
transaction，但每个组件仍保留独立的 recovery journal 和 record。

`update self` 拒绝直接替换 产品管理的 CLI，应通过已校验的产品发行更新
该 CLI。独立 CLI 保留原有自更新流程；RPM 管理的 CLI 仍由原生包管理器更新。

### list 与 status

查看有效的 user-plus-system 视图：

```bash
anolisa list
anolisa list --installed
anolisa status
anolisa status <component>
```

user view 中两个 scope 都有同名组件时，user record 为 active，system record
仍作为 shadowed state 可见。system-mode view 只读取 system root，不枚举其他
用户的 state。

未跟踪的 RPM 观察使用与安装相同的包解析器。index 映射优先于历史包 alias
和 capability：提供 `anolisa-component(cosh)` 的旧 cosh-ng RPM 显示在
cosh-ng 下。`action=install` 表示生命周期操作，
并不代表已检查 native 依赖；请通过 RPM install dry-run 检查这些包能否共存。

### doctor

运行只读的 health、dependency、service、state 与 recovery journal 检查：

```bash
anolisa doctor
anolisa doctor <component>
anolisa --dry-run doctor <component>
```

`doctor` 扫描当前 visibility view 中的全部 root：user mode 包含 user root 和
可读的 system root，system mode 只包含 system root。当前调用不能修改某个
system root 时，它会在修复建议中补全
`sudo anolisa --install-mode system`。`--fix` 在当前版本中仍为保留参数；请
显式执行输出的 `fix_plan`。

对于 raw 安装，`status` 和 `doctor` 允许修改声明为 `type = "config"` 的文件
内容。文件缺失、不安全路径、意外符号链接、权限或 capability 偏移仍会检查失败；
普通 data 和 executable 文件仍需通过 SHA-256 校验。旧安装记录会从保存的
component manifest 在内存中恢复无歧义的 config 声明。若重叠的目录声明混合了
config 和不可变类型，旧文件因缺少来源映射而继续进行摘要校验。
请先备份编辑过的配置，再重新安装组件以记录准确类型。诊断不会改写状态文件。

### restart

重启所选 scope 安装记录中的 service：

```bash
anolisa --dry-run --install-mode user restart <component>
anolisa --install-mode user restart <component>
anolisa --dry-run --install-mode system restart <component>
sudo anolisa --install-mode system restart <component>
```

`--dry-run` 只列出将要重启的 unit，不会执行 `systemctl daemon-reload`
或 `systemctl restart`。system 模式预览只读已记录状态，不获取排他安装锁，
因此不需要 state root 的写权限。

### upgrade

预览或应用 system/RPM image 升级。raw-managed 组件会报告为 skipped，不会被
迁移到其他 backend：

```bash
anolisa --install-mode system --dry-run upgrade
sudo anolisa --install-mode system upgrade
sudo anolisa --install-mode system upgrade --target <profile>
```

### adopt、repair 与 forget

在不混淆 package ownership 的前提下管理状态：

```bash
sudo anolisa --install-mode system adopt <component>
sudo anolisa --install-mode system repair <component>
anolisa --install-mode user forget <component>
sudo anolisa --install-mode system forget <component>
```

`adopt` 将已有 system RPM 记录为 delegated-adopted，不取得 native removal
authority。`repair` 协调指定 scope 的 record 与 rpmdb 或中断 journal。
`forget` 只删除所选 scope 的记录，绝不执行 package 或 owned-file 删除；
user scope 的 forget 不能删除只是在视图中可见的 system 记录。

有 pending operation 时，即使安装尚未创建安装记录就失败，`forget` 也会
引导先运行 `repair`。repair 会将 journal 与 rpmdb 协调；forget 不能丢弃
可能已经改变系统的 transaction 证据。

### adapter

管理组件 adapter：

```bash
anolisa adapter scan
anolisa adapter enable <component> [framework]
anolisa adapter disable <component> [framework]
anolisa adapter status [component]
```

对于 OpenClaw 插件，`adapter enable` 默认同意插件声明的能力。ANOLISA 仅在
对应命令的 help 列出 `--accept-capabilities` 时，将该参数添加到安装和激活
命令。可通过以下选项拒绝自动授予同意：

```bash
anolisa adapter enable tokenless openclaw --no-accept-capabilities
anolisa --dry-run adapter enable tokenless openclaw --no-accept-capabilities
```

dry-run 计划中的两条命令均不带同意参数。此选项仅适用于 OpenClaw 插件
adapter；其他框架和仅含 skill 的 adapter 会拒绝它。它不撤销 OpenClaw 中
已经授予的能力，未提供同意参数的宿主仍按自身策略处理。
`ANOLISA_ACCEPT_CAPABILITIES` 或 `AGENT_MEMORY_ACCEPT_CAPABILITIES` 等环境
设置不控制 `adapter enable`；此入口须使用 CLI 选项。

若 OpenClaw 需要新的授权，其拒绝仍作为执行失败报告（`EXECUTION_FAILED`，
退出码 1），adapter 收据会保留，可通过
`anolisa adapter disable tokenless openclaw` 清理。在另行授权前，请审阅宿主
报告的能力要求。Capability consent 不授予 `--allow-unsafe-plugin-install`
权限；同意被拒绝时会单独诊断，不归为插件安全扫描拒绝。

对于 OpenCode，可用以下命令管理已安装的 Tokenless 插件：

```bash
anolisa adapter enable tokenless opencode
anolisa adapter status tokenless
anolisa adapter disable tokenless opencode
```

driver 从 PATH 查找 `opencode`，也可通过 `OPENCODE_BIN` 指定。它将 manifest 中的
`.js` 或 `.ts` 入口注册为 `plugins/<plugin_id>.<扩展名>`，配置目录依次取
`OPENCODE_CONFIG_DIR`、`XDG_CONFIG_HOME/opencode`、`~/.config/opencode`。
自定义目录必须是绝对路径。后续 status 和 disable 应使用相同的目录配置；若配置
发生变化，需恢复原环境后重试清理。本 driver 不管理项目级插件安装或 npm 插件。

enable 会接管指向同一插件源的已有符号链接，包括 Tokenless 独立安装脚本创建的
链接；相对链接展开后必须与记录的源路径一致。目录别名链接会被拒绝，以确保包卸载后
仍能清理。接管后 disable 可以删除该链接。同名文件、目录或指向不同目标的链接会被保留；
清理失败时保留 receipt，解决冲突后可以重试。同路径升级以原子操作替换入口。
若 enable 或 disable 中断，保持相同配置重试命令即可恢复未完成的变更。如果错误
提示保留了某个条目，请保留其恢复目录，解决公开路径上的冲突后重试；恢复也支持
被移走的目录。切回独立安装脚本前，请先完成 ANOLISA disable 和恢复操作；独立脚本不会
处理这些 journal。配置目录所在的文件系统必须支持原子交换和不覆盖的 rename，才能完成
替换和清理；不支持时操作会失败，仅解决路径冲突不能补足文件系统能力。如果原安装脚本使用了
`TOKENLESS_OPENCODE_CONFIG_DIR`，通过 ANOLISA 启用前需将
`OPENCODE_CONFIG_DIR` 设置为相同目录。

启用或禁用后需重启 OpenCode。status 检查链接和安装源；若启用中断或清理仍需重试，即使
当前链接匹配也会保留 `cleanup_failed`，需重试 enable 或完成 disable 才能解除。
运行时加载状态报告为
`unknown`；链接存在不代表运行中的 OpenCode 已加载插件。`--dry-run` 仅预览操作，
不修改插件文件或 receipt。

#### 千问办公 QwenWorkCN（macOS）

```bash
anolisa --dry-run adapter enable tokenless qwenwork
anolisa adapter enable tokenless qwenwork
anolisa adapter status tokenless
anolisa adapter disable tokenless qwenwork
```

driver 旁加载 receipt 归属文件，不修改桌面应用的激活状态。enable/disable 后重启千问办公，
开启新会话。status 将加载和激活状态报告为 `unknown`，文件存在不代表已激活。
后续操作保持相同的 `TOKENLESS_QWENWORK_HOME`；`TOKENLESS_QWENWORK_APP` 可覆盖检测到的
应用 Bundle。已有脚本安装需先用原卸载器移除，再通过 CLI 启用。修改过的文件和无关文件
会保留。依赖、迁移及 1.2.5 原生 Bash / 1.3.2 工作区 Shell 兼容性基线详见[千问办公集成](../tokenless/framework-integration.md#千问办公macos)。

### logs 与 bug report

查看组件日志或生成诊断包：

```bash
anolisa logs <component>
anolisa logs <component> --limit 50
anolisa logs <component> --severity warn
anolisa logs <component> --since '2026-10-01T00:30:00Z'
anolisa bug
```

`--level` 是 `--severity` 的别名。

`--since` 接受包含边界的 RFC3339 起始时间，按实际时间比较时区偏移和小数秒。
例如，`2026-10-01T08:30:00+08:00` 和 `2026-10-01T00:30:00Z` 选择相同的时间窗口。
`--limit` 按追加顺序保留最近的匹配记录。

无效的 `--since` 值返回 `INVALID_ARGUMENT`。启用时间筛选时，`started_at` 无法
解析为 RFC3339 的记录不匹配；省略 `--since` 可查看这些记录。格式错误的 JSON
仍会导致查询报错。查询不会修改日志文件。

使用 `--component cosh-ng` 时，`anolisa bug` 还会调用已安装的
`cosh-shell` 二进制，将脱敏诊断包导出到全新的私有路径（`0600`，绝不
覆盖已有文件），并在报告中汇总其健康检查 finding ID 与清单摘要。二进
制从 cosh-ng 安装的私有 libexec 位置解析（raw 契约目录与 RPM 的
`/usr/libexec` 路径），可用 `COSH_SHELL_BIN` 覆盖，PATH 仅作为开发场
景回退；报告中的手工与复现命令使用解析后的绝对路径，保证可直接执行。
诊断包写入调用者本人的 state 根目录——即使诊断的是 system scope 安
装——且不会被上传；请在本地审阅后再附加到 issue。无法产出诊断包时，
报告会显式说明原因，并给出可手工执行的 `diagnostics export` 命令。

---

## 恢复行为

install、uninstall、update、adopt 和 repair 会先在所选 state root 写入
recovery intent，再执行 lifecycle 副作用。native package 操作采用
forward-only 策略：如果 dnf 可能已经提交而 ANOLISA record 尚未提交，journal
会保持 pending，`anolisa repair <component>` 会重新观察 rpmdb。owned-file
操作保留已校验的 backup，并在失败时按逆序补偿。`forget` 是原子的 record-only
状态更新，不执行 package/file 副作用，也不创建 recovery journal。

`upgrade` 仍是兼容性 orchestrator，不是 planner/journal consumer。它会拒绝
已有的 pending recovery，并在 transaction 失败后重新观察 rpmdb，但不会创建
per-component recovery journal。`upgrade` 若被进程中断，应先运行
`anolisa doctor`，处理报告的 component drift 后再执行其他 lifecycle 修改。

不要为了解除阻塞而直接删除 pending journal。先运行 `doctor` 确认其 scope
和 subject，再执行带完整 scope 的 `repair` 命令。malformed 或 ambiguous
journal 会有意保持 pending，等待人工检查。

---

## 全局选项

| 选项 | 说明 |
|------|------|
| `--install-mode user\|system` | 选择已有组件命令的修改 scope |
| `--prefix <PATH>` | 覆盖已有组件命令的安装前缀 |
| `--dry-run` | 输出计划但不执行 |
| `--json` | 输出机器可读的 JSON |
| `-v, --verbose` | 增加详细程度 |
| `-q, --quiet` | 隐藏非错误输出 |
| `--no-color` | 禁用彩色输出 |
| `--version` | 显示 CLI 版本 |
| `--help` | 显示命令帮助 |

---

## 已有组件流程示例

先安装需要使用的 Agent，再扫描它的 adapter，并用扫描结果中的名称替换
`<framework>`：

```bash
curl -fsSL https://get.agentic-os.sh | bash -s -- --cli-only
anolisa env
anolisa install tokenless
anolisa adapter scan
anolisa adapter enable tokenless <framework>
anolisa doctor
anolisa status
```

---

## 配置

system mode 从 `/etc/anolisa/repo.toml` 读取 backend 选择和 endpoint，
user mode 从 `~/.config/anolisa/repo.toml` 读取：

```toml
schema_version = 1
default_backend = "raw"

[backends.raw]
base_url = "https://repo.example.com/anolisa/v1/"
```

raw backend 每次执行都会重新拉取 distribution index。当前不会使用缓存的 index 作为回退，因此仓库不可达时命令会直接失败。
旧的 `cache_ttl_secs` 和 `offline_fallback` 字段仍可正常解析以保持向后兼容，但当前 raw backend 不会使用这些值。

RPM 仓库需要代理或私有 CA 时，可在 `/etc/yum.conf` 的 `[main]` 段直接配置
`proxy`、`proxy_username`、`proxy_password` 或 `sslcacert`；该文件不存在时
使用 `/etc/dnf/dnf.conf`。未明确配置时，ANOLISA 使用环境代理（`http_proxy`、
`https_proxy`、`all_proxy`、`no_proxy`）和系统信任的证书。

CLI 参数只影响当前执行的操作，不存在 `[install] mode` 配置。

---

## 参见

- [安装指南](installation.md)
- [故障排查](troubleshooting.md)
