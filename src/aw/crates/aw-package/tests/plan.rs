//! Installation plans describe existing Preview operations without reserving files.

use aw_package::{install, plan_install, Component, InventoryState, Manifest, Payload};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    bundle: PathBuf,
    prefix: PathBuf,
}

impl Fixture {
    fn new(provider: bool) -> Self {
        let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/package-plan");
        fs::create_dir_all(&target).unwrap();
        let directory = tempfile::tempdir_in(target).unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let root = directory.path().canonicalize().unwrap();
        let bundle = root.join("bundle");
        let names: &[&str] = if provider {
            &[
                "libexec/aw/providers/tokenless/aw-provider-tokenless",
                "libexec/aw/providers/tokenless/tokenless",
                "libexec/aw/providers/tokenless/rtk",
            ]
        } else {
            &["bin/aw", "bin/aw-package"]
        };
        let mut files = BTreeMap::new();
        for name in names {
            let path = bundle.join("payload").join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"unexecuted payload").unwrap();
            files.insert(
                (*name).to_owned(),
                Payload {
                    sha256: aw_package::digest(&path).unwrap(),
                    mode: 0o755,
                },
            );
        }
        let manifest = Manifest {
            format: 1,
            components: vec![Component {
                format: 1,
                component: if provider {
                    "aw-provider-tokenless"
                } else {
                    "aw-core"
                }
                .into(),
                version: "0.1.0-preview.1".into(),
                source_commit: "a".repeat(40),
                os: "linux".into(),
                arch: std::env::consts::ARCH.into(),
                provider_protocol: "aw-provider/v1alpha1".into(),
                requires: if provider {
                    BTreeMap::from([("aw-core".into(), "0.1.0-preview.1".into())])
                } else {
                    BTreeMap::new()
                },
                files,
            }],
        };
        fs::write(
            bundle.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        Self {
            prefix: root.join("prefix"),
            root,
            bundle,
            _directory: directory,
        }
    }
}

#[test]
fn absent_prefix_plan_has_fixed_categories_without_creating_owned_resources() {
    let f = Fixture::new(false);
    let before = fs::read_dir(&f.root).unwrap().count();
    let plan = plan_install(&f.bundle, &f.prefix).unwrap();
    assert_eq!(plan.existing, InventoryState::Absent);
    assert!(plan.conflicts.is_empty());
    assert_eq!(plan.source.kind, "local_bundle");
    assert!(plan.source.payload_verified);
    assert!(!plan.source.publisher_verified);
    assert_eq!(plan.components[0].name, "aw-core");
    assert!(plan.components[0].installed_version.is_none());
    assert_eq!(plan.files.payloads.len(), 2);
    assert_eq!(
        plan.files.receipts,
        vec![f.prefix.join(".aw-packages/aw-core.json")]
    );
    assert_eq!(plan.files.lock, f.prefix.join(".aw-install.lock"));
    assert!(plan.files.creates_prefix);
    assert!(!plan.privileges.automatic_elevation);
    assert!(!plan.activation.starts_services);
    assert!(!plan.activation.registers_providers);
    assert!(!plan.activation.connects_agents);
    assert!(!plan.not_checked.is_empty());
    assert!(!f.prefix.exists());
    assert_eq!(fs::read_dir(&f.root).unwrap().count(), before);
    let json = serde_json::to_value(plan).unwrap();
    for category in ["source", "components", "files", "privileges", "activation"] {
        assert!(json.get(category).is_some(), "{category}");
    }
    assert!(json.get("can_execute").is_none());
}

#[test]
fn existing_receipts_and_changed_payloads_surface_conflicts_without_repair() {
    let f = Fixture::new(false);
    install(&f.bundle, &f.prefix).unwrap();
    let receipt = f.prefix.join(".aw-packages/aw-core.json");
    let before = fs::read(&receipt).unwrap();
    let plan = plan_install(&f.bundle, &f.prefix).unwrap();
    assert_eq!(plan.existing, InventoryState::Installed);
    assert!(!plan.files.creates_prefix);
    assert_eq!(
        plan.components[0].installed_version.as_deref(),
        Some("0.1.0-preview.1")
    );
    assert!(plan
        .conflicts
        .iter()
        .any(|message| message.contains("already installed")));
    fs::write(f.prefix.join("bin/aw"), b"changed user file").unwrap();
    let plan = plan_install(&f.bundle, &f.prefix).unwrap();
    assert!(plan
        .conflicts
        .iter()
        .any(|message| message.contains("modified or missing")));
    assert_eq!(
        fs::read(f.prefix.join("bin/aw")).unwrap(),
        b"changed user file"
    );
    assert_eq!(fs::read(receipt).unwrap(), before);
}

#[test]
fn unmanaged_prefix_and_missing_dependencies_cannot_appear_ready() {
    let f = Fixture::new(false);
    fs::create_dir(&f.prefix).unwrap();
    let plan = plan_install(&f.bundle, &f.prefix).unwrap();
    assert_eq!(plan.existing, InventoryState::Unmanaged);
    assert!(plan
        .conflicts
        .iter()
        .any(|message| message.contains("without AW package receipts")));
    assert_eq!(fs::read_dir(&f.prefix).unwrap().count(), 0);
    let provider = Fixture::new(true);
    let plan = plan_install(&provider.bundle, &provider.prefix).unwrap();
    assert!(plan
        .conflicts
        .iter()
        .any(|message| message.contains("requires matching aw-core")));
    assert!(!provider.prefix.exists());
}

#[test]
fn corrupt_or_oversized_bundle_metadata_never_creates_a_prefix() {
    let f = Fixture::new(false);
    fs::write(f.bundle.join("payload/bin/aw"), b"corrupt").unwrap();
    assert!(plan_install(&f.bundle, &f.prefix).is_err());
    assert!(!f.prefix.exists());
    let f = Fixture::new(false);
    fs::OpenOptions::new()
        .write(true)
        .open(f.bundle.join("manifest.json"))
        .unwrap()
        .set_len(4 * 1024 * 1024 + 1)
        .unwrap();
    assert!(plan_install(&f.bundle, &f.prefix).is_err());
    assert!(!f.prefix.exists());
}
