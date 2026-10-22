//! Read-only integrity checks reuse the open bundle's bounded payload reader.

use super::{filesystem as paths, receipt::Receipt};
use crate::{
    integrity::{
        ComponentIntegrity, IntegrityCode, IntegrityFinding, IntegrityReport, IntegrityStatus,
    },
    open_bundle::{reader, Payload},
    Error, InventoryState, Result,
};
use std::{fs::File, os::unix::fs::MetadataExt, path::Path};

pub(super) fn inspect(prefix: &Path, root: &File, receipt: &Receipt) -> Result<IntegrityReport> {
    let mut report = IntegrityReport {
        prefix: prefix.into(),
        state: InventoryState::Installed,
        checked_files: 0,
        checked_components: 0,
        unhealthy_components: 0,
        components: vec![],
    };
    for package in &receipt.packages {
        let mut component = ComponentIntegrity {
            component: package.id.clone(),
            version: package.version.clone(),
            status: IntegrityStatus::Checked,
            checked_files: 0,
            recorded_files: package.files.len(),
            findings: vec![],
        };
        for (path, expected) in &package.files {
            match check(root, path, expected, receipt.scope.uid) {
                Ok(()) => component.checked_files += 1,
                Err((code, message)) => component.findings.push(IntegrityFinding {
                    code,
                    path: Some(path.clone()),
                    related_component: None,
                    message,
                }),
            }
        }
        if component.findings.is_empty() {
            report.checked_components += 1;
        } else {
            component.status = IntegrityStatus::Unhealthy;
            report.unhealthy_components += 1;
        }
        report.checked_files += component.checked_files;
        report.components.push(component);
    }
    Ok(report)
}

fn check(
    root: &File,
    path: &str,
    expected: &Payload,
    owner: u32,
) -> std::result::Result<(), (IntegrityCode, String)> {
    paths::parent_for(root, path, false)
        .map_err(|error| (IntegrityCode::UnsafeFile, error.to_string()))?;
    let mut file = reader::payload(root, path).map_err(classify)?;
    let metadata = file.metadata().map_err(|error| classify(error.into()))?;
    if metadata.uid() != owner {
        return Err((
            IntegrityCode::UnsafeFile,
            "payload ownership differs from the installation scope".into(),
        ));
    }
    if metadata.mode() & 0o7777 != expected.mode {
        return Err((
            IntegrityCode::ModeMismatch,
            format!(
                "file mode differs from receipt: expected {:04o}, found {:04o}",
                expected.mode,
                metadata.mode() & 0o7777
            ),
        ));
    }
    if metadata.len() != expected.size {
        return Err((
            IntegrityCode::ChecksumMismatch,
            format!(
                "file size differs from receipt: expected {}, found {}",
                expected.size,
                metadata.len()
            ),
        ));
    }
    reader::verify(&mut file, expected, path).map_err(classify)
}

fn classify(error: Error) -> (IntegrityCode, String) {
    let code = match &error {
        Error::Io(error) if error.kind() == std::io::ErrorKind::NotFound => {
            IntegrityCode::MissingFile
        }
        Error::Io(error) if matches!(error.raw_os_error(), Some(libc::ELOOP | libc::ENOTDIR)) => {
            IntegrityCode::UnsafeFile
        }
        Error::Invalid(message) if message.starts_with("payload checksum mismatch:") => {
            IntegrityCode::ChecksumMismatch
        }
        Error::Invalid(message) if message.starts_with("bundle data changed while reading:") => {
            IntegrityCode::ChangedDuringRead
        }
        Error::Invalid(message) if message.starts_with("bundle data must be") => {
            IntegrityCode::UnsafeFile
        }
        _ => IntegrityCode::ReadFailed,
    };
    (code, error.to_string())
}
