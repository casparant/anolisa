//! Receipt inventory stays bounded and never repairs, installs or starts software.

use aw_package::{inventory, Component, InventoryState, Payload};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::{
        ffi::OsStrExt,
        fs::{symlink, PermissionsExt},
    },
    path::{Path, PathBuf},
};

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    prefix: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/inventory");
        fs::create_dir_all(&target).unwrap();
        let directory = tempfile::tempdir_in(target).unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let root = directory.path().canonicalize().unwrap();
        Self {
            prefix: root.join("prefix"),
            root,
            _directory: directory,
        }
    }

    fn receipts(&self) -> PathBuf {
        let directory = self.prefix.join(".aw-packages");
        fs::create_dir_all(&directory).unwrap();
        for path in [&self.prefix, &directory] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        directory
    }

    fn record(&self) -> PathBuf {
        let receipt = self.receipts().join("aw-core.json");
        let component = Component {
            format: 1,
            component: "aw-core".into(),
            version: "0.1.0-preview.1".into(),
            source_commit: "a".repeat(40),
            os: "linux".into(),
            arch: std::env::consts::ARCH.into(),
            provider_protocol: "aw-provider/v1alpha1".into(),
            requires: BTreeMap::new(),
            files: BTreeMap::from([(
                "bin/aw".into(),
                Payload {
                    sha256: "a".repeat(64),
                    mode: 0o755,
                },
            )]),
        };
        fs::write(&receipt, serde_json::to_vec(&component).unwrap()).unwrap();
        fs::set_permissions(&receipt, fs::Permissions::from_mode(0o644)).unwrap();
        receipt
    }
}

#[test]
fn absent_unmanaged_and_recorded_prefixes_are_distinguished_without_writes() {
    let fixture = Fixture::new();
    let absent = inventory(&fixture.prefix).unwrap();
    assert_eq!(absent.state, InventoryState::Absent);
    assert!(absent.components.is_empty());
    assert!(!fixture.prefix.exists());
    fs::create_dir(&fixture.prefix).unwrap();
    assert_eq!(
        inventory(&fixture.prefix).unwrap().state,
        InventoryState::Unmanaged
    );
    assert_eq!(fs::read_dir(&fixture.prefix).unwrap().count(), 0);
    let receipt = fixture.record();
    let original = fs::read(&receipt).unwrap();
    let recorded = inventory(&fixture.prefix).unwrap();
    assert_eq!(recorded.state, InventoryState::Installed);
    assert_eq!(recorded.components.len(), 1);
    assert_eq!(recorded.components[0].component, "aw-core");
    assert_eq!(fs::read(receipt).unwrap(), original);
    assert!(!fixture.prefix.join(".aw-install.lock").exists());
    assert!(
        !fixture.prefix.join("bin/aw").exists(),
        "inventory does not repair files"
    );
    assert_eq!(
        serde_json::to_value(recorded).unwrap()["state"],
        "installed"
    );
}

#[test]
fn symlinks_hardlinks_and_world_writable_metadata_are_rejected() {
    for kind in ["prefix", "directory", "file", "hardlink", "writable"] {
        let fixture = Fixture::new();
        let receipt = fixture.record();
        match kind {
            "prefix" => {
                let alias = fixture.root.join("alias");
                symlink(&fixture.prefix, &alias).unwrap();
                assert!(inventory(&alias).is_err());
                continue;
            }
            "directory" => {
                let moved = fixture.root.join("receipts");
                fs::rename(receipt.parent().unwrap(), &moved).unwrap();
                symlink(moved, fixture.prefix.join(".aw-packages")).unwrap();
            }
            "file" => {
                let moved = fixture.root.join("receipt.json");
                fs::rename(&receipt, &moved).unwrap();
                symlink(moved, &receipt).unwrap();
            }
            "hardlink" => fs::hard_link(&receipt, fixture.root.join("linked.json")).unwrap(),
            "writable" => fs::set_permissions(&receipt, fs::Permissions::from_mode(0o666)).unwrap(),
            _ => unreachable!(),
        }
        assert!(inventory(&fixture.prefix).is_err(), "{kind}");
        assert!(!fixture.prefix.join(".aw-install.lock").exists());
    }
}

#[test]
fn oversized_directory_receipt_and_fifo_are_rejected_without_blocking() {
    for kind in ["directory", "receipt", "fifo"] {
        let fixture = Fixture::new();
        let directory = fixture.receipts();
        match kind {
            "directory" => {
                for index in 0..65 {
                    fs::write(directory.join(format!("ignored-{index}")), b"").unwrap();
                }
            }
            "receipt" => {
                fs::File::create(directory.join("aw-core.json"))
                    .unwrap()
                    .set_len(4 * 1024 * 1024 + 1)
                    .unwrap();
            }
            "fifo" => {
                let path =
                    std::ffi::CString::new(directory.join("aw-core.json").as_os_str().as_bytes())
                        .unwrap();
                // SAFETY: the NUL-terminated path points inside this test's owned directory.
                assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
            }
            _ => unreachable!(),
        }
        assert!(inventory(&fixture.prefix).is_err(), "{kind}");
        assert!(!fixture.prefix.join(".aw-install.lock").exists());
    }
}

#[test]
fn malformed_or_unsafe_receipt_metadata_is_not_reported_as_installed() {
    for kind in ["identity", "path", "digest"] {
        let fixture = Fixture::new();
        let receipt = fixture.record();
        let mut component: Component =
            serde_json::from_slice(&fs::read(&receipt).unwrap()).unwrap();
        match kind {
            "identity" => component.component = "other".into(),
            "path" => {
                let payload = component.files.remove("bin/aw").unwrap();
                component.files.insert("bin/../../outside".into(), payload);
            }
            "digest" => component.files.get_mut("bin/aw").unwrap().sha256 = "invalid".into(),
            _ => unreachable!(),
        }
        fs::write(&receipt, serde_json::to_vec(&component).unwrap()).unwrap();
        assert!(inventory(&fixture.prefix).is_err(), "{kind}");
    }
}
