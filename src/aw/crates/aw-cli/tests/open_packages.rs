//! External local tools and Providers share lifecycle commands without implicit activation.

use serde_json::{json, Value};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

const CONTENT: &[u8] = b"This fixture must never be executed by installation.\n";

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    bundle: PathBuf,
    prefix: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/open-cli");
        fs::create_dir_all(&target).unwrap();
        let directory = tempfile::tempdir_in(target).unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let root = directory.path().canonicalize().unwrap();
        let bundle = root.join("bundle");
        let mut packages = vec![];
        for (id, version, path, provider) in [
            ("example.mant", "7.3", "bin/mant", false),
            (
                "example.mant-provider",
                "0.4",
                "libexec/mant/provider",
                true,
            ),
        ] {
            let file = bundle.join("payload").join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, CONTENT).unwrap();
            let mut package = json!({
                "id": id, "version": version, "kind": if provider { "provider" } else { "tool" },
                "platform": {"os": "linux", "arch": std::env::consts::ARCH},
                "requires": [], "files": {path: {"size": CONTENT.len(), "sha256": aw_package::digest(&file).unwrap(), "mode": 493}},
            });
            if provider {
                package["provider"] = json!({"protocol":"aw-provider/v1alpha1", "executable":path});
                package["requires"] = json!([{"id":"example.mant", "version":"7.3"}]);
            }
            packages.push(package);
        }
        fs::write(
            bundle.join("manifest.json"),
            serde_json::to_vec(&json!({"format":2,"packages":packages})).unwrap(),
        )
        .unwrap();
        Self {
            prefix: root.join("installed"),
            root,
            bundle,
            _directory: directory,
        }
    }

    fn invoke(&self, arguments: &[&str]) -> aw_exec::Output {
        aw_exec::run(
            &aw_exec::CommandSpec {
                program: env!("CARGO_BIN_EXE_aw").into(),
                args: arguments.iter().map(Into::into).collect(),
                cwd: self.root.clone(),
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

    fn install(&self, extra: &[&str]) -> aw_exec::Output {
        let mut args = vec![
            "install",
            "--prefix",
            self.prefix.to_str().unwrap(),
            "--bundle",
            self.bundle.to_str().unwrap(),
        ];
        args.extend_from_slice(extra);
        self.invoke(&args)
    }

    fn operation(&self, command: &str, extra: &[&str]) -> aw_exec::Output {
        let mut args = vec![command, "--prefix", self.prefix.to_str().unwrap()];
        args.extend_from_slice(extra);
        self.invoke(&args)
    }
}

fn succeeded(output: aw_exec::Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    output.stdout
}

fn failed(output: aw_exec::Output) -> aw_exec::Output {
    assert_eq!(output.status.code(), Some(1));
    output
}

fn root_user() -> bool {
    // SAFETY: geteuid reads the process identity without side effects.
    unsafe { libc::geteuid() == 0 }
}

#[test]
fn external_plan_exposes_independent_packages_without_creating_a_prefix() {
    let f = Fixture::new();
    let output = f.install(&["--dry-run", "--json"]);
    assert_eq!(output.status.code(), Some(if root_user() { 1 } else { 0 }));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["components"][0]["name"], "example.mant");
    assert_eq!(report["components"][1]["version"], "0.4");
    assert_eq!(report["source"]["publisher_verified"], false);
    assert_eq!(report["activation"]["registers_providers"], false);
    assert!(!f.prefix.exists());
}

#[test]
fn external_install_requires_explicit_unverified_consent() {
    let f = Fixture::new();
    let output = failed(f.install(&[]));
    if !root_user() {
        assert!(String::from_utf8_lossy(&output.stderr).contains("--allow-unverified"));
    }
    assert!(!f.prefix.exists());
    let output = f.install(&["--allow-unverified"]);
    if root_user() {
        failed(output);
        assert!(!f.prefix.exists());
    } else {
        succeeded(output);
        assert!(f.prefix.join(".aw-installation.json").is_file());
        assert!(!f.prefix.join(".aw-packages").exists());
    }
}

#[test]
fn external_install_list_doctor_and_uninstall_preserve_unknown_data() {
    let f = Fixture::new();
    let output = f.install(&["--allow-unverified"]);
    if root_user() {
        failed(output);
        assert!(!f.prefix.exists());
        return;
    }
    succeeded(output);
    let report: Value =
        serde_json::from_slice(&succeeded(f.operation("list", &["--installed", "--json"])))
            .unwrap();
    assert_eq!(report["receipt_format"], 2);
    assert_eq!(report["backend"], "raw");
    assert_eq!(report["publisher_verified"], false);
    assert_eq!(report["runtime_checked"], false);
    assert_eq!(report["components"][0]["kind"], "tool");
    assert_eq!(report["components"][1]["kind"], "provider");
    let report: Value =
        serde_json::from_slice(&succeeded(f.operation("doctor", &["--json"]))).unwrap();
    assert_eq!(report["checked_components"], 2);
    assert_eq!(report["unhealthy_components"], 0);
    assert!(!f.root.join("aw.yaml").exists());
    assert_eq!(fs::read(f.prefix.join("bin/mant")).unwrap(), CONTENT);
    fs::write(f.prefix.join("notes.txt"), b"user data").unwrap();
    succeeded(f.operation("uninstall", &[]));
    assert!(!f.prefix.join(".aw-installation.json").exists());
    assert!(!f.prefix.join("bin/mant").exists());
    assert_eq!(fs::read(f.prefix.join("notes.txt")).unwrap(), b"user data");
}

#[test]
fn external_reinstall_and_modified_payload_removal_fail_without_overwrite() {
    let f = Fixture::new();
    let output = f.install(&["--allow-unverified"]);
    if root_user() {
        failed(output);
        assert!(!f.prefix.exists());
        return;
    }
    succeeded(output);
    let original = fs::read(f.prefix.join(".aw-installation.json")).unwrap();
    failed(f.install(&["--allow-unverified"]));
    fs::write(f.prefix.join("bin/mant"), b"user changed this file").unwrap();
    let report: Value =
        serde_json::from_slice(&failed(f.operation("doctor", &["--json"])).stdout).unwrap();
    assert_eq!(report["unhealthy_components"], 1);
    failed(f.operation("uninstall", &[]));
    assert_eq!(
        fs::read(f.prefix.join("bin/mant")).unwrap(),
        b"user changed this file"
    );
    assert_eq!(
        fs::read(f.prefix.join(".aw-installation.json")).unwrap(),
        original
    );
    fs::write(f.prefix.join("bin/mant"), CONTENT).unwrap();
    succeeded(f.operation("uninstall", &[]));
}

#[test]
fn malformed_open_bundle_and_mixed_native_options_never_mutate_target() {
    let f = Fixture::new();
    for flags in [
        vec!["--allow-unverified", "--config", "unused.yaml"],
        vec!["--allow-unverified", "--allow-unverified"],
    ] {
        failed(f.install(&flags));
        assert!(!f.prefix.exists());
    }
    let path = f.bundle.join("manifest.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["packages"][0]["post_install"] = json!("must not run");
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    failed(f.install(&["--allow-unverified"]));
    assert!(!f.prefix.exists());
}

#[test]
fn compatibility_entrypoint_cannot_mutate_open_or_mixed_prefixes() {
    let f = Fixture::new();
    fs::create_dir(&f.prefix).unwrap();
    fs::write(
        f.prefix.join(".aw-installation.json"),
        b"not a trusted receipt",
    )
    .unwrap();
    for mixed in [false, true] {
        if mixed {
            fs::create_dir(f.prefix.join(".aw-packages")).unwrap();
        }
        for command in ["install", "uninstall"] {
            let mut arguments = vec![
                command.into(),
                "--prefix".into(),
                f.prefix.as_os_str().into(),
            ];
            if command == "install" {
                arguments.extend(["--bundle".into(), f.bundle.as_os_str().into()]);
            }
            let output = aw_exec::run(
                &aw_exec::CommandSpec {
                    program: env!("CARGO_BIN_EXE_aw-package").into(),
                    args: arguments,
                    cwd: f.root.clone(),
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
            .unwrap();
            assert_eq!(output.status.code(), Some(1));
            assert!(!f.prefix.join(".aw-install.lock").exists());
            assert_eq!(
                fs::read(f.prefix.join(".aw-installation.json")).unwrap(),
                b"not a trusted receipt"
            );
        }
    }
}
