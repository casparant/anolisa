# AW Preview 封装工具

[English](README.md)

由 distribution 维护的历史 AW Preview 归档构建器。它组合 AW、选定的 Provider
桥接与后端程序，用于独立集成验收。完整 ANOLISA 发行使用另一套
[产品封装入口](../../README_zh.md)。

在 Linux 上从仓库根目录执行：

```bash
cargo run --locked --release --manifest-path distribution/tooling/aw-preview/Cargo.toml \
  --bin anolisa-aw-preview -- --version 0.1.0-preview.1 --component all \
  --output "$PWD/target/aw-preview-packages"
```

`--component core` 是默认值；`sec-core`、`tokenless` 生成所选扩展，`all` 还生成
all-in-one。历史 `aw-build` Cargo binary 在源码树中转调此构建器。

`templates/` 维护 sec-core、Tokenless 及组合的具体配置默认值。Provider 包携带
对应的 `templates/<name>.json`，all-in-one 携带全部三份。AW 通过
`aw config init --template ABS_FILE` 读取显式选择的一份文件，可重复使用
`--template-var NAME=VALUE` 传入字面值；具体后端名称不再内置于 AW 配置生成器。
详见 [AW 包参考](../../../docs/user-guide/zh/aw/packages.md)。
