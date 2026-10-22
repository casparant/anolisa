//! Doctor reports installation integrity without executing payloads or changing state.

use aw_package::{Component, Payload};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{symlink, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Fixture {
    _directory: tempfile::TempDir,
    prefix: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/preview");
        fs::create_dir_all(&target).unwrap();
        let directory = tempfile::tempdir_in(target).unwrap();
        let prefix = directory.path().canonicalize().unwrap().join("installed");
        Self {
            _directory: directory,
            prefix,
        }
    }

    fn component(&self, path: &str) -> Component {
        fs::create_dir_all(self.prefix.join(".aw-packages")).unwrap();
        let mut files = BTreeMap::new();
        for path in ["bin/aw", "bin/aw-package", path] {
            let destination = self.prefix.join(path);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::write(&destination, b"doctor must not execute this payload").unwrap();
            fs::set_permissions(&destination, fs::Permissions::from_mode(0o755)).unwrap();
            files.insert(
                path.into(),
                Payload {
                    sha256: aw_package::digest(&destination).unwrap(),
                    mode: 0o755,
                },
            );
        }
        Component {
            format: 1,
            component: "aw-core".into(),
            version: "0.1.0-preview.1".into(),
            source_commit: "a".repeat(40),
            os: "linux".into(),
            arch: std::env::consts::ARCH.into(),
            provider_protocol: "aw-provider/v1alpha1".into(),
            requires: BTreeMap::new(),
            files,
        }
    }

    fn receipt(&self) -> PathBuf {
        self.prefix.join(".aw-packages/aw-core.json")
    }

    fn save(&self, component: &Component) {
        fs::write(self.receipt(), serde_json::to_vec(component).unwrap()).unwrap();
    }

    fn doctor(&self, json: bool) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_aw"));
        command.args(["doctor", "--prefix"]).arg(&self.prefix);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    }
}

fn report(output: Output, expected_exit: i32) -> Value {
    assert_eq!(output.status.code(), Some(expected_exit));
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["scope"], "installation_integrity");
    assert!(report.get("runtime_healthy").is_none());
    report
}

fn human(output: Output, expected_exit: i32) -> String {
    assert_eq!(output.status.code(), Some(expected_exit));
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    let output = String::from_utf8(output.stdout).unwrap();
    assert!(output.contains("Read-only installation integrity"));
    assert!(output.contains("Runtime health is not checked"));
    output
}

#[test]
fn help_needs_no_prefix_and_rejects_mixed_or_unknown_arguments() {
    human(
        Command::new(env!("CARGO_BIN_EXE_aw"))
            .args(["doctor", "--help"])
            .output()
            .unwrap(),
        0,
    );
    let fixture = Fixture::new();
    let prefix = fixture.prefix.to_str().unwrap();
    for arguments in [
        vec![],
        vec!["--prefix"],
        vec!["--prefix", ""],
        vec!["--prefix", "relative"],
        vec!["--prefix", "--json"],
        vec!["--prefix", prefix, "--prefix", prefix],
        vec!["--prefix", prefix, "--json", "--json"],
        vec!["--prefix", prefix, "--fix"],
        vec!["--prefix", prefix, "--"],
        vec!["--prefix", prefix, "unexpected"],
        vec!["--help", "--prefix", prefix],
        vec!["--help", "--json"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_aw"))
            .arg("doctor")
            .args(&arguments)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
        assert!(String::from_utf8(output.stderr).unwrap().starts_with("aw:"));
        assert!(!fixture.prefix.exists());
    }
}

#[test]
fn absent_and_unmanaged_are_not_reported_as_success() {
    let fixture = Fixture::new();
    for state in ["absent", "unmanaged"] {
        let result = report(fixture.doctor(true), 1);
        assert_eq!(result["state"], state);
        assert_eq!(result["checked_components"], 0);
        assert_eq!(result["unhealthy_components"], 0);
        assert_eq!(result["components"], serde_json::json!([]));
        assert!(human(fixture.doctor(false), 1).contains("nothing checked"));
        if state == "absent" {
            assert!(!fixture.prefix.exists());
            fs::create_dir(&fixture.prefix).unwrap();
        }
    }
    assert_eq!(fs::read_dir(&fixture.prefix).unwrap().count(), 0);
}

#[test]
fn checked_installation_returns_zero_without_writes_or_execution() {
    let fixture = Fixture::new();
    fixture.save(&fixture.component("bin/aw"));
    let bytes = fs::read(fixture.receipt()).unwrap();
    let inode = fs::metadata(fixture.receipt()).unwrap().ino();
    let result = report(fixture.doctor(true), 0);
    assert_eq!(result["state"], "installed");
    assert_eq!(result["checked_components"], 1);
    assert_eq!(result["checked_files"], 2);
    assert_eq!(result["unhealthy_components"], 0);
    assert_eq!(result["components"][0]["status"], "checked");
    assert!(human(fixture.doctor(false), 0).contains("aw-core\t0.1.0-preview.1\tchecked\t2/2"));
    assert_eq!(fs::read(fixture.receipt()).unwrap(), bytes);
    assert_eq!(fs::metadata(fixture.receipt()).unwrap().ino(), inode);
    assert!(!fixture.prefix.join(".aw-install.lock").exists());
}

#[test]
fn incomplete_receipts_fail_even_when_the_installed_files_exist() {
    for empty in [true, false] {
        let fixture = Fixture::new();
        let mut component = fixture.component("bin/aw");
        if empty {
            component.files.clear();
        } else {
            component.files.remove("bin/aw-package").unwrap();
        }
        fixture.save(&component);
        let before = fs::read(fixture.receipt()).unwrap();
        let result = report(fixture.doctor(true), 1);
        assert_eq!(result["state"], "installed");
        assert_eq!(result["checked_components"], 0);
        assert_eq!(result["unhealthy_components"], 1);
        assert_eq!(result["components"][0]["status"], "unhealthy");
        assert!(result["components"][0]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|finding| finding["code"] == "incomplete_receipt"));
        assert!(human(fixture.doctor(false), 1).contains("IncompleteReceipt"));
        assert!(fixture.prefix.join("bin/aw-package").is_file());
        assert_eq!(fs::read(fixture.receipt()).unwrap(), before);
        assert!(!fixture.prefix.join(".aw-install.lock").exists());
    }
}

#[test]
fn damaged_and_missing_files_return_findings_without_repairs() {
    let fixture = Fixture::new();
    fixture.save(&fixture.component("bin/aw"));
    let payload = fixture.prefix.join("bin/aw");
    fs::write(&payload, b"changed payload").unwrap();
    fs::set_permissions(&payload, fs::Permissions::from_mode(0o644)).unwrap();
    let result = report(fixture.doctor(true), 1);
    assert_eq!(result["unhealthy_components"], 1);
    assert_eq!(result["components"][0]["status"], "unhealthy");
    let findings = result["components"][0]["findings"].as_array().unwrap();
    for code in ["mode_mismatch", "checksum_mismatch"] {
        assert!(findings.iter().any(|finding| finding["code"] == code));
    }
    let output = human(fixture.doctor(false), 1);
    assert!(output.contains("ModeMismatch bin/aw"));
    assert!(output.contains("ChecksumMismatch bin/aw"));
    assert_eq!(fs::read(&payload).unwrap(), b"changed payload");
    assert_eq!(fs::metadata(&payload).unwrap().mode() & 0o7777, 0o644);
    fs::remove_file(&payload).unwrap();
    let result = report(fixture.doctor(true), 1);
    assert_eq!(
        result["components"][0]["findings"][0]["code"],
        "missing_file"
    );
    assert!(!payload.exists());
    assert!(!fixture.prefix.join(".aw-install.lock").exists());
}

#[test]
fn unsafe_payloads_and_malformed_receipts_fail_without_following_or_rewriting() {
    let fixture = Fixture::new();
    fixture.save(&fixture.component("bin/aw"));
    let payload = fixture.prefix.join("bin/aw");
    let outside = fixture.prefix.parent().unwrap().join("outside");
    fs::write(&outside, b"unchanged external payload").unwrap();
    fs::remove_file(&payload).unwrap();
    symlink(&outside, &payload).unwrap();
    let result = report(fixture.doctor(true), 1);
    assert_eq!(result["checked_files"], 1);
    assert_eq!(
        result["components"][0]["findings"][0]["code"],
        "unsafe_file"
    );
    assert_eq!(fs::read(&outside).unwrap(), b"unchanged external payload");
    fs::write(fixture.receipt(), b"invalid receipt").unwrap();
    let output = fixture.doctor(true);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr).unwrap().starts_with("aw:"));
    assert_eq!(fs::read(fixture.receipt()).unwrap(), b"invalid receipt");
    assert!(!fixture.prefix.join(".aw-install.lock").exists());
}

#[test]
fn human_metadata_is_escaped_but_json_preserves_the_recorded_values() {
    let fixture = Fixture::new();
    let path = "bin/aw\n\u{1b}[2J";
    let mut component = fixture.component(path);
    component.version = "version\t\u{1b}[2J".into();
    component
        .requires
        .insert("missing\nprovider".into(), "version\rrequired".into());
    fixture.save(&component);
    fs::write(fixture.prefix.join(path), b"changed").unwrap();
    let output = human(fixture.doctor(false), 1);
    assert!(!output.contains('\u{1b}'));
    assert!(!output.contains('\r'));
    assert!(!output.contains("bin/aw\n"));
    assert!(!output.contains(&component.version));
    assert!(output.contains("version\\t\\u{1b}[2J"));
    assert!(output.contains("bin/aw\\n\\u{1b}[2J"));
    assert!(output.contains("missing\\nprovider"));
    assert!(output.contains("version\\rrequired"));
    let result = report(fixture.doctor(true), 1);
    assert_eq!(result["components"][0]["version"], component.version);
    let findings = result["components"][0]["findings"].as_array().unwrap();
    assert!(findings.iter().any(|finding| finding["path"] == path));
    assert!(findings
        .iter()
        .any(|finding| finding["related_component"] == "missing\nprovider"));
}
