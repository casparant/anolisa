//! Real process death exercises both resource and program filesystem workspaces.

use super::*;

#[test]
fn mapped_resource_rollback_removes_payload_and_both_locators_after_sigkill() {
    let fixture = Fixture::new();
    fixture.record(&fixture.root.join(".fixture-resources"), b"", 0o600);
    fixture.kill("precommit", false);
    let root = fixture.root.join("resources");
    assert!(root.join("schema.json").is_file());
    assert!(fixture.prefix.join("bin/tool").is_file());
    let report = fixture.recover();
    assert_eq!((report.rolled_back, report.committed), (1, 0));
    assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
    assert!(!root.exists());
    assert!(!fixture.prefix.exists());
    assert_eq!(fixture.recover().rolled_back, 0);
}

#[test]
fn mapped_resource_commit_survives_sigkill_and_cleanup_reuses_persisted_mapping() {
    for phase in ["postcommit", "precleanup"] {
        let fixture = Fixture::new();
        fixture.record(&fixture.root.join(".fixture-resources"), b"", 0o600);
        fixture.kill(phase, false);
        let report = fixture.recover();
        assert_eq!((report.rolled_back, report.committed), (0, 1));
        assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
        let root = fixture.root.join("resources");
        assert_eq!(
            disk::read(root.join("schema.json")).unwrap(),
            b"resource bytes\n"
        );
        assert_eq!(
            disk::read(fixture.prefix.join(".aw-resources-dir")).unwrap(),
            root.as_os_str().as_encoded_bytes()
        );
        let inventory = Inventory::read(&fixture.paths, None).unwrap();
        assert!(inventory.doctor().unwrap().healthy());
        drop(inventory);
        let removal = crate::plan::uninstall(&fixture.paths, &fixture.prefix, &[]).unwrap();
        assert!(removal.ready(), "{:?}", removal.problems);
        crate::plan::execute(&fixture.paths, &removal, &AtomicBool::new(false)).unwrap();
        assert!(!root.join("schema.json").exists());
        assert!(root.join(".aw-installation-prefix").is_file());
    }
}
