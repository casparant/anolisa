//! Runtime leases are independent of package-record contents and never create files.

#![cfg(target_os = "linux")]

use aw_exec::{CommandSpec, LeaseError, PackageLeases};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    os::unix::fs::{symlink, DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        // Runner checkouts may have writable ancestors, unlike installed prefixes.
        let id = fs::read_to_string("/proc/sys/kernel/random/uuid").unwrap();
        let path = Path::new("/tmp").join(format!("aw-package-leases-{}", id.trim()));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }

    fn prefix(&self, name: &str) -> PathBuf {
        let prefix = self.0.join(name);
        fs::create_dir_all(prefix.join(".aw-packages")).unwrap();
        fs::create_dir(prefix.join("bin")).unwrap();
        for directory in [&prefix, &prefix.join(".aw-packages"), &prefix.join("bin")] {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::write(prefix.join(".aw-install.lock"), []).unwrap();
        fs::set_permissions(
            prefix.join(".aw-install.lock"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        fs::write(prefix.join("bin/runtime"), b"#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(
            prefix.join("bin/runtime"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        prefix
    }

    fn lock(prefix: &Path) -> File {
        File::open(prefix.join(".aw-install.lock")).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn active_shared_lease_blocks_mutation_until_the_last_owner_releases_it() {
    let fixture = Fixture::new();
    let prefix = fixture.prefix("installation");
    let first = PackageLeases::for_paths([prefix.join("bin/runtime")]).unwrap();
    let second = PackageLeases::for_paths([prefix.join("bin/runtime")]).unwrap();
    let writer = Fixture::lock(&prefix);
    assert!(writer.try_lock().is_err());
    drop(first);
    assert!(writer.try_lock().is_err());
    drop(second);
    writer.try_lock().unwrap();
}

#[test]
fn exclusive_mutation_rejects_new_runtime_and_drops_partial_shared_leases() {
    let fixture = Fixture::new();
    let first = fixture.prefix("a");
    let second = fixture.prefix("z");
    let writer = Fixture::lock(&second);
    writer.try_lock().unwrap();
    assert!(matches!(
        PackageLeases::for_paths([first.join("bin/runtime"), second.join("bin/runtime")]),
        Err(LeaseError::Busy(path)) if path == second
    ));
    Fixture::lock(&first).try_lock().unwrap();
    drop(writer);
    PackageLeases::for_paths([second.join("bin/runtime")]).unwrap();
}

#[test]
fn path_resolution_and_symbolic_aliases_hold_the_actual_managed_executable() {
    let fixture = Fixture::new();
    let prefix = fixture.prefix("installation");
    let alias = fixture.0.join("alias");
    symlink(prefix.join("bin"), &alias).unwrap();
    let mut command = CommandSpec {
        program: "runtime".into(),
        args: Vec::new(),
        cwd: fixture.0.clone(),
        environment: BTreeMap::from([("PATH".into(), alias.clone().into_os_string())]),
    };
    let lease = PackageLeases::acquire([&mut command]).unwrap();
    assert_eq!(command.program, alias.join("runtime"));
    assert!(Fixture::lock(&prefix).try_lock().is_err());
    drop(lease);
    Fixture::lock(&prefix).try_lock().unwrap();
}

#[test]
fn absolute_argument_paths_protect_backends_even_when_the_executable_is_unmanaged() {
    let fixture = Fixture::new();
    let first = fixture.prefix("backend");
    let second = fixture.prefix("rtk");
    let mut command = CommandSpec {
        program: "/bin/sh".into(),
        args: vec![
            first.join("bin/runtime").into_os_string(),
            format!("--backend={}", second.join("bin/runtime").display()).into(),
        ],
        cwd: fixture.0.clone(),
        environment: BTreeMap::new(),
    };
    let lease = PackageLeases::acquire([&mut command]).unwrap();
    for prefix in [&first, &second] {
        assert!(Fixture::lock(prefix).try_lock().is_err());
    }
    drop(lease);
    for prefix in [&first, &second] {
        Fixture::lock(prefix).try_lock().unwrap();
    }
}

#[test]
fn missing_symbolic_hardlinked_and_writable_locks_are_never_treated_as_unmanaged() {
    let fixture = Fixture::new();
    let prefix = fixture.prefix("installation");
    let lock = prefix.join(".aw-install.lock");
    fs::remove_file(&lock).unwrap();
    assert!(PackageLeases::for_paths([prefix.join("bin/runtime")]).is_err());
    let outside = fixture.0.join("outside");
    fs::write(&outside, []).unwrap();
    symlink(&outside, &lock).unwrap();
    assert!(PackageLeases::for_paths([prefix.join("bin/runtime")]).is_err());
    fs::remove_file(&lock).unwrap();
    fs::hard_link(&outside, &lock).unwrap();
    assert!(matches!(
        PackageLeases::for_paths([prefix.join("bin/runtime")]),
        Err(LeaseError::Unsafe(_))
    ));
    fs::remove_file(&lock).unwrap();
    fs::write(&lock, []).unwrap();
    fs::set_permissions(&lock, fs::Permissions::from_mode(0o666)).unwrap();
    assert!(matches!(
        PackageLeases::for_paths([prefix.join("bin/runtime")]),
        Err(LeaseError::Unsafe(_))
    ));
}

#[test]
fn shared_lease_lookup_does_not_create_records_or_locks() {
    let fixture = Fixture::new();
    let uninstalled = fixture.0.join("new/output");
    let leases = PackageLeases::for_paths([uninstalled]).unwrap();
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 0);
    drop(leases);
    let mut command = CommandSpec {
        program: "runtime".into(),
        args: Vec::new(),
        cwd: fixture.0.clone(),
        environment: BTreeMap::new(),
    };
    assert!(matches!(
        PackageLeases::acquire([&mut command]),
        Err(LeaseError::Invalid(_))
    ));
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 0);
}

#[test]
fn absent_arguments_retain_managed_symbolic_ancestor_protection() {
    let fixture = Fixture::new();
    let prefix = fixture.prefix("installation");
    let alias = fixture.0.join("alias");
    symlink(prefix.join("bin"), &alias).unwrap();
    let lease = PackageLeases::for_paths([alias.join("missing/child")]).unwrap();
    assert!(Fixture::lock(&prefix).try_lock().is_err());
    drop(lease);
    Fixture::lock(&prefix).try_lock().unwrap();
    fs::remove_file(&alias).unwrap();
    symlink(prefix.join("missing"), &alias).unwrap();
    assert!(PackageLeases::for_paths([alias.join("child")]).is_err());
}

#[test]
fn symbolic_parent_components_follow_filesystem_resolution() {
    let fixture = Fixture::new();
    let prefix = fixture.prefix("installation");
    let alias = fixture.0.join("alias");
    symlink(prefix.join("bin"), &alias).unwrap();
    let lease = PackageLeases::for_paths([alias.join("../bin/runtime")]).unwrap();
    assert!(Fixture::lock(&prefix).try_lock().is_err());
    drop(lease);
    fs::remove_file(&alias).unwrap();
    symlink(&prefix, &alias).unwrap();
    let lease = PackageLeases::for_paths([alias.join("bin/runtime")]).unwrap();
    assert!(Fixture::lock(&prefix).try_lock().is_err());
    drop(lease);
    Fixture::lock(&prefix).try_lock().unwrap();
}

impl Fixture {
    fn resources(&self, prefix: &Path, name: &str) -> PathBuf {
        let resources = self.0.join(name);
        fs::create_dir_all(resources.join("aw/native")).unwrap();
        for directory in [
            &resources,
            &resources.join("aw"),
            &resources.join("aw/native"),
        ] {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::write(resources.join("aw/native/hook.mjs"), b"export {};\n").unwrap();
        Self::pointer(&resources.join(".aw-installation-prefix"), prefix);
        Self::pointer(&prefix.join(".aw-resources-dir"), &resources);
        resources
    }

    fn pointer(path: &Path, target: &Path) {
        use std::os::unix::ffi::OsStrExt;
        fs::write(path, target.as_os_str().as_bytes()).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
    }
}

#[test]
fn external_resources_and_their_aliases_hold_the_program_prefix_lock() {
    let fixture = Fixture::new();
    let prefix = fixture.prefix("program");
    let resources = fixture.resources(&prefix, "portable-data");
    let alias = fixture.0.join("resource-alias");
    symlink(&resources, &alias).unwrap();
    symlink(&resources, prefix.join("share")).unwrap();
    for path in [
        resources.join("aw/native/hook.mjs"),
        alias.join("aw/native/hook.mjs"),
        alias.join("aw/not-yet-created/file"),
        prefix.join("share/aw/native/hook.mjs"),
    ] {
        let lease = PackageLeases::for_paths([path]).unwrap();
        assert!(Fixture::lock(&prefix).try_lock().is_err());
        drop(lease);
        Fixture::lock(&prefix).try_lock().unwrap();
    }
}

#[test]
fn external_resource_arguments_share_the_existing_nonblocking_busy_gate() {
    let fixture = Fixture::new();
    let prefix = fixture.prefix("program");
    let resources = fixture.resources(&prefix, "resources");
    let mut command = CommandSpec {
        program: "/bin/sh".into(),
        args: vec![format!(
            "--loader={}",
            resources.join("aw/native/hook.mjs").display()
        )
        .into()],
        cwd: fixture.0.clone(),
        environment: BTreeMap::new(),
    };
    let writer = Fixture::lock(&prefix);
    writer.try_lock().unwrap();
    assert!(matches!(
        PackageLeases::acquire([&mut command]),
        Err(LeaseError::Busy(path)) if path == prefix
    ));
    drop(writer);
    let lease = PackageLeases::acquire([&mut command]).unwrap();
    assert!(Fixture::lock(&prefix).try_lock().is_err());
    drop(lease);
    Fixture::lock(&prefix).try_lock().unwrap();
}

#[test]
fn external_resource_pointers_must_agree_in_both_directions() {
    let fixture = Fixture::new();
    let prefix = fixture.prefix("program");
    let other = fixture.prefix("other-program");
    let resources = fixture.resources(&prefix, "resources");
    let path = resources.join("aw/native/hook.mjs");
    Fixture::pointer(&resources.join(".aw-installation-prefix"), &other);
    assert!(PackageLeases::for_paths([path.clone()]).is_err());
    Fixture::pointer(&other.join(".aw-resources-dir"), &resources);
    assert!(PackageLeases::for_paths([prefix.join("bin/runtime")]).is_err());
    Fixture::pointer(&resources.join(".aw-installation-prefix"), &prefix);
    PackageLeases::for_paths([path]).unwrap();
}

#[test]
fn missing_external_resource_pointers_fail_when_either_binding_is_discoverable() {
    let fixture = Fixture::new();
    let prefix = fixture.prefix("program");
    let resources = fixture.resources(&prefix, "resources");
    let forward = resources.join(".aw-installation-prefix");
    let reverse = prefix.join(".aw-resources-dir");
    fs::remove_file(&reverse).unwrap();
    assert!(PackageLeases::for_paths([resources.join("aw/native/hook.mjs")]).is_err());
    Fixture::pointer(&reverse, &resources);
    fs::remove_file(&forward).unwrap();
    assert!(PackageLeases::for_paths([prefix.join("bin/runtime")]).is_err());
    symlink(&resources, prefix.join("share")).unwrap();
    assert!(PackageLeases::for_paths([prefix.join("share/aw/native/hook.mjs")]).is_err());
    Fixture::pointer(&forward, &prefix);
    fs::remove_file(prefix.join(".aw-install.lock")).unwrap();
    assert!(PackageLeases::for_paths([resources.join("aw/native/hook.mjs")]).is_err());
}

#[test]
fn external_resource_markers_reject_symbolic_hardlinked_and_writable_files() {
    let fixture = Fixture::new();
    let prefix = fixture.prefix("program");
    let resources = fixture.resources(&prefix, "resources");
    let path = resources.join("aw/native/hook.mjs");
    for (marker, target) in [
        (resources.join(".aw-installation-prefix"), prefix.clone()),
        (prefix.join(".aw-resources-dir"), resources.clone()),
    ] {
        fs::set_permissions(&marker, fs::Permissions::from_mode(0o666)).unwrap();
        assert!(PackageLeases::for_paths([path.clone()]).is_err());
        fs::set_permissions(&marker, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(PackageLeases::for_paths([path.clone()]).is_err());
        fs::remove_file(&marker).unwrap();
        let outside = fixture.0.join("outside-marker");
        Fixture::pointer(&outside, &target);
        symlink(&outside, &marker).unwrap();
        assert!(PackageLeases::for_paths([path.clone()]).is_err());
        fs::remove_file(&marker).unwrap();
        fs::hard_link(&outside, &marker).unwrap();
        assert!(PackageLeases::for_paths([path.clone()]).is_err());
        fs::remove_file(&marker).unwrap();
        fs::remove_file(&outside).unwrap();
        Fixture::pointer(&marker, &target);
    }
    fs::set_permissions(&resources, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(PackageLeases::for_paths([path]).is_err());
}

#[test]
fn external_resource_markers_reject_unbounded_and_noncanonical_targets() {
    let fixture = Fixture::new();
    let prefix = fixture.prefix("program");
    let resources = fixture.resources(&prefix, "resources");
    let forward = resources.join(".aw-installation-prefix");
    let alias = fixture.0.join("program-alias");
    symlink(&prefix, &alias).unwrap();
    use std::os::unix::ffi::OsStrExt;
    for bytes in [
        Vec::new(),
        vec![b'a'; 4097],
        b"relative/program".to_vec(),
        prefix.join(".").as_os_str().as_bytes().to_vec(),
        prefix.join("bin/..").as_os_str().as_bytes().to_vec(),
        alias.as_os_str().as_bytes().to_vec(),
    ] {
        fs::write(&forward, bytes).unwrap();
        assert!(PackageLeases::for_paths([resources.join("aw/native/hook.mjs")]).is_err());
    }
    Fixture::pointer(&forward, &prefix);
    PackageLeases::for_paths([resources.join("aw/native/hook.mjs")]).unwrap();
}

#[test]
fn external_resource_bindings_preserve_raw_non_utf8_path_bytes() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let fixture = Fixture::new();
    let prefix = fixture.prefix("program");
    let resources = fixture.resources(&prefix, "resources");
    let non_utf8 = fixture
        .0
        .join(OsString::from_vec(b"resources-\xff".to_vec()));
    fs::rename(&resources, &non_utf8).unwrap();
    Fixture::pointer(&prefix.join(".aw-resources-dir"), &non_utf8);
    let lease = PackageLeases::for_paths([non_utf8.join("aw/native/hook.mjs")]).unwrap();
    assert!(Fixture::lock(&prefix).try_lock().is_err());
    drop(lease);
    Fixture::lock(&prefix).try_lock().unwrap();
}
