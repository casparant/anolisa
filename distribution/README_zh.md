# ANOLISA distribution

[English](README.md)

ANOLISA 发行工具将可独立交付的组件组织成完整产品发行。可选产品集成放在各自
源码根目录的 `.anolisa/` 中；组件代码继续位于 `src/`，Provider 实现继续位于
根目录的 `providers/`。能力组件自己管理独立 RPM 和 tarball 交付；这里提供共享
封装工具、产品组合、服务预设和旧安装迁移。

ANOLISA 直接调用 `crates/local-package` 安装产品 raw，AW CLI 不在这条调用链中。
共享库负责文件校验、包记录、事务和恢复。AW 保留运行框架、配置、Agent Adapter、
Provider 协议和本地扩展兼容入口。

## 维护入口

| 任务 | 入口 |
|------|------|
| 修改本地文件校验或事务规则 | `crates/local-package/` |
| 构建历史 AW Preview 包 | `tooling/aw-preview/` |
| 修改独立 RPM／tarball 交付 | 组件自己的 Make 目标、spec 与 packaging 目录 |
| 修改组件的 ANOLISA 适配 | `src/<component>/.anolisa/recipe.json` 与可选 `component.toml.in` |
| 修改 Provider 的 ANOLISA 适配 | `providers/<provider>/.anolisa/`；实现与独立交付仍由 Provider 自己维护 |
| 修改 ANOLISA CLI 自身的产品封装 | `distribution/anolisa/.anolisa/` |
| 修改产品包组合 | `release/linux.json` |
| 修改服务预设或容器允许列表 | `release/services.json` |
| 整理已经编译好的组件文件 | `packaging/component.py` |
| 生成完整 tarball | `anolisa/packaging/release/build.sh` |
| 准备可选产品源码快照 | `packaging/snapshot.py`；组件源码构建保留自己的入口 |
| 从已校验归档构建容器 | `containers/Containerfile` |

`scripts/build-all.sh` 继续作为开发构建和安装便利入口。独立构建保留组件自己
实际拥有的 spec 和打包入口，产品配方消费组件交付件。组件发布不要求 ANOLISA CLI、
产品 profile 或 AW。

组件可以继续共用仓库根目录的 `LICENSE` 等源码元数据，保留已有源码软链。
独立交付不要求在每个组件内另存许可证源文件；发布归档与软件包在组装产物时包含实际许可证内容。

源码内的 `.anolisa/` 集中存放可选的 ANOLISA 包映射、安装描述及产品专用封装输入。
独立组件维护者可以忽略或不携带这个目录，继续通过自己的 RPM／raw 入口发布。
同时服务于独立交付的 spec、tarball 脚本、服务模板和原生集成保留在组件原有位置。
`distribution/` 发现这些可选集成并维护共享产品工具，不再保存第二棵组件／Provider
配方目录。源码存放位置不改变包 ID 或 release tag。

上述独立性也适用于 os-skills 和 ktuner。COSH 承担产品 Agent 角色，保留其发行
组合，本次不增加 COSH 的 AW Adapter。AW 本身仍可独立使用；Provider 源码目录
只是仓库存放位置，不是运行时注册机制。

## 组件独立交付

CLI 与 daemon 保持在同一个后端包中，原生 Agent 插件和 skills 可以分别选择。
旧的大 RPM 包名保留为依赖包装包，让普通 RPM 升级继续保留原来提供的集成。
例如，`tokenless` 会拉入后端和原生插件；AW 组合选择 `tokenless-core` 与
`aw-provider-tokenless`，不需要带上所有原生插件。RPM 文件始终由原生包
管理器管理，产品安装器不会把这些文件重复登记到 raw 包数据库。

继续使用组件自己的 Make 目标和打包入口生成独立交付件。独立源码包保留组件
自己的构建和封装输入，不依赖产品 checkout。`packaging/snapshot.py` 支持可选
的发行快照，不是组件发布的强制入口。

无需编译即可拆分已经发布的 raw 包：

```bash
python3 distribution/packaging/component.py \
  --component tokenless --raw-input /absolute/tokenless.tar.gz \
  --version 0.8.8 --arch x86_64 --source-commit FULL_40_HEX_COMMIT \
  --output /absolute/new-package-directory
```

版本与提交必须替换为该交付件已核实的来源。输出是共享安装库使用的本地包目录。
单独打包 Provider 时，可以用 `--dependency-version ID=VERSION` 指明另外
交付的依赖版本。预编译 Cargo 组件使用 `--binary` 代替 `--raw-input`。

组件资源使用 `share/<component>/adapters/<agent>/`、
`share/<component>/skills/` 和 `share/<component>/templates/`。
`share/anolisa/` 存放产品登记与组合元数据。RPM 由对应子包保留需要兼容的
旧资源路径，内容为同版副本。配置与数据保留组件原有位置，调整静态资源目录
不会顺带搬迁数据库。

已有兼容路径和标识中仍可能包含 `anolisa`；组件自己独立交付和维护的模块可保留
历史名称。这仅是命名兼容，不允许常规 CLI、daemon 或原生 Agent 插件强制依赖
ANOLISA SDK、CLI 或产品服务。实际产品耦合应剥离，或放入可选的 ANOLISA 集成包。
保留旧资源和插件身份，不等于将所有历史名字都视为运行依赖。

## 完整产品发行

在支持的 Linux 构建环境中从固定源码生成发行：

```bash
distribution/anolisa/packaging/release/build.sh --output-dir "$PWD/target/product-release"
```

已经准备完整包集合时，使用 `--staged-input DIR --source-commit COMMIT
--epoch UNIX_SECONDS`，此路径不编译。归档包含全部可分发子包，以及 schema 2
的 `release.json`。安装时只选择指定组合及其依赖，但选择前会校验全部交付文件。

| Profile | Scope | 内容 |
|---------|-------|------|
| `complete` | user 或 system | 全部交付包，包括后端、原生集成、Provider 和技能 |
| `minimal` | user 或 system | AW 与 ANOLISA CLI，需显式选择的精简组合 |
| `standard-user` | user | 支持用户使用的组件、skills 和 Tokenless Provider |
| `standard-system` | system | 后端组件、skills 和已实现的 Provider |
| `native-<agent>` | system | 标准后端，加上该 Agent 已有的原生集成子包 |

user、system 和容器默认选择 `complete`。
Profile 只决定文件，不会自动启动 daemon。归档安装器普通安装成功后调用 `anolisa setup`：
无人值守执行显示概览，终端执行可以引导当前用户明确创建配置。
`anolisa overview` 显示实际记录的组件版本和本地可用状态，
`--candidate-bundle DIR` 仅比较已验证的本地候选发行，不执行升级。
网站显式 `--rpm` 流程以 system 模式执行 `install --all --backend rpm`，
安装全部已发布的原生组件。对应的 RPM 依赖包装包可以这样生成：

```bash
python3 distribution/packaging/product.py --format rpm-spec \
  --profile standard-system --version PRODUCT_VERSION --output /tmp/anolisa-standard.spec
```

生成的 noarch 包依赖实际组件子包。`anolisa-policy` RPM 提供明确禁用的服务
默认值，管理员另行应用 preset。离线安装、选择组合和有记录的迁移见
[安装指南](../docs/user-guide/zh/getting-started/installation.md)。

安装 system tarball 后，以 root 查看、发布后端原有的 unit 模板：

```bash
anolisa release services --bundle /absolute/unpacked-bundle --scope system \
  --service agent-sec-core --service agentsight --service ws-ckpt --json
anolisa release services --bundle /absolute/unpacked-bundle --scope system \
  --service agent-sec-core --service agentsight --service ws-ckpt --apply
anolisa preset plan --preset host --scope system
anolisa preset apply --preset host --scope system
```

Profile 或组件选集必须与产品安装时相同。Unit 发布会验证已安装文件，将
程序绑定到所选版本，拒绝覆盖未知的已有 unit，再重新加载 systemd，不启动
服务或修改开机启用状态。Preset 应用默认保留已有的开机启用选择；另加
`--initialize` 才会要求 systemd 对尚未初始化的 unit 应用管理员可覆盖的
preset 策略。组件配置保留既有位置和默认值。RPM 自己交付 unit；COSH 系统
gateway 需要自身的隔离和实例配置，不通过这个通用模板发布入口注册。

## 容器

在仓库构建上下文的 `bundle/` 放置已校验并解包的完整发行：

```bash
docker build -f distribution/containers/Containerfile -t anolisa:local .
docker run --rm -it \
  --mount type=bind,source=/absolute/aw-config,target=/home/anolisa/.config/aw,readonly \
  anolisa:local
```

镜像以 UID 10001 安装 `complete`，最终阶段只复制已安装文件，默认执行
`anolisa serve --preset container`。挂载的配置目录需要包含真实的 `aw.yaml`。
安装阶段与最终阶段的程序、状态和资源路径保持一致。构建镜像时可以通过
`ANOLISA_PROFILE` 选择其他支持 user 的包组合。
镜像使用 Ubuntu 24.04，交付的 GNU 二进制需要兼容该运行环境；构建时的安装
步骤会拒绝无法运行的 AW 或 CLI 二进制。

容器 preset 目前只启动 AW。特权后端 daemon、FUSE、eBPF 和 systemd
隔离功能不会隐式启用。Provider 和 MCP stdio 服务继续按会话启动。
前台管理器配合 `tini` 负责就绪检查、传递退出信号、停止依赖进程与回收子进程；宿主机
继续使用 systemd 管理服务。

## 验证

```bash
python3 -m unittest discover -s distribution/anolisa/packaging/release/tests -v
bash website/scripts/test-install.sh
cd distribution/anolisa
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo doc --workspace --no-deps --locked
```

打包 fixture 可以在 macOS 和 Linux 上检查已经生成的文件。真实 AW 安装、
RPM 升级事务与容器验收需要对应的 Linux 环境，不能在 macOS 上编译仅支持
Linux 的组件。
