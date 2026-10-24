//! Injected AW reports exercise offline contracts and product recovery without certifying Linux AW.

use super::{
    aw::{Backend, Output},
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

struct Fake<'a> {
    fixture: &'a Fixture,
    installed: RefCell<BTreeMap<String, Value>>,
    calls: RefCell<Vec<String>>,
    fail: RefCell<Option<String>>,
    lost_report: RefCell<bool>,
}
impl Backend for Fake<'_> {
    fn invoke(&self, args: &[String]) -> Result<Output> {
        assert!(args.windows(2).any(|args| args == ["--scope", "user"]));
        assert!(args.iter().any(|arg| arg == "--json"));
        match args[0].as_str() {
            "list" => {
                if self.installed.borrow().is_empty() {
                    assert!(
                        self.fixture.layout.resources.parent().unwrap().is_dir(),
                        "AW requires the new resources directory's direct parent before planning"
                    );
                    assert!(
                        !self.fixture.layout.resources.exists(),
                        "AW must create and record its real resource root"
                    );
                }
                assert!(
                    !args.iter().any(|arg| arg == "--prefix"),
                    "fresh scope listings must not request a nonexistent prefix"
                );
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
            "doctor" => {
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
            "install" => {
                assert!(args.windows(2).any(|args| args[0] == "--resources-dir"
                    && Path::new(&args[1]) == self.fixture.layout.resources));
                let directory =
                    args.windows(2).find(|args| args[0] == "--bundle").unwrap()[1].as_str();
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
                        "injected lost AW report after actual commit".into(),
                    ));
                }
                Ok(Output {
                    success: true,
                    report: json!({"format":1,"status":"succeeded","mutation":{"committed":true,"recovery":"not_attempted"}}),
                })
            }
            other => panic!("unexpected test AW operation {other}"),
        }
    }
}

#[test]
fn full_verification_and_release_dry_run_do_not_create_layout_or_run_aw() {
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
        user.aw_database,
        fixture.root.join("custom-state/aw/packages")
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
    assert_eq!(system.aw_database, PathBuf::from("/var/lib/aw/packages"));
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
        fn invoke(&self, args: &[String]) -> Result<Output> {
            assert_eq!(args[0], "list");
            assert!(!args.iter().any(|argument| argument == "--prefix"));
            Ok(Output {
                success: true,
                report: self.0.clone(),
            })
        }
    }
    let fixture = Fixture::new();
    let backend = Inventory(
        json!({"format":1,"status":"succeeded","result":{"kind":"packages_listed","inventory":{"database_path":fixture.layout.aw_database.join("installed.json"),"generation":2,"source":"database","installations":[{"prefix":fixture.root.join("other-release"),"packages":[{"package_id":"unrelated"}]},{"prefix":fixture.layout.prefix,"resources":fixture.layout.resources,"installation_id":"fixture-installation","scope":"user","manager":"raw","registration":"registered","packages":[{"package_id":"aw-core"}]}]}}}),
    );
    let packages = aw::inventory(&backend, &fixture.layout).unwrap();
    assert_eq!(
        packages.installation_id.as_deref(),
        Some("fixture-installation")
    );
    assert_eq!(packages.packages, vec![json!({"package_id":"aw-core"})]);
    let empty = Inventory(
        json!({"format":1,"status":"succeeded","result":{"kind":"packages_listed","inventory":{"generation":null,"source":"absent","installations":[]}}}),
    );
    assert!(
        aw::inventory(&empty, &fixture.layout)
            .unwrap()
            .packages
            .is_empty()
    );
}

#[test]
fn a_lost_aw_commit_report_remains_unknown_until_database_and_files_are_checked() {
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
fn fresh_install_prepares_resource_parent_before_aw_creates_the_resource_root() {
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
