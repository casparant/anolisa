//! Version dispatch is read-only and legacy mutations cannot bypass v2 ownership.

use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    path::{Path, PathBuf},
};

fn directory() -> (tempfile::TempDir, PathBuf) {
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/package-formats");
    fs::create_dir_all(&target).unwrap();
    let directory = tempfile::tempdir_in(target).unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let root = directory.path().canonicalize().unwrap();
    (directory, root)
}

#[test]
fn bundle_version_selection_is_bounded_and_does_not_validate_or_execute() {
    let (_temporary, root) = directory();
    for version in [1, 2] {
        fs::write(
            root.join("manifest.json"),
            format!("{{\"format\":{version}}}"),
        )
        .unwrap();
        assert_eq!(aw_package::bundle_format(&root).unwrap(), version);
    }
    for bytes in [
        b"{}".as_slice(),
        b"{\"format\":3}",
        b"{\"format\":\"2\"}",
        b"{\"format\":2.0}",
    ] {
        fs::write(root.join("manifest.json"), bytes).unwrap();
        assert!(aw_package::bundle_format(&root).is_err());
    }
    fs::OpenOptions::new()
        .write(true)
        .open(root.join("manifest.json"))
        .unwrap()
        .set_len(4 * 1024 * 1024 + 1)
        .unwrap();
    assert!(aw_package::bundle_format(&root).is_err());
}

#[test]
fn mixed_markers_and_linked_records_are_never_silently_adopted() {
    let (_temporary, root) = directory();
    let prefix = root.join("prefix");
    assert_eq!(aw_package::installed_format(&prefix).unwrap(), 1);
    assert!(!prefix.exists());
    fs::create_dir(&prefix).unwrap();
    assert_eq!(aw_package::installed_format(&prefix).unwrap(), 1);
    let marker = prefix.join(".aw-installation.json");
    symlink(root.join("nonexistent"), &marker).unwrap();
    assert_eq!(aw_package::installed_format(&prefix).unwrap(), 2);
    assert!(aw_package::open_install::read_receipt(&prefix).is_err());
    fs::create_dir(prefix.join(".aw-packages")).unwrap();
    assert!(aw_package::installed_format(&prefix).is_err());
    assert!(marker.is_symlink());
}

#[test]
fn legacy_library_rejects_v2_and_mixed_prefixes_before_locking_or_writes() {
    let (_temporary, root) = directory();
    let prefix = root.join("prefix");
    fs::create_dir(&prefix).unwrap();
    fs::write(
        prefix.join(".aw-installation.json"),
        b"unvalidated v2 record",
    )
    .unwrap();
    fs::write(prefix.join("unowned.txt"), b"preserve me").unwrap();
    for mixed in [false, true] {
        if mixed {
            fs::create_dir(prefix.join(".aw-packages")).unwrap();
        }
        assert!(aw_package::install(&root.join("nonexistent-bundle"), &prefix).is_err());
        assert!(aw_package::uninstall(&prefix).is_err());
        assert!(!prefix.join(".aw-install.lock").exists());
        assert_eq!(
            fs::read(prefix.join("unowned.txt")).unwrap(),
            b"preserve me"
        );
        assert_eq!(
            fs::read(prefix.join(".aw-installation.json")).unwrap(),
            b"unvalidated v2 record"
        );
    }
}
