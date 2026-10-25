//! Human setup remains read-only when piped and publishes only explicit private user configuration.

mod common;

use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Output};

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    prefix: PathBuf,
    resources: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let prefix = root.join("product");
        let resources = root.join("resources");
        fs::create_dir_all(prefix.join("bin")).unwrap();
        fs::create_dir_all(resources.join("anolisa")).unwrap();
        fs::create_dir(root.join("home")).unwrap();
        let aw = prefix.join("bin/aw");
        fs::write(&aw, b"#!/bin/sh\n[ \"$*\" = 'agent list --json' ] || exit 3\nprintf '%s\\n' '{\"format\":1,\"adapters\":[{\"name\":\"example\"}]}'\n").unwrap();
        fs::set_permissions(&aw, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(resources.join("anolisa/installed-product.json"), serde_json::to_vec(&json!({
            "schema_version":1,"product_version":"1.0.0","release_id":"fixture","source_commit":"a".repeat(40),
            "os":"linux","arch":std::env::consts::ARCH,"manifest_sha256":"b".repeat(64),
            "packages":[{"package_id":"aw-core","version":"0.1.0"}]
        })).unwrap()).unwrap();
        Self {
            _temporary: temporary,
            root,
            prefix,
            resources,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        common::run_with_path_env(
            args,
            &[
                ("HOME", self.root.join("home").as_path()),
                ("XDG_CONFIG_HOME", self.root.join("config").as_path()),
                ("XDG_STATE_HOME", self.root.join("state").as_path()),
                ("XDG_DATA_HOME", self.root.join("data").as_path()),
                ("XDG_CACHE_HOME", self.root.join("cache").as_path()),
                ("ANOLISA_RELEASE_PREFIX", &self.prefix),
                ("ANOLISA_RELEASE_RESOURCES", &self.resources),
            ],
        )
    }

    fn assert_not_written(&self) {
        for name in ["config", "state", "data", "cache"] {
            assert!(!self.root.join(name).exists(), "setup wrote {name}");
        }
        assert!(
            fs::read_dir(self.root.join("home"))
                .unwrap()
                .next()
                .is_none()
        );
    }
}

fn envelope(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], true);
    result
}

#[test]
fn piped_setup_returns_one_offline_overview_without_creating_configuration_or_state() {
    let fixture = Fixture::new();
    let output = fixture.run(&["--json", "setup", "--scope", "user"]);
    let response = envelope(&output);
    assert_eq!(response["command"], "setup");
    assert_eq!(response["data"]["overview"]["latest"], "not_checked");
    assert!(response["data"]["configuration"].is_null());
    assert!(response["data"]["service_plan"].is_null());
    assert!(response["data"]["next_actions"].as_array().unwrap().len() >= 3);
    fixture.assert_not_written();
    let output = fixture.run(&["--no-color", "setup", "--scope", "user"]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("needs configuration"));
    assert!(!text.contains("Create a new user AW configuration?"));
    fixture.assert_not_written();
}

#[test]
fn explicit_configuration_uses_the_installed_catalog_and_keeps_literal_arguments_private() {
    let fixture = Fixture::new();
    let config = fixture.root.join("config/anolisa/aw.yaml");
    let program = fixture.prefix.join("bin/aw");
    let args = [
        "--json",
        "setup",
        "--scope",
        "user",
        "--non-interactive",
        "--write-config",
        config.to_str().unwrap(),
        "--agent",
        "example",
        "--agent-bin",
        program.to_str().unwrap(),
        "--agent-arg=--prompt=literal value",
        "--yes",
    ];
    let response = envelope(&fixture.run(&args));
    assert_eq!(
        response["data"]["configuration"]["created"],
        json!([config])
    );
    assert!(
        !String::from_utf8(serde_json::to_vec(&response).unwrap())
            .unwrap()
            .contains("literal value")
    );
    let bytes = fs::read(&config).unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        value["spec"]["agents"]["agent"]["argv"][1],
        "--prompt=literal value"
    );
    assert_eq!(value["spec"]["providers"], json!({}));
    assert_eq!(value["spec"]["events"], json!({}));
    assert_eq!(
        fs::metadata(&config).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let again = fixture.run(&args);
    assert!(!again.status.success());
    assert_eq!(fs::read(&config).unwrap(), bytes);
    assert!(!fixture.root.join("state").exists());
}

#[test]
fn operator_help_describes_offline_inspection_and_explicit_setup_choices() {
    let output = common::run(&["overview", "--help"]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("--candidate-bundle"));
    assert!(text.contains("--check-services"));
    assert!(text.contains("without downloading or upgrading"));
    let output = common::run(&["setup", "--help"]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    for flag in [
        "--non-interactive",
        "--write-config",
        "--agent-bin",
        "--provider-template",
        "--preset",
    ] {
        assert!(text.contains(flag), "{flag}");
    }
}
