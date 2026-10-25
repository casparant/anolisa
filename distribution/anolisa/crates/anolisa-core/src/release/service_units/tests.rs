//! Injected manager tests exercise unit publication without contacting host systemd.

use super::*;
use crate::release::{Environment, InstallReport, PackageResult, ProviderConfiguration};
use serde_json::json;
use std::{
    cell::Cell,
    os::unix::fs::{PermissionsExt, symlink},
};

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    bundle: Bundle,
    layout: ReleaseLayout,
    manager: Fake,
}
struct Fake {
    directory: PathBuf,
    override_path: PathBuf,
    reloads: Cell<usize>,
    fail: Cell<bool>,
}
impl Manager for Fake {
    fn directory(&self) -> PathBuf {
        self.directory.clone()
    }
    fn overrides(&self, _unit: &str) -> Vec<PathBuf> {
        vec![self.override_path.clone()]
    }
    fn reload(&self) -> io::Result<()> {
        self.reloads.set(self.reloads.get() + 1);
        if self.fail.get() {
            Err(io::Error::other("injected reload failure"))
        } else {
            Ok(())
        }
    }
}
impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let source = root.join("source");
        let mut references = Vec::new();
        for (id, payloads) in [
            ("aw-core", vec![("bin/aw", "#!/bin/sh\nexit 0\n", 0o755)]),
            (
                "anolisa-cli",
                vec![("bin/anolisa", "#!/bin/sh\nexit 0\n", 0o755)],
            ),
            (
                "ws-ckpt-core",
                vec![
                    ("bin/ws-ckpt", "#!/bin/sh\nexit 0\n", 0o755),
                    (
                        "share/ws-ckpt/templates/ws-ckpt.service",
                        "[Unit]\nDescription=fixture\n[Service]\nExecStart=\"{bindir}/ws-ckpt\" daemon\nExecReload=\"{bindir}/ws-ckpt\" reload\n[Install]\nWantedBy=multi-user.target\n",
                        0o644,
                    ),
                ],
            ),
        ] {
            let directory = source.join("packages").join(id);
            let mut records = BTreeMap::new();
            for (name, bytes, mode) in payloads {
                let path = directory.join("payload").join(name);
                files::tree(path.parent().unwrap(), 0o755).unwrap();
                fs::write(&path, bytes).unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
                records.insert(
                    name,
                    json!({"sha256":files::digest(&path).unwrap(),"mode":mode}),
                );
            }
            let description = json!({"schema_version":1,"package_id":id,"version":"1.0.0","os":"linux","arch":std::env::consts::ARCH,"files":records,"dependencies":[]});
            let bytes = serde_json::to_vec_pretty(&description).unwrap();
            fs::write(directory.join("package.json"), &bytes).unwrap();
            references.push(json!({"package_id":id,"version":"1.0.0","path":format!("packages/{id}"),"description_sha256":files::digest_bytes(&bytes)}));
        }
        let manifest = json!({"schema_version":1,"product_version":"1.0.0","release_id":"unit-fixture","source_commit":"a".repeat(40),"os":"linux","arch":std::env::consts::ARCH,"abi":"gnu","min_installer_version":"0.1.0","packages":references,"entrypoints":{"aw":"bin/aw","anolisa":"bin/anolisa","ws-ckpt":"bin/ws-ckpt"}});
        fs::write(
            source.join("release.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let bundle = Bundle::read(&source).unwrap();
        let environment = Environment::from_values(root.join("home"), |_| None).unwrap();
        let mut layout =
            ReleaseLayout::resolve(&bundle.manifest, Scope::User, &environment).unwrap();
        layout.scope = Scope::System;
        let manager = Fake {
            directory: root.join("units"),
            override_path: root.join("admin/ws-ckpt.service"),
            reloads: Cell::new(0),
            fail: Cell::new(false),
        };
        let fixture = Self {
            _temporary: temporary,
            root,
            bundle,
            layout,
            manager,
        };
        fixture.install_record();
        fixture
    }
    fn install_record(&self) {
        for package in &self.bundle.packages {
            for (name, payload) in &package.files {
                let destination = resource(&self.layout, name);
                files::tree(destination.parent().unwrap(), 0o755).unwrap();
                fs::copy(
                    self.bundle
                        .root
                        .join("packages")
                        .join(&package.package_id)
                        .join("payload")
                        .join(name),
                    &destination,
                )
                .unwrap();
                fs::set_permissions(destination, fs::Permissions::from_mode(payload.mode)).unwrap();
            }
        }
        files::tree(&self.layout.base, 0o755).unwrap();
        if self.layout.current().is_symlink() {
            fs::remove_file(self.layout.current()).unwrap();
        }
        symlink(&self.layout.prefix, self.layout.current()).unwrap();
        let report = InstallReport {
            schema_version: 1,
            release_id: self.bundle.manifest.release_id.clone(),
            manifest_sha256: self.bundle.manifest_sha256.clone(),
            installation_id: Some("fixture".into()),
            provider_configuration: ProviderConfiguration {
                sha256: "a".repeat(64),
            },
            prefix: self.layout.prefix.clone(),
            resources: self.layout.resources.clone(),
            resource_alias: self.layout.resource_alias(),
            scope: Scope::System,
            record_path: self.layout.record(&self.bundle.manifest.release_id),
            phase: "active".into(),
            dry_run: false,
            packages: self
                .bundle
                .packages
                .iter()
                .map(|package| PackageResult {
                    package_id: package.package_id.clone(),
                    version: package.version.clone(),
                    committed: true,
                    observation: "committed".into(),
                    verified: true,
                    package_operation: None,
                })
                .collect(),
            activated: true,
            error: None,
            warnings: Vec::new(),
        };
        files::tree(report.record_path.parent().unwrap(), 0o700).unwrap();
        files::json(&report.record_path, &report).unwrap();
    }
    fn refresh_source(&mut self, release_id: &str, program: Option<&[u8]>) {
        let source = self.root.join("source");
        let description_path = source.join("packages/ws-ckpt-core/package.json");
        let mut description: serde_json::Value =
            serde_json::from_slice(&fs::read(&description_path).unwrap()).unwrap();
        if let Some(bytes) = program {
            let path = source.join("packages/ws-ckpt-core/payload/bin/ws-ckpt");
            fs::write(&path, bytes).unwrap();
            description["files"]["bin/ws-ckpt"]["sha256"] = json!(files::digest_bytes(bytes));
        }
        let description_bytes = serde_json::to_vec_pretty(&description).unwrap();
        fs::write(&description_path, &description_bytes).unwrap();
        let manifest_path = source.join("release.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["release_id"] = json!(release_id);
        for package in manifest["packages"].as_array_mut().unwrap() {
            if package["package_id"] == "ws-ckpt-core" {
                package["description_sha256"] = json!(files::digest_bytes(&description_bytes));
            }
        }
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        self.bundle = Bundle::read(&source).unwrap();
        let environment = Environment::from_values(self.root.join("home"), |_| None).unwrap();
        self.layout =
            ReleaseLayout::resolve(&self.bundle.manifest, Scope::User, &environment).unwrap();
        self.layout.scope = Scope::System;
        self.install_record();
    }
    fn plan(&self) -> Result<ServiceUnitPlan> {
        plan_with(
            &self.bundle,
            &self.layout,
            &["ws-ckpt".into()],
            &self.manager,
        )
    }
}

#[test]
fn planning_is_read_only_and_publication_only_reloads_the_manager() {
    let fixture = Fixture::new();
    let plan = fixture.plan().unwrap();
    assert!(!directory(&fixture.layout).exists());
    assert!(!fixture.manager.directory.exists());
    let report = apply_with(&fixture.bundle, &fixture.layout, &plan, &fixture.manager).unwrap();
    assert!(report.error.is_none(), "{report:?}");
    assert!(report.daemon_reloaded);
    assert_eq!(fixture.manager.reloads.get(), 1);
    let unit = fs::read_to_string(&plan.units[0].destination).unwrap();
    assert!(unit.contains(&format!(
        "ExecStart=\"{}/bin/ws-ckpt\" daemon",
        fixture.layout.prefix.display()
    )));
    assert!(!unit.contains("{bindir}"));
    assert!(
        plan.units[0]
            .configuration_paths
            .contains(&PathBuf::from("/etc/ws-ckpt/config.toml"))
    );
    assert_eq!(
        files::digest(&plan.units[0].destination).unwrap(),
        plan.units[0].rendered_sha256
    );
}

#[test]
fn unknown_native_unit_is_preserved_even_if_its_bytes_match() {
    let fixture = Fixture::new();
    let (mut action, bytes) = render(&fixture.bundle, &fixture.layout, "ws-ckpt").unwrap();
    action.destination = fixture.manager.directory.join(&action.unit);
    files::tree(&fixture.manager.directory, 0o755).unwrap();
    files::publish(&action.destination, &bytes, 0o644).unwrap();
    assert!(
        fixture
            .plan()
            .unwrap_err()
            .to_string()
            .contains("no product ownership")
    );
    assert_eq!(fs::read(action.destination).unwrap(), bytes);
}

#[test]
fn administrator_override_blocks_publication_without_writes() {
    let fixture = Fixture::new();
    files::tree(fixture.manager.override_path.parent().unwrap(), 0o755).unwrap();
    fs::write(&fixture.manager.override_path, "administrator unit").unwrap();
    assert!(
        fixture
            .plan()
            .unwrap_err()
            .to_string()
            .contains("outside this product's ownership")
    );
    assert!(!directory(&fixture.layout).exists());
}

#[test]
fn failed_reload_retains_owned_units_and_identical_retry_succeeds() {
    let fixture = Fixture::new();
    let plan = fixture.plan().unwrap();
    fixture.manager.fail.set(true);
    let first = apply_with(&fixture.bundle, &fixture.layout, &plan, &fixture.manager).unwrap();
    assert!(first.error.as_ref().unwrap().contains("reload failed"));
    assert_eq!(first.phase, "reload_pending");
    assert!(plan.units[0].destination.is_file());
    fixture.manager.fail.set(false);
    let retry = fixture.plan().unwrap();
    assert_eq!(retry, plan);
    let report = apply_with(&fixture.bundle, &fixture.layout, &retry, &fixture.manager).unwrap();
    assert!(report.error.is_none());
    assert!(report.daemon_reloaded);
    assert_eq!(fixture.manager.reloads.get(), 2);
}

#[test]
fn changed_installed_template_and_stale_plan_are_refused() {
    let fixture = Fixture::new();
    let mut plan = fixture.plan().unwrap();
    plan.units[0].destination = fixture.root.join("unrelated-unit");
    assert!(apply_with(&fixture.bundle, &fixture.layout, &plan, &fixture.manager).is_err());
    let template = fixture
        .layout
        .resources
        .join("ws-ckpt/templates/ws-ckpt.service");
    fs::write(template, "modified template").unwrap();
    assert!(fixture.plan().is_err());
    assert!(!fixture.root.join("unrelated-unit").exists());
}

#[test]
fn user_scope_and_services_without_a_safe_system_template_are_refused() {
    let mut fixture = Fixture::new();
    assert!(
        plan_with(
            &fixture.bundle,
            &fixture.layout,
            &["cosh-gateway".into()],
            &fixture.manager
        )
        .is_err()
    );
    fixture.layout.scope = Scope::User;
    assert!(fixture.plan().is_err());
}

#[test]
fn an_administrator_edit_after_a_failed_reload_is_never_overwritten() {
    let fixture = Fixture::new();
    let plan = fixture.plan().unwrap();
    fixture.manager.fail.set(true);
    apply_with(&fixture.bundle, &fixture.layout, &plan, &fixture.manager).unwrap();
    fs::write(
        &plan.units[0].destination,
        "administrator changed this unit",
    )
    .unwrap();
    assert!(fixture.plan().is_err());
    assert_eq!(
        fs::read_to_string(&plan.units[0].destination).unwrap(),
        "administrator changed this unit"
    );
}

#[test]
fn real_size_programs_use_streaming_digest_instead_of_the_template_byte_limit() {
    let mut fixture = Fixture::new();
    fixture.refresh_source("unit-large-program", Some(&vec![b'x'; 2 * LIMIT as usize]));
    let plan = fixture.plan().unwrap();
    assert!(
        apply_with(&fixture.bundle, &fixture.layout, &plan, &fixture.manager)
            .unwrap()
            .error
            .is_none()
    );
}

#[test]
fn completed_publication_is_idempotent_and_does_not_reload_again() {
    let fixture = Fixture::new();
    let plan = fixture.plan().unwrap();
    apply_with(&fixture.bundle, &fixture.layout, &plan, &fixture.manager).unwrap();
    let before = fs::read(&plan.record_path).unwrap();
    let report = apply_with(&fixture.bundle, &fixture.layout, &plan, &fixture.manager).unwrap();
    assert_eq!(report.phase, "complete");
    assert_eq!(fixture.manager.reloads.get(), 1);
    assert_eq!(fs::read(plan.record_path).unwrap(), before);
}

#[test]
fn a_new_installed_release_backs_up_only_the_previously_owned_unit() {
    let mut fixture = Fixture::new();
    let first = fixture.plan().unwrap();
    apply_with(&fixture.bundle, &fixture.layout, &first, &fixture.manager).unwrap();
    let original = fs::read(&first.units[0].destination).unwrap();
    fixture.refresh_source("unit-fixture-upgrade", None);
    let next = fixture.plan().unwrap();
    assert_eq!(
        next.units[0].previous_sha256,
        Some(files::digest_bytes(&original))
    );
    let backup = next.units[0].backup.as_ref().unwrap();
    assert!(!backup.exists());
    let report = apply_with(&fixture.bundle, &fixture.layout, &next, &fixture.manager).unwrap();
    assert!(report.error.is_none(), "{report:?}");
    assert_eq!(fs::read(backup).unwrap(), original);
    let unit = fs::read_to_string(&next.units[0].destination).unwrap();
    assert!(unit.contains(&fixture.layout.prefix.to_string_lossy().to_string()));
    assert_ne!(
        next.units[0].rendered_sha256,
        first.units[0].rendered_sha256
    );
    assert_eq!(fixture.manager.reloads.get(), 2);
}

#[test]
fn templates_remain_bounded_even_when_their_package_digest_is_valid() {
    let mut fixture = Fixture::new();
    let source = fixture.root.join("source/packages/ws-ckpt-core");
    let name = "share/ws-ckpt/templates/ws-ckpt.service";
    let bytes = vec![b'x'; LIMIT as usize + 1];
    fs::write(source.join("payload").join(name), &bytes).unwrap();
    let path = source.join("package.json");
    let mut description: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    description["files"][name]["sha256"] = json!(files::digest_bytes(&bytes));
    fs::write(path, serde_json::to_vec_pretty(&description).unwrap()).unwrap();
    fixture.refresh_source("unit-large-template", None);
    assert!(
        fixture
            .plan()
            .unwrap_err()
            .to_string()
            .contains("byte limit")
    );
    assert!(!directory(&fixture.layout).exists());
}

#[test]
fn an_install_in_progress_blocks_unit_publication_before_any_unit_writes() {
    let fixture = Fixture::new();
    let plan = fixture.plan().unwrap();
    let _install_lock =
        crate::release::install::ProductLock::acquire(&fixture.layout.state.join("release.lock"))
            .unwrap();
    assert!(apply_with(&fixture.bundle, &fixture.layout, &plan, &fixture.manager).is_err());
    assert!(!directory(&fixture.layout).exists());
    assert!(!fixture.manager.directory.exists());
    assert_eq!(fixture.manager.reloads.get(), 0);
}
