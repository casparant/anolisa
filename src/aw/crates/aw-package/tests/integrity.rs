//! Package diagnostics remain bounded, read-only and separate from runtime health.

use aw_package::{
    integrity::{inspect, IntegrityCode, IntegrityReport, IntegrityStatus},
    Component, InventoryState, Payload,
};
use std::{
    collections::BTreeMap,
    ffi::CString,
    fs::{self, File, Permissions},
    os::unix::{
        ffi::OsStrExt,
        fs::{symlink, MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

struct Fixture {
    _directory: tempfile::TempDir,
    prefix: PathBuf,
}

fn required_paths(component: &str) -> &'static [&'static str] {
    match component {
        "aw-core" => &["bin/aw", "bin/aw-package"],
        "aw-provider-sec-core" => &[
            "libexec/aw/providers/sec-core/aw-provider-sec-core",
            "libexec/aw/providers/sec-core/agent-sec-cli",
            "libexec/aw/providers/sec-core/agent-sec-daemon",
            "libexec/aw/providers/sec-core/agent-sec-daemon-health",
        ],
        "aw-provider-tokenless" => &[
            "libexec/aw/providers/tokenless/aw-provider-tokenless",
            "libexec/aw/providers/tokenless/tokenless",
            "libexec/aw/providers/tokenless/rtk",
        ],
        _ => unreachable!(),
    }
}

impl Fixture {
    fn new() -> Self {
        let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/preview");
        fs::create_dir_all(&target).unwrap();
        let directory = tempfile::tempdir_in(target).unwrap();
        let prefix = directory.path().canonicalize().unwrap().join("installed");
        fs::create_dir(&prefix).unwrap();
        fs::set_permissions(&prefix, Permissions::from_mode(0o755)).unwrap();
        fs::create_dir(prefix.join(".aw-packages")).unwrap();
        Self {
            _directory: directory,
            prefix,
        }
    }

    fn component(&self, name: &str, path: &str) -> Component {
        let mut files = BTreeMap::new();
        for path in required_paths(name).iter().copied().chain([path]) {
            let destination = self.prefix.join(path);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::write(&destination, b"not executed by integrity inspection").unwrap();
            fs::set_permissions(&destination, Permissions::from_mode(0o755)).unwrap();
            files.insert(
                path.into(),
                Payload {
                    sha256: aw_package::digest(&destination).unwrap(),
                    mode: 0o755,
                },
            );
        }
        Component {
            format: 1,
            component: name.into(),
            version: "0.1.0-preview.1".into(),
            source_commit: "a".repeat(40),
            os: "linux".into(),
            arch: std::env::consts::ARCH.into(),
            provider_protocol: "aw-provider/v1alpha1".into(),
            requires: if name == "aw-core" {
                BTreeMap::new()
            } else {
                BTreeMap::from([("aw-core".into(), "0.1.0-preview.1".into())])
            },
            files,
        }
    }

    fn save(&self, component: &Component) {
        fs::write(
            self.prefix
                .join(".aw-packages")
                .join(format!("{}.json", component.component)),
            serde_json::to_vec(component).unwrap(),
        )
        .unwrap();
    }

    fn report(&self) -> IntegrityReport {
        inspect(&self.prefix).unwrap()
    }
}

fn has(report: &IntegrityReport, code: IntegrityCode) -> bool {
    report.components.iter().any(|component| {
        component
            .findings
            .iter()
            .any(|finding| finding.code == code)
    })
}

#[test]
fn checked_installation_is_read_only_and_does_not_execute_payloads() {
    let fixture = Fixture::new();
    let component = fixture.component("aw-core", "bin/aw");
    fixture.save(&component);
    let receipt = fixture.prefix.join(".aw-packages/aw-core.json");
    let bytes = fs::read(&receipt).unwrap();
    let metadata = fs::metadata(&receipt).unwrap();
    let report = fixture.report();
    assert_eq!(report.state, InventoryState::Installed);
    assert_eq!(report.checked_components, 1);
    assert_eq!(report.unhealthy_components, 0);
    assert_eq!(report.checked_files, 2);
    assert_eq!(report.components[0].status, IntegrityStatus::Checked);
    assert_eq!(fs::read(&receipt).unwrap(), bytes);
    assert_eq!(fs::metadata(receipt).unwrap().ino(), metadata.ino());
    assert!(!fixture.prefix.join(".aw-install.lock").exists());
    let json = serde_json::to_value(report).unwrap();
    assert_eq!(json["components"][0]["status"], "checked");
    assert!(json.get("runtime_healthy").is_none());
}

#[test]
fn absent_and_unmanaged_prefixes_are_not_reported_as_checked_components() {
    let fixture = Fixture::new();
    let absent = fixture.prefix.join("absent");
    let report = inspect(&absent).unwrap();
    assert_eq!(report.state, InventoryState::Absent);
    assert_eq!(report.checked_components, 0);
    assert!(!absent.exists());
    let report = fixture.report();
    assert_eq!(report.state, InventoryState::Unmanaged);
    assert_eq!(report.checked_components, 0);
    assert!(!fixture.prefix.join(".aw-install.lock").exists());
}

#[test]
fn missing_hash_and_mode_findings_do_not_hide_other_components() {
    let fixture = Fixture::new();
    let core = fixture.component("aw-core", "bin/aw");
    let sec = fixture.component("aw-provider-sec-core", "libexec/sec-core");
    let tokenless = fixture.component("aw-provider-tokenless", "libexec/tokenless");
    for component in [&core, &sec, &tokenless] {
        fixture.save(component);
    }
    fs::remove_file(fixture.prefix.join("bin/aw")).unwrap();
    fs::write(fixture.prefix.join("libexec/sec-core"), b"changed").unwrap();
    fs::set_permissions(
        fixture.prefix.join("libexec/tokenless"),
        Permissions::from_mode(0o644),
    )
    .unwrap();
    let report = fixture.report();
    assert_eq!(report.components.len(), 3);
    assert_eq!(report.unhealthy_components, 3);
    assert!(has(&report, IntegrityCode::MissingFile));
    assert!(has(&report, IntegrityCode::ChecksumMismatch));
    assert!(has(&report, IntegrityCode::ModeMismatch));
    assert!(!has(&report, IntegrityCode::IncompleteReceipt));
    assert_eq!(
        fs::read(fixture.prefix.join("libexec/sec-core")).unwrap(),
        b"changed"
    );
}

#[test]
fn symlinks_parent_symlinks_hardlinks_and_special_files_are_not_hashed() {
    for mode in ["symlink", "parent", "hardlink", "fifo", "directory"] {
        let fixture = Fixture::new();
        let component = fixture.component("aw-core", "bin/aw");
        fixture.save(&component);
        let path = fixture.prefix.join("bin/aw");
        let outside = fixture.prefix.parent().unwrap().join("outside");
        fs::write(&outside, b"outside data must stay unchanged").unwrap();
        fs::remove_file(&path).unwrap();
        match mode {
            "symlink" => symlink(&outside, &path).unwrap(),
            "parent" => {
                fs::remove_file(fixture.prefix.join("bin/aw-package")).unwrap();
                fs::remove_dir(fixture.prefix.join("bin")).unwrap();
                symlink(fixture.prefix.parent().unwrap(), fixture.prefix.join("bin")).unwrap();
            }
            "hardlink" => fs::hard_link(&outside, &path).unwrap(),
            "fifo" => {
                let name = CString::new(path.as_os_str().as_bytes()).unwrap();
                // SAFETY: name is NUL-terminated and this creates only the owned fixture FIFO.
                assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
            }
            "directory" => fs::create_dir(&path).unwrap(),
            _ => unreachable!(),
        }
        let report = fixture.report();
        assert!(
            has(&report, IntegrityCode::UnsafeFile),
            "{mode}: {report:?}"
        );
        assert_eq!(report.checked_files, usize::from(mode != "parent"));
        assert_eq!(report.unhealthy_components, 1);
        assert!(report.components[0].findings.iter().any(|finding| {
            finding.code == IntegrityCode::UnsafeFile && finding.path.as_deref() == Some("bin/aw")
        }));
        assert_eq!(
            fs::read(&outside).unwrap(),
            b"outside data must stay unchanged"
        );
    }
}

#[test]
fn missing_and_incompatible_dependencies_are_explicit() {
    for mode in ["missing", "version", "source", "architecture", "matching"] {
        let fixture = Fixture::new();
        let mut core = fixture.component("aw-core", "bin/aw");
        let mut provider = fixture.component("aw-provider-sec-core", "libexec/sec-core");
        provider
            .requires
            .insert("aw-core".into(), core.version.clone());
        match mode {
            "version" => core.version = "0.2.0-preview.1".into(),
            "source" => core.source_commit = "b".repeat(40),
            "architecture" => core.arch = "other".into(),
            _ => {}
        }
        if mode != "missing" {
            fixture.save(&core);
        }
        fixture.save(&provider);
        let report = fixture.report();
        if mode == "matching" {
            assert_eq!(report.unhealthy_components, 0);
        } else {
            assert!(
                has(
                    &report,
                    if mode == "missing" {
                        IntegrityCode::DependencyMissing
                    } else {
                        IntegrityCode::DependencyMismatch
                    }
                ),
                "{mode}: {report:?}"
            );
        }
    }
}

#[test]
fn duplicate_payload_ownership_marks_every_claimant_unhealthy() {
    let fixture = Fixture::new();
    let core = fixture.component("aw-core", "bin/aw");
    let mut provider = fixture.component(
        "aw-provider-sec-core",
        "libexec/aw/providers/sec-core/aw-provider-sec-core",
    );
    provider
        .files
        .insert("bin/aw".into(), core.files["bin/aw"].clone());
    fixture.save(&core);
    fixture.save(&provider);
    let report = fixture.report();
    assert_eq!(report.unhealthy_components, 2);
    for component in &report.components {
        assert!(component
            .findings
            .iter()
            .any(|finding| finding.code == IntegrityCode::PathConflict));
    }
}

#[test]
fn oversized_sparse_payload_is_reported_without_reading_or_repairing_it() {
    let fixture = Fixture::new();
    let component = fixture.component("aw-core", "bin/aw");
    fixture.save(&component);
    let path = fixture.prefix.join("bin/aw");
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(512 * 1024 * 1024 + 1)
        .unwrap();
    let report = fixture.report();
    assert!(has(&report, IntegrityCode::LimitExceeded));
    assert_eq!(report.checked_files, 1);
    assert_eq!(report.unhealthy_components, 1);
    assert_eq!(fs::metadata(path).unwrap().len(), 512 * 1024 * 1024 + 1);
}

#[test]
fn malformed_or_escaping_receipts_fail_closed_without_creating_state() {
    for invalid in ["malformed", "escape"] {
        let fixture = Fixture::new();
        let mut component = fixture.component("aw-core", "bin/aw");
        if invalid == "escape" {
            let payload = component.files.remove("bin/aw").unwrap();
            component.files.insert("bin/../../outside".into(), payload);
            fixture.save(&component);
        } else {
            fs::write(
                fixture.prefix.join(".aw-packages/aw-core.json"),
                b"not JSON",
            )
            .unwrap();
        }
        assert!(inspect(&fixture.prefix).is_err());
        assert!(!fixture.prefix.join(".aw-install.lock").exists());
    }
}

#[test]
fn empty_receipts_never_pass_preview_component_integrity() {
    for name in ["aw-core", "aw-provider-sec-core", "aw-provider-tokenless"] {
        let fixture = Fixture::new();
        fixture.save(&fixture.component("aw-core", "bin/aw"));
        let mut component = fixture.component(name, required_paths(name)[0]);
        component.files.clear();
        fixture.save(&component);
        let report = fixture.report();
        assert_eq!(report.state, InventoryState::Installed);
        assert_eq!(report.unhealthy_components, 1);
        let subject = report
            .components
            .iter()
            .find(|value| value.component == name)
            .unwrap();
        assert_eq!(subject.status, IntegrityStatus::Unhealthy);
        assert_eq!(subject.recorded_files, 0);
        assert_eq!(subject.checked_files, 0);
        assert_eq!(subject.findings.len(), required_paths(name).len());
        assert!(subject.findings.iter().all(|finding| {
            finding.code == IntegrityCode::IncompleteReceipt
                && required_paths(name).contains(&finding.path.as_deref().unwrap())
        }));
        assert!(!fixture.prefix.join(".aw-install.lock").exists());
    }
}

#[test]
fn each_mandatory_preview_path_must_be_recorded_even_when_its_file_exists() {
    for name in ["aw-core", "aw-provider-sec-core", "aw-provider-tokenless"] {
        for path in required_paths(name) {
            let fixture = Fixture::new();
            fixture.save(&fixture.component("aw-core", "bin/aw"));
            let mut component = fixture.component(name, required_paths(name)[0]);
            fixture.save(&component);
            assert_eq!(fixture.report().unhealthy_components, 0);
            component.files.remove(*path).unwrap();
            fixture.save(&component);
            let receipt = fixture
                .prefix
                .join(".aw-packages")
                .join(format!("{name}.json"));
            let before = fs::read(&receipt).unwrap();
            let report = fixture.report();
            assert_eq!(report.unhealthy_components, 1);
            let subject = report
                .components
                .iter()
                .find(|value| value.component == name)
                .unwrap();
            assert_eq!(subject.status, IntegrityStatus::Unhealthy);
            assert_eq!(subject.recorded_files, required_paths(name).len() - 1);
            assert_eq!(subject.checked_files, subject.recorded_files);
            assert_eq!(subject.findings.len(), 1);
            assert_eq!(subject.findings[0].code, IntegrityCode::IncompleteReceipt);
            assert_eq!(subject.findings[0].path.as_deref(), Some(*path));
            assert!(fixture.prefix.join(path).is_file());
            assert_eq!(fs::read(&receipt).unwrap(), before);
        }
    }
}

#[test]
fn mandatory_executable_modes_cannot_be_redefined_as_matching_data_files() {
    for name in ["aw-core", "aw-provider-sec-core", "aw-provider-tokenless"] {
        for path in required_paths(name) {
            let fixture = Fixture::new();
            fixture.save(&fixture.component("aw-core", "bin/aw"));
            let mut component = fixture.component(name, required_paths(name)[0]);
            component.files.get_mut(*path).unwrap().mode = 0o644;
            fs::set_permissions(fixture.prefix.join(path), Permissions::from_mode(0o644)).unwrap();
            fixture.save(&component);
            let report = fixture.report();
            assert_eq!(report.unhealthy_components, 1);
            let subject = report
                .components
                .iter()
                .find(|value| value.component == name)
                .unwrap();
            assert_eq!(subject.checked_files, subject.recorded_files);
            assert_eq!(subject.findings.len(), 1);
            assert_eq!(subject.findings[0].code, IntegrityCode::IncompleteReceipt);
            assert_eq!(subject.findings[0].path.as_deref(), Some(*path));
            assert!(!has(&report, IntegrityCode::ModeMismatch));
            assert_eq!(
                fs::metadata(fixture.prefix.join(path)).unwrap().mode() & 0o7777,
                0o644
            );
        }
    }
}

#[test]
fn provider_receipts_must_declare_the_same_version_core_requirement() {
    for name in ["aw-provider-sec-core", "aw-provider-tokenless"] {
        for omitted in [true, false] {
            let fixture = Fixture::new();
            fixture.save(&fixture.component("aw-core", "bin/aw"));
            let mut component = fixture.component(name, required_paths(name)[0]);
            if omitted {
                component.requires.clear();
            } else {
                component
                    .requires
                    .insert("aw-core".into(), "0.2.0-preview.1".into());
            }
            fixture.save(&component);
            let report = fixture.report();
            assert_eq!(report.unhealthy_components, 1);
            let subject = report
                .components
                .iter()
                .find(|value| value.component == name)
                .unwrap();
            assert_eq!(subject.status, IntegrityStatus::Unhealthy);
            assert_eq!(subject.checked_files, subject.recorded_files);
            assert!(subject.findings.iter().any(|finding| {
                finding.code == IntegrityCode::IncompleteReceipt
                    && finding.related_component.as_deref() == Some("aw-core")
            }));
            assert!(!fixture.prefix.join(".aw-install.lock").exists());
        }
    }
}
