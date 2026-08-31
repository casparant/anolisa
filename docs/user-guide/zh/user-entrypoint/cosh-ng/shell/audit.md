# 审计与事故导出

[English](../../../../en/user-entrypoint/cosh-ng/shell/audit.md)

在 Enhanced cosh 会话中使用 `/audit`，可以检查审计存储、追踪当前会话的
脱敏时间线，或导出有界的事故包。排查过程因此可以留在 Agent 执行任务的
同一个终端中。

## 命令

| 命令 | 结果 |
|---|---|
| `/audit` 或 `/audit status` | 显示审计模式、存储、保留策略和 reader 健康状态。 |
| `/audit trace current` | 显示与当前 Shell 会话关联的事件。 |
| `/audit export current <dir>` | 导出当前会话的脱敏事故包。 |

例如：

```text
/audit status
/audit trace current
/audit export current /tmp/cosh-audit-incident
```

`trace` 和 `export` 使用当前 Shell 会话的稳定身份，不需要用户复制内部
run ID。导出目录包含规范事件、摘要、manifest 和 checksum。它与 Shell
diagnostics bundle 相互独立，不包含未脱敏的 secret。
请使用绝对导出路径。相对路径会从 `cosh-shell` 的启动目录解析；执行 `cd`
之后，该目录可能与当前 prompt 所在目录不同。

## 可用性与边界

`/audit` 是 contextual Shell 命令，不会出现在普通 `/help` 列表或 slash hint 中。
`/audit status` 不需要 session identity；trace 和 export 则需要当前 Shell
session。Status 和 trace 只读；export 只写入选定的事故目录。Shell 会调用一个
单一职责的内部审计工具；该工具不是受支持的公开命令。

Shell 不会为此启动命令解释器，而是直接传递参数；最多等待 3 秒，最多接收
256 KiB 结构化输出，校验 success envelope，并对呈现结果再次脱敏。内部工具
缺失、超时、响应格式错误或查询失败时，终端会显示 `Audit unavailable`，然后恢复
prompt。

审计存储和保留策略设置见[配置](../configuration.md)。
