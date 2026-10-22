# 安装与首次运行

[English](../../../en/user-entrypoint/aw/installation.md)

AW 需要 Linux。下文首次运行示例还需要已登录的 Qoder CLI 1.1.64 或更新版本，
仅安装软件包不需要 Agent。其他 Agent 见[配置与 Agent 启动](configuration.md)。

## 获取安装包

AW 当前使用 Preview 包，尚未接入 ANOLISA CLI 或 RPM。
从[通过的 AW Integration 构建](https://github.com/agentic-os-org/ANOLISA/actions/workflows/aw-preview.yml)
下载 `aw-preview-linux-x86_64-…` artifact，解压 ZIP 后得到安装包和 `SHA256SUMS`。
CI 包面向 Linux x86_64，要求 glibc 2.39 或更新版本，例如 Ubuntu 24.04。

需要自行编译或使用 Linux aarch64 时，按[源码构建](../../../../developer-guide/zh/aw/building.md)
生成安装包，再继续本页。构建过程需要联网下载依赖。

| 安装包 | 内容 |
| --- | --- |
| `aw-core` | AW 启动与配置工具 |
| `aw-provider-tokenless` | Tokenless 与其所需 RTK，需先安装 core |
| `aw-provider-sec-core` | 安全 Provider 和 sec-core 程序，需先安装 core |
| `aw-all-in-one` | 上述三个组件 |

这些是 format 1 Preview 包。分包必须来自同一次构建；选择 all-in-one 时直接
安装到新目录，无需再安装分包。其 manifest 和安装记录格式保持不变。

## 安装

在解压后的 artifact 目录执行。以下版本应与下载包名一致；首次体验使用 core，
需要全部能力时将 `AW_BUNDLE_KIND` 改为 `aw-all-in-one`。

```bash
sha256sum -c SHA256SUMS
export AW_VERSION=0.1.0-preview.1
export AW_BUNDLE_KIND=aw-core
export AW_BUNDLE="$AW_BUNDLE_KIND-$AW_VERSION-linux-$(uname -m)"
export AW_PREFIX="/opt/aw-preview/$AW_VERSION"
tar -xzf "$AW_BUNDLE.tar.gz"
sudo install -d -m 755 /opt/aw-preview
sudo "./$AW_BUNDLE/aw-package" install --prefix "$AW_PREFIX"
export PATH="$AW_PREFIX/bin:$PATH"
aw --help
```

校验结果应全部为 `OK`，安装完成会打印 `Installed verified AW components`。
校验和只能证明内容与下载的清单一致，不认证发布者身份。
安装器拒绝覆盖已有组件；升级请使用[新安装目录](operations.md)。
后续终端也需要设置 `AW_PREFIX` 和 `PATH`，可以写入自己的 shell 配置。

无需系统后台的个人安装，可把 `AW_PREFIX` 改成自己拥有的绝对路径，并去掉安装命令的
`sudo`。安装目录的父目录不得被其他用户写入。root 后台应使用 root 管理的安装目录。

## 预览本地 Preview 包安装

已有 AW 时，可以使用同一个本地安装器：

```bash
aw install --prefix "$AW_PREFIX" --bundle "/absolute/path/to/unpacked-bundle" --dry-run
aw install --prefix "$AW_PREFIX" --bundle "/absolute/path/to/unpacked-bundle" --dry-run --json
aw install --prefix "$AW_PREFIX" --bundle "/absolute/path/to/unpacked-bundle"
```

前两条命令只读取安装包与目标，不创建安装目录、锁或安装记录。core、Provider 分包
与 all-in-one 使用相同的 Plan 分类：`Source`、`Components`、`Files`、`Privileges`、
`Activation`、`Conflicts`。预览校验文件摘要，并报告已知的依赖或覆盖冲突。
冲突为空不代表已预留资源：锁是否可用、实际写入权限和磁盘容量尚未检查，
真正安装时还会重新检查输入。

`--json` 当前必须搭配 `--dry-run`。发现冲突时输出 Plan 并以 `1` 退出；
输入无效时可能在生成 Plan 前失败。去掉 `--dry-run` 后，无冲突的 Plan 会直接进入
安装，不另行询问确认。AW 使用当前用户的权限，不自动执行 `sudo`。
目标中已经安装过同一组件时，请使用新目录。

这套 format 1 流程接受解压后的 Preview 包，其中的 Provider 包仍要求先安装
同一次构建的 core。尚不支持组件名、下载 URL 或 RPM/DEB 包。
安装不启动服务、不注册 Agent Hook、
不启用 Provider。独立的 `aw-package` 安装器继续用于首次引导安装和兼容。

## 安装开放的本地包

format 2 允许作者提供本地工具或 Provider，不必加入软件目录，也不必跟随 AW 的版本。
当前原型只支持新的用户级 raw 安装目录：使用非 root 用户，父目录须已存在、归自己所有，
且其他用户不能写入。不覆盖、接管或追加已有目录，包括 format 1 安装目录。
不查询软件源，也不下载依赖。

以下 Linux shell 示例生成名为 `example.mant` 的无害本地测试脚本。
它不是 manT 项目的软件或发布物，也不证明真实项目的兼容性。
执行前需已有 `aw`，且位于 `PATH`：

```bash
AW_DEMO_ROOT="$(mktemp -d "$HOME/aw-local-demo.XXXXXX")"
AW_DEMO_BUNDLE="$AW_DEMO_ROOT/bundle"
AW_DEMO_PREFIX="$AW_DEMO_ROOT/installed"
mkdir -p "$AW_DEMO_BUNDLE/payload/bin"
printf '%s\n' '#!/bin/sh' 'echo "format-2 fixture, not the manT project"' \
  > "$AW_DEMO_BUNDLE/payload/bin/mant-demo"
chmod 755 "$AW_DEMO_BUNDLE/payload/bin/mant-demo"
AW_DEMO_SIZE="$(wc -c < "$AW_DEMO_BUNDLE/payload/bin/mant-demo")"
AW_DEMO_SHA="$(sha256sum "$AW_DEMO_BUNDLE/payload/bin/mant-demo" | cut -d ' ' -f 1)"
AW_DEMO_ARCH="$(uname -m)"
cat > "$AW_DEMO_BUNDLE/manifest.json" <<EOF
{
  "format": 2,
  "packages": [{
    "id": "example.mant",
    "version": "0.1.0-demo",
    "kind": "tool",
    "platform": {"os": "linux", "arch": "$AW_DEMO_ARCH"},
    "requires": [],
    "files": {
      "bin/mant-demo": {"size": $AW_DEMO_SIZE, "sha256": "$AW_DEMO_SHA", "mode": 493}
    }
  }]
}
EOF
aw install --bundle "$AW_DEMO_BUNDLE" --prefix "$AW_DEMO_PREFIX" --dry-run
aw install --bundle "$AW_DEMO_BUNDLE" --prefix "$AW_DEMO_PREFIX" --allow-unverified
aw list --installed --prefix "$AW_DEMO_PREFIX" --json
aw doctor --prefix "$AW_DEMO_PREFIX"
```

manifest 与 `payload/` 同级，文件键为相对安装目录的路径。`mode` 使用 JSON 十进制
整数：`493` 表示 `0755`，`420` 表示 `0644`。摘要使用小写 SHA-256，`size` 为精确
字节数。未知字段、链接、特殊文件，以及不在 `bin/`、`libexec/`、`share/` 内的路径
都会被拒绝。限制为 manifest 4 MiB、最多 64 个包、8,192 个文件、单文件 512 MiB、
文件总量 2 GiB。

ID 是 `example.mant` 这类带命名空间的小写 ASCII 标识，不认证发布者身份。
每个包有独立版本；`requires` 使用 `{"id": "example.dependency", "version": "1.0.0"}`
这样的精确依赖条目，所有依赖必须位于同一 bundle，且不能形成环。
不支持版本范围或跨安装目录依赖。真实 Provider 使用 `"kind": "provider"`，并增加
`"provider": {"protocol": "aw-provider/v1alpha1", "executable": "bin/my-provider"}`；
入口须为本包拥有的 `0755` 文件。`tool` 不得携带 `provider` 声明。
声明协议不等于已经验证其实现。

Plan 沿用相同分类。`--dry-run` 无需授权、不写入；增加 `--json` 可获得结构化输出。
真正安装时每次都须提供 `--allow-unverified`：摘要只证明内容一致，不认证发布者，
该选项也不保存永久受信来源。AW 复制文件并写入 `.aw-installation.json`，记录 `user`
范围、安装用户 UID 和 `raw` 后端，不执行包内脚本、不修改 `PATH`、不启动服务、
不注册 Provider，也不连接 Agent Hook。已安装 Provider 仍需
[显式配置](configuration.md#编辑-provider-声明)。

要移除整个演示安装，保留 `PATH` 中原有的 AW 二进制并执行：

```bash
aw uninstall --prefix "$AW_DEMO_PREFIX"
```

卸载先校验记录中的文件，并保留未知文件。它不检查运行中的引用，须先停止使用该
目录的程序。源包 `AW_DEMO_ROOT/bundle` 不会被移除，详见[运行维护](operations.md)。

## 创建配置

以普通用户创建配置，数据保存在安装目录之外。

```bash
export AW_HOME="$HOME/aw"
export AW_QODER="$(command -v qodercli)"
install -d -m 700 "$AW_HOME" "$AW_HOME/workspace"
qodercli --version
aw config init --prefix "$AW_PREFIX" \
  --config "$AW_HOME/aw.yaml" --state-dir "$AW_HOME/state" \
  --qoder "$AW_QODER"
aw validate --config "$AW_HOME/aw.yaml"
```

`AW_QODER` 应为真实可执行文件的绝对路径。`config init` 创建完整配置，不覆盖已有文件。
安全与压缩需另行启用，见 [sec-core](security.md)和 [Tokenless](tokenless.md)。

## 启动并验证

```bash
cd "$AW_HOME/workspace"
aw run --config "$AW_HOME/aw.yaml" --agent qoder
```

应看到 AW 实例信息并进入 Qoder。请它用工具读取当前目录，按原生提示处理权限确认。
确认工具能返回结果后退出 Qoder，在同一终端执行。

```bash
aw status --config "$AW_HOME/aw.yaml"
aw stop --config "$AW_HOME/aw.yaml"
```

启用 Provider 后，按对应页面验证效果。
