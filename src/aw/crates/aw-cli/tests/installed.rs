//! Installed-package listing summarizes receipts without verifying or executing payloads.
#![cfg(target_os = "linux")]

use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

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

fn record(prefix: &Path, name: &str) -> (PathBuf, Value) {
    let directory = prefix.join(".aw-packages");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!("{name}.json"));
    let record = json!({
        "format": 1,
        "component": name,
        "version": "0.1.0-preview.1",
        "source_commit": "a".repeat(40),
        "os": "linux",
        "arch": std::env::consts::ARCH,
        "provider_protocol": "aw-provider/v1alpha1",
        "requires": {},
        "files": {"bin/not-executed-or-verified": {"sha256": "a".repeat(64), "mode": 493}},
    });
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    (path, record)
}

fn report(prefix: &Path, cwd: &Path) -> Value {
    let output = invoke(
        &[
            "list",
            "--installed",
            "--prefix",
            prefix.to_str().unwrap(),
            "--json",
        ],
        cwd,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["scope"], "installation_records");
    assert_eq!(report["payload_verified"], false);
    assert_eq!(report["runtime_checked"], false);
    assert!(report["notice"].as_str().unwrap().contains("not checked"));
    report
}

#[test]
fn absent_and_unmanaged_prefixes_are_reported_without_creating_state() {
    let directory = tempfile::tempdir().unwrap();
    let prefix = directory.path().join("prefix");
    let absent = report(&prefix, directory.path());
    assert_eq!(absent["state"], "absent");
    assert_eq!(absent["components"], json!([]));
    assert!(!prefix.exists());
    fs::create_dir(&prefix).unwrap();
    fs::write(prefix.join("unowned-file"), b"unchanged").unwrap();
    let unmanaged = report(&prefix, directory.path());
    assert_eq!(unmanaged["state"], "unmanaged");
    assert_eq!(unmanaged["components"], json!([]));
    assert_eq!(fs::read_dir(&prefix).unwrap().count(), 1);
    assert_eq!(fs::read(prefix.join("unowned-file")).unwrap(), b"unchanged");
}

#[test]
fn receipts_are_summarized_without_payloads_or_health_claims() {
    let directory = tempfile::tempdir().unwrap();
    let prefix = directory.path().join("prefix");
    let (core_path, core) = record(&prefix, "aw-core");
    let (provider_path, mut provider) = record(&prefix, "aw-provider-tokenless");
    provider["requires"] = json!({"aw-core": "0.1.0-preview.1"});
    provider["files"] = Value::Object(
        (0..1000)
            .map(|index| {
                (
                    format!("libexec/private-payload-{index}"),
                    json!({"sha256": "b".repeat(64), "mode": 493}),
                )
            })
            .collect(),
    );
    fs::write(&provider_path, serde_json::to_vec(&provider).unwrap()).unwrap();
    let before = fs::read(&provider_path).unwrap();
    let installed = report(&prefix, directory.path());
    assert_eq!(installed["state"], "installed");
    assert_eq!(installed["prefix"], prefix.to_str().unwrap());
    assert_eq!(installed["components"].as_array().unwrap().len(), 2);
    assert_eq!(
        installed["components"][0],
        json!({
            "name": "aw-core", "version": "0.1.0-preview.1", "os": "linux",
            "arch": std::env::consts::ARCH, "source_commit": "a".repeat(40),
            "file_count": 1, "requires": {},
        })
    );
    assert_eq!(installed["components"][1]["file_count"], 1000);
    assert_eq!(installed["components"][1]["requires"], provider["requires"]);
    assert!(!installed.to_string().contains("private-payload"));
    assert!(installed.to_string().len() < 4096);
    assert!(!prefix.join("bin").exists());
    assert!(!prefix.join(".aw-install.lock").exists());
    assert_eq!(fs::read(&provider_path).unwrap(), before);
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(core_path).unwrap()).unwrap(),
        core
    );
    assert_eq!(fs::read_dir(&prefix).unwrap().count(), 1);
}

#[test]
fn text_output_escapes_metadata_control_characters() {
    let directory = tempfile::tempdir().unwrap();
    let prefix = directory.path().join("prefix\nspoofed");
    let (receipt, mut document) = record(&prefix, "aw-core");
    document["version"] = json!("1.0\n\u{1b}[31m\r");
    document["source_commit"] = json!("source\t\u{202e}");
    document["requires"] = json!({"dependency\n": "version\t"});
    fs::write(&receipt, serde_json::to_vec(&document).unwrap()).unwrap();
    let output = invoke(
        &["list", "--installed", "--prefix", prefix.to_str().unwrap()],
        directory.path(),
    );
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("payload integrity and runtime activation are not checked"));
    assert!(text.contains("State: installed (recorded)"));
    assert!(text.contains("prefix\\nspoofed"));
    assert!(text.contains("1.0\\n\\u{1b}[31m\\r"));
    assert!(text.contains("source\\t\\u{202e}"));
    assert!(text.contains("dependency\\n=version\\t"));
    assert!(!text.contains('\u{1b}'));
    assert!(!text.contains('\u{202e}'));
    assert_eq!(
        report(&prefix, directory.path())["components"][0]["version"],
        document["version"]
    );
}

#[test]
fn malformed_receipts_fail_instead_of_reporting_an_empty_installation() {
    let directory = tempfile::tempdir().unwrap();
    let prefix = directory.path().join("prefix");
    let (path, _) = record(&prefix, "aw-core");
    fs::write(&path, b"not-json").unwrap();
    let output = invoke(
        &[
            "list",
            "--installed",
            "--prefix",
            prefix.to_str().unwrap(),
            "--json",
        ],
        directory.path(),
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
    assert_eq!(fs::read(path).unwrap(), b"not-json");
    assert!(!prefix.join(".aw-install.lock").exists());
}

#[test]
fn invalid_receipt_errors_escape_untrusted_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let prefix = directory.path().join("prefix");
    let (path, mut document) = record(&prefix, "aw-core");
    document["files"] = json!({
        "invalid\u{1b}[31m\npath": {"sha256": "a".repeat(64), "mode": 493}
    });
    fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    let output = invoke(
        &["list", "--installed", "--prefix", prefix.to_str().unwrap()],
        directory.path(),
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("invalid\\u{1b}[31m\\npath"));
    assert!(!error.contains('\u{1b}'));
}

#[test]
fn options_are_explicit_and_reject_unknown_duplicate_or_relative_input() {
    let directory = tempfile::tempdir().unwrap();
    for arguments in [
        vec!["list"],
        vec!["list", "--json"],
        vec!["list", "--prefix", "/absent"],
        vec!["list", "--installed"],
        vec!["list", "--installed", "--prefix"],
        vec!["list", "--installed", "--prefix", "--json"],
        vec!["list", "--installed", "--prefix", ""],
        vec!["list", "--installed", "--prefix", "relative"],
        vec!["list", "--installed", "--installed", "--prefix", "/absent"],
        vec![
            "list",
            "--installed",
            "--json",
            "--json",
            "--prefix",
            "/absent",
        ],
        vec![
            "list",
            "--installed",
            "--prefix",
            "/absent",
            "--prefix",
            "/another",
        ],
        vec!["list", "--installed", "--prefix", "/absent", "--unknown"],
        vec!["list", "--installed", "--prefix", "/absent", "position"],
        vec!["list", "--installed", "--help"],
    ] {
        let output = invoke(&arguments, directory.path());
        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
        assert!(!output.stderr.is_empty(), "{arguments:?}");
    }
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn help_requires_no_prefix_and_does_not_accept_extra_options() {
    let directory = tempfile::tempdir().unwrap();
    let help = invoke(&["list", "--help"], directory.path());
    assert!(help.status.success());
    let text = String::from_utf8(help.stdout).unwrap();
    assert!(text.contains("aw list --installed --prefix ABSOLUTE_DIR [--json]"));
    assert!(text.contains("No software is started or repaired"));
    let invalid = invoke(&["list", "--help", "--json"], directory.path());
    assert_eq!(invalid.status.code(), Some(1));
    assert!(invalid.stdout.is_empty());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}
