//! Bounded, independently versioned local bundles for tools and Providers.

pub(crate) mod reader;
mod validation;

use crate::{require, Result};
use serde::{de::MapAccess, Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt, path::Path};

pub use validation::validate;

pub(crate) const MANIFEST_LIMIT: usize = 4 * 1024 * 1024;
pub(crate) const FILE_LIMIT: u64 = 512 * 1024 * 1024;
pub(crate) const TOTAL_LIMIT: u64 = 2 * 1024 * 1024 * 1024;

/// A complete local dependency closure, separate from Preview format 1.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    /// Schema version, currently exactly 2.
    pub format: u8,
    /// Independently versioned packages with disjoint payload paths.
    pub packages: Vec<Package>,
}

/// Software identity and payload declaration, not permission to execute it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Package {
    /// Namespaced ASCII identity; this does not authenticate a publisher.
    pub id: String,
    /// Package version, independent of the AW version.
    pub version: String,
    /// Whether this package exposes a Provider protocol entrypoint.
    pub kind: Kind,
    /// Supported host platform.
    pub platform: Platform,
    /// Exact dependencies that must be present in this same bundle.
    #[serde(default)]
    pub requires: Vec<Dependency>,
    /// Prefix-relative regular files, without duplicate JSON object keys.
    #[serde(deserialize_with = "unique_files")]
    pub files: BTreeMap<String, Payload>,
    /// Provider declaration required only for Provider packages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Provider>,
}

/// Package role; installation does not activate either role.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A standalone tool without a Provider declaration.
    Tool,
    /// A process implementing the declared Provider wire protocol.
    Provider,
}

/// Native platform admitted by this local bundle implementation.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Platform {
    /// Operating system; currently Linux only.
    pub os: String,
    /// Rust host architecture name, such as `x86_64` or `aarch64`.
    pub arch: String,
}

/// An exact, bundle-local package dependency.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    /// Namespaced identity of another package in the bundle.
    pub id: String,
    /// Required version, matched exactly rather than as a version range.
    pub version: String,
}

/// Bounded integrity metadata for one owned regular file.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    /// Exact byte length, checked before and during reads.
    pub size: u64,
    /// Lowercase SHA-256 digest of the payload bytes.
    pub sha256: String,
    /// Destination permissions, restricted to 0644 or 0755.
    pub mode: u32,
}

/// A declared Provider entrypoint, without generated runtime configuration.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    /// Supported wire protocol, independent of the package version.
    pub protocol: String,
    /// Prefix-relative executable owned by this same package.
    pub executable: String,
}

/// Verified manifest and identity of its exact original JSON bytes.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Inspection {
    /// Validated metadata and verified local payload declarations.
    pub manifest: Bundle,
    /// SHA-256 of the raw manifest, not a reserialized representation.
    pub manifest_sha256: String,
}

/// Read and verify a local bundle without creating files or executing software.
///
/// This verifies consistency and bounded reads, not publisher authenticity.
/// Execution must reopen and revalidate payloads because inspection reserves
/// neither the source directory nor its contents.
pub fn inspect(bundle: &Path) -> Result<Inspection> {
    let root = reader::root(bundle)?;
    let bytes = reader::document(&root, "manifest.json", MANIFEST_LIMIT)?;
    let manifest: Bundle = serde_json::from_slice(&bytes)?;
    validate(&manifest)?;
    for package in &manifest.packages {
        for (path, expected) in &package.files {
            let mut file = reader::payload(&root, &format!("payload/{path}"))?;
            reader::verify(&mut file, expected, path)?;
        }
    }
    Ok(Inspection {
        manifest,
        manifest_sha256: format!("{:x}", Sha256::digest(&bytes)),
    })
}

fn unique_files<'de, D>(deserializer: D) -> std::result::Result<BTreeMap<String, Payload>, D::Error>
where
    D: Deserializer<'de>,
{
    struct Files;
    impl<'de> serde::de::Visitor<'de> for Files {
        type Value = BTreeMap<String, Payload>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a payload object with unique relative paths")
        }

        fn visit_map<M>(self, mut map: M) -> std::result::Result<Self::Value, M::Error>
        where
            M: MapAccess<'de>,
        {
            let mut files = BTreeMap::new();
            while let Some((path, payload)) = map.next_entry::<String, Payload>()? {
                if files.insert(path.clone(), payload).is_some() {
                    return Err(serde::de::Error::custom(format!(
                        "duplicate payload: {path}"
                    )));
                }
                if files.len() > 8192 {
                    return Err(serde::de::Error::custom("too many payload files"));
                }
            }
            Ok(files)
        }
    }
    deserializer.deserialize_map(Files)
}

pub(crate) fn regular_relative(path: &str) -> Result<()> {
    require(
        !path.is_empty()
            && path.len() <= 4096
            && !path.chars().any(char::is_control)
            && !path.contains('\\')
            && path
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".." && part.len() <= 255),
        format!("invalid relative bundle path: {path:?}"),
    )
}
