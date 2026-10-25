//! Removal preserves active/unknown controls and uses the same package runtime leases.

use super::*;
use crate::release::{Environment, PackageResult};
use serde_json::json;
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};

struct Fixture {
    _temporary: tempfile::TempDir,
    bundle: Bundle,
    layout: ReleaseLayout,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let source = root.join("bundle");
        let payload = source.join("packages/anolisa-cli/payload/bin/anolisa");
        files::tree(payload.parent().unwrap(), 0o755).unwrap();
        fs::write(&payload, b"fixture CLI").unwrap();
        fs::set_permissions(&payload, fs::Permissions::from_mode(0o755)).unwrap();
        let description = serde_json::to_vec(&json!({
            "schema_version":1,"package_id":"anolisa-cli","version":"1.0.0",
            "os":"linux","arch":std::env::consts::ARCH,"dependencies":[],
            "files":{"bin/anolisa":{"sha256":files::digest(&payload).unwrap(),"mode":493}}
        }))
        .unwrap();
        fs::write(
            source.join("packages/anolisa-cli/package.json"),
            &description,
        )
        .unwrap();
        fs::write(source.join("release.json"), serde_json::to_vec(&json!({
            "schema_version":1,"product_version":"1.0.0","release_id":"old-release",
            "source_commit":"a".repeat(40),"os":"linux","arch":std::env::consts::ARCH,
            "abi":"gnu","min_installer_version":"0.1.0",
            "packages":[{"package_id":"anolisa-cli","version":"1.0.0","path":"packages/anolisa-cli",
                "description_sha256":files::digest_bytes(&description)}],
            "entrypoints":{"anolisa":"bin/anolisa"}
        })).unwrap()).unwrap();
        let bundle = Bundle::read(&source).unwrap();
        let home = root.join("home");
        files::tree(&home, 0o700).unwrap();
        let environment = Environment::from_values(home, |_| None).unwrap();
        let layout = ReleaseLayout::resolve(&bundle.manifest, Scope::User, &environment).unwrap();
        files::tree(&layout.state, 0o700).unwrap();
        files::tree(&layout.prefix, 0o755).unwrap();
        files::tree(&layout.resources, 0o755).unwrap();
        let fixture = Self {
            _temporary: temporary,
            bundle,
            layout,
        };
        fixture.record(Some("fixture-installation".into()));
        fixture
    }

    fn record(&self, installation_id: Option<String>) {
        let report = InstallReport {
            schema_version: 1,
            release_id: self.bundle.manifest.release_id.clone(),
            manifest_sha256: self.bundle.manifest_sha256.clone(),
            installation_id,
            provider_configuration: super::super::configuration::prepare(
                &self.bundle,
                &self.layout,
            )
            .unwrap(),
            prefix: self.layout.prefix.clone(),
            resources: self.layout.resources.clone(),
            resource_alias: self.layout.resource_alias(),
            scope: self.layout.scope,
            record_path: self.layout.record(&self.bundle.manifest.release_id),
            phase: "active".into(),
            dry_run: false,
            activated: false,
            error: None,
            warnings: Vec::new(),
            packages: vec![PackageResult {
                package_id: "anolisa-cli".into(),
                version: "1.0.0".into(),
                committed: true,
                observation: "committed".into(),
                verified: true,
                package_operation: None,
            }],
        };
        files::tree(report.record_path.parent().unwrap(), 0o700).unwrap();
        files::json(&report.record_path, &report).unwrap();
    }

    fn active(&self, prefix: &Path, release_id: &str) {
        files::tree(&self.layout.base, 0o755).unwrap();
        symlink(prefix, self.layout.current()).unwrap();
        files::json(&self.layout.state.join(format!("active-{}.json", self.layout.arch_id)), &json!({
            "schema_version":1,"release_id":release_id,"prefix":prefix,"manifest_sha256":"b".repeat(64)
        })).unwrap();
    }
}

#[test]
fn removal_preflight_is_read_only_for_an_inactive_record() {
    let fixture = Fixture::new();
    let record = fixture.layout.record(&fixture.bundle.manifest.release_id);
    let before = fs::read(&record).unwrap();
    preflight(&fixture.bundle, &fixture.layout).unwrap();
    assert_eq!(fs::read(record).unwrap(), before);
    assert!(!fixture.layout.state.join("release.lock").exists());
    assert!(!fixture.layout.package_database.exists());
}

#[test]
fn removal_refuses_the_active_release_and_keeps_current_untouched() {
    let fixture = Fixture::new();
    fixture.active(&fixture.layout.prefix, "old-release");
    assert!(
        preflight(&fixture.bundle, &fixture.layout)
            .unwrap_err()
            .to_string()
            .contains("active releases cannot")
    );
    assert_eq!(
        fs::read_link(fixture.layout.current()).unwrap(),
        fixture.layout.prefix
    );
}

#[test]
fn removal_preserves_unknown_or_changed_current_and_public_entrypoints() {
    let fixture = Fixture::new();
    fs::write(fixture.layout.current(), b"unknown current").unwrap();
    assert!(preflight(&fixture.bundle, &fixture.layout).is_err());
    assert_eq!(
        fs::read(fixture.layout.current()).unwrap(),
        b"unknown current"
    );
    fs::remove_file(fixture.layout.current()).unwrap();
    files::tree(&fixture.layout.bin, 0o755).unwrap();
    let command = fixture.layout.bin.join("anolisa");
    files::publish(&command, b"user command", 0o755).unwrap();
    assert!(preflight(&fixture.bundle, &fixture.layout).is_err());
    assert_eq!(fs::read(command).unwrap(), b"user command");
}

#[test]
fn removal_refuses_an_inconsistent_other_release_activation_record() {
    let fixture = Fixture::new();
    let other = fixture.layout.base.join("releases/new-release");
    fixture.active(&other, "another-release");
    assert!(preflight(&fixture.bundle, &fixture.layout).is_err());
    assert_eq!(fs::read_link(fixture.layout.current()).unwrap(), other);
}

#[test]
fn recorded_system_unit_ownership_blocks_release_removal() {
    let fixture = Fixture::new();
    let directory = fixture.layout.state.join("service-units");
    files::tree(&directory, 0o700).unwrap();
    files::json(&directory.join("ownership.json"), &json!({"units":{"fixture.service":{
        "release_id":"old-release","destination":"/usr/local/lib/systemd/system/fixture.service",
        "sha256":"c".repeat(64)
    }}})).unwrap();
    assert!(
        preflight(&fixture.bundle, &fixture.layout)
            .unwrap_err()
            .to_string()
            .contains("published system units")
    );
}

#[cfg(target_os = "linux")]
#[test]
fn removal_commits_owned_payloads_and_retries_without_touching_unknown_files() {
    use local_package::{
        database::{DatabasePaths, PackageManager},
        plan,
    };
    let fixture = Fixture::new();
    // A product installation owns an empty prefix only after its package transaction.
    fs::remove_dir(&fixture.layout.prefix).unwrap();
    fs::remove_dir(&fixture.layout.resources).unwrap();
    let paths = DatabasePaths::user_for(
        PackageManager::Anolisa,
        Some(&fixture.layout.xdg_state),
        None,
    )
    .unwrap();
    let addition = plan::install_with_resources(
        &paths,
        &fixture.bundle.root.join("packages/anolisa-cli"),
        &fixture.layout.prefix,
        Some(&fixture.layout.resources),
    )
    .unwrap();
    plan::execute(
        &paths,
        &addition,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    fixture.record(addition.installation_id);
    super::super::resources::publish(&fixture.layout).unwrap();
    let unknown = fixture.layout.prefix.join("user-note");
    fs::write(&unknown, b"keep").unwrap();
    let dry = remove_release(&fixture.bundle, &fixture.layout, true).unwrap();
    assert_eq!(dry.phase, "planned");
    assert!(fixture.layout.prefix.join("bin/anolisa").exists());
    let report = remove_release(&fixture.bundle, &fixture.layout, false).unwrap();
    assert_eq!(report.phase, "removed", "{:?}", report.error);
    assert!(report.committed);
    assert!(!fixture.layout.prefix.join("bin/anolisa").exists());
    assert!(!fixture.layout.resource_alias().exists());
    assert_eq!(fs::read(unknown).unwrap(), b"keep");
    assert_eq!(
        remove_release(&fixture.bundle, &fixture.layout, false)
            .unwrap()
            .phase,
        "removed"
    );
}
