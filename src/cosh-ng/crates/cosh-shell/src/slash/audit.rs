//! Thin bounded `/audit` facade over the single-purpose audit utility.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use wait_timeout::ChildExt;

use crate::runtime::state::InlineState;

const AUDIT_TOOL_TIMEOUT: Duration = Duration::from_secs(3);
const AUDIT_TOOL_MAX_OUTPUT: usize = 256 * 1024;
const AUDIT_USAGE: &str =
    "usage: /audit status | /audit trace current | /audit export current <dir>";
const AUDIT_EXPORT_USAGE: &str = "usage: /audit export current <dir>";

pub(super) fn render_audit_command<W: Write>(
    arguments: &str,
    state: &InlineState,
    output: &mut W,
) -> std::io::Result<()> {
    let result = resolve_arguments(arguments, state)
        .and_then(|args| audit_program().and_then(|program| run_audit_tool(&program, &args)));
    match result {
        Ok(data) => {
            writeln!(output, "\r\nAudit")?;
            writeln!(output, "{}", safe_render_data(&data))?;
        }
        Err(error) => {
            let safe = crate::evidence::redact_sensitive_text(&error).0;
            writeln!(output, "\r\nAudit unavailable: {safe}")?;
            writeln!(
                output,
                "Audit export is a redacted incident bundle; diagnostics export is a separate Shell diagnostics bundle."
            )?;
        }
    }
    Ok(())
}

fn resolve_arguments(arguments: &str, state: &InlineState) -> Result<Vec<String>, String> {
    let (keyword, rest) = split_keyword(arguments);
    match keyword {
        "" | "status" if rest.is_empty() => Ok(vec!["status".to_string()]),
        "trace" => {
            let (target, rest) = split_keyword(rest);
            if target != "current" || !rest.is_empty() {
                return Err(AUDIT_USAGE.to_string());
            }
            Ok(vec!["trace".to_string(), current_session(state)?])
        }
        "export" => {
            let (target, destination) = split_keyword(rest);
            if target != "current" {
                return Err(AUDIT_USAGE.to_string());
            }
            // The destination is the whole remainder rather than one whitespace
            // token, so a directory containing spaces stays intact. It reaches
            // `cosh-audit` as a single `Command::args` entry and never a shell, so
            // the path needs no quoting from the user.
            let destination = destination.trim_end();
            if destination.is_empty() {
                return Err(AUDIT_EXPORT_USAGE.to_string());
            }
            Ok(vec![
                "export".to_string(),
                "--output".to_string(),
                destination.to_string(),
                "--identity".to_string(),
                current_session(state)?,
            ])
        }
        _ => Err(AUDIT_USAGE.to_string()),
    }
}

/// Splits one leading keyword from the remaining raw argument text.
fn split_keyword(arguments: &str) -> (&str, &str) {
    let trimmed = arguments.trim_start();
    match trimmed.find(char::is_whitespace) {
        Some(boundary) => (&trimmed[..boundary], trimmed[boundary..].trim_start()),
        None => (trimmed, ""),
    }
}

fn current_session(state: &InlineState) -> Result<String, String> {
    state
        .shell_session_id
        .clone()
        .ok_or_else(|| "current Shell session is unavailable".to_string())
}

fn audit_program() -> Result<String, String> {
    let executable =
        std::env::current_exe().map_err(|error| format!("cannot locate cosh-shell: {error}"))?;
    resolve_audit_program(std::env::var_os("COSH_AUDIT_BIN").as_deref(), &executable)
}

fn resolve_audit_program(
    override_path: Option<&OsStr>,
    executable: &Path,
) -> Result<String, String> {
    let path = match override_path {
        Some(value) => {
            let path = PathBuf::from(value);
            if !path.is_absolute() {
                return Err("COSH_AUDIT_BIN must be an absolute path".to_string());
            }
            path
        }
        None => executable
            .parent()
            .ok_or_else(|| "cosh-shell executable has no parent directory".to_string())?
            .join("cosh-audit"),
    };
    if !path.is_file() {
        return Err(format!("cosh-audit is unavailable at {}", path.display()));
    }
    Ok(path.to_string_lossy().into_owned())
}

fn run_audit_tool(program: &str, arguments: &[String]) -> Result<serde_json::Value, String> {
    run_audit_tool_with_timeout(program, arguments, AUDIT_TOOL_TIMEOUT)
}

fn run_audit_tool_with_timeout(
    program: &str,
    arguments: &[String],
    timeout: Duration,
) -> Result<serde_json::Value, String> {
    let deadline = std::time::Instant::now() + timeout;
    let mut command = Command::new(program);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot start cosh-audit: {error}"))?;
    let process_group = child.id();
    let mut stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            terminate_audit_process(&mut child, process_group);
            return Err("cosh-audit stdout is unavailable".to_string());
        }
    };
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .by_ref()
            .take((AUDIT_TOOL_MAX_OUTPUT + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    let status = match child.wait_timeout(timeout) {
        Ok(Some(status)) => status,
        Ok(None) => {
            terminate_audit_process(&mut child, process_group);
            return Err("cosh-audit timed out".to_string());
        }
        Err(error) => {
            terminate_audit_process(&mut child, process_group);
            return Err(format!("wait for cosh-audit failed: {error}"));
        }
    };
    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
    let bytes = match receiver.recv_timeout(remaining) {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(error)) => return Err(format!("read cosh-audit output failed: {error}")),
        Err(_) => {
            terminate_audit_process(&mut child, process_group);
            return Err("cosh-audit output did not close before timeout".to_string());
        }
    };
    if bytes.len() > AUDIT_TOOL_MAX_OUTPUT {
        return Err("cosh-audit output exceeds limit".to_string());
    }
    let envelope: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| "cosh-audit returned malformed JSON".to_string())?;
    if !status.success() || envelope.get("ok").and_then(|value| value.as_bool()) != Some(true) {
        return Err("cosh-audit query failed".to_string());
    }
    envelope
        .get("data")
        .cloned()
        .ok_or_else(|| "cosh-audit response has no data".to_string())
}

#[cfg(unix)]
fn kill_audit_process_group(process_group: u32) {
    use nix::sys::signal::{killpg, Signal};
    use nix::unistd::Pid;

    let _ = killpg(Pid::from_raw(process_group as i32), Signal::SIGKILL);
}

#[cfg(not(unix))]
fn kill_audit_process_group(_process_group: u32) {}

fn terminate_audit_process(child: &mut std::process::Child, process_group: u32) {
    kill_audit_process_group(process_group);
    let _ = child.kill();
    let _ = child.wait();
}

fn safe_render_data(data: &serde_json::Value) -> String {
    let rendered = serde_json::to_string_pretty(data).unwrap_or_else(|_| "{}".to_string());
    crate::evidence::redact_sensitive_text(&rendered).0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> InlineState {
        InlineState {
            shell_session_id: Some("shell-session-1".to_string()),
            ..InlineState::default()
        }
    }

    #[test]
    fn current_trace_uses_stable_shell_session_id() {
        assert_eq!(
            resolve_arguments("trace current", &state()).unwrap(),
            ["trace", "shell-session-1"]
        );
    }

    #[test]
    fn export_destination_keeps_spaces_in_one_argument() {
        assert_eq!(
            resolve_arguments("export current /tmp/my dir", &state()).unwrap(),
            [
                "export",
                "--output",
                "/tmp/my dir",
                "--identity",
                "shell-session-1"
            ]
        );
    }

    #[test]
    fn keywords_tolerate_repeated_whitespace_before_the_destination() {
        assert_eq!(
            resolve_arguments("export \t current \t /tmp/my  dir ", &state()).unwrap()[2],
            "/tmp/my  dir"
        );
        assert_eq!(
            resolve_arguments("trace \t current", &state()).unwrap(),
            ["trace", "shell-session-1"]
        );
        assert_eq!(
            resolve_arguments("  status  ", &state()).unwrap(),
            ["status"]
        );
    }

    #[test]
    fn unsupported_invocations_keep_their_existing_usage_text() {
        let state = state();
        assert_eq!(
            resolve_arguments("", &state).unwrap(),
            ["status"],
            "empty arguments still report status"
        );
        assert_eq!(
            resolve_arguments("export current", &state).unwrap_err(),
            AUDIT_EXPORT_USAGE
        );
        assert_eq!(
            resolve_arguments("export current   ", &state).unwrap_err(),
            AUDIT_EXPORT_USAGE
        );
        for arguments in [
            "export",
            "export previous /tmp/out",
            "trace",
            "trace current extra",
            "status extra",
            "bogus",
        ] {
            assert_eq!(
                resolve_arguments(arguments, &state).unwrap_err(),
                AUDIT_USAGE,
                "{arguments}"
            );
        }
    }

    #[test]
    fn session_scoped_subcommands_require_a_shell_session() {
        let state = InlineState::default();
        assert!(resolve_arguments("trace current", &state).is_err());
        assert!(resolve_arguments("export current /tmp/my dir", &state).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn bounded_cli_accepts_success_and_rejects_malformed_output() {
        let root = std::env::temp_dir().join(format!(
            "cosh-shell-audit-cli-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&root).unwrap();
        let ok = root.join("ok.sh");
        let malformed = root.join("malformed.sh");
        let failed = root.join("failed.sh");
        let oversized = root.join("oversized.sh");
        let slow = root.join("slow.sh");
        let inherited_stdout = root.join("inherited-stdout.sh");
        std::fs::write(
            &ok,
            "#!/bin/sh\nprintf '%s' '{\"ok\":true,\"data\":{\"mode\":\"best_effort\"}}'\n",
        )
        .unwrap();
        std::fs::write(&malformed, "#!/bin/sh\nprintf '%s' 'not-json'\n").unwrap();
        std::fs::write(&failed, "#!/bin/sh\nprintf '%s' '{\"ok\":false}'\nexit 1\n").unwrap();
        std::fs::write(
            &oversized,
            "#!/bin/sh\ndd if=/dev/zero bs=262145 count=1 2>/dev/null\n",
        )
        .unwrap();
        std::fs::write(&slow, "#!/bin/sh\nsleep 1\n").unwrap();
        std::fs::write(
            &inherited_stdout,
            "#!/bin/sh\n(sleep 2) &\nprintf '%s' '{\"ok\":true,\"data\":{}}'\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ok, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(&malformed, std::fs::Permissions::from_mode(0o700)).unwrap();
        for path in [&failed, &oversized, &slow, &inherited_stdout] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert_eq!(
            run_audit_tool(ok.to_str().unwrap(), &[]).unwrap()["mode"],
            "best_effort"
        );
        assert!(run_audit_tool(malformed.to_str().unwrap(), &[]).is_err());
        assert!(run_audit_tool(failed.to_str().unwrap(), &[]).is_err());
        assert!(run_audit_tool(oversized.to_str().unwrap(), &[]).is_err());
        assert!(run_audit_tool_with_timeout(
            slow.to_str().unwrap(),
            &[],
            Duration::from_millis(20)
        )
        .is_err());
        let started = std::time::Instant::now();
        assert!(run_audit_tool_with_timeout(
            inherited_stdout.to_str().unwrap(),
            &[],
            Duration::from_millis(50)
        )
        .is_err());
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(run_audit_tool("/definitely/missing/cosh-audit", &[]).is_err());
        let fake_shell = root.join("cosh-shell");
        assert!(resolve_audit_program(Some(OsStr::new("cosh-audit")), &fake_shell).is_err());
        assert!(resolve_audit_program(None, &fake_shell).is_err());
        assert_eq!(
            resolve_audit_program(Some(ok.as_os_str()), &fake_shell).unwrap(),
            ok.to_string_lossy()
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
