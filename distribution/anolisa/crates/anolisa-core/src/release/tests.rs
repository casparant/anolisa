//! Injected package operations exercise offline product contracts and recovery on any Unix host.

use super::{
    store::{Backend, Operation, Output},
    *,
};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    bundle: Bundle,
    layout: ReleaseLayout,
}
impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let home = root.join("home");
        fs::create_dir(&home).unwrap();
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
        let source = root.join("source");
        let mut packages = Vec::new();
        for (id, file, bytes, dependencies) in [
            ("aw-core", "bin/aw", "#!/bin/sh\nexit 0\n", json!([])),
            (
                "anolisa-cli",
                "bin/anolisa",
                "#!/bin/sh\ncase ${1-} in\nresource-test) prefix=$(CDPATH='' cd -P -- \"$(dirname -- \"$0\")/..\" && pwd -P); cat \"$prefix/share/anolisa/fixture.txt\" ;;\ncontext-test) printf '%s\\n' \"$HOME\" \"${XDG_DATA_HOME-}\" \"${XDG_RUNTIME_DIR-}\" \"${ANOLISA_RELEASE_PREFIX-}\" \"${ANOLISA_RELEASE_RESOURCES-}\" ;;\n*) printf '%s\\n' \"$0:$*\" ;;\nesac\n",
                json!([{"package_id":"aw-core","version":"1.0.0"}]),
            ),
        ] {
            let directory = source.join("packages").join(id);
            let payload = directory.join("payload").join(file);
            files::tree(payload.parent().unwrap(), 0o755).unwrap();
            fs::write(&payload, bytes).unwrap();
            fs::set_permissions(&payload, fs::Permissions::from_mode(0o755)).unwrap();
            let mut description = json!({"schema_version":1,"package_id":id,"version":"1.0.0","os":"linux","arch":std::env::consts::ARCH,"files":{file:{"sha256":files::digest(&payload).unwrap(),"mode":493}},"dependencies":dependencies});
            if id == "aw-core" {
                let resource = directory.join("payload/share/anolisa/fixture.txt");
                files::tree(resource.parent().unwrap(), 0o755).unwrap();
                fs::write(&resource, b"release resources\n").unwrap();
                fs::set_permissions(&resource, fs::Permissions::from_mode(0o644)).unwrap();
                description["files"]["share/anolisa/fixture.txt"] =
                    json!({"sha256":files::digest(&resource).unwrap(),"mode":420});
            }
            let bytes = serde_json::to_vec_pretty(&description).unwrap();
            fs::write(directory.join("package.json"), &bytes).unwrap();
            packages.push(json!({"package_id":id,"version":"1.0.0","path":format!("packages/{id}"),"description_sha256":files::digest_bytes(&bytes)}));
        }
        let manifest = json!({"schema_version":1,"product_version":"1.0.0","release_id":"test-release","source_commit":"a".repeat(40),"os":"linux","arch":std::env::consts::ARCH,"abi":"gnu","min_installer_version":"0.1.0","packages":packages,"entrypoints":{"aw":"bin/aw","anolisa":"bin/anolisa"}});
        fs::write(
            source.join("release.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let bundle = Bundle::read(&source).unwrap();
        let environment = Environment::from_values(home, |_| None).unwrap();
        let layout = ReleaseLayout::resolve(&bundle.manifest, Scope::User, &environment).unwrap();
        Self {
            _temporary: temporary,
            root,
            bundle,
            layout,
        }
    }
    fn with_sec_core(mut self) -> Self {
        for (id, file, dependencies) in [
            ("sec-core", "bin/agent-sec-cli", json!([])),
            (
                "aw-provider-sec-core",
                "libexec/aw/providers/sec-core/aw-provider-sec-core",
                json!([
                    {"package_id":"aw-core","version":"1.0.0"},
                    {"package_id":"sec-core","version":"1.0.0"}
                ]),
            ),
        ] {
            let directory = self.bundle.root.join("packages").join(id);
            let payload = directory.join("payload").join(file);
            files::tree(payload.parent().unwrap(), 0o755).unwrap();
            fs::write(&payload, b"#!/bin/sh\nexit 0\n").unwrap();
            fs::set_permissions(&payload, fs::Permissions::from_mode(0o755)).unwrap();
            let mut description = json!({"schema_version":1,"package_id":id,"version":"1.0.0","os":"linux","arch":std::env::consts::ARCH,"files":{file:{"sha256":files::digest(&payload).unwrap(),"mode":493}},"dependencies":dependencies});
            if id == "aw-provider-sec-core" {
                description["provider"] = json!({"protocol":"aw-provider/v1alpha1"});
            }
            let bytes = serde_json::to_vec_pretty(&description).unwrap();
            fs::write(directory.join("package.json"), &bytes).unwrap();
            self.change_manifest(|manifest| {
                manifest["packages"].as_array_mut().unwrap().push(json!({"package_id":id,"version":"1.0.0","path":format!("packages/{id}"),"description_sha256":files::digest_bytes(&bytes)}));
            });
        }
        self.bundle = Bundle::read(&self.bundle.root).unwrap();
        self
    }

    fn with_choices(mut self) -> Self {
        self.change_manifest(|manifest| {
            manifest["schema_version"] = json!(2);
            manifest["distribution"] = json!({
                "schema_version": 1,
                "default_profiles": {"user":"minimal","system":"standard-system"},
                "profiles": {
                    "minimal":{"packages":["aw-core","anolisa-cli"],"scopes":["user","system"],"preset":"disabled"},
                    "standard-user":{"packages":["aw-core","anolisa-cli","aw-provider-sec-core"],"scopes":["user"],"preset":"disabled"},
                    "standard-system":{"packages":["aw-core","anolisa-cli","aw-provider-sec-core"],"scopes":["system"],"preset":"disabled"}
                },
                "components":{"sec-core":["aw-provider-sec-core"]},
                "presets":{"disabled":{"services":[]}}
            });
            manifest["entrypoints"]["agent-sec-cli"] = json!("bin/agent-sec-cli");
        });
        self.bundle = Bundle::read(&self.bundle.root).unwrap();
        self
    }

    fn backend(&self) -> Fake<'_> {
        Fake {
            fixture: self,
            installed: RefCell::new(BTreeMap::new()),
            calls: RefCell::new(Vec::new()),
            fail: RefCell::new(None),
            lost_report: RefCell::new(false),
        }
    }
    fn change_manifest(&self, mutate: impl FnOnce(&mut Value)) {
        let path = self.bundle.root.join("release.json");
        let mut value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        mutate(&mut value);
        fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
}

#[test]
fn profile_choices_resolve_dependencies_and_keep_source_evidence() {
    let fixture = Fixture::new().with_sec_core().with_choices();
    let user = fixture.bundle.select(Scope::User, None, &[]).unwrap();
    let system = fixture.bundle.select(Scope::System, None, &[]).unwrap();
    assert_eq!(user.packages.len(), 2);
    assert_eq!(system.packages.len(), 4);
    assert_ne!(user.manifest.release_id, system.manifest.release_id);
    assert_eq!(user.manifest_sha256, fixture.bundle.manifest_sha256);
    assert_eq!(system.manifest_sha256, fixture.bundle.manifest_sha256);
    assert_eq!(
        user.manifest.release_id,
        fixture
            .bundle
            .select(Scope::User, Some("minimal"), &[])
            .unwrap()
            .manifest
            .release_id
    );
    let component = fixture
        .bundle
        .select(Scope::System, None, &["sec-core".into()])
        .unwrap();
    assert_eq!(component.packages.len(), 4);
    assert!(
        component
            .packages
            .iter()
            .any(|package| package.package_id == "sec-core")
    );
    user.recheck().unwrap();
    let unselected = fixture
        .bundle
        .root
        .join("packages/sec-core/payload/bin/agent-sec-cli");
    fs::write(unselected, b"changed unselected backend").unwrap();
    assert!(
        user.recheck().is_err(),
        "selection must still bind all delivered input bytes"
    );
}

#[test]
fn invalid_profile_and_component_choices_are_rejected() {
    let fixture = Fixture::new().with_sec_core().with_choices();
    assert!(
        fixture
            .bundle
            .select(Scope::User, Some("standard-system"), &[])
            .is_err()
    );
    assert!(
        fixture
            .bundle
            .select(Scope::System, Some("missing"), &[])
            .is_err()
    );
    assert!(
        fixture
            .bundle
            .select(Scope::System, None, &["missing".into()])
            .is_err()
    );
    assert!(
        fixture
            .bundle
            .select(Scope::System, None, &["sec-core".into(), "sec-core".into()])
            .is_err()
    );
    fixture.change_manifest(|manifest| {
        manifest["distribution"]["profiles"]["minimal"]["packages"] =
            json!(["aw-core", "anolisa-cli", "missing"])
    });
    assert!(Bundle::read(&fixture.bundle.root).is_err());
}

#[test]
fn legacy_complete_bundle_keeps_its_original_installation_identity() {
    let fixture = Fixture::new();
    let selected = fixture.bundle.select(Scope::User, None, &[]).unwrap();
    assert_eq!(
        selected.manifest.release_id,
        fixture.bundle.manifest.release_id
    );
    assert_eq!(selected.packages.len(), fixture.bundle.packages.len());
    assert!(
        fixture
            .bundle
            .select(Scope::User, Some("minimal"), &[])
            .is_err()
    );
}

#[test]
fn switching_to_a_smaller_profile_removes_only_unchanged_managed_launchers() {
    let mut fixture = Fixture::new().with_sec_core().with_choices();
    fixture.bundle = fixture
        .bundle
        .select(Scope::User, Some("standard-user"), &[])
        .unwrap();
    let environment = Environment::from_values(fixture.root.join("home"), |_| None).unwrap();
    fixture.layout =
        ReleaseLayout::resolve(&fixture.bundle.manifest, Scope::User, &environment).unwrap();
    let old_prefix = fixture.layout.prefix.clone();
    {
        let backend = fixture.backend();
        assert!(
            install::execute(&fixture.bundle, &fixture.layout, false, &backend)
                .unwrap()
                .succeeded()
        );
    }
    let old_command = fixture.layout.bin.join("agent-sec-cli");
    let original = fs::read(&old_command).unwrap();
    fs::write(
        &old_command,
        [original.as_slice(), b"# administrator edit\n"].concat(),
    )
    .unwrap();
    fixture.bundle = Bundle::read(&fixture.bundle.root)
        .unwrap()
        .select(Scope::User, Some("minimal"), &[])
        .unwrap();
    fixture.layout =
        ReleaseLayout::resolve(&fixture.bundle.manifest, Scope::User, &environment).unwrap();
    let backend = fixture.backend();
    let error = install::execute(&fixture.bundle, &fixture.layout, false, &backend).unwrap_err();
    assert!(error.to_string().contains("obsolete product command"));
    assert_eq!(fs::read_link(fixture.layout.current()).unwrap(), old_prefix);
    assert!(
        fs::read_to_string(&old_command)
            .unwrap()
            .contains("administrator edit")
    );
    fs::write(&old_command, original).unwrap();
    assert!(
        install::execute(&fixture.bundle, &fixture.layout, false, &backend)
            .unwrap()
            .succeeded()
    );
    assert!(!old_command.exists());
    assert!(
        old_prefix.join("bin/agent-sec-cli").is_file(),
        "prior version payload remains available for explicit rollback"
    );
    assert!(fixture.layout.bin.join("aw").is_file());
    assert!(fixture.layout.bin.join("anolisa").is_file());
}

fn legacy_source(fixture: &Fixture) -> anolisa_platform::fs_layout::FsLayout {
    use crate::{domain::Installation, state_store::StateStore};
    let source = anolisa_platform::fs_layout::FsLayout::user_with_overrides(
        fixture.root.join("home"),
        None,
        None,
        None,
        None,
        None,
    );
    files::tree(&source.bin_dir, 0o755).unwrap();
    let mut owned = Vec::new();
    for name in ["aw", "anolisa"] {
        let path = source.bin_dir.join(name);
        fs::write(&path, format!("#!/bin/sh\nprintf old-{name}\\n\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        if name == "aw" {
            owned.push(
                json!({"path":path,"owner":"anolisa","sha256":files::digest(&path).unwrap()}),
            );
        }
    }
    let installation: Installation = serde_json::from_value(json!({
        "kind":"component","name":"aw-core","scope":{"user":{"uid":nix::unistd::geteuid().as_raw()}},
        "binding":{"authority":"owned","artifact":{"version":"0.9.0","files":owned}},
        "status":"installed","installed_at":"2026-10-25T00:00:00Z"
    })).unwrap();
    let mut store = StateStore::empty_for_layout(&source);
    store.upsert(installation);
    store
        .save(&source.state_dir.join("installed.toml"))
        .unwrap();
    source
}

fn legacy_plan(fixture: &Fixture, source: &anolisa_platform::fs_layout::FsLayout) -> MigrationPlan {
    plan_migration(
        &fixture.bundle,
        &fixture.layout,
        source,
        None,
        vec![],
        Some(files::digest(&source.bin_dir.join("anolisa")).unwrap()),
    )
    .unwrap()
}

#[test]
fn migration_plan_is_read_only_and_unknown_commands_block_transfer() {
    let fixture = Fixture::new();
    let source = legacy_source(&fixture);
    let state = source.state_dir.join("installed.toml");
    let before = fs::read(&state).unwrap();
    let without_cli_adoption = plan_migration(
        &fixture.bundle,
        &fixture.layout,
        &source,
        None,
        vec![],
        None,
    )
    .unwrap();
    assert!(
        without_cli_adoption
            .blockers
            .iter()
            .any(|message| message.contains("unproven existing command"))
    );
    let plan = legacy_plan(&fixture, &source);
    assert!(plan.blockers.is_empty());
    assert_eq!(plan.retire, ["aw-core"]);
    assert_eq!(plan.entrypoints.len(), 2);
    assert_eq!(fs::read(state).unwrap(), before);
    assert!(!fixture.layout.state.join("migrations").exists());
    let path = fixture.root.join("migration.json");
    plan.write(&path).unwrap();
    let loaded = MigrationPlan::read(&path).unwrap();
    assert_eq!(loaded.id, plan.id);
    let mut changed: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    changed["target_prefix"] = json!("/unreviewed");
    fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert!(MigrationPlan::read(&path).is_err());
}

#[test]
fn migration_rejects_changed_source_before_installing() {
    let fixture = Fixture::new();
    let source = legacy_source(&fixture);
    let plan = legacy_plan(&fixture, &source);
    fs::write(source.bin_dir.join("aw"), b"changed old command").unwrap();
    assert!(
        migration::apply_with(&plan, &fixture.bundle, &fixture.layout, &source, || panic!(
            "changed source must not install"
        ))
        .is_err()
    );
    assert!(!fixture.layout.state.join("migrations").exists());
}

#[test]
fn migration_failure_preserves_old_records_and_successful_retry_retires_them() {
    use crate::state_store::StateStore;
    let fixture = Fixture::new();
    let source = legacy_source(&fixture);
    let plan = legacy_plan(&fixture, &source);
    let original = fs::read(&plan.source_state).unwrap();
    let old_cli = fs::read(source.bin_dir.join("anolisa")).unwrap();
    let backend = fixture.backend();
    *backend.fail.borrow_mut() = Some("aw-core".into());
    let failed = migration::apply_with(&plan, &fixture.bundle, &fixture.layout, &source, || {
        install::execute(&fixture.bundle, &fixture.layout, false, &backend)
    })
    .unwrap();
    assert!(!failed.succeeded());
    assert_eq!(fs::read(&plan.source_state).unwrap(), original);
    assert_eq!(fs::read(source.bin_dir.join("anolisa")).unwrap(), old_cli);
    *backend.fail.borrow_mut() = None;
    let succeeded = migration::apply_with(&plan, &fixture.bundle, &fixture.layout, &source, || {
        install::execute(&fixture.bundle, &fixture.layout, false, &backend)
    })
    .unwrap();
    assert!(succeeded.succeeded());
    let store = StateStore::load_for_layout(&plan.source_state, plan.uid, &source).unwrap();
    assert!(store.installations.is_empty());
    let backup = fixture.layout.state.join("migrations").join(&plan.id);
    assert_eq!(
        fs::read(backup.join("installed.toml.before")).unwrap(),
        original
    );
    assert_eq!(fs::read(backup.join("anolisa.before")).unwrap(), old_cli);
    let resumed = migration::apply_with(&plan, &fixture.bundle, &fixture.layout, &source, || {
        panic!("completed migration must not reinstall")
    })
    .unwrap();
    assert!(resumed.succeeded());
}

#[test]
fn standalone_cli_migration_records_state_absence_without_fabricating_old_rows() {
    let fixture = Fixture::new();
    let source = legacy_source(&fixture);
    let state = source.state_dir.join("installed.toml");
    fs::remove_file(&state).unwrap();
    fs::remove_file(source.bin_dir.join("aw")).unwrap();
    let unproven = plan_migration(
        &fixture.bundle,
        &fixture.layout,
        &source,
        None,
        vec![],
        None,
    )
    .unwrap();
    assert!(!unproven.source_state_present);
    assert!(!unproven.blockers.is_empty());
    let plan = legacy_plan(&fixture, &source);
    assert!(plan.blockers.is_empty());
    assert!(!plan.source_state_present);
    assert!(plan.retire.is_empty());
    assert!(!state.exists());
    let backend = fixture.backend();
    let report = migration::apply_with(&plan, &fixture.bundle, &fixture.layout, &source, || {
        install::execute(&fixture.bundle, &fixture.layout, false, &backend)
    })
    .unwrap();
    assert!(report.succeeded());
    assert!(
        !state.exists(),
        "CLI-only migration must not invent legacy managed state"
    );
    assert!(
        fixture
            .layout
            .state
            .join("migrations")
            .join(&plan.id)
            .join("installed.toml.absent")
            .is_file()
    );
    assert!(
        migration::apply_with(&plan, &fixture.bundle, &fixture.layout, &source, || panic!(
            "completed CLI-only migration must not reinstall"
        ))
        .unwrap()
        .succeeded()
    );
}

#[test]
fn standalone_cli_retry_rejects_newly_appeared_source_state() {
    use crate::state_store::StateStore;
    let fixture = Fixture::new();
    let source = legacy_source(&fixture);
    let state = source.state_dir.join("installed.toml");
    fs::remove_file(&state).unwrap();
    fs::remove_file(source.bin_dir.join("aw")).unwrap();
    let plan = legacy_plan(&fixture, &source);
    let backend = fixture.backend();
    *backend.fail.borrow_mut() = Some("aw-core".into());
    let failed = migration::apply_with(&plan, &fixture.bundle, &fixture.layout, &source, || {
        install::execute(&fixture.bundle, &fixture.layout, false, &backend)
    })
    .unwrap();
    assert!(!failed.succeeded());
    StateStore::empty_for_layout(&source).save(&state).unwrap();
    assert!(
        migration::apply_with(&plan, &fixture.bundle, &fixture.layout, &source, || panic!(
            "changed source presence must not install"
        ))
        .is_err()
    );
}

#[test]
fn legacy_pending_recovery_blocks_plans_and_prepared_migration_retries() {
    use crate::transaction::Transaction;
    let fixture = Fixture::new();
    let source = legacy_source(&fixture);
    let plan = legacy_plan(&fixture, &source);
    let journals = source.state_dir.join("journal");
    drop(
        Transaction::begin_with_subject(
            "install",
            Some("aw-core"),
            plan.source_state.clone(),
            &journals,
        )
        .unwrap(),
    );
    let blocked = legacy_plan(&fixture, &source);
    assert!(
        blocked
            .blockers
            .iter()
            .any(|message| message.contains("pending legacy recovery"))
    );
    assert!(
        migration::apply_with(&plan, &fixture.bundle, &fixture.layout, &source, || panic!(
            "pending recovery must not install"
        ))
        .is_err()
    );
    fs::remove_dir_all(&journals).unwrap();
    let backend = fixture.backend();
    *backend.fail.borrow_mut() = Some("aw-core".into());
    assert!(
        !migration::apply_with(&plan, &fixture.bundle, &fixture.layout, &source, || {
            install::execute(&fixture.bundle, &fixture.layout, false, &backend)
        })
        .unwrap()
        .succeeded()
    );
    drop(
        Transaction::begin_with_subject(
            "uninstall",
            Some("aw-core"),
            plan.source_state.clone(),
            &journals,
        )
        .unwrap(),
    );
    assert!(
        migration::apply_with(&plan, &fixture.bundle, &fixture.layout, &source, || panic!(
            "new recovery must block an already prepared retry"
        ))
        .is_err()
    );
}

#[test]
fn migration_rejects_quarantined_selected_records_and_malformed_journals() {
    use crate::state_store::StateStore;
    let fixture = Fixture::new();
    let source = legacy_source(&fixture);
    let path = source.state_dir.join("installed.toml");
    let mut store =
        StateStore::load_for_layout(&path, nix::unistd::geteuid().as_raw(), &source).unwrap();
    store.quarantined.push(serde_json::from_value(json!({
        "record":{"kind":"component","name":"anolisa","version":"0.9.0","status":"installed","installed_at":"2026-10-25T00:00:00Z"},
        "reason":{"reason":"no_evidence"}
    })).unwrap());
    store.save(&path).unwrap();
    assert!(
        legacy_plan(&fixture, &source)
            .blockers
            .iter()
            .any(|message| message.contains("quarantined legacy state"))
    );
    let journals = source.state_dir.join("journal");
    files::tree(&journals, 0o700).unwrap();
    fs::write(
        journals.join("unknown.journal.toml"),
        b"malformed, unknown recovery scope",
    )
    .unwrap();
    assert!(
        plan_migration(
            &fixture.bundle,
            &fixture.layout,
            &source,
            None,
            vec![],
            None
        )
        .is_err()
    );
}

struct Fake<'a> {
    fixture: &'a Fixture,
    installed: RefCell<BTreeMap<String, Value>>,
    calls: RefCell<Vec<String>>,
    fail: RefCell<Option<String>>,
    lost_report: RefCell<bool>,
}

#[test]
fn a_product_selection_can_install_without_an_aw_executable_or_dependency() {
    let mut fixture = Fixture::new();
    let descriptor = fixture
        .bundle
        .root
        .join("packages/anolisa-cli/package.json");
    let mut package: Value = serde_json::from_slice(&fs::read(&descriptor).unwrap()).unwrap();
    package["dependencies"] = json!([]);
    fs::write(&descriptor, serde_json::to_vec(&package).unwrap()).unwrap();
    let digest = files::digest(&descriptor).unwrap();
    fixture.change_manifest(|manifest| {
        manifest["schema_version"] = json!(2);
        manifest["min_installer_version"] = json!("0.3.19");
        manifest["packages"].as_array_mut().unwrap().retain(|package| package["package_id"] != "aw-core");
        manifest["packages"][0]["description_sha256"] = json!(digest);
        manifest["entrypoints"].as_object_mut().unwrap().remove("aw");
        manifest["distribution"] = json!({
            "schema_version":1,
            "default_profiles":{"user":"cli-only","system":"cli-only"},
            "profiles":{"cli-only":{"packages":["anolisa-cli"],"scopes":["user","system"],"preset":"disabled"}},
            "components":{"anolisa-cli":["anolisa-cli"]},
            "presets":{"disabled":{"services":[]}}
        });
    });
    fs::remove_dir_all(fixture.bundle.root.join("packages/aw-core")).unwrap();
    fixture.bundle = Bundle::read(&fixture.bundle.root)
        .unwrap()
        .select(Scope::User, None, &[])
        .unwrap();
    fixture.layout = ReleaseLayout::resolve(
        &fixture.bundle.manifest,
        Scope::User,
        &Environment::from_values(fixture.root.join("home"), |_| None).unwrap(),
    )
    .unwrap();
    let backend = fixture.backend();
    let report = install::execute(&fixture.bundle, &fixture.layout, false, &backend).unwrap();
    assert!(report.succeeded(), "{:?}", report.error);
    assert_eq!(report.packages.len(), 1);
    assert!(fixture.layout.prefix.join("bin/anolisa").is_file());
    assert!(!fixture.layout.prefix.join("bin/aw").exists());
}

#[test]
fn historical_product_mutation_fields_remain_readable() {
    let result: PackageResult = serde_json::from_value(json!({
        "package_id":"anolisa-cli","version":"1.0.0","committed":true,
        "observation":"committed","verified":true,
        "aw":{"format":1,"mutation":{"committed":true}}
    }))
    .unwrap();
    assert_eq!(
        result.package_operation.unwrap()["mutation"]["committed"],
        true
    );
}
impl Backend for Fake<'_> {
    fn invoke(&self, operation: Operation<'_>) -> Result<Output> {
        match operation {
            Operation::Inventory => {
                if self.installed.borrow().is_empty() {
                    assert!(
                        self.fixture.layout.resources.parent().unwrap().is_dir(),
                        "The package store requires the new resources directory's direct parent before planning"
                    );
                    assert!(
                        !self.fixture.layout.resources.exists(),
                        "The package store must create and record its real resource root"
                    );
                }
                let installations = if self.installed.borrow().is_empty() {
                    Vec::new()
                } else {
                    vec![
                        json!({"prefix":self.fixture.layout.prefix,"resources":self.fixture.layout.resources,"installation_id":"fixture-installation","scope":"user","manager":"raw","registration":"registered","packages":self.installed.borrow().values().collect::<Vec<_>>()}),
                    ]
                };
                Ok(Output {
                    success: true,
                    report: json!({"format":1,"status":"succeeded","result":{"kind":"packages_listed","inventory":{"installations":installations}}}),
                })
            }
            Operation::Doctor => {
                let mut issues = Vec::new();
                for package in self.installed.borrow().values() {
                    for (name, payload) in package["files"].as_object().unwrap() {
                        let path = match name.strip_prefix("share/") {
                            Some(relative) => self.fixture.layout.resources.join(relative),
                            None => self.fixture.layout.prefix.join(name),
                        };
                        if files::digest(&path).ok().as_deref() != payload["sha256"].as_str() {
                            issues.push(json!({"path":path,"message":"modified"}));
                        }
                    }
                }
                Ok(Output {
                    success: issues.is_empty(),
                    report: json!({"format":1,"status":if issues.is_empty() {"succeeded"} else {"failed"},"result":{"kind":"package_files_checked","report":{"issues":issues}}}),
                })
            }
            Operation::Install(directory) => {
                let reference = self
                    .fixture
                    .bundle
                    .manifest
                    .packages
                    .iter()
                    .find(|package| {
                        self.fixture.bundle.root.join(&package.path) == Path::new(directory)
                    })
                    .unwrap();
                self.calls.borrow_mut().push(reference.package_id.clone());
                if self.fail.borrow().as_deref() == Some(reference.package_id.as_str()) {
                    return Ok(Output {
                        success: false,
                        report: json!({"format":1,"status":"failed","mutation":{"committed":false,"recovery":"complete"}}),
                    });
                }
                let package = self
                    .fixture
                    .bundle
                    .packages
                    .iter()
                    .find(|package| package.package_id == reference.package_id)
                    .unwrap();
                for (name, payload) in &package.files {
                    let target = match name.strip_prefix("share/") {
                        Some(relative) => self.fixture.layout.resources.join(relative),
                        None => self.fixture.layout.prefix.join(name),
                    };
                    files::tree(target.parent().unwrap(), 0o755)?;
                    fs::copy(Path::new(directory).join("payload").join(name), &target).unwrap();
                    fs::set_permissions(&target, fs::Permissions::from_mode(payload.mode)).unwrap();
                }
                self.installed.borrow_mut().insert(package.package_id.clone(), json!({"package_id":package.package_id,"version":package.version,"files":package.files,"dependencies":package.dependencies.iter().map(|dependency| json!({"package_id":dependency.package_id,"version":dependency.version,"installation_id":"fixture-installation"})).collect::<Vec<_>>(),"source":{"bundle_sha256":reference.description_sha256}}));
                if *self.lost_report.borrow() {
                    return Err(Error::Invalid(
                        "injected lost package report after actual commit".into(),
                    ));
                }
                Ok(Output {
                    success: true,
                    report: json!({"format":1,"status":"succeeded","mutation":{"committed":true,"recovery":"not_attempted"}}),
                })
            }
        }
    }
}

#[test]
fn full_verification_and_release_dry_run_do_not_create_layout_or_install_packages() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    let report = install::execute(&fixture.bundle, &fixture.layout, true, &backend).unwrap();
    assert!(report.succeeded());
    assert!(report.dry_run);
    assert!(backend.calls.borrow().is_empty());
    assert!(!fixture.layout.prefix.exists());
    assert!(!fixture.layout.resources.exists());
    assert!(!fixture.layout.state.exists());
    assert!(!fixture.layout.bin.exists());
}

#[test]
fn interrupted_package_sequence_resumes_only_after_observed_health_checks() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    *backend.fail.borrow_mut() = Some("anolisa-cli".into());
    let first = install::execute(&fixture.bundle, &fixture.layout, false, &backend).unwrap();
    assert!(!first.succeeded());
    assert!(first.packages[0].committed);
    assert!(!first.packages[1].committed);
    assert!(!first.activated);
    assert!(!fixture.layout.current().exists());
    assert!(first.record_path.is_file());
    *backend.fail.borrow_mut() = None;
    let resumed = install::execute(&fixture.bundle, &fixture.layout, false, &backend).unwrap();
    assert!(resumed.succeeded(), "{:?}", resumed.error);
    assert!(
        resumed
            .packages
            .iter()
            .all(|package| package.committed && package.verified)
    );
    assert_eq!(
        fs::read_link(fixture.layout.current()).unwrap(),
        fixture.layout.prefix
    );
    assert_eq!(
        backend
            .calls
            .borrow()
            .iter()
            .filter(|id| *id == "aw-core")
            .count(),
        1
    );
    assert_eq!(
        status(&fixture.bundle, &fixture.layout)
            .unwrap()
            .unwrap()
            .phase,
        "active"
    );
    let output = std::process::Command::new(fixture.layout.bin.join("anolisa"))
        .arg("sample")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains(&format!(
        "{}/bin/anolisa:sample",
        fixture.layout.prefix.display()
    )));
}

#[test]
fn resume_never_skips_a_damaged_previously_committed_package() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    *backend.fail.borrow_mut() = Some("anolisa-cli".into());
    install::execute(&fixture.bundle, &fixture.layout, false, &backend).unwrap();
    fs::write(
        fixture.layout.prefix.join("bin/aw"),
        b"changed installed AW",
    )
    .unwrap();
    *backend.fail.borrow_mut() = None;
    let report = install::execute(&fixture.bundle, &fixture.layout, false, &backend).unwrap();
    assert!(!report.succeeded());
    assert!(
        report
            .error
            .as_deref()
            .unwrap()
            .contains("verification failed")
    );
    assert!(!fixture.layout.current().exists());
    assert_eq!(backend.calls.borrow().len(), 2);
}

#[test]
fn unknown_legacy_entrypoint_is_preserved_before_the_first_product_write() {
    let fixture = Fixture::new();
    files::tree(&fixture.layout.bin, 0o755).unwrap();
    let old = fixture.layout.bin.join("anolisa");
    fs::write(&old, b"existing standalone CLI").unwrap();
    fs::set_permissions(&old, fs::Permissions::from_mode(0o755)).unwrap();
    let backend = fixture.backend();
    let error = install::execute(&fixture.bundle, &fixture.layout, false, &backend).unwrap_err();
    assert!(error.to_string().contains("unverified ownership"));
    assert_eq!(fs::read(old).unwrap(), b"existing standalone CLI");
    assert!(backend.calls.borrow().is_empty());
    assert!(!fixture.layout.state.exists());
}

#[test]
fn symlink_payload_and_changed_input_are_rejected_without_writes() {
    let fixture = Fixture::new();
    let source = fixture.bundle.root.join("packages/aw-core/payload/bin/aw");
    fs::remove_file(&source).unwrap();
    symlink("/bin/sh", &source).unwrap();
    assert!(
        Bundle::read(&fixture.bundle.root)
            .unwrap_err()
            .to_string()
            .contains("symbolic link")
    );
    assert!(!fixture.layout.state.exists());
}

#[test]
fn immutable_release_identity_is_not_reused_for_different_bytes() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    install::execute(&fixture.bundle, &fixture.layout, false, &backend).unwrap();
    fixture.change_manifest(|value| value["product_version"] = json!("1.0.1"));
    let changed = Bundle::read(&fixture.bundle.root).unwrap();
    assert!(
        status(&changed, &fixture.layout)
            .unwrap_err()
            .to_string()
            .contains("conflicts")
    );
    assert_eq!(backend.calls.borrow().len(), 2);
}

#[test]
fn dependencies_must_precede_dependents_and_entrypoints_must_be_owned() {
    let fixture = Fixture::new();
    fixture.change_manifest(|value| value["packages"].as_array_mut().unwrap().reverse());
    assert!(
        Bundle::read(&fixture.bundle.root)
            .unwrap_err()
            .to_string()
            .contains("requires earlier")
    );
    let fixture = Fixture::new();
    fixture.change_manifest(|value| value["entrypoints"]["unknown"] = json!("bin/not-owned"));
    assert!(
        Bundle::read(&fixture.bundle.root)
            .unwrap_err()
            .to_string()
            .contains("owned executable")
    );
}

#[test]
fn duplicate_json_keys_are_not_silently_collapsed() {
    assert!(
        files::decode::<Value>(br#"{"files":{"bin/a":1,"bin/a":2}}"#)
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
}

#[test]
fn xdg_overrides_are_resolved_without_rebasing_programs_or_aw_system_database() {
    let fixture = Fixture::new();
    let environment = Environment::from_values(fixture.root.join("home"), |name| match name {
        "XDG_STATE_HOME" => Some(fixture.root.join("custom-state")),
        "XDG_DATA_HOME" => Some(PathBuf::from("relative-ignored")),
        "XDG_CONFIG_HOME" => Some(PathBuf::new()),
        _ => None,
    })
    .unwrap();
    let user = ReleaseLayout::resolve(&fixture.bundle.manifest, Scope::User, &environment).unwrap();
    assert_eq!(user.state, fixture.root.join("custom-state/anolisa"));
    assert_eq!(
        user.package_database,
        fixture.root.join("custom-state/anolisa/packages")
    );
    assert_eq!(user.data, fixture.root.join("home/.local/share/anolisa"));
    assert_eq!(user.bin, fixture.root.join("home/.local/bin"));
    assert!(
        user.prefix
            .starts_with(fixture.root.join("home/.local/lib"))
    );
    let system =
        ReleaseLayout::resolve(&fixture.bundle.manifest, Scope::System, &environment).unwrap();
    assert_eq!(
        system.prefix,
        PathBuf::from("/usr/local/lib/anolisa/releases/test-release")
    );
    assert_eq!(
        system.resources,
        PathBuf::from("/usr/local/share/anolisa/releases/test-release")
    );
    assert_eq!(system.bin, PathBuf::from("/usr/local/bin"));
    assert_eq!(system.cache, PathBuf::from("/var/cache/anolisa"));
    assert_eq!(system.config, PathBuf::from("/usr/local/etc/anolisa"));
    assert_eq!(system.state, PathBuf::from("/var/lib/anolisa"));
    assert_eq!(
        system.package_database,
        PathBuf::from("/var/lib/anolisa/packages")
    );
}

#[test]
fn shared_home_launchers_select_architecture_before_freezing_real_prefix() {
    let fixture = Fixture::new();
    let mut other = fixture.bundle.manifest.clone();
    other.arch = if other.arch == "x86_64" {
        "aarch64"
    } else {
        "x86_64"
    }
    .into();
    let environment = Environment::from_values(fixture.root.join("home"), |_| None).unwrap();
    let other = ReleaseLayout::resolve(&other, Scope::User, &environment).unwrap();
    assert_eq!(
        activate::script(&fixture.layout, "bin/anolisa"),
        activate::script(&other, "bin/anolisa")
    );
    assert_ne!(fixture.layout.prefix, other.prefix);
}

#[test]
fn scope_inventory_selects_only_the_final_real_prefix_and_allows_other_releases() {
    struct Inventory(Value);
    impl Backend for Inventory {
        fn invoke(&self, operation: Operation<'_>) -> Result<Output> {
            assert!(matches!(operation, Operation::Inventory));
            Ok(Output {
                success: true,
                report: self.0.clone(),
            })
        }
    }
    let fixture = Fixture::new();
    let backend = Inventory(
        json!({"format":1,"status":"succeeded","result":{"kind":"packages_listed","inventory":{"database_path":fixture.layout.package_database.join("installed.json"),"generation":2,"source":"database","installations":[{"prefix":fixture.root.join("other-release"),"packages":[{"package_id":"unrelated"}]},{"prefix":fixture.layout.prefix,"resources":fixture.layout.resources,"installation_id":"fixture-installation","scope":"user","manager":"raw","registration":"registered","packages":[{"package_id":"aw-core"}]}]}}}),
    );
    let packages = store::inventory(&backend, &fixture.layout).unwrap();
    assert_eq!(
        packages.installation_id.as_deref(),
        Some("fixture-installation")
    );
    assert_eq!(packages.packages, vec![json!({"package_id":"aw-core"})]);
    let empty = Inventory(
        json!({"format":1,"status":"succeeded","result":{"kind":"packages_listed","inventory":{"generation":null,"source":"absent","installations":[]}}}),
    );
    assert!(
        store::inventory(&empty, &fixture.layout)
            .unwrap()
            .packages
            .is_empty()
    );
}

#[test]
fn a_lost_package_commit_report_remains_unknown_until_database_and_files_are_checked() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    *backend.lost_report.borrow_mut() = true;
    let interrupted = install::execute(&fixture.bundle, &fixture.layout, false, &backend).unwrap();
    assert!(!interrupted.succeeded());
    assert_eq!(interrupted.packages[0].observation, "unknown");
    assert!(backend.installed.borrow().contains_key("aw-core"));
    *backend.lost_report.borrow_mut() = false;
    let resumed = install::execute(&fixture.bundle, &fixture.layout, false, &backend).unwrap();
    assert!(resumed.succeeded(), "{:?}", resumed.error);
    assert_eq!(resumed.packages[0].observation, "committed");
    assert_eq!(
        backend
            .calls
            .borrow()
            .iter()
            .filter(|id| *id == "aw-core")
            .count(),
        1
    );
}

fn resume_with_changed_session_runtime(initial_runtime: bool) {
    let fixture = Fixture::new().with_sec_core();
    let runtime = fixture.root.join("session-runtime");
    files::tree(&runtime, 0o700).unwrap();
    let environment = Environment::from_values(fixture.root.join("home"), |name| {
        (name == "XDG_RUNTIME_DIR").then(|| runtime.clone())
    })
    .unwrap();
    let with_runtime =
        ReleaseLayout::resolve(&fixture.bundle.manifest, Scope::User, &environment).unwrap();
    let without_runtime = &fixture.layout;
    let (initial, next) = if initial_runtime {
        (&with_runtime, without_runtime)
    } else {
        (without_runtime, &with_runtime)
    };
    assert_eq!(initial.config, next.config);
    assert_eq!(initial.state, next.state);
    let backend = fixture.backend();
    let first = install::execute(&fixture.bundle, initial, false, &backend).unwrap();
    assert!(first.succeeded(), "{:?}", first.error);
    let path = initial
        .config
        .join("releases")
        .join(&initial.arch_id)
        .join("test-release.providers.json");
    let original = fs::read(&path).unwrap();
    let value: Value = serde_json::from_slice(&original).unwrap();
    assert_eq!(value["kind"], "anolisa-provider-template");
    assert_eq!(value["runtime_context"]["scope"], "invoking-user");
    assert_eq!(
        value["providers"]["sec-core"]["transport_template"]["argv"][4],
        "{user_runtime_dir}/agent-sec-core.sock"
    );
    assert!(value["providers"]["sec-core"].get("transport").is_none());
    assert_eq!(
        files::digest_bytes(&original),
        first.provider_configuration.sha256
    );
    let resumed = install::execute(&fixture.bundle, next, false, &backend).unwrap();
    assert!(resumed.succeeded(), "{:?}", resumed.error);
    assert_eq!(
        resumed.provider_configuration.sha256,
        first.provider_configuration.sha256
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(
        resumed
            .warnings
            .iter()
            .any(|warning| warning.contains("unrendered invoking-user-XDG templates"))
    );
    assert_eq!(backend.calls.borrow().len(), 4);
    assert!(
        !with_runtime.runtime.as_ref().unwrap().exists(),
        "installation must not create a persistent replacement for session runtime"
    );

    let changed = [original.as_slice(), b"\n"].concat();
    fs::write(&path, &changed).unwrap();
    let record = fs::read(&resumed.record_path).unwrap();
    let error = install::execute(&fixture.bundle, next, false, &backend).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("existing release Provider definitions changed"),
        "{error}"
    );
    assert_eq!(fs::read(&path).unwrap(), changed);
    assert_eq!(fs::read(&resumed.record_path).unwrap(), record);
    assert_eq!(backend.calls.borrow().len(), 4);
}

#[test]
fn retry_after_headless_install_preserves_session_independent_provider_template() {
    resume_with_changed_session_runtime(false);
}

#[test]
fn retry_without_runtime_preserves_session_independent_provider_template() {
    resume_with_changed_session_runtime(true);
}

#[test]
fn resources_are_installed_separately_and_direct_private_commands_use_the_verified_alias() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    let report = install::execute(&fixture.bundle, &fixture.layout, false, &backend).unwrap();
    assert!(report.succeeded(), "{:?}", report.error);
    assert_eq!(report.resources, fixture.layout.resources);
    assert_eq!(report.resource_alias, fixture.layout.resource_alias());
    assert_eq!(
        fs::read_link(&report.resource_alias).unwrap(),
        report.resources
    );
    assert!(
        fs::symlink_metadata(&report.resource_alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read(fixture.layout.resources.join("anolisa/fixture.txt")).unwrap(),
        b"release resources\n"
    );
    let output = std::process::Command::new(fixture.layout.prefix.join("bin/anolisa"))
        .arg("resource-test")
        .env_remove("ANOLISA_RELEASE_PREFIX")
        .env_remove("ANOLISA_RELEASE_RESOURCES")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"release resources\n");
    let runtime = fixture.root.join("caller-runtime");
    let data = fixture.root.join("caller-data");
    let home = fixture.root.join("caller-home");
    let output = std::process::Command::new(fixture.layout.bin.join("anolisa"))
        .arg("context-test")
        .env("HOME", &home)
        .env("XDG_DATA_HOME", &data)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("ANOLISA_RELEASE_PREFIX", "/untrusted/program-hint")
        .env("ANOLISA_RELEASE_RESOURCES", "/untrusted/resource-hint")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "{}\n{}\n{}\n{}\n{}\n",
            home.display(),
            data.display(),
            runtime.display(),
            fixture.layout.prefix.display(),
            fixture.layout.resources.display()
        )
    );
}

#[test]
fn changed_resource_alias_is_preserved_and_refused_before_more_package_operations() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    let report = install::execute(&fixture.bundle, &fixture.layout, false, &backend).unwrap();
    assert!(report.succeeded(), "{:?}", report.error);
    fs::remove_file(&report.resource_alias).unwrap();
    let unknown = fixture.root.join("unrelated-resources");
    symlink(&unknown, &report.resource_alias).unwrap();
    let record = fs::read(&report.record_path).unwrap();
    let error = install::execute(&fixture.bundle, &fixture.layout, false, &backend).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("resource alias has unknown ownership or target"),
        "{error}"
    );
    assert_eq!(fs::read_link(&report.resource_alias).unwrap(), unknown);
    assert_eq!(fs::read(&report.record_path).unwrap(), record);
    assert_eq!(backend.calls.borrow().len(), 2);
}

#[test]
fn missing_product_alias_can_be_republished_after_health_checks_without_reinstalling_packages() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    let report = install::execute(&fixture.bundle, &fixture.layout, false, &backend).unwrap();
    assert!(report.succeeded(), "{:?}", report.error);
    fs::remove_file(&report.resource_alias).unwrap();
    let resumed = install::execute(&fixture.bundle, &fixture.layout, false, &backend).unwrap();
    assert!(resumed.succeeded(), "{:?}", resumed.error);
    assert_eq!(
        fs::read_link(&report.resource_alias).unwrap(),
        fixture.layout.resources
    );
    assert_eq!(backend.calls.borrow().len(), 2);
}

#[test]
fn unknown_preexisting_resource_root_is_not_adopted() {
    let fixture = Fixture::new();
    files::tree(&fixture.layout.resources, 0o755).unwrap();
    let backend = fixture.backend();
    let error = install::execute(&fixture.bundle, &fixture.layout, false, &backend).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("resources directory already exists"),
        "{error}"
    );
    assert!(!fixture.layout.prefix.exists());
    assert!(!fixture.layout.state.exists());
    assert!(backend.calls.borrow().is_empty());
}

#[test]
fn system_provider_templates_use_invoking_user_xdg_and_never_the_root_install_session() {
    let fixture = Fixture::new().with_sec_core();
    let environment = Environment::from_values(fixture.root.join("home"), |_| None).unwrap();
    let system =
        ReleaseLayout::resolve(&fixture.bundle.manifest, Scope::System, &environment).unwrap();
    let template = configuration::document(&fixture.bundle, &system).unwrap();
    assert_eq!(template["runtime_context"]["scope"], "invoking-user");
    assert_eq!(
        template["runtime_context"]["user_runtime_dir"]["environment"],
        "XDG_RUNTIME_DIR"
    );
    assert!(template["runtime_context"]["user_runtime_dir"]["fallback"].is_null());
    assert_eq!(
        template["providers"]["sec-core"]["transport_template"]["argv"][4],
        "{user_runtime_dir}/agent-sec-core.sock"
    );
    assert!(template["providers"]["sec-core"].get("transport").is_none());
    let bytes = serde_json::to_string(&template).unwrap();
    assert!(!bytes.contains("/root"));
    assert!(!bytes.contains("/var/lib/anolisa/data"));
    assert!(!bytes.contains("/run/anolisa"));
}

#[test]
fn new_provider_template_permissions_follow_explicit_scope_without_changing_existing_files() {
    let fixture = Fixture::new().with_sec_core();
    for (scope, directory_mode, file_mode) in
        [(Scope::User, 0o700, 0o600), (Scope::System, 0o755, 0o644)]
    {
        let mut layout = fixture.layout.clone();
        layout.scope = scope;
        // Exercise the system permission policy in a private fixture, not privileged FHS writes.
        layout.config = fixture
            .root
            .join(format!("{}-configuration", scope.as_str()));
        let context = configuration::prepare(&fixture.bundle, &layout).unwrap();
        configuration::publish(&fixture.bundle, &layout, &context, &mut Vec::new()).unwrap();
        let directory = layout.config.join("releases").join(&layout.arch_id);
        let path = directory.join("test-release.providers.json");
        assert_eq!(
            fs::metadata(&layout.config).unwrap().permissions().mode() & 0o777,
            directory_mode
        );
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            directory_mode
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            file_mode
        );
        let changed = b"administrator edited definitions\n";
        fs::write(&path, changed).unwrap();
        let error = configuration::publish(&fixture.bundle, &layout, &context, &mut Vec::new())
            .unwrap_err();
        assert!(
            error.to_string().contains("Provider definitions changed"),
            "{error}"
        );
        assert_eq!(fs::read(&path).unwrap(), changed);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            file_mode
        );
    }
}

#[test]
fn fresh_install_prepares_resource_parent_before_packages_create_the_resource_root() {
    let fixture = Fixture::new();
    assert!(!fixture.layout.resources.parent().unwrap().exists());
    assert!(!fixture.layout.resources.exists());
    let backend = fixture.backend();
    // Fake's first inventory call checks that the parent exists and the resource root does not.
    let report = install::execute(&fixture.bundle, &fixture.layout, false, &backend).unwrap();
    assert!(report.succeeded(), "{:?}", report.error);
    assert!(fixture.layout.resources.is_dir());
    assert_eq!(backend.calls.borrow().len(), 2);
}

#[test]
fn public_version_summary_lists_only_selected_packages_and_no_private_state() {
    let mut fixture = Fixture::new().with_sec_core().with_choices();
    fixture.bundle = fixture
        .bundle
        .select(Scope::User, Some("minimal"), &[])
        .unwrap();
    let environment = Environment::from_values(fixture.root.join("home"), |_| None).unwrap();
    fixture.layout =
        ReleaseLayout::resolve(&fixture.bundle.manifest, Scope::User, &environment).unwrap();
    let backend = fixture.backend();
    let report = install::execute(&fixture.bundle, &fixture.layout, false, &backend).unwrap();
    assert!(report.succeeded(), "{:?}", report.error);
    let path = fixture
        .layout
        .resources
        .join("anolisa/installed-product.json");
    let bytes = fs::read(&path).unwrap();
    let summary: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(summary["packages"].as_array().unwrap().len(), 2);
    assert!(
        !String::from_utf8(bytes)
            .unwrap()
            .contains(&fixture.root.to_string_lossy().to_string())
    );
    assert!(
        summary["packages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|package| package["package_id"] != "sec-core")
    );
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o644
    );
    assert_eq!(
        fs::metadata(&fixture.layout.state)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

#[test]
fn changed_public_version_summary_refuses_resume_without_overwriting_it() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    assert!(
        install::execute(&fixture.bundle, &fixture.layout, false, &backend)
            .unwrap()
            .succeeded()
    );
    let path = fixture
        .layout
        .resources
        .join("anolisa/installed-product.json");
    fs::write(&path, b"unknown administrator contents\n").unwrap();
    let error = install::execute(&fixture.bundle, &fixture.layout, false, &backend).unwrap_err();
    assert!(
        error.to_string().contains("public product summary"),
        "{error}"
    );
    assert_eq!(fs::read(path).unwrap(), b"unknown administrator contents\n");
}
