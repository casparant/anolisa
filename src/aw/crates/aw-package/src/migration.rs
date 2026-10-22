//! Read-only migration evidence from explicitly supplied legacy state documents.

mod legacy;
mod wire;

use crate::{read_only, require, Error, Result};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{Metadata, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
};

const SOURCE_LIMIT: usize = 8;
const DOCUMENT_LIMIT: usize = 4 * 1024 * 1024;
const RECORD_LIMIT: usize = 4096;
const SUMMARY_LIMIT: usize = 512;

/// Unlocked record inspection, not an executable migration or current host status.
#[derive(Clone, Debug, Serialize)]
pub struct Inspection {
    /// Version of this output, independent of the source document schema.
    pub format: u8,
    /// Evidence boundary, always `legacy_installation_records`.
    pub scope: &'static str,
    /// Always `read_only`; inspection does not acquire management authority.
    pub mode: &'static str,
    /// Supplied documents, in caller order.
    pub sources: Vec<Source>,
    /// Repeated identities requiring review, not confirmed runtime conflicts.
    pub overlaps: Vec<Overlap>,
    /// Evidence actually examined by this call.
    pub checked: Vec<&'static str>,
    /// Facts this inspection does not establish.
    pub not_checked: Vec<&'static str>,
    /// Always empty: no changes are performed or proposed for execution.
    pub changes: Vec<String>,
}

/// One supplied document, keeping its claims separate from observed file ownership.
#[derive(Clone, Debug, Serialize)]
pub struct Source {
    /// Explicit document path; no paths named inside it are opened.
    pub path: PathBuf,
    /// Compatibility format name, always `legacy_distribution`.
    pub source_format: &'static str,
    /// Original schema, without an in-memory upgrade.
    pub schema_version: u32,
    /// Actual uid from the opened file descriptor, not inferred installation scope.
    pub file_owner_uid: u32,
    /// File-level installation mode claimed by the document.
    pub recorded_install_mode: String,
    /// Legacy diagnostic prefix; it may not contain all installed files.
    pub recorded_prefix: String,
    /// Installation claims, including quarantined and legacy objects.
    pub records: Vec<Record>,
    /// Number of unverified framework integration receipts.
    pub adapter_claim_count: usize,
    /// Number of unverified historical resource-root anchors.
    pub adapter_trust_root_count: usize,
    /// Number of preserved quarantined installation records.
    pub quarantined_count: usize,
    /// Number of obsolete capability/osbase or unknown object kinds retained.
    pub legacy_object_count: usize,
}

/// A compact record summary; no file paths, service commands or private payloads.
#[derive(Clone, Debug, Serialize)]
pub struct Record {
    /// Original object vocabulary; not a new CLI namespace.
    pub kind: String,
    /// Recorded identity; inspection does not rename historical aliases.
    pub name: String,
    /// Claimed version or last cached native-package version.
    pub recorded_version: Option<String>,
    /// Historical lifecycle label, not a current health check.
    pub recorded_status: String,
    /// Claimed installation scope, distinct from the state file's owner.
    pub recorded_scope: Scope,
    /// Claimed installation mechanism, or `unknown` when not established.
    pub recorded_backend: String,
    /// Historical management claim, never a newly granted permission.
    pub recorded_relation: String,
    /// Native package identity, when explicitly recorded.
    pub recorded_package: Option<String>,
    /// Recorded payload entries; their paths and contents are not inspected.
    pub file_count: usize,
    /// Recorded service entries; no service manager is queried.
    pub service_count: usize,
    /// Evidence limitations or inconsistent claims requiring review.
    pub findings: Vec<Finding>,
}

/// Recorded scope; old user documents have no uid and retain `None`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Scope {
    /// `user` or `system`, as declared in the document.
    pub kind: String,
    /// User identity only when the source schema explicitly records it.
    pub uid: Option<u32>,
}

/// Fixed, non-sensitive diagnostic text independent of private source values.
#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    /// Stable machine-readable classification.
    pub code: &'static str,
    /// Short explanation without embedded source content.
    pub message: &'static str,
}

/// Repeated object identity across documents or scopes, without precedence claims.
#[derive(Clone, Debug, Serialize)]
pub struct Overlap {
    /// Original object kind.
    pub kind: String,
    /// Exact recorded name; aliases are not guessed.
    pub name: String,
    /// Zero-based indices into the report's sources.
    pub source_indices: Vec<usize>,
    /// Distinct recorded scopes involved in this candidate overlap.
    pub scopes: Vec<Scope>,
}

/// Inspect one to eight explicitly supplied schema 1–6 state files without writes.
///
/// Reads at most 4 MiB per document and 4096 aggregate records. Metadata must be
/// a single-link regular file owned by root or the current user, without group or
/// world write access. Every path segment is opened without following symlinks.
/// Source documents remain historical claims: no package database, recorded path,
/// framework resource, executable, service or network endpoint is accessed.
///
/// # Errors
/// Rejects missing, unsafe, duplicate, oversized, malformed or unsupported inputs.
/// Parse failures deliberately omit source excerpts and private field contents.
pub fn inspect(states: &[PathBuf]) -> Result<Inspection> {
    require(
        !states.is_empty() && states.len() <= SOURCE_LIMIT,
        "migration inspection requires between one and eight state files",
    )?;
    let mut seen_paths = BTreeSet::new();
    let mut seen_files = BTreeSet::new();
    let mut remaining = RECORD_LIMIT;
    let mut sources = Vec::new();
    for path in states {
        require(
            path.is_absolute()
                && path.file_name().is_some()
                && !path
                    .components()
                    .any(|part| matches!(part, Component::ParentDir)),
            "state paths must be absolute and contain no '..' segments",
        )?;
        summary(&path.to_string_lossy())?;
        require(seen_paths.insert(path.clone()), "duplicate state path")?;
        let (bytes, metadata) = read(path)?;
        require(
            seen_files.insert((metadata.dev(), metadata.ino())),
            "duplicate state file identity",
        )?;
        let mut source = legacy::summarize(&bytes, &mut remaining)?;
        source.path = path.clone();
        source.file_owner_uid = metadata.uid();
        sources.push(source);
    }
    Ok(Inspection {
        format: 1,
        scope: "legacy_installation_records",
        mode: "read_only",
        overlaps: overlaps(&sources),
        sources,
        checked: vec!["provided_state_documents", "state_file_metadata"],
        not_checked: vec![
            "native_package_database",
            "payload_integrity",
            "services",
            "agent_integration",
            "migration_compatibility",
            "current_runtime_precedence",
        ],
        changes: Vec::new(),
    })
}

fn read(path: &Path) -> Result<(Vec<u8>, Metadata)> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::Invalid("state path has no parent".into()))?;
    let parent = read_only::directory(parent)?
        .ok_or_else(|| Error::Invalid("explicit state file is absent".into()))?;
    let name = path
        .file_name()
        .ok_or_else(|| Error::Invalid("state path has no name".into()))?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(read_only::descriptor_path(&parent).join(name))?;
    let before = file.metadata()?;
    // SAFETY: geteuid only reads the effective identity and cannot change state.
    let uid = unsafe { libc::geteuid() };
    require(
        before.is_file() && before.nlink() == 1
            && (before.uid() == 0 || before.uid() == uid) && before.mode() & 0o022 == 0,
        "state metadata must be a single-link regular file owned by the current user or root and not group/world writable",
    )?;
    require(
        before.len() <= DOCUMENT_LIMIT as u64,
        "state document exceeds byte limit",
    )?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(DOCUMENT_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)?;
    require(
        bytes.len() <= DOCUMENT_LIMIT,
        "state document exceeds byte limit",
    )?;
    let after = file.metadata()?;
    require(
        before.len() == after.len()
            && before.uid() == after.uid()
            && before.mode() == after.mode()
            && before.nlink() == after.nlink()
            && before.mtime() == after.mtime()
            && before.mtime_nsec() == after.mtime_nsec()
            && before.ctime() == after.ctime()
            && before.ctime_nsec() == after.ctime_nsec(),
        "state document changed while reading; retry inspection",
    )?;
    Ok((bytes, after))
}

fn summary(value: &str) -> Result<()> {
    require(
        value.len() <= SUMMARY_LIMIT,
        "state summary field exceeds byte limit",
    )
}

fn finding(code: &'static str, message: &'static str) -> Finding {
    Finding { code, message }
}

#[derive(Default)]
struct RepeatedIdentity {
    indices: BTreeSet<usize>,
    scopes: BTreeSet<Scope>,
}

fn overlaps(sources: &[Source]) -> Vec<Overlap> {
    let mut identities = BTreeMap::<(String, String), RepeatedIdentity>::new();
    for (index, source) in sources.iter().enumerate() {
        for record in &source.records {
            let repeated = identities
                .entry((record.kind.clone(), record.name.clone()))
                .or_default();
            repeated.indices.insert(index);
            repeated.scopes.insert(record.recorded_scope.clone());
        }
    }
    identities
        .into_iter()
        .filter(|(_, repeated)| repeated.indices.len() > 1 || repeated.scopes.len() > 1)
        .map(|((kind, name), repeated)| Overlap {
            kind,
            name,
            source_indices: repeated.indices.into_iter().collect(),
            scopes: repeated.scopes.into_iter().collect(),
        })
        .collect()
}
