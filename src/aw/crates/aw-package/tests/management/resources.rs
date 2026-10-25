//! External resource locations remain authoritative across plans, verification and removal.

use super::*;

#[test]
fn mapped_plan_is_read_only_and_records_actual_resource_destinations() {
    let fixture = Fixture::new();
    let root = fixture.directory.path().join("resources");
    let bundle = fixture.bundle("example.schema", "1.0.0", "share/example/schema.json", &[]);
    let before = snapshot(fixture.directory.path());
    let addition =
        plan::install_with_resources(&fixture.paths, &bundle, &fixture.prefix, Some(&root))
            .unwrap();
    assert!(addition.ready(), "{:?}", addition.problems);
    assert_eq!(addition.resources.as_ref(), Some(&root));
    assert_eq!(
        addition.files[0].destination,
        root.join("example/schema.json")
    );
    assert_eq!(before, snapshot(fixture.directory.path()));
    let report = plan::execute(&fixture.paths, &addition, &AtomicBool::new(false)).unwrap();
    assert!(report.committed);
    assert!(root.join("example/schema.json").is_file());
    assert!(!fixture.prefix.join("share").exists());
    assert_eq!(
        fs::read(root.join(".aw-installation-prefix")).unwrap(),
        fixture.prefix.as_os_str().as_encoded_bytes()
    );
    assert_eq!(
        fs::read(fixture.prefix.join(".aw-resources-dir")).unwrap(),
        root.as_os_str().as_encoded_bytes()
    );
    let inventory = Inventory::read(&fixture.paths, Some(&fixture.prefix)).unwrap();
    assert_eq!(inventory.installations[0].resources.as_ref(), Some(&root));
    assert_eq!(
        inventory.installations[0].file_path("share/example/schema.json"),
        root.join("example/schema.json")
    );
    assert!(inventory.doctor().unwrap().healthy());
}

#[test]
fn appended_packages_reuse_mapping_and_selective_uninstall_preserves_unknown_files() {
    let fixture = Fixture::new();
    let root = fixture.directory.path().join("resources");
    let one = fixture.bundle("example.one", "1.0.0", "share/one.json", &[]);
    let addition =
        plan::install_with_resources(&fixture.paths, &one, &fixture.prefix, Some(&root)).unwrap();
    plan::execute(&fixture.paths, &addition, &AtomicBool::new(false)).unwrap();
    let two = fixture.bundle("example.two", "1.0.0", "share/two.json", &[]);
    let other = fixture.directory.path().join("different-resources");
    assert!(
        plan::install_with_resources(&fixture.paths, &two, &fixture.prefix, Some(&other)).is_err()
    );
    fixture.install(&two);
    assert!(root.join("two.json").is_file());
    fs::write(root.join("user-file"), b"keep").unwrap();
    let removal =
        plan::uninstall(&fixture.paths, &fixture.prefix, &["example.one".into()]).unwrap();
    assert!(removal.ready(), "{:?}", removal.problems);
    assert_eq!(removal.files[0].destination, root.join("one.json"));
    plan::execute(&fixture.paths, &removal, &AtomicBool::new(false)).unwrap();
    assert!(!root.join("one.json").exists());
    assert!(root.join("two.json").is_file());
    let removal = plan::uninstall(&fixture.paths, &fixture.prefix, &[]).unwrap();
    plan::execute(&fixture.paths, &removal, &AtomicBool::new(false)).unwrap();
    assert_eq!(fs::read(root.join("user-file")).unwrap(), b"keep");
    assert!(root.join(".aw-installation-prefix").is_file());
    let inventory = Inventory::read(&fixture.paths, None).unwrap();
    assert!(inventory.installations[0].packages.is_empty());
    assert!(inventory.doctor().unwrap().healthy());
}

#[test]
fn doctor_and_removal_detect_tampering_at_the_physical_resource_path() {
    let fixture = Fixture::new();
    let root = fixture.directory.path().join("resources");
    let bundle = fixture.bundle("example.schema", "1.0.0", "share/schema.json", &[]);
    let addition =
        plan::install_with_resources(&fixture.paths, &bundle, &fixture.prefix, Some(&root))
            .unwrap();
    plan::execute(&fixture.paths, &addition, &AtomicBool::new(false)).unwrap();
    fs::write(root.join("schema.json"), b"changed").unwrap();
    let inventory = Inventory::read(&fixture.paths, None).unwrap();
    let report = inventory.doctor().unwrap();
    assert!(report
        .issues
        .iter()
        .any(|issue| issue.path == root.join("schema.json")));
    drop(inventory);
    let removal = plan::uninstall(&fixture.paths, &fixture.prefix, &[]).unwrap();
    assert!(!removal.ready());
    assert!(removal
        .problems
        .iter()
        .any(|problem| problem.path.as_ref() == Some(&root.join("schema.json"))));
    fs::remove_file(root.join(".aw-installation-prefix")).unwrap();
    assert!(!Inventory::read(&fixture.paths, None)
        .unwrap()
        .doctor()
        .unwrap()
        .healthy());
}

#[test]
fn unsafe_or_preexisting_resource_roots_are_rejected_before_writes() {
    let fixture = Fixture::new();
    let bundle = fixture.bundle("example.schema", "1.0.0", "share/schema.json", &[]);
    for root in [
        &fixture.prefix,
        &fixture.paths.root,
        fixture.directory.path(),
    ] {
        assert!(
            plan::install_with_resources(&fixture.paths, &bundle, &fixture.prefix, Some(root))
                .is_err()
        );
    }
    let root = fixture.directory.path().join("resources");
    symlink(fixture.directory.path(), &root).unwrap();
    assert!(
        plan::install_with_resources(&fixture.paths, &bundle, &fixture.prefix, Some(&root))
            .is_err()
    );
    assert!(!fixture.prefix.exists());
    assert!(!fixture.paths.root.exists());
}
