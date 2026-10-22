//! External local bundles retain distinct ownership, trust and runtime boundaries.

use aw_package::{
    integrity::IntegrityCode,
    open_bundle::{Bundle, Dependency, Kind, Package, Payload, Platform, Provider},
    open_install, InventoryState,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    os::{
        fd::AsRawFd,
        unix::fs::{symlink, MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    bundle: PathBuf,
    prefix: PathBuf,
    manifest: Bundle,
}

impl Fixture {
    fn new() -> Self {
        // The documented Linux gate runs as an ordinary user.
        assert_ne!(
            unsafe { libc::geteuid() },
            0,
            "run installation tests as an ordinary user"
        );
        let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/open-install");
        fs::create_dir_all(&target).unwrap();
        let directory = tempfile::tempdir_in(target).unwrap();
        let root = directory.path().canonicalize().unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let bundle = root.join("bundle");
        fs::create_dir(&bundle).unwrap();
        let manifest = Bundle {
            format: 2,
            packages: vec![Package {
                id: "example.mant".into(),
                version: "7.8.9".into(),
                kind: Kind::Tool,
                platform: Platform {
                    os: "linux".into(),
                    arch: std::env::consts::ARCH.into(),
                },
                requires: vec![],
                files: BTreeMap::new(),
                provider: None,
            }],
        };
        let mut fixture = Self {
            _directory: directory,
            prefix: root.join("prefix"),
            root,
            bundle,
            manifest,
        };
        fixture.payload("bin/mant", b"#!/bin/sh\nexit 91\n", 0o755);
        fixture.payload("share/mant/readme", b"external tool documentation", 0o644);
        fixture.save();
        fixture
    }

    fn payload(&mut self, name: &str, bytes: &[u8], mode: u32) {
        let path = self.bundle.join("payload").join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
        self.manifest.packages[0].files.insert(
            name.into(),
            Payload {
                size: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(bytes)),
                mode,
            },
        );
    }

    fn save(&self) {
        fs::write(
            self.bundle.join("manifest.json"),
            serde_json::to_vec_pretty(&self.manifest).unwrap(),
        )
        .unwrap();
    }

    fn install(&self) -> open_install::Receipt {
        open_install::install(&self.bundle, &self.prefix, &AtomicBool::new(false)).unwrap()
    }

    fn entries(&self) -> Vec<String> {
        let mut entries = fs::read_dir(&self.root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        entries.sort();
        entries
    }
}

#[test]
fn plan_is_read_only_and_records_no_activation_or_publisher_trust() {
    let fixture = Fixture::new();
    let before = fixture.entries();
    let plan = open_install::plan(&fixture.bundle, &fixture.prefix).unwrap();
    assert_eq!(fixture.entries(), before);
    assert!(!fixture.prefix.exists());
    assert_eq!(plan.existing, InventoryState::Absent);
    assert!(plan.conflicts.is_empty());
    assert_eq!(plan.components[0].name, "example.mant");
    assert_eq!(plan.components[0].version, "7.8.9");
    assert!(plan.source.payload_verified);
    assert!(!plan.source.publisher_verified);
    assert!(!plan.privileges.automatic_elevation);
    assert!(!plan.activation.starts_services);
    assert!(!plan.activation.registers_providers);
    assert!(!plan.activation.connects_agents);
}

#[test]
fn install_publishes_a_complete_independently_versioned_receipt_without_execution() {
    let mut fixture = Fixture::new();
    let marker = fixture.root.join("executed");
    fixture.payload(
        "bin/mant",
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()).as_bytes(),
        0o755,
    );
    fixture.save();
    let receipt = fixture.install();
    assert!(!marker.exists());
    assert_eq!(receipt.receipt_format, 2);
    assert_eq!(receipt.backend, "raw");
    assert_eq!(receipt.scope.kind, "user");
    assert_eq!(receipt.scope.uid, unsafe { libc::geteuid() });
    assert!(!receipt.source.publisher_verified);
    assert_eq!(receipt.packages[0].id, "example.mant");
    assert_eq!(receipt.packages[0].version, "7.8.9");
    assert_eq!(
        receipt,
        open_install::read_receipt(&fixture.prefix).unwrap()
    );
    assert_eq!(
        fixture.prefix.join("bin/mant").metadata().unwrap().mode() & 0o7777,
        0o755
    );
    assert_eq!(
        fixture
            .prefix
            .join(".aw-installation.json")
            .metadata()
            .unwrap()
            .mode()
            & 0o7777,
        0o600
    );
    assert!(!fixture.prefix.join(".aw-packages").exists());
    assert!(!fixture.prefix.join(".aw-install.lock").exists());
    assert_eq!(fixture.entries(), vec!["bundle", "prefix"]);
    let report = open_install::inspect(&fixture.prefix).unwrap();
    assert_eq!(report.checked_components, 1);
    assert_eq!(report.checked_files, 2);
    assert_eq!(report.unhealthy_components, 0);
}

#[test]
fn provider_declaration_and_bundle_local_dependency_do_not_activate_anything() {
    let mut fixture = Fixture::new();
    fixture.manifest.packages[0].kind = Kind::Provider;
    fixture.manifest.packages[0].provider = Some(Provider {
        protocol: "aw-provider/v1alpha1".into(),
        executable: "bin/mant".into(),
    });
    let bytes = b"library contents";
    fs::create_dir_all(fixture.bundle.join("payload/libexec/example")).unwrap();
    fs::write(fixture.bundle.join("payload/libexec/example/helper"), bytes).unwrap();
    fixture.manifest.packages[0].requires.push(Dependency {
        id: "example.helper".into(),
        version: "2026.10".into(),
    });
    fixture.manifest.packages.push(Package {
        id: "example.helper".into(),
        version: "2026.10".into(),
        kind: Kind::Tool,
        platform: fixture.manifest.packages[0].platform.clone(),
        requires: vec![],
        provider: None,
        files: BTreeMap::from([(
            "libexec/example/helper".into(),
            Payload {
                size: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(bytes)),
                mode: 0o644,
            },
        )]),
    });
    fixture.save();
    let receipt = fixture.install();
    assert_eq!(receipt.packages.len(), 2);
    assert_eq!(
        open_install::inspect(&fixture.prefix)
            .unwrap()
            .checked_components,
        2
    );
    assert!(!fixture.prefix.join("aw.yaml").exists());
    open_install::uninstall(&fixture.prefix, &AtomicBool::new(false)).unwrap();
    assert!(!fixture.prefix.exists());
}

#[test]
fn existing_and_symlink_prefixes_are_never_overwritten() {
    for symlinked in [false, true] {
        let fixture = Fixture::new();
        let target = fixture.root.join("existing");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep"), b"user content").unwrap();
        if symlinked {
            symlink(&target, &fixture.prefix).unwrap();
        } else {
            fs::create_dir(&fixture.prefix).unwrap();
        }
        assert!(!open_install::plan(&fixture.bundle, &fixture.prefix)
            .unwrap()
            .conflicts
            .is_empty());
        assert!(
            open_install::install(&fixture.bundle, &fixture.prefix, &AtomicBool::new(false))
                .is_err()
        );
        assert_eq!(fs::read(target.join("keep")).unwrap(), b"user content");
        assert!(!fixture.prefix.join(".aw-installation.json").exists());
        assert!(!fixture
            .entries()
            .iter()
            .any(|entry| entry.starts_with(".aw-")));
    }
}

#[test]
fn cancellation_or_source_changes_after_plan_leave_no_published_prefix() {
    let fixture = Fixture::new();
    assert!(open_install::plan(&fixture.bundle, &fixture.prefix)
        .unwrap()
        .conflicts
        .is_empty());
    assert!(
        open_install::install(&fixture.bundle, &fixture.prefix, &AtomicBool::new(true)).is_err()
    );
    fs::write(
        fixture.bundle.join("payload/bin/mant"),
        b"changed after planning",
    )
    .unwrap();
    assert!(
        open_install::install(&fixture.bundle, &fixture.prefix, &AtomicBool::new(false)).is_err()
    );
    assert!(!fixture.prefix.exists());
    assert_eq!(fixture.entries(), vec!["bundle"]);
}

#[test]
fn diagnostics_aggregate_missing_mode_and_checksum_findings() {
    let mut fixture = Fixture::new();
    fixture.payload("share/mant/extra", b"extra content", 0o644);
    fixture.save();
    fixture.install();
    fs::remove_file(fixture.prefix.join("share/mant/readme")).unwrap();
    fs::set_permissions(
        fixture.prefix.join("bin/mant"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    fs::write(fixture.prefix.join("share/mant/extra"), b"wrong content").unwrap();
    let report = open_install::inspect(&fixture.prefix).unwrap();
    assert_eq!(report.unhealthy_components, 1);
    let codes = report.components[0]
        .findings
        .iter()
        .map(|finding| finding.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&IntegrityCode::MissingFile));
    assert!(codes.contains(&IntegrityCode::ModeMismatch));
    assert!(codes.contains(&IntegrityCode::ChecksumMismatch));
    assert_eq!(
        fs::read(fixture.prefix.join("share/mant/extra")).unwrap(),
        b"wrong content"
    );
}

#[test]
fn receipt_trust_scope_and_mixed_formats_are_rejected() {
    for change in ["trust", "scope", "uid", "unknown", "mixed"] {
        let fixture = Fixture::new();
        fixture.install();
        let path = fixture.prefix.join(".aw-installation.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        match change {
            "trust" => value["source"]["publisher_verified"] = true.into(),
            "scope" => value["scope"]["kind"] = "system".into(),
            "uid" => value["scope"]["uid"] = 0.into(),
            "unknown" => value["install_script"] = "anything".into(),
            "mixed" => fs::create_dir(fixture.prefix.join(".aw-packages")).unwrap(),
            _ => unreachable!(),
        }
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(
            open_install::read_receipt(&fixture.prefix).is_err(),
            "{change}"
        );
        assert!(
            open_install::uninstall(&fixture.prefix, &AtomicBool::new(false)).is_err(),
            "{change}"
        );
        assert!(fixture.prefix.join("bin/mant").exists());
    }
}

#[test]
fn uninstall_preserves_unknown_files_and_removes_only_recorded_payloads() {
    let fixture = Fixture::new();
    fixture.install();
    fs::write(fixture.prefix.join("user-note"), b"retain root file").unwrap();
    fs::write(
        fixture.prefix.join("share/mant/user-note"),
        b"retain nested file",
    )
    .unwrap();
    let outside = fixture.root.join("outside");
    fs::write(&outside, b"retain symlink target").unwrap();
    symlink(&outside, fixture.prefix.join("user-link")).unwrap();
    open_install::uninstall(&fixture.prefix, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        fs::read(fixture.prefix.join("user-note")).unwrap(),
        b"retain root file"
    );
    assert_eq!(
        fs::read(fixture.prefix.join("share/mant/user-note")).unwrap(),
        b"retain nested file"
    );
    assert_eq!(fs::read(outside).unwrap(), b"retain symlink target");
    assert!(fixture.prefix.join("user-link").is_symlink());
    assert!(!fixture.prefix.join("bin/mant").exists());
    assert!(!fixture.prefix.join("share/mant/readme").exists());
    assert!(!fixture.prefix.join(".aw-installation.json").exists());
    assert!(!fixture
        .entries()
        .iter()
        .any(|entry| entry.starts_with(".aw-")));
}

#[test]
fn changed_or_unsafe_payloads_block_uninstall_before_any_mutation() {
    for change in ["bytes", "mode", "symlink", "hardlink", "missing"] {
        let fixture = Fixture::new();
        fixture.install();
        let path = fixture.prefix.join("bin/mant");
        match change {
            "bytes" => fs::write(&path, b"user changed contents").unwrap(),
            "mode" => fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap(),
            "symlink" => {
                fs::remove_file(&path).unwrap();
                symlink(fixture.bundle.join("payload/bin/mant"), &path).unwrap();
            }
            "hardlink" => fs::hard_link(&path, fixture.root.join("link")).unwrap(),
            "missing" => fs::remove_file(&path).unwrap(),
            _ => unreachable!(),
        }
        let before = fs::read(fixture.prefix.join(".aw-installation.json")).unwrap();
        assert!(
            open_install::uninstall(&fixture.prefix, &AtomicBool::new(false)).is_err(),
            "{change}"
        );
        assert_eq!(
            fs::read(fixture.prefix.join(".aw-installation.json")).unwrap(),
            before
        );
        assert!(fixture.prefix.join("share/mant/readme").exists());
        assert!(!fixture
            .entries()
            .iter()
            .any(|entry| entry.starts_with(".aw-")));
    }
}

#[test]
fn busy_directory_lock_and_cancellation_leave_installation_unchanged() {
    let fixture = Fixture::new();
    fixture.install();
    let held = File::open(&fixture.prefix).unwrap();
    assert_eq!(
        unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    assert!(open_install::uninstall(&fixture.prefix, &AtomicBool::new(false)).is_err());
    drop(held);
    assert!(open_install::uninstall(&fixture.prefix, &AtomicBool::new(true)).is_err());
    assert_eq!(
        open_install::inspect(&fixture.prefix)
            .unwrap()
            .unhealthy_components,
        0
    );
    open_install::uninstall(&fixture.prefix, &AtomicBool::new(false)).unwrap();
    assert!(!fixture.prefix.exists());
}

#[test]
fn writable_parent_and_receipt_links_are_rejected_without_mutation() {
    let fixture = Fixture::new();
    fs::set_permissions(&fixture.root, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(!open_install::plan(&fixture.bundle, &fixture.prefix)
        .unwrap()
        .conflicts
        .is_empty());
    assert!(
        open_install::install(&fixture.bundle, &fixture.prefix, &AtomicBool::new(false)).is_err()
    );
    fs::set_permissions(&fixture.root, fs::Permissions::from_mode(0o700)).unwrap();
    fixture.install();
    let receipt = fixture.prefix.join(".aw-installation.json");
    fs::hard_link(&receipt, fixture.root.join("receipt-copy")).unwrap();
    assert!(open_install::read_receipt(&fixture.prefix).is_err());
    assert!(open_install::uninstall(&fixture.prefix, &AtomicBool::new(false)).is_err());
    assert!(fixture.prefix.join("bin/mant").exists());
}
