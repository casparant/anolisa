# Supported Platforms and Linux Distributions

[中文版](../../../zh/user-entrypoint/cosh-ng/supported-distros.md)

cosh-ng runs its interactive terminal on Linux and macOS. Linux provides the
complete runtime environment; macOS supports the interactive experience with
the limitations stated below.

| Platform | Interactive shell | Support level |
|---|---|---|
| Linux | Bash or zsh | Full cosh-ng functionality |
| macOS arm64 | Bash or zsh | Limited functionality; Linux-only capabilities are unavailable |

## Linux distributions

Alibaba Cloud Linux 4 has the recommended RPM installation path. Other Linux
distributions can build cosh-ng from source, but the published raw package is
not currently a portable contract across every distribution. See the
[quick start](QUICKSTART.md) for the supported installation paths.

## Before installation

Run `anolisa env` on the target host to inspect the detected platform and
available backends. After installation, verify the public launcher with
`command -v cosh` before starting `cosh` in a workspace. When ANOLISA owns the
installation, `anolisa status cosh-ng` reports the component version.
