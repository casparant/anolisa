//! Format 2 receipts record executor-owned installation identity and source evidence.

use super::filesystem as paths;
use crate::{
    open_bundle::{self, reader, Bundle, Package},
    require, Result,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

pub(super) const NAME: &str = ".aw-installation.json";
pub(super) const LIMIT: usize = 4 * 1024 * 1024;

/// Verified local source identity; a checksum does not authenticate its publisher.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// Source mechanism, exactly `local_bundle` in this format.
    pub kind: String,
    /// Original absolute bundle location, retained only as provenance.
    pub path: PathBuf,
    /// Always false until a separately designed publisher-verification scheme exists.
    pub publisher_verified: bool,
}

/// Executor-selected privilege scope; manifests cannot assign it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    /// Exactly `user` for this raw installation implementation.
    pub kind: String,
    /// Non-root effective identity that installed the prefix.
    pub uid: u32,
}

/// Complete ownership record for one independently installed bundle closure.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    /// Receipt version, exactly 2 and separate from Preview receipts.
    pub receipt_format: u8,
    /// Packages validated at installation, including their exact owned files.
    pub packages: Vec<Package>,
    /// Digest of the exact original manifest bytes, not canonical reserialization.
    pub manifest_sha256: String,
    /// Non-authoritative source location and publisher-verification boundary.
    pub source: Source,
    /// User ownership chosen by the executor, not the bundle author.
    pub scope: Scope,
    /// Installation backend, exactly `raw` in this prototype.
    pub backend: String,
}

pub(super) fn read(root: &File) -> Result<Receipt> {
    let bytes = reader::document(root, NAME, LIMIT)?;
    let receipt: Receipt = serde_json::from_slice(&bytes)?;
    validate(&receipt)?;
    let metadata = root.metadata()?;
    let record = reader::payload(root, NAME)?.metadata()?;
    require(
        metadata.uid() == receipt.scope.uid && record.uid() == receipt.scope.uid,
        "receipt user identity does not match prefix and receipt ownership",
    )?;
    require(
        !super::has_child(root, ".aw-packages")?,
        "mixed Preview and format 2 ownership records are unsupported",
    )?;
    Ok(receipt)
}

pub(super) fn validate(receipt: &Receipt) -> Result<()> {
    require(
        receipt.receipt_format == 2
            && receipt.backend == "raw"
            && receipt.scope.kind == "user"
            && receipt.scope.uid != 0
            && receipt.source.kind == "local_bundle"
            && !receipt.source.publisher_verified,
        "unsupported installation receipt format, scope, backend or trust declaration",
    )?;
    require(
        receipt.manifest_sha256.len() == 64
            && receipt
                .manifest_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "invalid receipt manifest digest",
    )?;
    require(
        receipt.source.path.is_absolute()
            && !receipt
                .source
                .path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir)),
        "receipt source must be an absolute local path",
    )?;
    open_bundle::validate(&Bundle {
        format: 2,
        packages: receipt.packages.clone(),
    })
}

pub(super) fn new(bundle: &Path, inspected: open_bundle::Inspection) -> Receipt {
    Receipt {
        receipt_format: 2,
        packages: inspected.manifest.packages,
        manifest_sha256: inspected.manifest_sha256,
        source: Source {
            kind: "local_bundle".into(),
            path: bundle.into(),
            publisher_verified: false,
        },
        scope: Scope {
            kind: "user".into(),
            uid: paths::uid(),
        },
        backend: "raw".into(),
    }
}
