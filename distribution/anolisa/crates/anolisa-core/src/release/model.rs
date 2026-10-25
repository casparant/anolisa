//! Descriptors bind a fixed, ordered package composition to independently verified bytes.

use super::{Error, Result, files, require, selection::Distribution};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// The product composition delivered in `release.json`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Product manifest format; only version one is supported.
    pub schema_version: u32,
    /// Product version, independent of component versions.
    pub product_version: String,
    /// Immutable, path-safe identity of this build.
    pub release_id: String,
    /// Exact source commit used by the distributor.
    pub source_commit: String,
    /// Target operating system, currently Linux.
    pub os: String,
    /// Rust architecture identifier, currently x86_64 or aarch64.
    pub arch: String,
    /// Target userspace ABI, currently GNU/glibc.
    pub abi: String,
    /// Oldest ANOLISA installer allowed to interpret this manifest.
    pub min_installer_version: String,
    /// Packages in dependency order; paths are not discovered by enumeration.
    pub packages: Vec<PackageRef>,
    /// Public command names mapped to owned executable paths inside the prefix.
    pub entrypoints: BTreeMap<String, String>,
    /// Versioned product choices; absent only in legacy complete bundles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distribution: Option<Distribution>,
}

/// A package selected by the immutable product composition.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageRef {
    /// AW package identity.
    pub package_id: String,
    /// Exact package SemVer.
    pub version: String,
    /// Bundle-relative directory, exactly `packages/<package_id>`.
    pub path: String,
    /// Lowercase SHA-256 of the exact `package.json` bytes.
    pub description_sha256: String,
}

/// Ordinary regular-file ownership accepted by AW's local package format.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    /// Lowercase SHA-256 of the file bytes.
    pub sha256: String,
    /// Ordinary permissions, encoded as decimal 420 or 493.
    pub mode: u32,
}

/// Exact, same-prefix package dependency.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    /// Required package identity.
    pub package_id: String,
    /// Required exact SemVer.
    pub version: String,
}

/// Optional software protocol metadata; installing it does not enable a Provider.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    /// Supported AW wire protocol.
    pub protocol: String,
}

/// The generic AW package description; no scripts or host actions are accepted.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalPackage {
    /// AW local description version.
    pub schema_version: u32,
    /// Package identity matching the product reference.
    pub package_id: String,
    /// Independent exact SemVer.
    pub version: String,
    /// Required operating system.
    pub os: String,
    /// Required native architecture.
    pub arch: String,
    /// Owned prefix-relative regular files.
    pub files: BTreeMap<String, Payload>,
    /// Exact requirements supplied earlier in this release.
    pub dependencies: Vec<Requirement>,
    /// Optional full source commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_commit: Option<String>,
    /// Optional Provider protocol, never launch configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Provider>,
}

/// A fully verified input; execution rechecks this evidence before each invocation.
#[derive(Clone, Debug, Serialize)]
pub struct Bundle {
    /// Canonical input directory, without symbolic-link traversal.
    pub root: PathBuf,
    /// Immutable composition and platform facts.
    pub manifest: Manifest,
    /// Digest binding the exact release description.
    pub manifest_sha256: String,
    /// Package descriptions in installation order.
    pub packages: Vec<LocalPackage>,
    /// Total payload bytes; the complete prefix must fit AW's verification budget.
    pub payload_bytes: u64,
}

impl Bundle {
    /// Verify every descriptor, dependency, mode and file before any installation write.
    ///
    /// # Errors
    /// Rejects unsafe paths, unsupported formats, duplicate ownership, invalid ordering,
    /// incompatible installer versions and damaged or oversized payloads.
    pub fn read(root: &Path) -> Result<Self> {
        files::absolute(root)?;
        files::directory(root)?;
        let bytes = files::read(&root.join("release.json"), 4 * 1024 * 1024)?;
        let manifest: Manifest = files::decode(&bytes)?;
        require(
            matches!(manifest.schema_version, 1 | 2),
            "unsupported product schema_version",
        )?;
        require(
            (manifest.schema_version == 2) == manifest.distribution.is_some(),
            "schema two requires distribution choices; schema one has none",
        )?;
        identifier(&manifest.release_id)?;
        version(&manifest.product_version)?;
        hex(&manifest.source_commit, 40)?;
        let minimum = version(&manifest.min_installer_version)?;
        let current = version(env!("CARGO_PKG_VERSION"))?;
        require(
            current >= minimum,
            format!("installer {current} is older than required {minimum}"),
        )?;
        require(
            manifest.os == "linux"
                && matches!(manifest.arch.as_str(), "x86_64" | "aarch64")
                && manifest.abi == "gnu",
            "release requires a supported Linux GNU target",
        )?;
        require(
            !manifest.packages.is_empty() && manifest.packages.len() <= 1024,
            "invalid release package count",
        )?;
        let mut available = BTreeMap::new();
        let mut owned = BTreeMap::new();
        let mut packages = Vec::new();
        let mut payload_bytes = 0_u64;
        for reference in &manifest.packages {
            identifier(&reference.package_id)?;
            version(&reference.version)?;
            hex(&reference.description_sha256, 64)?;
            require(
                reference.path == format!("packages/{}", reference.package_id),
                "package path must be packages/<package_id>",
            )?;
            require(
                !available.contains_key(&reference.package_id),
                "duplicate product package ID",
            )?;
            let directory = root.join(&reference.path);
            match std::fs::symlink_metadata(directory.join("manifest.json")) {
                Ok(_) => {
                    return Err(Error::Invalid(
                        "generic product package must not contain a Preview manifest".into(),
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(files::io(&directory, error)),
            }
            let descriptor = files::read(&directory.join("package.json"), 1024 * 1024)?;
            require(
                files::digest_bytes(&descriptor) == reference.description_sha256,
                format!(
                    "package description digest mismatch: {}",
                    reference.package_id
                ),
            )?;
            let package: LocalPackage = files::decode(&descriptor)?;
            require(
                package.schema_version == 1
                    && package.package_id == reference.package_id
                    && package.version == reference.version
                    && package.os == manifest.os
                    && package.arch == manifest.arch,
                "package identity or platform differs from release",
            )?;
            if let Some(commit) = &package.source_commit {
                hex(commit, 40)?;
            }
            if let Some(provider) = &package.provider {
                require(
                    provider.protocol == "aw-provider/v1alpha1",
                    "unsupported Provider protocol",
                )?;
            }
            let mut required = BTreeSet::new();
            for dependency in &package.dependencies {
                identifier(&dependency.package_id)?;
                version(&dependency.version)?;
                require(
                    required.insert(&dependency.package_id)
                        && available.get(&dependency.package_id) == Some(&dependency.version),
                    format!(
                        "{} requires earlier same-prefix {}@{}",
                        package.package_id, dependency.package_id, dependency.version
                    ),
                )?;
            }
            require(
                !package.files.is_empty() && package.files.len() <= 16384,
                "invalid package file count",
            )?;
            for (name, payload) in &package.files {
                payload_path(name)?;
                hex(&payload.sha256, 64)?;
                require(
                    matches!(payload.mode, 0o644 | 0o755),
                    "payload mode must be 0644 or 0755",
                )?;
                require(
                    owned.insert(name.clone(), payload.clone()).is_none(),
                    format!("duplicate release file ownership: {name}"),
                )?;
                let source = directory.join("payload").join(name);
                let metadata = files::regular(&source)?;
                use std::os::unix::fs::PermissionsExt;
                require(
                    metadata.permissions().mode() & 0o7777 == payload.mode,
                    format!("payload mode mismatch: {}", source.display()),
                )?;
                require(
                    metadata.len() <= 1024 * 1024 * 1024,
                    "payload exceeds AW's single-file byte limit",
                )?;
                payload_bytes = payload_bytes
                    .checked_add(metadata.len())
                    .ok_or_else(|| Error::Invalid("payload size overflow".into()))?;
                require(
                    payload_bytes <= 4 * 1024 * 1024 * 1024,
                    "release exceeds AW's complete-prefix byte limit",
                )?;
                require(
                    files::digest(&source)? == payload.sha256,
                    format!("payload digest mismatch: {}", source.display()),
                )?;
            }
            available.insert(package.package_id.clone(), package.version.clone());
            packages.push(package);
        }
        require(
            owned.len() <= 32767,
            "release exceeds AW's complete-prefix file limit",
        )?;
        for name in owned.keys() {
            for parent in Path::new(name).ancestors().skip(1) {
                require(
                    !parent
                        .to_str()
                        .is_some_and(|parent| owned.contains_key(parent)),
                    "payload file is another owned file's parent",
                )?;
            }
        }
        require(
            manifest
                .entrypoints
                .get("anolisa")
                .is_some_and(|path| path == "bin/anolisa"),
            "release requires the anolisa public entrypoint",
        )?;
        for (name, target) in &manifest.entrypoints {
            identifier(name)?;
            payload_path(target)?;
            require(
                owned
                    .get(target)
                    .is_some_and(|payload| payload.mode == 0o755),
                format!("entrypoint {name} does not select an owned executable"),
            )?;
        }
        if let Some(distribution) = &manifest.distribution {
            distribution.validate(&manifest)?;
        }
        require(
            files::read(&root.join("release.json"), 4 * 1024 * 1024)? == bytes,
            "release changed during verification",
        )?;
        Ok(Self {
            root: root.to_owned(),
            manifest,
            manifest_sha256: files::digest_bytes(&bytes),
            packages,
            payload_bytes,
        })
    }

    pub(super) fn command_evidence(&self) -> Result<()> {
        require(
            files::digest_bytes(&files::read(
                &self.root.join("release.json"),
                4 * 1024 * 1024,
            )?) == self.manifest_sha256,
            "release descriptor changed before package operation",
        )?;
        for reference in &self.manifest.packages {
            let path = self.root.join(&reference.path).join("package.json");
            require(
                files::digest_bytes(&files::read(&path, 1024 * 1024)?)
                    == reference.description_sha256,
                "package descriptor changed before package operation",
            )?;
        }
        Ok(())
    }

    /// Recheck the complete immutable input before executing another package operation.
    ///
    /// # Errors
    /// Rejects changed manifests, payloads or package composition.
    pub fn recheck(&self) -> Result<()> {
        let actual = Self::read(&self.root)?;
        require(
            actual.manifest_sha256 == self.manifest_sha256,
            "release descriptor changed after verification",
        )
    }
}

pub(super) fn identifier(value: &str) -> Result<()> {
    require(
        !value.is_empty()
            && value.len() <= 128
            && value.as_bytes()[0].is_ascii_alphanumeric()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte)),
        format!("invalid release identifier: {value}"),
    )
}

fn version(value: &str) -> Result<Version> {
    require(
        value.len() <= 128,
        "exact version exceeds AW's length limit",
    )?;
    Version::parse(value)
        .map_err(|error| Error::Invalid(format!("invalid exact SemVer {value}: {error}")))
}

fn hex(value: &str, count: usize) -> Result<()> {
    require(
        value.len() == count
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "expected lowercase hexadecimal digest or source commit",
    )
}

pub(super) fn payload_path(value: &str) -> Result<()> {
    let mut parts = value.split('/');
    require(
        matches!(parts.next(), Some("bin" | "libexec" | "share"))
            && !value.contains('\\')
            && !value.contains('\0')
            && parts.clone().next().is_some()
            && parts.all(|part| !part.is_empty() && part != "." && part != ".."),
        format!("invalid AW payload path: {value}"),
    )
}
