# 完整产品发行合同

[English](2026-10-24_all-in-one-release.md)

ANOLISA 将展开的 AW 包组成一个 Linux 归档，负责组件组合、下载源、产品记录与激活。AW 通过已有数据库和事务校验、提交本地包文件。

## 构建与归档

[`packaging/release/linux.json`](../../packaging/release/linux.json)定义完整组合、必需资源、依赖、构建适配和公开命令。组合选择 cosh-ng，不同时包含与它冲突的历史 cosh。backend 与 Provider bridge 分别拥有自己的文件；声明 Provider 协议不等于启用实例或授予运行权限。

源码模式要求干净的 Linux x86_64 checkout 和现有发行 Cross 环境，固定独立源码快照，隔离 worktree、Cargo 输出、日志和产物。暂存模式要求明确提供完整包集合，不代表已重新编译或运行组件。

组装器核对包说明、版本、摘要、权限、ELF 架构和 GNU loader、精确依赖、文件冲突、资源与 AW 容量限制。源码证明记录宿主 libc 和 ELF 依赖；`abi = "gnu"` 不代表所有构建宿主都具有相同的最低 glibc 基线。tar 顺序、归属、时间和 gzip 元信息经过规范化，发布只写新输出目录，保留已有产物。

归档包含 `release.json`、`install.sh`、逐文件校验、构建证明和 `packages/<id>/{package.json,payload/}`。许可证随包资源交付。启动器直接使用归档中的 ANOLISA，不嵌套组件 tarball，也不重复 bootstrap 二进制。

## 元数据 v1

`release.json` 必须包含 `schema_version`、`product_version`、`release_id`、`source_commit`、`os`、`arch`、`abi`、`min_installer_version`、`packages` 和 `entrypoints`；拒绝未知字段与不支持的 schema。

每个包引用包含 `package_id`、独立 `version`、相对 `path` 和 `description_sha256`。精确依赖必须在包列表中先出现。`entrypoints` 将公开命令名映射到可执行 payload。release ID 绑定不可变清单和包内容。

明确提供 HTTPS 制品基址时，还生成产品下载索引，包含 `schema_version`、`product_version` 和 `artifacts`，制品字段为 `os`、`arch`、`abi`、`url`、`sha256`、`size`。该 JSON 协议与现有 TOML CLI 自更新清单分别维护。摘要证明内容完整性，不独立证明发布者身份。

## 安装与恢复

归档中的 CLI 先核验整套输入，再调用固定的 AW。每个包分别提交 AW 事务，产品记录保留实际提交与恢复事实。同版本续做核对包说明和已安装内容后跳过健康包，拒绝覆盖用户修改或归属不明的文件。

| Scope | 私有程序根目录 | 资源根目录 | 公共命令 |
| --- | --- | --- | --- |
| User | `~/.local/lib/<arch>-linux-gnu/anolisa/releases/<id>` | `$XDG_DATA_HOME/anolisa/resources/releases/<arch>-linux-gnu/<id>` | `~/.local/bin` |
| System tarball | `/usr/local/lib/anolisa/releases/<id>` | `/usr/local/share/anolisa/releases/<id>` | `/usr/local/bin` |

用户模板、操作记录和下载缓存遵循对应 XDG 根目录。系统模板使用
`/usr/local/etc/anolisa`，操作记录使用 `/var/lib/anolisa`，产品缓存使用
`/var/cache/anolisa`。无论软件安装范围是什么，普通运行用户仍使用自己的 XDG
目录；明确配置的系统服务另行选择服务身份和路径。

AW 数据库保持 `$XDG_STATE_HOME/aw/packages` 和 `/var/lib/aw/packages`。安装器传入已解析的绝对状态根，不使用旧 `FsLayout::system(prefix)` 构造版本目录。AW 直接登记最终真实 prefix，不传 `current`，也不在登记后移动目录。提交 payload 前探测必需的文件系统操作。

未知的已有命令入口会保留，并在安装 payload 前报告冲突。完成包与配置工作后才发布外部启动器和当前版本引用。启动器固定一次解析出的真实版本；用户启动器还按宿主架构选择版本。包提交、产品记录和激活分别持久化，报告可能包含已提交包和未激活的产品，不宣称整套原子回滚。

`aw install --resources-dir` 记录不可变的外置资源根目录。逻辑 `share/` 路径映射
到该目录，`bin/`、`libexec/` 保留在程序目录。inventory、doctor、卸载和恢复使用
登记的映射；程序与资源分别在自身文件系统暂存，包提交前探测实际目录支持的操作。
ANOLISA 记录并核对程序目录中指向同版资源根目录的 `share` 兼容链接，保留组件
现有的相对资源查找方式。

禁用的 Provider 模板固定程序、资源路径及内容摘要，描述调用用户的 XDG 目录解析
规则，不保存安装者的数据目录或 socket。将模板渲染为真实 AW transport argv 是
单独的明确操作；AW 不在 argv 中展开 shell 变量。登录会话变化不改变模板字节，
用户改过的模板需要明确恢复。

Provider 模板不会自动启用。安装文件不代表已启动服务、设置 capability 或迁移历史数据。本轮支持新安装和完全相同发行的续做，接管旧产品数据需要明确的迁移操作。

## 兼容与验证

新的 Linux 默认下载选择产品协议；显式旧参数和 CLI 环境覆盖项保留原路线与含义，macOS 继续下载独立 CLI。混用输入会拒绝，产品失败不悄悄切换安装器。独立更新器发现 AW 登记或锁时，会在获取清单或替换前拒绝直接更新 CLI；已有 RPM 委托保留。

组装测试使用完整组合的合成输入。独立 Linux 实测用真实 AW、ANOLISA 和小型组合覆盖跨文件系统用户资源、映射资源的 doctor 与卸载、root 安装后的普通用户命令和查询、续做和篡改拒绝，二者验证范围不同。完整源码构建通过准备好的发行 runner 和显式 `full_build` CI 输入执行。
