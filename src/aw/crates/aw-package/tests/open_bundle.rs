//! External bundle admission is bounded and never activates supplied programs.

use aw_package::open_bundle::{
    self, Bundle, Dependency, Kind, Package, Payload, Platform, Provider,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::CString,
    fs,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Path, PathBuf},
};

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    bundle: PathBuf,
    manifest: Bundle,
}

impl Fixture {
    fn new() -> Self {
        let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/open-bundle");
        fs::create_dir_all(&target).unwrap();
        let directory = tempfile::tempdir_in(target).unwrap();
        let root = directory.path().canonicalize().unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let bundle = root.join("bundle");
        fs::create_dir_all(bundle.join("payload/bin")).unwrap();
        for path in [
            &bundle,
            &bundle.join("payload"),
            &bundle.join("payload/bin"),
        ] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let bytes = b"unexecuted external binary";
        fs::write(bundle.join("payload/bin/mant"), bytes).unwrap();
        fs::set_permissions(
            bundle.join("payload/bin/mant"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        let manifest = Bundle {
            format: 2,
            packages: vec![Package {
                id: "example.mant".into(),
                version: "3.7.1".into(),
                kind: Kind::Tool,
                platform: Platform {
                    os: "linux".into(),
                    arch: std::env::consts::ARCH.into(),
                },
                requires: vec![],
                files: BTreeMap::from([(
                    "bin/mant".into(),
                    Payload {
                        size: bytes.len() as u64,
                        sha256: format!("{:x}", Sha256::digest(bytes)),
                        mode: 0o755,
                    },
                )]),
                provider: None,
            }],
        };
        let fixture = Self {
            _directory: directory,
            root,
            bundle,
            manifest,
        };
        fixture.save();
        fixture
    }

    fn save(&self) {
        fs::write(
            self.bundle.join("manifest.json"),
            serde_json::to_vec(&self.manifest).unwrap(),
        )
        .unwrap();
        fs::set_permissions(
            self.bundle.join("manifest.json"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
    }

    fn add_provider(&mut self) {
        let source = self.bundle.join("payload/bin/mant");
        let destination = self.bundle.join("payload/bin/mant-provider");
        fs::copy(source, &destination).unwrap();
        self.manifest.packages.push(Package {
            id: "example.mant-provider".into(),
            version: "9.1.0".into(),
            kind: Kind::Provider,
            platform: self.manifest.packages[0].platform.clone(),
            requires: vec![Dependency {
                id: "example.mant".into(),
                version: "3.7.1".into(),
            }],
            files: BTreeMap::from([(
                "bin/mant-provider".into(),
                self.manifest.packages[0].files["bin/mant"].clone(),
            )]),
            provider: Some(Provider {
                protocol: "aw-provider/v1alpha1".into(),
                executable: "bin/mant-provider".into(),
            }),
        });
        self.save();
    }
}

#[test]
fn tool_and_provider_versions_are_independent_and_inspection_is_read_only() {
    let mut f = Fixture::new();
    f.add_provider();
    let before = fs::read(f.bundle.join("manifest.json")).unwrap();
    let inspected = open_bundle::inspect(&f.bundle).unwrap();
    assert_eq!(inspected.manifest, f.manifest);
    assert_eq!(
        inspected.manifest_sha256,
        format!("{:x}", Sha256::digest(&before))
    );
    assert_eq!(fs::read(f.bundle.join("manifest.json")).unwrap(), before);
    assert_eq!(fs::read_dir(&f.root).unwrap().count(), 1);
    assert!(!f.bundle.join(".aw-installation.json").exists());
}

#[test]
fn manifest_digest_preserves_original_whitespace() {
    let f = Fixture::new();
    let compact = open_bundle::inspect(&f.bundle).unwrap();
    let pretty = serde_json::to_vec_pretty(&f.manifest).unwrap();
    fs::write(f.bundle.join("manifest.json"), &pretty).unwrap();
    let expanded = open_bundle::inspect(&f.bundle).unwrap();
    assert_eq!(compact.manifest, expanded.manifest);
    assert_ne!(compact.manifest_sha256, expanded.manifest_sha256);
    assert_eq!(
        expanded.manifest_sha256,
        format!("{:x}", Sha256::digest(pretty))
    );
}

#[test]
fn unknown_fields_and_duplicate_payload_keys_are_rejected() {
    let mut f = Fixture::new();
    f.add_provider();
    for pointer in [
        "",
        "/packages/1",
        "/packages/1/platform",
        "/packages/1/requires/0",
        "/packages/1/files/bin~1mant-provider",
        "/packages/1/provider",
    ] {
        let mut value = serde_json::to_value(&f.manifest).unwrap();
        value
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("extra".into(), serde_json::Value::Bool(true));
        assert!(
            serde_json::from_value::<Bundle>(value).is_err(),
            "{pointer}"
        );
    }
    let payload = serde_json::to_string(&f.manifest.packages[0].files["bin/mant"]).unwrap();
    let encoded = serde_json::to_string(&f.manifest).unwrap().replace(
        &format!("\"files\":{{\"bin/mant\":{payload}}}"),
        &format!("\"files\":{{\"bin/mant\":{payload},\"bin/mant\":{payload}}}"),
    );
    assert!(serde_json::from_str::<Bundle>(&encoded).is_err());
}

#[test]
fn identity_version_platform_and_format_are_checked() {
    let f = Fixture::new();
    for id in [
        "mant",
        "Example.mant",
        ".mant",
        "example..mant",
        "../mant",
        "例.mant",
        "example.-mant",
    ] {
        let mut manifest = f.manifest.clone();
        manifest.packages[0].id = id.into();
        assert!(open_bundle::validate(&manifest).is_err(), "{id}");
    }
    for version in ["", " 1.0", "1\n0", "\u{001b}[31m"] {
        let mut manifest = f.manifest.clone();
        manifest.packages[0].version = version.into();
        assert!(open_bundle::validate(&manifest).is_err());
    }
    let mut manifest = f.manifest.clone();
    manifest.packages[0].version = "v".repeat(129);
    assert!(open_bundle::validate(&manifest).is_err());
    manifest = f.manifest.clone();
    manifest.format = 1;
    assert!(open_bundle::validate(&manifest).is_err());
    manifest = f.manifest.clone();
    manifest.packages[0].platform.os = "macos".into();
    assert!(open_bundle::validate(&manifest).is_err());
    manifest = f.manifest.clone();
    manifest.packages[0].platform.arch = "other".into();
    assert!(open_bundle::validate(&manifest).is_err());
}

#[test]
fn provider_requires_compatible_protocol_and_its_own_executable() {
    let mut f = Fixture::new();
    f.manifest.packages[0].kind = Kind::Provider;
    assert!(open_bundle::validate(&f.manifest).is_err());
    f.manifest.packages[0].provider = Some(Provider {
        protocol: "aw-provider/v1alpha1".into(),
        executable: "bin/mant".into(),
    });
    assert!(open_bundle::validate(&f.manifest).is_ok());
    let valid = f.manifest.clone();
    f.manifest.packages[0].kind = Kind::Tool;
    assert!(open_bundle::validate(&f.manifest).is_err());
    f.manifest = valid.clone();
    f.manifest.packages[0].provider.as_mut().unwrap().protocol = "future".into();
    assert!(open_bundle::validate(&f.manifest).is_err());
    f.manifest = valid.clone();
    f.manifest.packages[0].provider.as_mut().unwrap().executable = "bin/unowned".into();
    assert!(open_bundle::validate(&f.manifest).is_err());
    f.manifest = valid;
    f.manifest.packages[0]
        .files
        .get_mut("bin/mant")
        .unwrap()
        .mode = 0o644;
    assert!(open_bundle::validate(&f.manifest).is_err());
}

#[test]
fn dependency_closure_rejects_missing_mismatch_duplicates_and_cycles() {
    let mut f = Fixture::new();
    f.add_provider();
    let valid = f.manifest.clone();
    f.manifest.packages[1].requires[0].id = "missing.tool".into();
    assert!(open_bundle::validate(&f.manifest).is_err());
    f.manifest = valid.clone();
    f.manifest.packages[1].requires[0].version = "3.8.0".into();
    assert!(open_bundle::validate(&f.manifest).is_err());
    f.manifest = valid.clone();
    let dependency = f.manifest.packages[1].requires[0].clone();
    f.manifest.packages[1].requires.push(dependency);
    assert!(open_bundle::validate(&f.manifest).is_err());
    f.manifest = valid;
    f.manifest.packages[0].requires.push(Dependency {
        id: "example.mant-provider".into(),
        version: "9.1.0".into(),
    });
    assert!(open_bundle::validate(&f.manifest)
        .unwrap_err()
        .to_string()
        .contains("cycle"));
}

#[test]
fn file_paths_are_canonical_and_ownership_is_disjoint() {
    let f = Fixture::new();
    for name in [
        "bin",
        "/bin/mant",
        "bin/../mant",
        "bin/./mant",
        "bin//mant",
        "etc/mant",
        "bin/mant\n",
        "bin\\mant",
    ] {
        let mut manifest = f.manifest.clone();
        manifest.packages[0].files = BTreeMap::from([(
            name.into(),
            f.manifest.packages[0].files["bin/mant"].clone(),
        )]);
        assert!(open_bundle::validate(&manifest).is_err(), "{name:?}");
    }
    let mut manifest = f.manifest.clone();
    let mut duplicate = manifest.packages[0].clone();
    duplicate.id = "other.mant".into();
    manifest.packages.push(duplicate);
    assert!(open_bundle::validate(&manifest).is_err());
    manifest = f.manifest.clone();
    manifest.packages[0].files.insert(
        "bin/mant/child".into(),
        f.manifest.packages[0].files["bin/mant"].clone(),
    );
    assert!(open_bundle::validate(&manifest)
        .unwrap_err()
        .to_string()
        .contains("parent"));
}

#[test]
fn package_file_and_byte_budgets_are_checked_without_reading_payloads() {
    let f = Fixture::new();
    let mut manifest = f.manifest.clone();
    manifest.packages.clear();
    assert!(open_bundle::validate(&manifest).is_err());
    manifest.packages = vec![f.manifest.packages[0].clone(); 65];
    assert!(open_bundle::validate(&manifest).is_err());
    manifest = f.manifest.clone();
    manifest.packages[0].files = (0..8193)
        .map(|i| {
            (
                format!("share/{i}"),
                f.manifest.packages[0].files["bin/mant"].clone(),
            )
        })
        .collect();
    assert!(open_bundle::validate(&manifest).is_err());
    manifest = f.manifest.clone();
    manifest.packages[0].files.get_mut("bin/mant").unwrap().size = 512 * 1024 * 1024 + 1;
    assert!(open_bundle::validate(&manifest).is_err());
    let mut payload = f.manifest.packages[0].files["bin/mant"].clone();
    payload.size = 512 * 1024 * 1024;
    manifest.packages[0].files = (0..5)
        .map(|i| (format!("share/{i}"), payload.clone()))
        .collect();
    assert!(open_bundle::validate(&manifest)
        .unwrap_err()
        .to_string()
        .contains("2 GiB"));
}

#[test]
fn hash_mode_and_actual_payload_length_must_match() {
    let mut f = Fixture::new();
    for mode in [0o600, 0o777, 0o4755] {
        f.manifest.packages[0]
            .files
            .get_mut("bin/mant")
            .unwrap()
            .mode = mode;
        assert!(open_bundle::validate(&f.manifest).is_err());
    }
    f.manifest.packages[0]
        .files
        .get_mut("bin/mant")
        .unwrap()
        .mode = 0o755;
    f.manifest.packages[0]
        .files
        .get_mut("bin/mant")
        .unwrap()
        .sha256 = "A".repeat(64);
    assert!(open_bundle::validate(&f.manifest).is_err());
    f.manifest.packages[0]
        .files
        .get_mut("bin/mant")
        .unwrap()
        .sha256 = "0".repeat(64);
    f.save();
    assert!(open_bundle::inspect(&f.bundle)
        .unwrap_err()
        .to_string()
        .contains("checksum"));
    f.manifest.packages[0]
        .files
        .get_mut("bin/mant")
        .unwrap()
        .size += 1;
    f.save();
    assert!(open_bundle::inspect(&f.bundle)
        .unwrap_err()
        .to_string()
        .contains("size"));
}

#[test]
fn oversized_metadata_and_sparse_payload_are_rejected_before_hashing() {
    let f = Fixture::new();
    fs::OpenOptions::new()
        .write(true)
        .open(f.bundle.join("manifest.json"))
        .unwrap()
        .set_len(4 * 1024 * 1024 + 1)
        .unwrap();
    assert!(open_bundle::inspect(&f.bundle).is_err());
    let f = Fixture::new();
    fs::OpenOptions::new()
        .write(true)
        .open(f.bundle.join("payload/bin/mant"))
        .unwrap()
        .set_len(512 * 1024 * 1024 + 1)
        .unwrap();
    assert!(open_bundle::inspect(&f.bundle)
        .unwrap_err()
        .to_string()
        .contains("size"));
}

#[test]
fn symlink_manifest_payload_parent_and_bundle_are_rejected() {
    for relative in ["manifest.json", "payload/bin/mant", "payload/bin"] {
        let f = Fixture::new();
        let original = f.bundle.join(relative);
        let moved = f.root.join("moved");
        fs::rename(&original, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &original).unwrap();
        assert!(open_bundle::inspect(&f.bundle).is_err(), "{relative}");
    }
    let f = Fixture::new();
    let link = f.root.join("linked-bundle");
    std::os::unix::fs::symlink(&f.bundle, &link).unwrap();
    assert!(open_bundle::inspect(&link).is_err());
}

#[test]
fn hardlinks_special_files_and_unsafe_permissions_are_rejected() {
    for relative in ["manifest.json", "payload/bin/mant"] {
        let f = Fixture::new();
        fs::hard_link(f.bundle.join(relative), f.root.join("extra-link")).unwrap();
        assert!(open_bundle::inspect(&f.bundle).is_err());
    }
    for relative in ["manifest.json", "payload/bin/mant"] {
        let f = Fixture::new();
        let path = f.bundle.join(relative);
        fs::remove_file(&path).unwrap();
        let path = CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: the path is a live NUL-terminated string in an owned fixture.
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        assert!(open_bundle::inspect(&f.bundle).is_err());
    }
    for relative in [
        "",
        "payload",
        "payload/bin",
        "payload/bin/mant",
        "manifest.json",
    ] {
        let f = Fixture::new();
        fs::set_permissions(f.bundle.join(relative), fs::Permissions::from_mode(0o777)).unwrap();
        assert!(open_bundle::inspect(&f.bundle).is_err(), "{relative}");
    }
}
