//! End-to-end checks for the single-purpose audit utility.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn audit_command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cosh-audit"));
    command
        .env("HOME", root)
        .env("COSH_AUDIT_DIR", root)
        .env("COSH_AUDIT_LOG", root.join("legacy.log"))
        .env_remove("COSH_AUDIT_POLICY");
    command
}

fn private_directory() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    directory
}

fn success(output: Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "status={} stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["ok"], true);
    assert_eq!(response["meta"]["subsystem"], "audit");
    response
}

#[test]
fn help_exposes_only_audit_operations() {
    let output = Command::new(env!("CARGO_BIN_EXE_cosh-audit"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    for command in [
        "check", "log", "status", "events", "trace", "export", "prune", "policy",
    ] {
        assert!(text.contains(command), "missing {command}: {text}");
    }
    for retired in ["pkg", "svc", "checkpoint"] {
        assert!(!text.contains(retired), "unexpected {retired}: {text}");
    }
}

#[test]
fn status_trace_and_export_share_the_canonical_store() {
    let directory = private_directory();
    let root = directory.path().canonicalize().unwrap();

    let checked = success(
        audit_command(&root)
            .args(["check", "--action-string", "echo hello"])
            .output()
            .unwrap(),
    );
    assert_eq!(checked["data"]["outcome"], "Allow");

    let events = success(
        audit_command(&root)
            .args(["events", "--event", "policy.decision", "--limit", "10"])
            .output()
            .unwrap(),
    );
    let event_id = events["data"]["events"][0]["event"]["event_id"]
        .as_str()
        .unwrap();

    let trace = success(
        audit_command(&root)
            .args(["trace", event_id])
            .output()
            .unwrap(),
    );
    assert_eq!(trace["data"]["events"].as_array().unwrap().len(), 1);

    let status = success(audit_command(&root).arg("status").output().unwrap());
    assert_eq!(status["data"]["root_label"], "audit/v1");

    let destination: PathBuf = root.join("incident bundle");
    let exported = success(
        audit_command(&root)
            .arg("export")
            .arg("--output")
            .arg(&destination)
            .output()
            .unwrap(),
    );
    assert_eq!(exported["data"]["output"], "incident bundle");
    let mut names = std::fs::read_dir(destination)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        [
            "SHA256SUMS",
            "events.jsonl",
            "manifest.json",
            "summary.json"
        ]
    );
}
