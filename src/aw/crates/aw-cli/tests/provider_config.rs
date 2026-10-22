//! Provider shortcuts retain offline publication, reference protection and failure atomicity.
#![cfg(target_os = "linux")]

use aw_package::config_edit::ConfigurationEditor;
use serde_json::{json, Value};
use std::{
    fs,
    os::unix::fs::{symlink, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

const MINIMAL: &str = include_str!("../../aw-config/examples/aw.minimal.yaml");
const SECRET: &str = "provider-shortcut-private-value";

struct Fixture {
    directory: tempfile::TempDir,
    config: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/provider-config-cli");
        fs::create_dir_all(&target).unwrap();
        let directory = tempfile::tempdir_in(target.canonicalize().unwrap()).unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let config = directory.path().join("aw.yaml");
        fs::write(&config, MINIMAL).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
        let fixture = Self { directory, config };
        fixture.definition(&fixture.provider());
        fixture
    }

    fn provider(&self) -> Value {
        json!({
            "protocol":"aw-provider/v1alpha1",
            "transport":{"type":"stdio","location":"agent","argv":["/bin/sh","-c",
                format!("touch '{}'", self.directory.path().join("executed").display())]},
            "timeout_ms":1000,"max_output_bytes":4096,"config":{"token":SECRET}
        })
    }

    fn definition(&self, value: &Value) {
        fs::write(
            self.directory.path().join("provider.yaml"),
            serde_yaml_ng::to_string(value).unwrap(),
        )
        .unwrap();
    }

    fn run(&self, arguments: &[&str]) -> aw_exec::Output {
        aw_exec::run(
            &aw_exec::CommandSpec {
                program: env!("CARGO_BIN_EXE_aw").into(),
                args: std::iter::once("provider")
                    .chain(arguments.iter().copied())
                    .map(Into::into)
                    .collect(),
                cwd: self.directory.path().into(),
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

    fn edit(&self, action: &str, name: &str) -> String {
        let mut arguments = vec![action, name, "--config", self.config.to_str().unwrap()];
        if action == "add" {
            arguments.extend(["--definition", "provider.yaml"]);
        }
        self.success(&arguments)
    }

    fn success(&self, arguments: &[&str]) -> String {
        let output = self.run(arguments);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(!text.contains(SECRET));
        assert!(text.contains("Offline only"));
        assert!(text.contains("no service reload or runtime activation"));
        text
    }

    fn failure(&self, arguments: &[&str]) -> String {
        let original = fs::read(&self.config).unwrap();
        let inode = fs::metadata(&self.config).unwrap().ino();
        let output = self.run(arguments);
        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        assert!(output.stdout.is_empty());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(!error.contains(SECRET));
        assert_eq!(fs::read(&self.config).unwrap(), original);
        assert_eq!(fs::metadata(&self.config).unwrap().ino(), inode);
        error
    }

    fn document(&self) -> Value {
        serde_yaml_ng::from_slice(&fs::read(&self.config).unwrap()).unwrap()
    }
}

#[test]
fn add_and_remove_are_offline_idempotent_and_preserve_unrelated_configuration() {
    let fixture = Fixture::new();
    let original = fixture.document();
    assert!(fixture
        .edit("add", "policy")
        .starts_with("Updated configuration\n"));
    let document = fixture.document();
    assert_eq!(document["spec"]["providers"]["policy"], fixture.provider());
    for field in ["daemon", "execution", "audit", "agents", "events"] {
        assert_eq!(document["spec"][field], original["spec"][field]);
    }
    for action in ["add", "remove", "remove"] {
        let before = fs::read(&fixture.config).unwrap();
        let inode = fs::metadata(&fixture.config).unwrap().ino();
        let was_present = fixture.document()["spec"]["providers"]
            .get("policy")
            .is_some();
        let output = fixture.edit(action, "policy");
        if action == "add" || !was_present {
            assert!(output.starts_with("Configuration unchanged\n"));
            assert_eq!(fs::read(&fixture.config).unwrap(), before);
            assert_eq!(fs::metadata(&fixture.config).unwrap().ino(), inode);
        }
        if action == "remove" {
            assert!(output.contains("no software is uninstalled"));
        }
    }
    assert_eq!(fixture.document(), original);
    assert_eq!(fs::metadata(&fixture.config).unwrap().mode() & 0o777, 0o600);
    assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 2);
}

#[test]
fn explicit_name_supports_hyphen_prefixed_provider_names() {
    let fixture = Fixture::new();
    for name in ["--provider", "--help"] {
        fixture.success(&[
            "add",
            "--name",
            name,
            "--definition",
            "provider.yaml",
            "--config",
            fixture.config.to_str().unwrap(),
        ]);
        assert_eq!(
            fixture.document()["spec"]["providers"][name],
            fixture.provider()
        );
        fixture.success(&[
            "remove",
            "--config",
            fixture.config.to_str().unwrap(),
            "--name",
            name,
        ]);
        assert!(fixture.document()["spec"]["providers"].get(name).is_none());
    }
    assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 2);
}

#[test]
fn remove_preserves_enabled_and_disabled_referencing_hooks() {
    for (event_enabled, step_enabled) in [(true, true), (true, false), (false, true)] {
        let fixture = Fixture::new();
        let mut editor = ConfigurationEditor::open(&fixture.config).unwrap();
        editor.add_provider("policy", fixture.provider()).unwrap();
        editor
            .add_event(
                "session.start",
                json!({
                    "enabled":event_enabled,"steps":[{
                        "id":"inspect","enabled":step_enabled,"provider":"policy",
                        "operation":"check","effects":["observe"],"on_error":"report"
                    }]
                }),
            )
            .unwrap();
        editor.save().unwrap();
        let error = fixture.failure(&[
            "remove",
            "policy",
            "--config",
            fixture.config.to_str().unwrap(),
        ]);
        assert!(error.contains("unknown Provider reference"));
        assert_eq!(
            fixture.document()["spec"]["events"]["session.start"]["steps"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}

#[test]
fn conflicting_or_invalid_provider_definitions_preserve_original_file() {
    let fixture = Fixture::new();
    fixture.edit("add", "policy");
    let mut conflict = fixture.provider();
    conflict["timeout_ms"] = json!(99);
    fixture.definition(&conflict);
    let error = fixture.failure(&[
        "add",
        "policy",
        "--config",
        fixture.config.to_str().unwrap(),
        "--definition",
        "provider.yaml",
    ]);
    assert!(error.contains("conflicting Provider name"));
    conflict["transport"]["type"] = json!(SECRET);
    fixture.definition(&conflict);
    fixture.failure(&[
        "add",
        "other",
        "--config",
        fixture.config.to_str().unwrap(),
        "--definition",
        "provider.yaml",
    ]);
    assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 2);
}

#[test]
fn definition_reads_are_bounded_regular_and_strict_without_executing_content() {
    let fixture = Fixture::new();
    for bytes in [
        b"config: {}\nconfig: {}\n".to_vec(),
        format!("[\"{SECRET}\"]").into_bytes(),
        vec![b' '; aw_config::MAX_DOCUMENT_BYTES + 1],
    ] {
        fs::write(fixture.directory.path().join("invalid.yaml"), bytes).unwrap();
        fixture.failure(&[
            "add",
            "policy",
            "--config",
            fixture.config.to_str().unwrap(),
            "--definition",
            "invalid.yaml",
        ]);
    }
    let fifo = fixture.directory.path().join("fifo");
    let path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    // SAFETY: path is a live NUL-terminated pathname inside this owned fixture.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    for path in ["fifo", ".", "missing.yaml"] {
        fixture.failure(&[
            "add",
            "policy",
            "--config",
            fixture.config.to_str().unwrap(),
            "--definition",
            path,
        ]);
    }
    let mut native = fixture.provider();
    native["protocol"] = json!("native-hook/v1alpha1");
    native["config"] = json!({});
    fs::write(
        fixture.directory.path().join("provider.json"),
        native.to_string(),
    )
    .unwrap();
    fixture.success(&[
        "add",
        "native",
        "--config",
        fixture.config.to_str().unwrap(),
        "--definition",
        "provider.json",
    ]);
    assert_eq!(fixture.document()["spec"]["providers"]["native"], native);
    assert!(!fixture.directory.path().join("executed").exists());
}

#[test]
fn malformed_options_are_rejected_before_configuration_access() {
    let fixture = Fixture::new();
    let absent = fixture.directory.path().join("missing.yaml");
    let absent = absent.to_str().unwrap();
    for arguments in [
        vec!["add", "policy", "--config", absent],
        vec!["add", "--config", absent, "--definition", "provider.yaml"],
        vec![
            "add",
            "policy",
            "--name",
            "other",
            "--config",
            absent,
            "--definition",
            "provider.yaml",
        ],
        vec![
            "remove", "--name", "policy", "--name", "other", "--config", absent,
        ],
        vec!["remove", "policy", "extra", "--config", absent],
        vec!["remove", "policy", "--config", absent, "--config", absent],
        vec![
            "remove",
            "policy",
            "--config",
            absent,
            "--definition",
            "provider.yaml",
        ],
        vec![
            "add",
            "policy",
            "--config",
            absent,
            "--definition",
            "provider.yaml",
            "--definition",
            "provider.yaml",
        ],
        vec!["remove", "policy", "--config"],
        vec!["add", "policy", "--config", absent, "--definition"],
        vec!["remove", "policy", "--config", absent, "--json"],
        vec!["remove", "--name", "", "--config", absent],
        vec!["add", "--help", "extra"],
    ] {
        let error = fixture.failure(&arguments);
        assert!(
            !error.contains("No such file"),
            "opened config before checking options: {error}"
        );
    }
    assert!(!fixture.directory.path().join("missing.yaml").exists());
    assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 2);
}

#[test]
fn configuration_paths_retain_editor_safety_and_help_explains_offline_changes() {
    let fixture = Fixture::new();
    fixture.failure(&[
        "add",
        "policy",
        "--config",
        "aw.yaml",
        "--definition",
        "provider.yaml",
    ]);
    let linked = fixture.directory.path().join("linked.yaml");
    symlink(&fixture.config, &linked).unwrap();
    fixture.failure(&[
        "add",
        "policy",
        "--config",
        linked.to_str().unwrap(),
        "--definition",
        "provider.yaml",
    ]);
    for action in ["add", "remove"] {
        let output = fixture.run(&[action, "--help"]);
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        for expected in [
            "offline configuration",
            "do not install, enable or reload",
            "not software uninstall",
            "disabled Hooks",
            "--name NAME",
        ] {
            assert!(text.contains(expected), "missing {expected}");
        }
    }
}
