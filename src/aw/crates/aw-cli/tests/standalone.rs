//! Unified CLI generates configurations without a Provider package.

use aw_package::{Component, Manifest, Payload};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap, fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command,
};

struct Fixture {
    directory: tempfile::TempDir,
    prefix: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/preview");
        fs::create_dir_all(&target).unwrap();
        let directory = tempfile::Builder::new()
            .prefix("standalone-cli-")
            .tempdir_in(target)
            .unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let root = directory.path().canonicalize().unwrap();
        let bundle = root.join("bundle");
        let mut files = BTreeMap::new();
        let name = "bin/aw";
        let path = bundle.join("payload").join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::copy("/bin/true", &path).unwrap();
        files.insert(
            name.to_string(),
            Payload {
                sha256: aw_package::digest(&path).unwrap(),
                mode: 0o755,
            },
        );
        fs::write(
            bundle.join("manifest.json"),
            serde_json::to_vec(&Manifest {
                format: 1,
                components: vec![Component {
                    format: 1,
                    component: "aw-core".into(),
                    version: "0.1.0-preview.1".into(),
                    source_commit: "a".repeat(40),
                    os: "linux".into(),
                    arch: std::env::consts::ARCH.into(),
                    provider_protocol: "aw-provider/v1alpha1".into(),
                    requires: BTreeMap::new(),
                    files,
                }],
            })
            .unwrap(),
        )
        .unwrap();
        let prefix = root.join("prefix");
        aw_package::install(&bundle, &prefix).unwrap();
        Self { directory, prefix }
    }
    fn command(&self) -> Command {
        let root = self.directory.path().canonicalize().unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_aw"));
        command
            .args(["config", "init"])
            .arg("--prefix")
            .arg(&self.prefix)
            .arg("--config")
            .arg(root.join("aw.yaml"))
            .arg("--state-dir")
            .arg(root.join("state"))
            .args(["--qoder", "/bin/true"]);
        command
    }
    fn config(&self) -> PathBuf {
        self.directory.path().join("aw.yaml")
    }
    fn read(&self) -> Value {
        aw_config::Validator::new()
            .unwrap()
            .parse(&fs::read(self.config()).unwrap())
            .unwrap()
            .as_value()
            .clone()
    }
}

#[test]
fn core_configuration_defaults_to_no_policy_and_refuses_overwrite() {
    let f = Fixture::new();
    assert!(f.command().output().unwrap().status.success());
    assert_eq!(f.read()["spec"]["providers"], json!({}));
    assert_eq!(f.read()["spec"]["events"], json!({}));
    let original = fs::read(f.config()).unwrap();
    assert!(!f.command().output().unwrap().status.success());
    assert_eq!(fs::read(f.config()).unwrap(), original);
    for arguments in [
        vec!["validate"],
        vec!["remove-provider", "--name", "absent"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_aw"))
            .arg("config")
            .args(arguments)
            .arg("--config")
            .arg(f.config().canonicalize().unwrap())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read(f.config()).unwrap(), original);
    }
    assert!(!f.prefix.join("libexec").exists());
}

#[test]
fn command_options_generate_policy_and_preserve_argument_boundaries() {
    let f = Fixture::new();
    let literal = "literal ; $(touch NEVER) $HOME";
    let output = f
        .command()
        .args([
            "--provider",
            "command",
            "--check",
            "/bin/true",
            "--effect",
            "block",
            "--reason-code",
            "parameter_match",
            "--",
            literal,
            "--socket",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = f.read();
    assert_eq!(
        value["spec"]["providers"]["command"]["transport"]["argv"],
        json!(["/bin/true", literal, "--socket"])
    );
    assert_eq!(
        value["spec"]["events"]["tool.before"]["steps"][0]["effects"],
        json!(["block"])
    );
}

#[test]
fn legacy_socket_and_mismatched_provider_options_never_silently_drop_security() {
    for args in [
        vec!["--socket", "/run/sec.sock"],
        vec!["--provider", "sec-core", "--socket", "/run/sec.sock"],
        vec!["--provider", "none", "--check", "/bin/true"],
        vec![
            "--provider",
            "command",
            "--check",
            "/bin/true",
            "--effect",
            "ask",
            "--reason-code",
            "check",
        ],
        vec!["--", "unexpected"],
    ] {
        let f = Fixture::new();
        assert!(!f.command().args(args).output().unwrap().status.success());
        assert!(!f.config().exists());
    }
}

#[test]
fn explicit_template_preserves_literal_variables_and_rejects_invalid_inputs() {
    let f = Fixture::new();
    let template = f.directory.path().join("provider-template.json");
    let definition = json!({"format":1,"providers":{"custom":{"transport":{"type":"stdio","location":"agent","argv":["/bin/true","${ARG}","${PREFIX}"]},"timeout_ms":1000,"max_output_bytes":32}},"events":{"tool.before":{"enabled":true,"required":true,"steps":[{"id":"custom","provider":"custom","effects":["observe"],"on_error":"report"}]}}});
    fs::write(&template, serde_json::to_vec(&definition).unwrap()).unwrap();
    let literal = "literal ; $(touch NEVER) ${STATE} = tail";
    let output = f
        .command()
        .arg("--template")
        .arg(&template)
        .arg("--template-var")
        .arg(format!("ARG={literal}"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        f.read()["spec"]["providers"]["custom"]["transport"]["argv"],
        json!(["/bin/true", literal, f.prefix])
    );
    fs::remove_file(f.config()).unwrap();
    for variables in [
        vec![],
        vec!["ARG=one", "ARG=two"],
        vec!["PREFIX=/other"],
        vec!["ARG"],
    ] {
        let mut command = f.command();
        command.arg("--template").arg(&template);
        for variable in variables {
            command.args(["--template-var", variable]);
        }
        assert!(!command.output().unwrap().status.success());
        assert!(!f.config().exists());
    }
    let mut bad = definition;
    bad["packages"] = json!(["other-provider"]);
    bad["file_hashes"] = json!({"HASH":{"package":"other-provider","path":"../outside"}});
    fs::write(&template, serde_json::to_vec(&bad).unwrap()).unwrap();
    assert!(!f
        .command()
        .arg("--template")
        .arg(template)
        .output()
        .unwrap()
        .status
        .success());
    assert!(!f.config().exists());
}

#[test]
fn native_or_unregistered_aw_initializes_without_package_database() {
    for existing in [false, true] {
        let f = Fixture::new();
        let prefix = f.directory.path().join("native-prefix");
        if existing {
            fs::create_dir(&prefix).unwrap();
        }
        let state_home = f.directory.path().join("empty-state-home");
        let output = Command::new(env!("CARGO_BIN_EXE_aw"))
            .args(["config", "init", "--prefix"])
            .arg(&prefix)
            .arg("--config")
            .arg(f.config())
            .arg("--state-dir")
            .arg(f.directory.path().join("state"))
            .args(["--qoder", "/bin/true"])
            .env("XDG_STATE_HOME", &state_home)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(f.read()["spec"]["providers"], json!({}));
        assert!(!prefix.join(".aw-packages").exists());
        assert!(!prefix.join(".aw-install.lock").exists());
        assert!(!state_home.exists());
    }
}
