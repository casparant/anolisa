# Audit and Incident Export

[中文版](../../../../zh/user-entrypoint/cosh-ng/shell/audit.md)

Use `/audit` inside an Enhanced cosh session to check audit storage, follow the
current session's redacted timeline, or export a bounded incident bundle. This
keeps investigation in the same terminal where the Agent work happened.

## Commands

| Command | Result |
|---|---|
| `/audit` or `/audit status` | Show audit mode, storage, retention, and reader health. |
| `/audit trace current` | Show events correlated with the current Shell session. |
| `/audit export current <dir>` | Export the current session's redacted incident bundle. |

For example:

```text
/audit status
/audit trace current
/audit export current /tmp/cosh-audit-incident
```

`trace` and `export` use the stable identity of the current Shell session; they
do not ask the user to copy an internal run ID. An export contains canonical
events, a summary, a manifest, and checksums. It is separate from the Shell
diagnostics bundle and does not include unredacted secrets.
Use an absolute export path. A relative path is resolved from the directory
where `cosh-shell` started, which may differ from the prompt after `cd`.

## Availability and bounds

`/audit` is a contextual Shell command and is intentionally omitted from the
ordinary `/help` list and slash hints. `/audit status` is available without a
session identity. Trace and export require the current Shell session; status
and trace are read-only, while export writes only the selected incident
directory. The Shell invokes an internal, single-purpose audit utility; that
utility is not a supported public command.

The Shell starts no command interpreter for this operation. It passes
arguments directly, waits for at most three seconds, accepts at most 256 KiB of
structured output, checks the success envelope, and redacts the rendered result
again. A missing utility, timeout, malformed response, or failed query is shown
as `Audit unavailable` and returns control to the prompt.

Audit storage and retention settings are documented in
[Configuration](../configuration.md).
