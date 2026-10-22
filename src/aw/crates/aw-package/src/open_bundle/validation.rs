//! Pure format-2 declaration checks shared by bundle and receipt admission.

use super::{regular_relative, Bundle, Kind, FILE_LIMIT, TOTAL_LIMIT};
use crate::{require, Error, Result};
use std::collections::{BTreeMap, BTreeSet};

/// Validate metadata without reading files, mutating configuration or resolving sources.
///
/// This also admits receipt metadata, so all count, path, platform, dependency
/// and Provider restrictions are checked independently of JSON deserialization.
pub fn validate(manifest: &Bundle) -> Result<()> {
    require(
        manifest.format == 2 && (1..=64).contains(&manifest.packages.len()),
        "format 2 requires between 1 and 64 packages",
    )?;
    let mut packages = BTreeMap::new();
    let mut files = BTreeSet::new();
    let mut total_size = 0_u64;
    for package in &manifest.packages {
        identity(&package.id)?;
        version(&package.version)?;
        require(
            packages.insert(package.id.as_str(), package).is_none(),
            format!("duplicate package: {}", package.id),
        )?;
        require(
            std::env::consts::OS == "linux"
                && package.platform.os == "linux"
                && package.platform.arch == std::env::consts::ARCH,
            format!("incompatible platform for {}", package.id),
        )?;
        require(!package.files.is_empty(), "package has no owned payloads")?;
        for (path, payload) in &package.files {
            regular_relative(path)?;
            require(
                matches!(path.split_once('/'), Some(("bin" | "libexec" | "share", _))),
                format!("payload must be inside bin, libexec or share: {path:?}"),
            )?;
            require(
                files.insert(path.as_str()) && files.len() <= 8192,
                "duplicate payload ownership or more than 8192 files",
            )?;
            require(
                matches!(payload.mode, 0o644 | 0o755)
                    && payload.size <= FILE_LIMIT
                    && payload.sha256.len() == 64
                    && payload
                        .sha256
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                format!("invalid payload size, hash or mode: {path:?}"),
            )?;
            total_size += payload.size;
            require(total_size <= TOTAL_LIMIT, "bundle payloads exceed 2 GiB")?;
        }
        match (&package.kind, &package.provider) {
            (Kind::Tool, None) => {}
            (Kind::Provider, Some(provider)) => {
                require(
                    provider.protocol == "aw-provider/v1alpha1"
                        && package
                            .files
                            .get(&provider.executable)
                            .is_some_and(|payload| payload.mode == 0o755),
                    format!(
                        "unsupported Provider protocol or unowned executable: {}",
                        package.id
                    ),
                )?;
            }
            _ => {
                return Err(Error::Invalid(format!(
                    "package kind and Provider declaration disagree: {}",
                    package.id
                )))
            }
        }
    }
    for path in &files {
        for (index, _) in path.match_indices('/') {
            require(
                !files.contains(&path[..index]),
                format!("payload file is also a parent directory: {path:?}"),
            )?;
        }
    }
    for package in &manifest.packages {
        require(
            package.requires.len() <= 64,
            "too many package dependencies",
        )?;
        let mut required = BTreeSet::new();
        for dependency in &package.requires {
            identity(&dependency.id)?;
            version(&dependency.version)?;
            require(
                required.insert(&dependency.id),
                format!("duplicate dependency: {}", dependency.id),
            )?;
            require(
                packages
                    .get(dependency.id.as_str())
                    .is_some_and(|target| target.version == dependency.version),
                format!(
                    "{} requires bundle-local {} at exactly {}",
                    package.id, dependency.id, dependency.version
                ),
            )?;
        }
    }
    let mut visited = BTreeSet::new();
    let mut active = BTreeSet::new();
    for id in packages.keys() {
        visit(id, &packages, &mut active, &mut visited)?;
    }
    Ok(())
}

fn identity(value: &str) -> Result<()> {
    require(
        value.len() <= 128
            && value.contains('.')
            && value.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && label.bytes().all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || byte == b'-'
                            || byte == b'_'
                    })
                    && label
                        .as_bytes()
                        .first()
                        .is_some_and(u8::is_ascii_alphanumeric)
                    && label
                        .as_bytes()
                        .last()
                        .is_some_and(u8::is_ascii_alphanumeric)
            }),
        format!("invalid namespaced package identity: {value:?}"),
    )
}

fn version(value: &str) -> Result<()> {
    require(
        !value.is_empty()
            && value.len() <= 128
            && value.trim() == value
            && !value.chars().any(char::is_control),
        format!("invalid package version: {value:?}"),
    )
}

fn visit<'a>(
    id: &'a str,
    packages: &BTreeMap<&'a str, &'a super::Package>,
    active: &mut BTreeSet<&'a str>,
    visited: &mut BTreeSet<&'a str>,
) -> Result<()> {
    if visited.contains(id) {
        return Ok(());
    }
    require(
        active.insert(id),
        format!("package dependency cycle at {id}"),
    )?;
    let package = packages
        .get(id)
        .ok_or_else(|| Error::Invalid(format!("missing bundle dependency: {id}")))?;
    for dependency in &package.requires {
        visit(&dependency.id, packages, active, visited)?;
    }
    active.remove(id);
    visited.insert(id);
    Ok(())
}
