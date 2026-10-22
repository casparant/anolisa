//! Migration inspection reports historical claims without touching referenced resources.

use aw_package::migration::inspect;
use std::{
    fs,
    os::unix::{
        ffi::OsStrExt,
        fs::{symlink, MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

struct Fixture {
    directory: tempfile::TempDir,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/migration");
        fs::create_dir_all(&target).unwrap();
        let directory = tempfile::tempdir_in(target).unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let root = directory.path().canonicalize().unwrap();
        Self { directory, root }
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.root.join(name);
        fs::write(&path, text).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        path
    }
}

fn header(schema: u32, mode: &str) -> String {
    format!("schema_version = {schema}\nupdated_at = '2026-07-01T00:00:00Z'\ninstall_mode = '{mode}'\nprefix = '/legacy/data'\nanolisa_version = '0.2.4'\n")
}

fn old(name: &str, extra: &str) -> String {
    format!("\n[[objects]]\nkind = 'component'\nname = '{name}'\nversion = '2.7.0'\nstatus = 'installed'\ninstalled_at = '2026-07-01T00:00:00Z'\n{extra}\n")
}

fn current(scope: &str, binding: &str) -> String {
    format!("\n[[installations]]\nkind = 'component'\nname = 'tokenless'\nstatus = 'installed'\ninstalled_at = '2026-07-01T00:00:00Z'\n{scope}\n{binding}\n")
}

const OWNED: &str = "[installations.binding]\nauthority = 'owned'\n[installations.binding.artifact]\nversion = '1.2.3'";
const RPM: &str = "[installations.binding]\nauthority = 'delegated'\npm = 'rpm'\n[installations.binding.package]\nresolution = 'resolved'\nname = 'tokenless-rpm'\n[installations.binding.relation]\nkind = 'managed'\nsince = '2026-07-01T00:00:00Z'\n[installations.binding.last_observed]\nversion = '1.2.3-1'\nobserved_at = '2026-07-01T00:00:00Z'";

#[test]
fn oldest_user_record_keeps_unknown_uid_and_remains_unchanged() {
    let fixture = Fixture::new();
    let text = header(1, "user") + &old("tokenless", "install_backend = 'raw'");
    let path = fixture.write("user.toml", &text);
    let report = inspect(std::slice::from_ref(&path)).unwrap();
    let source = &report.sources[0];
    assert_eq!(source.schema_version, 1);
    assert_eq!(source.file_owner_uid, fs::metadata(&path).unwrap().uid());
    assert_eq!(source.records[0].recorded_scope.kind, "user");
    assert_eq!(source.records[0].recorded_scope.uid, None);
    assert_eq!(source.records[0].recorded_relation, "owned");
    assert!(report.changes.is_empty());
    assert_eq!(report.mode, "read_only");
    assert_eq!(fs::read_to_string(path).unwrap(), text);
    assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 1);
    assert!(fixture.directory.path().exists());
}

#[test]
fn legacy_relations_and_conflicting_evidence_are_not_upgraded() {
    let fixture = Fixture::new();
    let text = header(4, "system")
        + &old(
            "managed",
            "install_backend = 'rpm'\nownership = 'rpm_managed'",
        )
        + &old(
            "observed",
            "install_backend = 'rpm'\nownership = 'rpm_observed'",
        )
        + &old(
            "adopted",
            "install_backend = 'rpm'\nownership = 'rpm_observed'\nadopted = true",
        )
        + &old(
            "contradictory",
            "install_backend = 'rpm'\nownership = 'raw_managed'",
        )
        + &old("unknown", "")
        + &old("deb", "install_backend = 'deb'");
    let report = inspect(&[fixture.write("system.toml", &text)]).unwrap();
    let rows = &report.sources[0].records;
    assert_eq!(
        rows.iter()
            .map(|row| row.recorded_relation.as_str())
            .collect::<Vec<_>>(),
        ["managed", "observed", "adopted", "unknown", "unknown", "unknown"]
    );
    assert!(rows[3]
        .findings
        .iter()
        .any(|finding| finding.code == "ambiguous_record"));
    assert_eq!(rows[5].recorded_backend, "deb");
    assert!(rows[5]
        .findings
        .iter()
        .any(|finding| finding.code == "unsupported_backend"));
}

#[test]
fn current_owned_and_delegated_scopes_preserve_explicit_claims() {
    let fixture = Fixture::new();
    let user = header(5, "user") + &current("[installations.scope.user]\nuid = 1234", OWNED);
    let system = header(6, "system") + &current("scope = 'system'", RPM)
        + "\n[[adapter_claims]]\ncomponent = 'tokenless'\nframework = 'example'\nprivate_token = 'DO_NOT_PRINT'\n[[adapter_trust_roots]]\ncomponent = 'tokenless'\nframework = 'example'\nroot = '/do-not-open'\n";
    let report = inspect(&[
        fixture.write("user.toml", &user),
        fixture.write("system.toml", &system),
    ])
    .unwrap();
    assert_eq!(report.sources[0].records[0].recorded_scope.uid, Some(1234));
    assert_eq!(
        report.sources[1].records[0].recorded_package.as_deref(),
        Some("tokenless-rpm")
    );
    assert_eq!(
        report.sources[1].records[0].recorded_version.as_deref(),
        Some("1.2.3-1")
    );
    assert_eq!(report.sources[1].adapter_claim_count, 1);
    assert_eq!(report.sources[1].adapter_trust_root_count, 1);
    assert_eq!(report.overlaps.len(), 1);
    assert_eq!(report.overlaps[0].source_indices, [0, 1]);
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains("DO_NOT_PRINT"));
    assert!(!json.contains("/do-not-open"));
    assert!(!json.contains("can_apply"));
}

#[test]
fn rpm_user_claim_and_file_mode_mismatch_are_findings_not_automatic_repairs() {
    let fixture = Fixture::new();
    let text = header(5, "system") + &current("[installations.scope.user]\nuid = 9001", RPM);
    let report = inspect(&[fixture.write("mismatch.toml", &text)]).unwrap();
    let row = &report.sources[0].records[0];
    assert_eq!(row.recorded_scope.kind, "user");
    assert_eq!(row.recorded_scope.uid, Some(9001));
    assert!(row
        .findings
        .iter()
        .any(|finding| finding.code == "scope_mismatch"));
    assert!(row
        .findings
        .iter()
        .any(|finding| finding.code == "native_backend_in_user_scope"));
}

#[test]
fn legacy_objects_and_quarantine_are_retained_without_referencing_paths() {
    let fixture = Fixture::new();
    let legacy = (header(3, "user") + &old("old-object", "install_backend = 'raw'\n[[objects.files]]\npath = '/do-not-access'\nowner = 'anolisa'\n[[objects.services]]\nname = 'do-not-start'\nmanager = 'systemd'"))
        .replace("kind = 'component'", "kind = 'capability'");
    let quarantined = header(5, "system") + "\n[[quarantined]]\n[quarantined.record]\nkind = 'osbase'\nname = 'old-kernel'\nversion = '0.1'\nstatus = 'failed'\ninstalled_at = '2026-07-01T00:00:00Z'\ninstall_backend = 'unknown'\n[quarantined.reason]\nreason = 'unknown_backend'\nbackend = 'unknown'\n";
    let report = inspect(&[
        fixture.write("old.toml", &legacy),
        fixture.write("quarantine.toml", &quarantined),
    ])
    .unwrap();
    assert_eq!(report.sources[0].legacy_object_count, 1);
    assert_eq!(report.sources[0].records[0].file_count, 1);
    assert_eq!(report.sources[0].records[0].service_count, 1);
    assert_eq!(report.sources[1].quarantined_count, 1);
    assert_eq!(report.sources[1].records[0].name, "old-kernel");
    assert!(report.sources[1].records[0]
        .findings
        .iter()
        .any(|finding| finding.code == "quarantined_record"));
    let output = serde_json::to_string(&report).unwrap();
    assert!(!output.contains("/do-not-access"));
    assert!(!output.contains("do-not-start"));
    assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 2);
}

#[test]
fn explicit_missing_duplicate_invalid_and_future_state_files_fail() {
    let fixture = Fixture::new();
    assert!(inspect(&[]).is_err());
    assert!(inspect(&[fixture.root.join("missing.toml")]).is_err());
    let path = fixture.write("valid.toml", &header(4, "system"));
    assert!(inspect(&[path.clone(), path.clone()]).is_err());
    let too_many_sources = vec![path.clone(); 9];
    assert!(inspect(&too_many_sources).is_err());
    assert!(inspect(&[PathBuf::from("relative.toml")]).is_err());
    assert!(inspect(&[fixture.root.join("..").join("missing.toml")]).is_err());
    for (index, text) in [
        header(7, "system"),
        header(0, "system"),
        "schema_version = 4".into(),
        header(4, "invalid"),
        header(4, "system") + "objects = 'bad'\n",
        header(4, "system") + "private_token = [SECRET_MUST_NOT_ECHO\n",
        header(5, "system") + &old("mismatched-shape", ""),
    ]
    .iter()
    .enumerate()
    {
        let error = inspect(&[fixture.write(&format!("invalid-{index}.toml"), text)]).unwrap_err();
        assert!(!error.to_string().contains("SECRET_MUST_NOT_ECHO"));
    }
}

#[test]
fn unsafe_links_permissions_and_nonregular_sources_fail_without_blocking() {
    for kind in [
        "symlink",
        "hardlink",
        "parent-link",
        "writable",
        "fifo",
        "directory",
    ] {
        let fixture = Fixture::new();
        let path = fixture.write("state.toml", &header(4, "user"));
        let input = match kind {
            "symlink" => {
                let link = fixture.root.join("link");
                symlink(&path, &link).unwrap();
                link
            }
            "hardlink" => {
                fs::hard_link(&path, fixture.root.join("second")).unwrap();
                path
            }
            "parent-link" => {
                let link = fixture.root.join("alias");
                symlink(&fixture.root, &link).unwrap();
                link.join("state.toml")
            }
            "writable" => {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
                path
            }
            "fifo" => {
                let fifo = fixture.root.join("fifo");
                let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
                // SAFETY: name is NUL-terminated and remains valid for this syscall.
                assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
                fifo
            }
            "directory" => fixture.root.clone(),
            _ => unreachable!(),
        };
        assert!(inspect(&[input]).is_err(), "{kind}");
    }
}

#[test]
fn document_summary_and_aggregate_record_limits_are_enforced() {
    let fixture = Fixture::new();
    let oversized = fixture.write("large.toml", &" ".repeat(4 * 1024 * 1024 + 1));
    assert!(inspect(&[oversized])
        .unwrap_err()
        .to_string()
        .contains("byte limit"));
    let text = header(4, "user") + &old(&"x".repeat(513), "install_backend = 'raw'");
    assert!(inspect(&[fixture.write("long-name.toml", &text)]).is_err());
    let mut text = header(4, "system");
    for index in 0..2049 {
        text.push_str(&old(
            &format!("component-{index}"),
            "install_backend = 'raw'",
        ));
    }
    let first = fixture.write("first.toml", &text);
    let second = fixture.write("second.toml", &text);
    assert!(inspect(&[first, second])
        .unwrap_err()
        .to_string()
        .contains("record limit"));
}

#[test]
fn controls_are_serialized_as_data_and_duplicate_claims_remain_visible() {
    let fixture = Fixture::new();
    let text = header(4, "system")
        + &old("repeat", "install_backend = 'raw'")
        + &old("repeat", "install_backend = 'raw'");
    let report = inspect(&[fixture.write("duplicates.toml", &text)]).unwrap();
    assert_eq!(report.sources[0].records.len(), 2);
    assert!(report.sources[0].records[1]
        .findings
        .iter()
        .any(|finding| finding.code == "duplicate_record"));
    let control = (header(4, "system") + &old("tokenless", "install_backend = 'raw'"))
        .replace("name = 'tokenless'", "name = \"tokenless\\u001b[31m\"");
    let report = inspect(&[fixture.write("control.toml", &control)]).unwrap();
    let output = serde_json::to_string(&report).unwrap();
    assert!(!output.contains('\u{001b}'));
    assert!(output.contains("\\u001b"));
}
