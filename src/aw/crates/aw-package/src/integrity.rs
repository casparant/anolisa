//! Bounded, read-only payload integrity diagnostics, independent of runtime health.

use crate::{inventory, read_only, Component, Error, InventoryState, Payload, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, Metadata, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

const MAX_FILES: usize = 8192;
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Result of checking recorded files and dependencies, not Provider readiness.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrityStatus {
    /// Required Preview records, all recorded files and dependencies passed inspection.
    Checked,
    /// At least one finding prevents a complete successful inspection.
    Unhealthy,
}

/// Stable finding categories for human and machine-readable diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrityCode {
    /// A receipt omits or misdeclares a mandatory Preview payload or dependency.
    IncompleteReceipt,
    /// A recorded file or its parent directory is absent.
    MissingFile,
    /// A symlink, special file, unsafe owner or writable path was encountered.
    UnsafeFile,
    /// Permission bits differ from the recorded payload mode.
    ModeMismatch,
    /// SHA-256 differs from the recorded payload digest.
    ChecksumMismatch,
    /// File contents, identity or metadata changed during inspection.
    ChangedDuringRead,
    /// Reading a recorded payload failed.
    ReadFailed,
    /// The file count or bounded hashing budget was exceeded.
    LimitExceeded,
    /// A declared component dependency is not installed in this prefix.
    DependencyMissing,
    /// A dependency differs in version, source commit or architecture.
    DependencyMismatch,
    /// Multiple receipts claim the same relative payload path.
    PathConflict,
    /// Recorded platform or Provider protocol does not match this runtime.
    IncompatibleMetadata,
}

/// One installation-integrity finding; diagnostics never execute repair actions.
#[derive(Clone, Debug, Serialize)]
pub struct IntegrityFinding {
    /// Machine-readable finding category.
    pub code: IntegrityCode,
    /// Relative recorded payload path, when this finding concerns a file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Other component involved in a dependency or ownership conflict.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_component: Option<String>,
    /// Diagnostic context, not a shell command or automatic remediation.
    pub message: String,
}

/// Diagnostics for one structurally valid installed component receipt.
#[derive(Clone, Debug, Serialize)]
pub struct ComponentIntegrity {
    /// Receipt component identity.
    pub component: String,
    /// Receipt version; this is not obtained by executing the installed binary.
    pub version: String,
    /// Whether all integrity checks completed without findings.
    pub status: IntegrityStatus,
    /// Number of files whose bounded hash and stable identity were checked.
    pub checked_files: usize,
    /// Total number of files declared by this receipt.
    pub recorded_files: usize,
    /// Deterministically ordered diagnostics for this component.
    pub findings: Vec<IntegrityFinding>,
}

/// An unlocked snapshot of package integrity, not service or Agent health.
#[derive(Clone, Debug, Serialize)]
pub struct IntegrityReport {
    /// Absolute inspected prefix.
    pub prefix: PathBuf,
    /// Distinguishes an absent or unmanaged prefix from a checked installation.
    pub state: InventoryState,
    /// Number of files with completed bounded hashing and identity checks.
    pub checked_files: usize,
    /// Number of components with no findings.
    pub checked_components: usize,
    /// Number of components with findings or incomplete checks.
    pub unhealthy_components: usize,
    /// Per-component installation findings, in component-name order.
    pub components: Vec<ComponentIntegrity>,
}

/// Inspect Preview receipt completeness, payload bytes, modes and build dependencies.
///
/// This reads at most 8,192 payloads, 512 MiB per file and a 2 GiB aggregate
/// hashing budget. Receipts have their own inventory limits. Every path segment
/// is opened relative to a held directory; symlinks and nonregular files are
/// rejected. Files are opened nonblocking and checked again after hashing.
/// Nothing is downloaded, executed, repaired, locked or written. This snapshot
/// can become stale immediately and does not authorize a later mutation.
///
/// # Errors
/// Rejects unsafe prefixes, malformed receipts and metadata inventory failures.
/// Individual payload failures become findings rather than hiding other files.
pub fn inspect(prefix: &Path) -> Result<IntegrityReport> {
    let inventory = inventory(prefix)?;
    let mut report = IntegrityReport {
        prefix: inventory.prefix,
        state: inventory.state,
        checked_files: 0,
        checked_components: 0,
        unhealthy_components: 0,
        components: vec![],
    };
    if inventory.components.is_empty() {
        return Ok(report);
    }
    let directory = read_only::directory(prefix)?.ok_or_else(|| {
        Error::Invalid("installation prefix disappeared; retry inspection".into())
    })?;
    let installed: BTreeMap<_, _> = inventory
        .components
        .iter()
        .map(|component| (component.component.as_str(), component))
        .collect();
    let mut owners: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for component in &inventory.components {
        for path in component.files.keys() {
            owners.entry(path).or_default().push(&component.component);
        }
    }
    let mut budget = Budget {
        files: MAX_FILES,
        bytes: MAX_TOTAL_BYTES,
    };
    for component in &inventory.components {
        let mut result = ComponentIntegrity {
            component: component.component.clone(),
            version: component.version.clone(),
            status: IntegrityStatus::Checked,
            checked_files: 0,
            recorded_files: component.files.len(),
            findings: vec![],
        };
        completeness(component, &mut result.findings)?;
        dependencies(component, &installed, &mut result.findings);
        for (path, payload) in &component.files {
            for owner in &owners[path.as_str()] {
                if *owner != component.component.as_str() {
                    result.findings.push(IntegrityFinding {
                        code: IntegrityCode::PathConflict,
                        path: Some(path.clone()),
                        related_component: Some((*owner).into()),
                        message: "multiple component receipts claim this path".into(),
                    });
                }
            }
            if budget.files == 0 {
                result.findings.push(finding(
                    IntegrityCode::LimitExceeded,
                    path,
                    "payload file-count limit reached",
                ));
                break;
            }
            budget.files -= 1;
            match payload_check(&directory, path, payload, &mut budget, &mut result.findings) {
                Ok(()) => result.checked_files += 1,
                Err(finding) => result.findings.push(finding),
            }
        }
        if result.findings.is_empty() {
            report.checked_components += 1;
        } else {
            result.status = IntegrityStatus::Unhealthy;
            report.unhealthy_components += 1;
        }
        report.checked_files += result.checked_files;
        report.components.push(result);
    }
    Ok(report)
}

fn completeness(component: &Component, findings: &mut Vec<IntegrityFinding>) -> Result<()> {
    for path in crate::manifest::required_payloads(&component.component)? {
        let message = match component.files.get(*path) {
            None => "receipt omits a required Preview payload",
            Some(payload) if payload.mode != 0o755 => {
                "receipt must declare required Preview executable mode 0755"
            }
            Some(_) => continue,
        };
        findings.push(finding(IntegrityCode::IncompleteReceipt, path, message));
    }
    if let Err(error) = crate::manifest::validate_core_requirement(component) {
        findings.push(IntegrityFinding {
            code: IntegrityCode::IncompleteReceipt,
            path: None,
            related_component: Some("aw-core".into()),
            message: error.to_string(),
        });
    }
    Ok(())
}

fn dependencies(
    component: &Component,
    installed: &BTreeMap<&str, &Component>,
    findings: &mut Vec<IntegrityFinding>,
) {
    if component.os != std::env::consts::OS
        || component.arch != std::env::consts::ARCH
        || component.provider_protocol != "aw-provider/v1alpha1"
    {
        findings.push(IntegrityFinding {
            code: IntegrityCode::IncompatibleMetadata,
            path: None,
            related_component: None,
            message: "receipt platform or Provider protocol is incompatible".into(),
        });
    }
    for (name, version) in &component.requires {
        let code = match installed.get(name.as_str()) {
            None => IntegrityCode::DependencyMissing,
            Some(required)
                if required.version == *version
                    && required.source_commit == component.source_commit
                    && required.arch == component.arch =>
            {
                continue
            }
            Some(_) => IntegrityCode::DependencyMismatch,
        };
        findings.push(IntegrityFinding {
            code,
            path: None,
            related_component: Some(name.clone()),
            message: format!("requires version {version} from the same build and architecture"),
        });
    }
}

struct Budget {
    files: usize,
    bytes: u64,
}

fn finding(code: IntegrityCode, path: &str, message: impl Into<String>) -> IntegrityFinding {
    IntegrityFinding {
        code,
        path: Some(path.into()),
        related_component: None,
        message: message.into(),
    }
}

fn io_finding(path: &str, error: std::io::Error) -> IntegrityFinding {
    let code = if error.kind() == std::io::ErrorKind::NotFound {
        IntegrityCode::MissingFile
    } else if matches!(error.raw_os_error(), Some(libc::ELOOP | libc::ENOTDIR)) {
        IntegrityCode::UnsafeFile
    } else {
        IntegrityCode::ReadFailed
    };
    finding(code, path, error.to_string())
}

fn open_payload(root: &File, path: &str) -> std::result::Result<File, IntegrityFinding> {
    let mut directory = root.try_clone().map_err(|error| io_finding(path, error))?;
    let mut parts = path.split('/').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            let name = read_only::descriptor_path(&directory).join(part);
            let before = fs::symlink_metadata(&name).map_err(|error| io_finding(path, error))?;
            if !before.is_file() {
                return Err(finding(
                    IntegrityCode::UnsafeFile,
                    path,
                    "payload is not a regular file",
                ));
            }
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(name)
                .map_err(|error| io_finding(path, error))?;
            let opened = file.metadata().map_err(|error| io_finding(path, error))?;
            if before.dev() != opened.dev() || before.ino() != opened.ino() {
                return Err(finding(
                    IntegrityCode::ChangedDuringRead,
                    path,
                    "payload changed while opening",
                ));
            }
            return Ok(file);
        }
        directory = read_only::child_directory(&directory, part)
            .map_err(|error| match error {
                Error::Io(error) => io_finding(path, error),
                error => finding(IntegrityCode::UnsafeFile, path, error.to_string()),
            })?
            .ok_or_else(|| {
                finding(
                    IntegrityCode::MissingFile,
                    path,
                    "payload directory is absent",
                )
            })?;
    }
    Err(finding(
        IntegrityCode::UnsafeFile,
        path,
        "payload path is empty",
    ))
}

fn payload_check(
    root: &File,
    path: &str,
    expected: &Payload,
    budget: &mut Budget,
    findings: &mut Vec<IntegrityFinding>,
) -> std::result::Result<(), IntegrityFinding> {
    let mut file = open_payload(root, path)?;
    let before = file.metadata().map_err(|error| io_finding(path, error))?;
    // SAFETY: geteuid only reads the process's effective identity.
    let current_user = unsafe { libc::geteuid() };
    if before.mode() & 0o7777 != expected.mode {
        findings.push(finding(
            IntegrityCode::ModeMismatch,
            path,
            format!(
                "expected mode {:04o}, found {:04o}",
                expected.mode,
                before.mode() & 0o7777
            ),
        ));
    }
    if !before.is_file()
        || before.nlink() != 1
        || (before.uid() != current_user && before.uid() != 0)
        || before.mode() & 0o022 != 0
    {
        return Err(finding(IntegrityCode::UnsafeFile, path,
            "payload must be a singly linked regular file, owned by the current user or root and not group/world writable"));
    }
    let read_limit = before.len().saturating_add(1);
    if before.len() > MAX_FILE_BYTES || read_limit > budget.bytes {
        return Err(finding(
            IntegrityCode::LimitExceeded,
            path,
            "payload exceeds the bounded hashing budget",
        ));
    }
    budget.bytes -= read_limit;
    let mut hasher = Sha256::new();
    let mut bytes = 0;
    let mut buffer = [0; 65536];
    let mut bounded = (&mut file).take(read_limit);
    loop {
        let count = bounded
            .read(&mut buffer)
            .map_err(|error| io_finding(path, error))?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        hasher.update(&buffer[..count]);
    }
    let after = file.metadata().map_err(|error| io_finding(path, error))?;
    let current = open_payload(root, path)?
        .metadata()
        .map_err(|error| io_finding(path, error))?;
    if bytes != before.len() || !same_file(&before, &after) || !same_file(&after, &current) {
        return Err(finding(
            IntegrityCode::ChangedDuringRead,
            path,
            "payload changed while reading; retry inspection",
        ));
    }
    if format!("{:x}", hasher.finalize()) != expected.sha256 {
        findings.push(finding(
            IntegrityCode::ChecksumMismatch,
            path,
            "payload SHA-256 differs from its receipt",
        ));
    }
    Ok(())
}

fn same_file(left: &Metadata, right: &Metadata) -> bool {
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.len() == right.len()
        && left.mode() == right.mode()
        && left.uid() == right.uid()
        && left.gid() == right.gid()
        && left.nlink() == right.nlink()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn aggregate_byte_budget_refuses_the_next_payload_before_hashing() {
        let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/preview");
        fs::create_dir_all(&target).unwrap();
        let temporary = tempfile::tempdir_in(target).unwrap();
        let prefix = temporary.path().canonicalize().unwrap();
        fs::create_dir(prefix.join("bin")).unwrap();
        for name in ["first", "second"] {
            let path = prefix.join("bin").join(name);
            fs::write(&path, b"data").unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let expected = Payload {
            mode: 0o755,
            sha256: crate::digest(&prefix.join("bin/first")).unwrap(),
        };
        let directory = read_only::directory(&prefix).unwrap().unwrap();
        let mut budget = Budget { files: 2, bytes: 5 };
        let mut findings = vec![];
        payload_check(
            &directory,
            "bin/first",
            &expected,
            &mut budget,
            &mut findings,
        )
        .unwrap();
        assert_eq!(budget.bytes, 0);
        let error = payload_check(
            &directory,
            "bin/second",
            &expected,
            &mut budget,
            &mut findings,
        )
        .unwrap_err();
        assert_eq!(error.code, IntegrityCode::LimitExceeded);
        assert!(findings.is_empty());
    }

    #[test]
    fn changed_size_mode_and_inode_invalidate_the_file_snapshot() {
        let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/preview");
        fs::create_dir_all(&target).unwrap();
        let temporary = tempfile::tempdir_in(target).unwrap();
        let path = temporary.path().join("payload");
        fs::write(&path, b"data").unwrap();
        let before = fs::metadata(&path).unwrap();
        assert!(same_file(&before, &before));
        fs::write(&path, b"longer contents").unwrap();
        assert!(!same_file(&before, &fs::metadata(&path).unwrap()));
        let before = fs::metadata(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(!same_file(&before, &fs::metadata(&path).unwrap()));
        let before = fs::metadata(&path).unwrap();
        let other = temporary.path().join("replacement");
        fs::write(&other, b"longer contents").unwrap();
        fs::rename(&other, &path).unwrap();
        assert!(!same_file(&before, &fs::metadata(&path).unwrap()));
    }
}
