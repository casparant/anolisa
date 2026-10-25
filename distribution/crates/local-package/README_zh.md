# 本地包安装库

[English](README.md)

为 ANOLISA 和 AW 扩展兼容提供本地包校验、文件归属及持久安装事务。
这个 Rust 库没有独立 CLI，不选择软件源、产品版本、profile 或服务；调用方明确
选择管理者和数据库。

| 调用方 | 用户数据库 | 系统数据库 |
|--------|------------|------------|
| ANOLISA 产品 raw | `$XDG_STATE_HOME/anolisa/packages` | `/var/lib/anolisa/packages` |
| AW 本地扩展及历史 Preview | `$XDG_STATE_HOME/aw/packages` | `/var/lib/aw/packages` |

用户路径回退到 `$HOME/.local/state`。`DatabasePaths::user_for` 和
`DatabasePaths::system_for` 选择 `PackageManager::Aw` 或
`PackageManager::Anolisa`。`open_bundle` 核验本地输入，`plan` 生成只读安装／卸载
计划，`transaction` 提交或恢复已记录的文件变更。查询和计划不创建数据库文件。

历史 `.aw-packages/aw-core.json` 文件名及运行锁标记继续兼容。新产品指针标记
管理者，未标记的旧指针按 AW 解释。调用方不能沿另一管理者的指针取得写入或卸载
权限。已有 AW 所有的产品安装核验通过后可保留原管理权，不自动转移或重复登记。
RPM 和 dpkg 保留原生文件归属。

`runtime/` 是较小的 `local-package-runtime` 库，负责文件运行读锁与锁发现。
AW 运行时复用它，不读取任一包数据库，也不依赖事务引擎。

库测试在 Linux 执行，文件事务使用 Linux 文件系统操作。限制、锁顺序和恢复规则
见[本地包设计](../../../src/aw/docs/design/zh/packaging/package-management.md)，
产品封装见[distribution 指南](../../README_zh.md)。
