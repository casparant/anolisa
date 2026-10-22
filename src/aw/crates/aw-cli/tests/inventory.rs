//! Configuration inventory is bounded, redacted and independent of installed programs.
#![cfg(target_os = "linux")]

use serde_json::{json, Value};
use std::{
    fs,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::Path,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

const MINIMAL: &str = include_str!("../../aw-config/examples/aw.minimal.yaml");
const SECRET: &str = "inventory-private-value-must-not-appear";

fn invoke(arguments: &[&str], cwd: &Path) -> aw_exec::Output {
    aw_exec::run(
        &aw_exec::CommandSpec {
            program: env!("CARGO_BIN_EXE_aw").into(),
            args: arguments.iter().map(Into::into).collect(),
            cwd: cwd.into(),
            environment: std::env::vars_os().collect(),
        },
        b"",
        aw_exec::Limits {
            input_bytes: 0,
            stdout_bytes: 65536,
            stderr_bytes: 65536,
        },
        Instant::now() + Duration::from_secs(5),
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn config(directory: &Path) -> Value {
    let marker = directory.join("unexpected-execution");
    let argv = json!([
        "/bin/sh",
        "-c",
        format!("touch '{}'; # {SECRET}", marker.display())
    ]);
    let mut document: Value = serde_yaml_ng::from_str(MINIMAL).unwrap();
    document["spec"]["agents"]["qoder"]["argv"] = argv.clone();
    document["spec"]["providers"] = json!({
        "policy": {
            "protocol": "aw-provider/v1alpha1",
            "transport": {"type": "stdio", "location": "agent", "argv": argv},
            "timeout_ms": 1000, "max_output_bytes": 4096,
            "config": {"token": SECRET, "env": {"TOKEN": SECRET}},
            "view": {
                "protocol": "aw-provider-view/v1alpha1", "implementation": "fixture",
                "transport": {"type": "stdio", "location": "agent", "argv": ["/not/executed", SECRET]}
            }
        },
        "unused": {
            "transport": {"type": "stdio", "location": "agent", "argv": ["/not/installed", SECRET]},
            "timeout_ms": 1000, "max_output_bytes": 4096
        }
    });
    document["spec"]["events"] = json!({
        "tool.before": {
            "enabled": true, "steps": [
                {"id": "first", "provider": "policy", "operation": "check", "effects": ["observe"], "on_error": "report"},
                {"id": "disabled", "provider": "policy", "operation": "check", "enabled": false, "effects": ["observe"], "on_error": "report"}
            ]
        },
        "tool.after": {
            "enabled": false, "steps": [
                {"id": "later", "provider": "policy", "operation": "check", "effects": ["observe"], "on_error": "report"}
            ]
        }
    });
    aw_config::Validator::new()
        .unwrap()
        .parse(&serde_json::to_vec(&document).unwrap())
        .unwrap();
    fs::write(
        directory.join("aw.json"),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();
    document
}

fn success(output: aw_exec::Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains(SECRET));
    let report: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(report["scope"], "configuration");
    assert_eq!(report["runtime_checked"], false);
    assert!(report["notice"]
        .as_str()
        .unwrap()
        .contains("runtime activation is not checked"));
    report
}

#[test]
fn provider_inventory_redacts_private_fields_and_reports_disabled_references() {
    let directory = tempfile::tempdir().unwrap();
    config(directory.path());
    let before = fs::read(directory.path().join("aw.json")).unwrap();
    let report = success(invoke(
        &["provider", "list", "--config", "aw.json", "--json"],
        directory.path(),
    ));
    assert_eq!(report["providers"].as_array().unwrap().len(), 2);
    let policy = &report["providers"][0];
    assert_eq!(policy["name"], "policy");
    assert_eq!(policy["protocol"], "aw-provider/v1alpha1");
    assert_eq!(policy["protocol_source"], "explicit");
    assert_eq!(policy["transport"], "stdio");
    assert_eq!(policy["referenced"], true);
    assert_eq!(
        policy["references"],
        json!([
            {"event":"tool.after","step":"later","event_enabled":false,"step_enabled":true},
            {"event":"tool.before","step":"first","event_enabled":true,"step_enabled":true},
            {"event":"tool.before","step":"disabled","event_enabled":true,"step_enabled":false}
        ])
    );
    assert_eq!(policy.as_object().unwrap().len(), 6);
    assert_eq!(
        report["providers"][1],
        json!({
            "name":"unused","protocol":"aw-hook/v1alpha1","protocol_source":"default",
            "transport":"stdio","referenced":false,"references":[]
        })
    );
    assert_eq!(fs::read(directory.path().join("aw.json")).unwrap(), before);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn provider_show_is_a_redacted_selection_not_a_configuration_dump() {
    let directory = tempfile::tempdir().unwrap();
    config(directory.path());
    let report = success(invoke(
        &[
            "provider", "show", "policy", "--json", "--config", "aw.json",
        ],
        directory.path(),
    ));
    assert_eq!(report["providers"].as_array().unwrap().len(), 1);
    assert_eq!(report["providers"][0]["name"], "policy");
    for key in [
        "config",
        "argv",
        "env",
        "view",
        "timeout_ms",
        "max_output_bytes",
    ] {
        assert!(report["providers"][0].get(key).is_none());
    }
    let absent = invoke(
        &["provider", "show", "missing", "--config", "aw.json"],
        directory.path(),
    );
    assert_eq!(absent.status.code(), Some(1));
    assert!(absent.stdout.is_empty());
    assert!(String::from_utf8(absent.stderr)
        .unwrap()
        .contains("not declared"));
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn agents_list_declared_targets_without_executing_or_exposing_arguments() {
    let directory = tempfile::tempdir().unwrap();
    config(directory.path());
    fs::set_permissions(
        directory.path().join("aw.json"),
        fs::Permissions::from_mode(0o444),
    )
    .unwrap();
    let report = success(invoke(
        &["agent", "list", "--config", "aw.json", "--json"],
        directory.path(),
    ));
    assert_eq!(
        report["agents"],
        json!([{"name":"qoder","adapter":"qoder"}])
    );
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn native_hooks_are_summarized_without_admission_or_execution() {
    let directory = tempfile::tempdir().unwrap();
    let mut document = config(directory.path());
    document["spec"]["providers"]["native"] = json!({
        "protocol":"native-hook/v1alpha1", "config":{},
        "transport":{"type":"stdio","location":"agent","argv":["/not/installed",SECRET]},
        "timeout_ms":1000,"max_output_bytes":4096
    });
    document["spec"]["events"]["tool.before"]["steps"] = json!([
        {"id":"native-step","provider":"native","native":{},"on_error":"block"}
    ]);
    fs::write(directory.path().join("aw.json"), document.to_string()).unwrap();
    let report = success(invoke(
        &[
            "provider", "show", "native", "--config", "aw.json", "--json",
        ],
        directory.path(),
    ));
    assert_eq!(report["providers"][0]["protocol"], "native-hook/v1alpha1");
    assert_eq!(report["providers"][0]["referenced"], true);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn text_output_is_redacted_and_distinguishes_configuration_from_runtime() {
    let directory = tempfile::tempdir().unwrap();
    config(directory.path());
    for arguments in [
        vec!["provider", "list", "--config", "aw.json"],
        vec!["provider", "show", "policy", "--config", "aw.json"],
        vec!["agent", "list", "--config", "aw.json"],
    ] {
        let output = invoke(&arguments, directory.path());
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("runtime activation is not checked"));
        assert!(!text.contains(SECRET));
        assert!(!text.contains("unexpected-execution"));
        if arguments[0] == "provider" {
            assert!(text.contains("event disabled"));
            assert!(text.contains("step disabled"));
        }
    }
    fs::write(directory.path().join("minimal.yaml"), MINIMAL).unwrap();
    let empty = invoke(
        &["provider", "list", "--config", "minimal.yaml"],
        directory.path(),
    );
    assert!(empty.status.success());
    assert!(String::from_utf8(empty.stdout)
        .unwrap()
        .contains("No Providers configured"));
    let empty = success(invoke(
        &["provider", "list", "--config", "minimal.yaml", "--json"],
        directory.path(),
    ));
    assert_eq!(empty["providers"], json!([]));
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[test]
fn inventory_rejects_invalid_duplicate_or_missing_options() {
    let directory = tempfile::tempdir().unwrap();
    for arguments in [
        vec!["provider", "list"],
        vec!["provider", "show", "--config", "missing"],
        vec!["provider", "list", "--config"],
        vec!["provider", "list", "--config", "--json"],
        vec![
            "provider", "list", "--config", "missing", "--config", "other",
        ],
        vec![
            "provider", "list", "--config", "missing", "--json", "--json",
        ],
        vec!["provider", "list", "--config", "missing", "--unsafe"],
        vec!["provider", "list", "unexpected", "--config", "missing"],
        vec!["provider", "install", "--config", "missing"],
        vec!["agent", "show", "qoder", "--config", "missing"],
        vec!["agent", "list", "--supported"],
        vec!["provider", "--help", "unexpected"],
        vec!["agent", "list", "--help", "unexpected"],
    ] {
        let output = invoke(&arguments, directory.path());
        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn inventory_help_needs_no_configuration_or_runtime() {
    let directory = tempfile::tempdir().unwrap();
    for arguments in [
        vec!["provider"],
        vec!["provider", "--help"],
        vec!["provider", "list", "--help"],
        vec!["provider", "show", "--help"],
        vec!["agent", "list", "--help"],
    ] {
        let output = invoke(&arguments, directory.path());
        assert!(output.status.success(), "{arguments:?}");
        assert!(String::from_utf8(output.stdout)
            .unwrap()
            .contains("Read-only configuration"));
    }
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn inventory_rejects_invalid_oversized_and_nonregular_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let mut invalid: Value = serde_yaml_ng::from_str(MINIMAL).unwrap();
    invalid["spec"]["agents"]["qoder"]["adapter"] = json!(SECRET);
    fs::write(directory.path().join("invalid.json"), invalid.to_string()).unwrap();
    fs::write(
        directory.path().join("large.yaml"),
        vec![b' '; aw_config::MAX_DOCUMENT_BYTES + 1],
    )
    .unwrap();
    let fifo =
        std::ffi::CString::new(directory.path().join("fifo").as_os_str().as_bytes()).unwrap();
    // SAFETY: fifo is a NUL-terminated pathname inside this test's owned directory.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    for path in ["invalid.json", "large.yaml", "fifo", "."] {
        let output = invoke(
            &["provider", "list", "--config", path, "--json"],
            directory.path(),
        );
        assert_eq!(output.status.code(), Some(1), "{path}");
        assert!(output.stdout.is_empty());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(!error.contains(SECRET));
        if matches!(path, "fifo" | ".") {
            assert!(error.contains("regular file"));
        }
    }
}

#[test]
fn inventory_uses_full_configuration_validation_before_rendering() {
    let directory = tempfile::tempdir().unwrap();
    let mut document = config(directory.path());
    document["spec"]["events"]["tool.before"]["steps"][0]["provider"] = json!("absent");
    fs::write(
        directory.path().join("unresolved.json"),
        document.to_string(),
    )
    .unwrap();
    fs::write(
        directory.path().join("duplicate.yaml"),
        "kind: AWConfiguration\nkind: AWConfiguration\n",
    )
    .unwrap();
    for path in ["unresolved.json", "duplicate.yaml"] {
        let output = invoke(
            &["agent", "list", "--config", path, "--json"],
            directory.path(),
        );
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(!String::from_utf8(output.stderr).unwrap().contains(SECRET));
    }
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 3);
}
