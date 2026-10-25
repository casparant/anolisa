//! Public product versions are useful evidence without becoming a runtime health certificate.

use super::*;
use crate::deployment::{DeploymentError, Scope, UnitState};
use serde_json::json;
use std::{
    cell::Cell,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

struct Manager {
    inspected: Cell<usize>,
    active: bool,
}
impl Systemctl for Manager {
    fn inspect(&self, _: Scope, _: &str) -> Result<UnitState, DeploymentError> {
        self.inspected.set(self.inspected.get() + 1);
        Ok(UnitState {
            loaded: true,
            enable_state: "disabled".into(),
            active: self.active,
        })
    }
    fn preset(&self, _: Scope, _: &str, _: bool) -> Result<(), DeploymentError> {
        panic!("overview must not change presets")
    }
    fn start(&self, _: Scope, _: &str) -> Result<(), DeploymentError> {
        panic!("overview must not start services")
    }
}

struct Fixture {
    _temporary: tempfile::TempDir,
    environment: Environment,
    prefix: PathBuf,
    resources: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let environment = Environment::from_values(root.clone(), |_| None).unwrap();
        let prefix = root.join("product");
        let resources = root.join("resources");
        fs::create_dir_all(prefix.join("bin")).unwrap();
        fs::create_dir_all(resources.join("anolisa")).unwrap();
        for command in ["ws-ckpt", "aw"] {
            let path = prefix.join("bin").join(command);
            fs::write(&path, b"fixture, never executed").unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::write(resources.join("anolisa/installed-product.json"), serde_json::to_vec(&json!({
            "schema_version":1,"product_version":"1.0.0","release_id":"fixture","source_commit":"a".repeat(40),
            "os":"linux","arch":std::env::consts::ARCH,"manifest_sha256":"b".repeat(64),
            "packages":[{"package_id":"ws-ckpt","version":"1.2.3"},{"package_id":"aw-core","version":"0.1.0"}]
        })).unwrap()).unwrap();
        Self {
            _temporary: temporary,
            environment,
            prefix,
            resources,
        }
    }
    fn request(&self) -> OverviewRequest<'_> {
        OverviewRequest {
            environment: &self.environment,
            visibility: Visibility::User,
            search_path: Vec::new(),
            product_prefix: Some(self.prefix.clone()),
            product_resources: Some(self.resources.clone()),
            legacy_layouts: Vec::new(),
            check_services: false,
            native_packages: None,
        }
    }
}

#[test]
fn public_summary_records_versions_but_never_claims_service_health() {
    let fixture = Fixture::new();
    let manager = Manager {
        inspected: Cell::new(0),
        active: false,
    };
    let report = overview(&fixture.request(), &manager);
    let ws = report
        .components
        .iter()
        .find(|row| row.component == "ws-ckpt")
        .unwrap();
    assert_eq!(ws.installations[0].version.as_deref(), Some("1.2.3"));
    assert_eq!(ws.installations[0].source, "public_product_summary");
    assert!(ws.service.as_ref().unwrap().active.is_none());
    assert_eq!(manager.inspected.get(), 0);
    assert_eq!(report.latest, "not_checked");
    assert!(
        report
            .components
            .iter()
            .find(|row| row.component == "aw")
            .unwrap()
            .availability
            == "needs_configuration"
    );
}

#[test]
fn explicit_native_inspection_distinguishes_stopped_from_active_but_unverified() {
    let fixture = Fixture::new();
    let mut request = fixture.request();
    request.check_services = true;
    for (active, expected) in [
        (false, "service_stopped"),
        (true, "service_active_health_unverified"),
    ] {
        let manager = Manager {
            inspected: Cell::new(0),
            active,
        };
        let report = overview(&request, &manager);
        assert_eq!(
            report
                .components
                .iter()
                .find(|row| row.component == "ws-ckpt")
                .unwrap()
                .availability,
            expected
        );
        assert_eq!(manager.inspected.get(), 1);
    }
}

#[test]
fn missing_commands_and_untrusted_summary_are_not_reported_as_usable_installations() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.prefix.join("bin/ws-ckpt")).unwrap();
    let manager = Manager {
        inspected: Cell::new(0),
        active: false,
    };
    let report = overview(&fixture.request(), &manager);
    assert_eq!(
        report
            .components
            .iter()
            .find(|row| row.component == "ws-ckpt")
            .unwrap()
            .availability,
        "files_missing"
    );
    let path = fixture.resources.join("anolisa/installed-product.json");
    let real = fixture.resources.join("summary.json");
    fs::rename(&path, &real).unwrap();
    symlink(real, &path).unwrap();
    let report = overview(&fixture.request(), &manager);
    assert!(
        report
            .components
            .iter()
            .find(|row| row.component == "ws-ckpt")
            .unwrap()
            .installations
            .is_empty()
    );
    assert!(!report.warnings.is_empty());
}

#[test]
fn a_local_candidate_compares_actual_versions_without_claiming_to_be_the_latest_release() {
    let fixture = Fixture::new();
    let manager = Manager {
        inspected: Cell::new(0),
        active: false,
    };
    let mut report = overview(&fixture.request(), &manager);
    let package = serde_json::from_value(
        json!({"schema_version":1,"package_id":"ws-ckpt","version":"1.3.0",
        "os":"linux","arch":std::env::consts::ARCH,"files":{},"dependencies":[]}),
    )
    .unwrap();
    let manifest = serde_json::from_value(
        json!({"schema_version":1,"product_version":"2.0.0","release_id":"candidate",
        "source_commit":"a".repeat(40),"os":"linux","arch":std::env::consts::ARCH,"abi":"gnu",
        "min_installer_version":"0.1.0","packages":[],"entrypoints":{}}),
    )
    .unwrap();
    let bundle = crate::release::Bundle {
        root: fixture.environment.home.join("candidate"),
        manifest,
        manifest_sha256: "b".repeat(64),
        packages: vec![package],
        payload_bytes: 0,
    };
    compare_candidate(&mut report, &bundle);
    assert_eq!(report.latest, "not_checked");
    let comparison = report.candidate.unwrap();
    assert_eq!(
        comparison
            .packages
            .iter()
            .find(|item| item.package_id == "ws-ckpt")
            .unwrap()
            .change,
        "update"
    );
    assert_eq!(
        comparison
            .packages
            .iter()
            .find(|item| item.package_id == "aw-core")
            .unwrap()
            .change,
        "not_in_candidate"
    );
    assert!(!comparison.guidance.is_empty());
    assert_eq!(manager.inspected.get(), 0);
}

#[test]
fn every_split_recipe_package_is_grouped_by_explicit_identity() {
    let fixture = Fixture::new();
    let packages = [
        ("cosh", "cosh-ng-core", "cosh"),
        ("memory", "agent-memory-core", "memory"),
        ("skillfs", "skillfs-core", "skillfs"),
        ("sec-core", "agent-sec-cli", "agent-sec-cli"),
    ];
    let mut identities = Vec::new();
    for (_, id, executable) in packages {
        let path = fixture.prefix.join("bin").join(executable);
        fs::write(&path, b"fixture, never executed").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        identities.push(id);
    }
    identities.extend([
        "agent-sec-sandbox",
        "agent-sec-python-runtime",
        "agent-sec-skills",
        "agent-sec-openclaw-hook",
        "agent-sec-hermes-hook",
        "agent-sec-codex-hook",
        "agent-sec-qoder-hook",
        "agent-sec-qwen-code-hook",
        "agent-sec-cosh-hook",
        "aw-provider-sec-core",
        "cosh-ng-skills",
        "skillfs-skills",
        "agent-memory-plugins",
    ]);
    let path = fixture.resources.join("anolisa/installed-product.json");
    let mut summary: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    summary["packages"] = json!(
        identities
            .iter()
            .map(|id| json!({"package_id":id,"version":"1.2.3"}))
            .collect::<Vec<_>>()
    );
    fs::write(path, serde_json::to_vec(&summary).unwrap()).unwrap();
    let manager = Manager {
        inspected: Cell::new(0),
        active: false,
    };
    let report = overview(&fixture.request(), &manager);
    for (name, id, _) in packages {
        let component = report
            .components
            .iter()
            .find(|row| row.component == name)
            .unwrap();
        assert!(component.command.is_some(), "{name}");
        assert!(
            component
                .installations
                .iter()
                .any(|item| item.package_id == id && item.version.as_deref() == Some("1.2.3")),
            "{id}"
        );
    }
    let security = report
        .components
        .iter()
        .find(|row| row.component == "sec-core")
        .unwrap();
    assert_eq!(security.plugin_packages.len(), 6);
    assert_eq!(security.provider_packages, vec!["aw-provider-sec-core"]);
    assert_eq!(
        report
            .components
            .iter()
            .flat_map(|row| &row.installations)
            .count(),
        identities.len()
    );
    assert!(!belongs(&DEFINITIONS[3], "agent-sec-unrecognized-hook"));
}

#[test]
fn active_product_commands_take_priority_over_historical_database_order() {
    let fixture = Fixture::new();
    let old = fixture.environment.home.join("inactive");
    fs::create_dir_all(old.join("bin")).unwrap();
    let old_aw = old.join("bin/aw");
    fs::write(&old_aw, b"inactive executable").unwrap();
    fs::set_permissions(&old_aw, fs::Permissions::from_mode(0o755)).unwrap();
    let evidence = [InstalledEvidence {
        package_id: "aw-core".into(),
        version: Some("0.0.1".into()),
        scope: "user".into(),
        source: "package_database".into(),
        prefix: Some(old),
        resources: None,
    }];
    let manager = Manager {
        inspected: Cell::new(0),
        active: false,
    };
    let row = collect::component(&DEFINITIONS[1], &evidence, &fixture.request(), &manager);
    assert_eq!(row.command, Some(fixture.prefix.join("bin/aw")));
}

#[test]
fn damaged_fifo_summary_is_rejected_without_waiting_for_a_writer() {
    let fixture = Fixture::new();
    let summary = fixture.resources.join("anolisa/installed-product.json");
    fs::remove_file(&summary).unwrap();
    nix::unistd::mkfifo(
        &summary,
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .unwrap();
    let manager = Manager {
        inspected: Cell::new(0),
        active: false,
    };
    let began = std::time::Instant::now();
    let report = overview(&fixture.request(), &manager);
    assert!(began.elapsed() < std::time::Duration::from_secs(1));
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("bounded regular file"))
    );
    assert!(
        report
            .components
            .iter()
            .all(|row| row.installations.is_empty())
    );
}
