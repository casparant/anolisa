//! Migration inspection reads explicit legacy documents without taking ownership.
#![cfg(target_os = "linux")]

use serde_json::Value;
use std::{
    fs,
    os::unix::fs::{symlink, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

// The fields mirror distribution/anolisa's InstalledState and StateFileV5 wire shapes.
const USER_V4: &str = r#"
schema_version = 4
updated_at = "2026-07-01T00:00:00Z"
install_mode = "user"
prefix = "/home/legacy/.local/share/anolisa"
anolisa_version = "0.2.4"

[[objects]]
kind = "component"
name = "tokenless"
version = "0.1.0"
status = "installed"
install_backend = "raw"
ownership = "raw_managed"
installed_at = "2026-07-01T00:00:00Z"

[[objects.files]]
path = "/home/legacy/.local/bin/tokenless"
owner = "anolisa"
kind = "file"

[[objects.services]]
name = "do-not-start.service"
manager = "systemd"
scope = "user"
"#;

const SYSTEM_V6: &str = r#"
schema_version = 6
updated_at = "2026-07-01T00:00:00Z"
install_mode = "system"
prefix = "/"
anolisa_version = "0.2.17"

[[installations]]
kind = "component"
name = "tokenless"
scope = "system"
status = "installed"
installed_at = "2026-07-01T00:00:00Z"

[installations.binding]
authority = "delegated"
pm = "rpm"
[installations.binding.package]
resolution = "resolved"
name = "aw-provider-tokenless"
[installations.binding.relation]
kind = "managed"
since = "2026-07-01T00:00:00Z"
[installations.binding.last_observed]
version = "0.2.0"
observed_at = "2026-07-01T00:00:00Z"

[[adapter_trust_roots]]
component = "tokenless"
framework = "opencode"
root = "/not-opened/framework-root"
"#;

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

fn write_state(directory: &Path, name: &str, document: &str) -> PathBuf {
    let path = directory.join(name);
    fs::write(&path, document).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    path
}

fn success(output: aw_exec::Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["scope"], "legacy_installation_records");
    assert_eq!(report["mode"], "read_only");
    assert_eq!(report["changes"], serde_json::json!([]));
    assert!(report.get("can_apply").is_none());
    report
}

#[test]
fn user_and_system_documents_remain_separate_read_only_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let user = write_state(directory.path(), "user.toml", USER_V4);
    let system = write_state(directory.path(), "system.toml", SYSTEM_V6);
    let user_before = fs::read(&user).unwrap();
    let system_before = fs::read(&system).unwrap();
    let report = success(invoke(
        &[
            "migrate",
            "inspect",
            "--state",
            user.to_str().unwrap(),
            "--json",
            "--state",
            system.to_str().unwrap(),
        ],
        directory.path(),
    ));
    let sources = report["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 2);
    assert_eq!(sources[0]["schema_version"], 4);
    assert_eq!(
        sources[0]["file_owner_uid"],
        fs::metadata(&user).unwrap().uid()
    );
    assert_eq!(sources[0]["records"][0]["name"], "tokenless");
    assert_eq!(sources[0]["records"][0]["recorded_scope"]["kind"], "user");
    assert!(sources[0]["records"][0]["recorded_scope"]["uid"].is_null());
    assert_eq!(sources[0]["records"][0]["recorded_backend"], "raw");
    assert_eq!(sources[0]["records"][0]["file_count"], 1);
    assert_eq!(sources[0]["records"][0]["service_count"], 1);
    assert_eq!(sources[1]["schema_version"], 6);
    assert_eq!(sources[1]["records"][0]["recorded_scope"]["kind"], "system");
    assert_eq!(sources[1]["records"][0]["recorded_backend"], "rpm");
    assert_eq!(sources[1]["records"][0]["recorded_relation"], "managed");
    assert_eq!(sources[1]["records"][0]["recorded_version"], "0.2.0");
    assert_eq!(sources[1]["adapter_trust_root_count"], 1);
    assert_eq!(report["overlaps"].as_array().unwrap().len(), 1);
    assert!(!report.to_string().contains("do-not-start"));
    assert!(!report.to_string().contains("not-opened/framework-root"));
    assert_eq!(fs::read(user).unwrap(), user_before);
    assert_eq!(fs::read(system).unwrap(), system_before);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[test]
fn text_distinguishes_recorded_status_unknown_uid_and_overlap_candidates() {
    let directory = tempfile::tempdir().unwrap();
    let user = write_state(directory.path(), "user.toml", USER_V4);
    let system = write_state(directory.path(), "system.toml", SYSTEM_V6);
    let output = invoke(
        &[
            "migrate",
            "inspect",
            "--state",
            user.to_str().unwrap(),
            "--state",
            system.to_str().unwrap(),
        ],
        directory.path(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Read-only migration inventory"));
    assert!(text.contains("Sources: 2  Records: 2"));
    assert!(text.contains("user (uid unknown)"));
    assert!(text.contains("STATUS (RECORDED)"));
    assert!(text.contains("Trust roots: 1"));
    assert!(text.contains("Overlap candidates (live precedence and conflicts are not checked)"));
    assert!(text.contains("Changes: none. This inspection is not a migration plan."));
    assert!(!text.contains("do-not-start"));
    assert!(!text.contains("not-opened/framework-root"));
}

#[test]
fn anomalous_and_unsupported_backends_remain_findings_not_apply_status() {
    let directory = tempfile::tempdir().unwrap();
    let user_rpm = SYSTEM_V6
        .replace("install_mode = \"system\"", "install_mode = \"user\"")
        .replace("scope = \"system\"", "scope = { user = { uid = 1000 } }");
    let rpm = write_state(directory.path(), "rpm-user.toml", &user_rpm);
    let deb = write_state(
        directory.path(),
        "deb-system.toml",
        &SYSTEM_V6.replace("pm = \"rpm\"", "pm = \"deb\""),
    );
    let report = success(invoke(
        &[
            "migrate",
            "inspect",
            "--state",
            rpm.to_str().unwrap(),
            "--state",
            deb.to_str().unwrap(),
            "--json",
        ],
        directory.path(),
    ));
    let rpm_record = &report["sources"][0]["records"][0];
    assert_eq!(rpm_record["recorded_backend"], "rpm");
    assert_eq!(
        rpm_record["recorded_scope"],
        serde_json::json!({"kind": "user", "uid": 1000})
    );
    assert!(rpm_record["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding["code"] == "native_backend_in_user_scope"));
    let deb_record = &report["sources"][1]["records"][0];
    assert_eq!(deb_record["recorded_backend"], "deb");
    assert!(deb_record["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding["code"] == "unsupported_backend"));
    assert_eq!(fs::read_to_string(rpm).unwrap(), user_rpm);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[test]
fn terminal_metadata_is_escaped_without_corrupting_json_values() {
    let directory = tempfile::tempdir().unwrap();
    let document = USER_V4.replace(
        "name = \"tokenless\"",
        "name = \"token\\nless\\u001b[31m\\u202e\"",
    );
    let state = write_state(directory.path(), "state\nfile.toml", &document);
    let output = invoke(
        &["migrate", "inspect", "--state", state.to_str().unwrap()],
        directory.path(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("state\\nfile.toml"));
    assert!(text.contains("token\\nless\\u{1b}[31m\\u{202e}"));
    assert!(!text.contains('\u{1b}'));
    assert!(!text.contains('\u{202e}'));
    let report = success(invoke(
        &[
            "migrate",
            "inspect",
            "--state",
            state.to_str().unwrap(),
            "--json",
        ],
        directory.path(),
    ));
    assert_eq!(
        report["sources"][0]["records"][0]["name"],
        "token\nless\u{1b}[31m\u{202e}"
    );
}

#[test]
fn malformed_future_missing_and_linked_documents_fail_without_partial_output() {
    let directory = tempfile::tempdir().unwrap();
    let good = write_state(directory.path(), "good.toml", USER_V4);
    let malformed = write_state(
        directory.path(),
        "malformed.toml",
        "schema_version = [private-field-must-not-leak",
    );
    let future = write_state(
        directory.path(),
        "future.toml",
        &USER_V4.replace("schema_version = 4", "schema_version = 99"),
    );
    let linked = directory.path().join("linked.toml");
    symlink(&good, &linked).unwrap();
    for invalid in [
        malformed,
        future,
        linked,
        directory.path().join("missing.toml"),
    ] {
        let output = invoke(
            &[
                "migrate",
                "inspect",
                "--state",
                good.to_str().unwrap(),
                "--state",
                invalid.to_str().unwrap(),
                "--json",
            ],
            directory.path(),
        );
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(!error.is_empty());
        assert!(!error.contains("private-field-must-not-leak"));
    }
    assert_eq!(fs::read_to_string(good).unwrap(), USER_V4);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 4);
}

#[test]
fn inspection_flags_are_strict_and_do_not_offer_mutating_actions() {
    let directory = tempfile::tempdir().unwrap();
    for arguments in [
        vec!["migrate", "inspect"],
        vec!["migrate", "inspect", "--json"],
        vec!["migrate", "inspect", "--state"],
        vec!["migrate", "inspect", "--state", "--json"],
        vec!["migrate", "inspect", "--state", "relative.toml"],
        vec!["migrate", "inspect", "--state", ""],
        vec![
            "migrate", "inspect", "--state", "/absent", "--state", "/absent",
        ],
        vec![
            "migrate", "inspect", "--state", "/absent", "--json", "--json",
        ],
        vec![
            "migrate", "inspect", "--state", "/absent", "--prefix", "/prefix",
        ],
        vec!["migrate", "inspect", "--state", "/absent", "--apply"],
        vec!["migrate", "inspect", "--state", "/absent", "position"],
        vec!["migrate", "inspect", "--state", "/absent", "--help"],
        vec!["migrate", "apply"],
        vec!["migrate", "plan"],
    ] {
        let output = invoke(&arguments, directory.path());
        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
        assert!(!output.stderr.is_empty());
    }
    let names = (0..9)
        .map(|index| format!("/state-{index}.toml"))
        .collect::<Vec<_>>();
    let mut arguments = vec!["migrate", "inspect"];
    for name in &names {
        arguments.extend(["--state", name.as_str()]);
    }
    let output = invoke(&arguments, directory.path());
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("at most 8"));
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn group_and_action_help_work_without_state_files() {
    let directory = tempfile::tempdir().unwrap();
    for arguments in [
        vec!["migrate"],
        vec!["migrate", "--help"],
        vec!["migrate", "inspect", "--help"],
    ] {
        let output = invoke(&arguments, directory.path());
        assert!(output.status.success(), "{arguments:?}");
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("aw migrate inspect --state ABSOLUTE_FILE"));
        assert!(text.contains("No automatic discovery or elevation"));
        assert!(!text.contains("aw migrate apply"));
    }
    for arguments in [
        vec!["migrate", "--help", "--json"],
        vec!["migrate", "inspect", "--help", "--json"],
    ] {
        let output = invoke(&arguments, directory.path());
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
    }
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}
