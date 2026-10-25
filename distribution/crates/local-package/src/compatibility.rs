//! Shared primitives retained for callers supporting historical AW installations.

use crate::{database::Installation, Component, Result};
use local_package_runtime::PackageLock;
use std::{
    collections::BTreeMap,
    fs::File,
    path::{Path, PathBuf},
};

/// Validate an absolute payload path and reject symbolic path ancestors.
pub fn absolute(path: &Path) -> Result<()> {
    crate::filesystem::absolute(path)
}

/// Validate that a directory belongs to the current user and is not publicly writable.
pub fn owned_directory(path: &Path) -> Result<()> {
    crate::filesystem::owned_directory(path)
}

/// Validate the owned parent of a prospective file.
pub fn parent(path: &Path) -> Result<()> {
    crate::filesystem::parent(path)
}

/// Publish a private file without replacing an existing destination.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    crate::filesystem::write_private(path, bytes)
}

/// Acquire the historical exclusive installation lock without waiting.
pub fn lock(prefix: &Path) -> Result<PackageLock> {
    crate::filesystem::lock(prefix)
}

/// Acquire an existing shared installation lock without creating or repairing it.
pub fn shared_lock(prefix: &Path) -> Result<PackageLock> {
    crate::filesystem::shared_lock(prefix)
}

/// Read selected historical component records without granting central database authority.
pub fn selected_receipts(
    prefix: &Path,
    names: Option<&[&str]>,
) -> Result<BTreeMap<String, Component>> {
    crate::install::selected_receipts(prefix, names)
}

/// Resolve logical payload paths using the recorded external resource root.
pub fn payload_path(prefix: &Path, resources: Option<&Path>, name: &str) -> PathBuf {
    crate::database::payload_path(prefix, resources, name)
}

/// Open a regular metadata file without following symbolic links.
pub fn safe_file(path: &Path) -> Result<File> {
    crate::database::safe_file(path)
}

/// Verify one owned payload against its recorded digest and mode within the remaining byte budget.
///
/// # Errors
/// Rejects unsafe files, modified contents, mode mismatches and exhausted byte limits.
pub fn verify_payload_file(path: &Path, expected: &crate::Payload, budget: &mut u64) -> Result<()> {
    crate::open_bundle::verify(path, expected, budget)
}

/// Verify both directions of an installation's external resource binding.
pub fn check_resources(installation: &Installation) -> Result<()> {
    crate::database::resources::check(installation)
}
