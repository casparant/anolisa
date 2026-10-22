//! User-scoped, no-overwrite lifecycle for independently versioned local bundles.

mod filesystem;
mod health;
mod receipt;
mod uninstall;

pub use receipt::{Receipt, Scope, Source};
pub use uninstall::uninstall;

use crate::{
    integrity::IntegrityReport,
    open_bundle::{self, reader},
    require, Error, InstallPlan, InventoryState, PlanActivation, PlanComponent, PlanFiles,
    PlanPrivileges, PlanSource, Result,
};
use filesystem as paths;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, Permissions},
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

/// Inspect a bundle and report a non-mutating, immediately staleable installation plan.
///
/// Payload hashes establish consistency, not publisher authenticity. Execution
/// checks inputs again; neither directories nor installation locks are created.
pub fn plan(bundle: &Path, prefix: &Path) -> Result<InstallPlan> {
    let inspection = open_bundle::inspect(bundle)?;
    let mut conflicts = vec![];
    if let Err(error) = paths::ordinary_user() {
        conflicts.push(error.to_string());
    }
    let existing = match paths::location(prefix) {
        Ok((parent, name)) => match paths::absent(&parent, &name) {
            Ok(()) => InventoryState::Absent,
            Err(error) => {
                conflicts.push(error.to_string());
                if read_receipt(prefix).is_ok() {
                    InventoryState::Installed
                } else {
                    InventoryState::Unmanaged
                }
            }
        },
        Err(error) => {
            conflicts.push(error.to_string());
            InventoryState::Unmanaged
        }
    };
    Ok(InstallPlan {
        prefix: prefix.into(), existing,
        source: PlanSource { kind: "local_bundle", path: bundle.into(), payload_verified: true, publisher_verified: false },
        components: inspection.manifest.packages.iter().map(|package| PlanComponent { name: package.id.clone(), version: package.version.clone(), installed_version: None }).collect(),
        files: PlanFiles {
            payloads: inspection.manifest.packages.iter().flat_map(|package| package.files.keys()).map(|path| prefix.join(path)).collect(),
            receipts: vec![prefix.join(receipt::NAME)],
            lock: prefix.into(),
            creates_prefix: existing == InventoryState::Absent,
        },
        privileges: PlanPrivileges { effective_uid: paths::uid(), automatic_elevation: false },
        activation: PlanActivation { starts_services: false, registers_providers: false, connects_agents: false },
        conflicts,
        not_checked: vec![
            "disk capacity and effective write access",
            "source and target changes after this snapshot; execution rechecks before atomic publication",
            "publisher authenticity; installation requires explicit consent for unverified local software",
            "Provider, Agent and service runtime readiness or references",
        ],
    })
}

/// Install a complete local bundle into a new user prefix without executing it.
///
/// The caller must obtain consent for unverified software. This library does not
/// authenticate publishers, change PATH, create Provider instances or elevate
/// privileges. Files and receipts are prepared privately and published with an
/// atomic no-overwrite rename; cancellation before publication leaves no prefix.
pub fn install(bundle: &Path, prefix: &Path, cancel: &AtomicBool) -> Result<Receipt> {
    paths::ordinary_user()?;
    paths::cancelled(cancel)?;
    let (parent, name) = paths::location(prefix)?;
    paths::absent(&parent, &name)?;
    let inspected = open_bundle::inspect(bundle)?;
    paths::cancelled(cancel)?;
    let source = reader::root(bundle)?;
    let manifest_bytes = reader::document(&source, "manifest.json", open_bundle::MANIFEST_LIMIT)?;
    require(
        format!("{:x}", Sha256::digest(&manifest_bytes)) == inspected.manifest_sha256,
        "bundle manifest changed after inspection; retry installation",
    )?;
    let record = receipt::new(bundle, inspected);
    let bytes = serde_json::to_vec_pretty(&record)?;
    require(
        bytes.len() <= receipt::LIMIT,
        "generated installation receipt exceeds byte limit",
    )?;
    let stage = tempfile::Builder::new()
        .prefix(".aw-install-")
        .tempdir_in(paths::descriptor(&parent))?;
    let stage_name = stage
        .path()
        .file_name()
        .ok_or_else(|| Error::Invalid("temporary installation has no name".into()))?
        .to_owned();
    let stage_root = paths::directory(&parent, &stage_name)?;
    for package in &record.packages {
        for (relative, expected) in &package.files {
            paths::cancelled(cancel)?;
            let mut input = reader::payload(&source, &format!("payload/{relative}"))?;
            let mut output = paths::create(&stage_root, relative)?;
            reader::copy_verified(&mut input, &mut output, expected, relative)?;
            output.set_permissions(Permissions::from_mode(expected.mode))?;
            output.sync_all()?;
        }
    }
    let mut output = paths::create(&stage_root, receipt::NAME)?;
    output.write_all(&bytes)?;
    output.sync_all()?;
    sync_directories(&stage_root, &record)?;
    paths::cancelled(cancel)?;
    paths::owned(&parent)?;
    paths::absent(&parent, &name)?;
    let (current_parent, _) = paths::location(prefix)?;
    require(
        paths::same_object(&parent.metadata()?, &current_parent.metadata()?),
        "installation parent changed; retry installation",
    )?;
    paths::rename(&parent, &stage_name, &parent, &name)?;
    // The original stage path no longer exists; retain only the published prefix.
    let _published_stage_name = stage.keep();
    parent.sync_all()?;
    Ok(record)
}

/// Read a bounded format 2 receipt without opening or executing its payloads.
/// Invalid metadata, mixed formats, links and unsafe ownership are rejected.
pub fn read_receipt(prefix: &Path) -> Result<Receipt> {
    receipt::read(&reader::root(prefix)?)
}

/// Verify recorded payloads without repairs, runtime queries or side effects.
/// Malformed receipts fail; individual payload failures become explicit findings.
pub fn inspect(prefix: &Path) -> Result<IntegrityReport> {
    let root = reader::root(prefix)?;
    let record = receipt::read(&root)?;
    health::inspect(prefix, &root, &record)
}

fn has_child(root: &File, name: &str) -> Result<bool> {
    match fs::symlink_metadata(paths::descriptor(root).join(name)) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn sync_directories(root: &File, receipt: &Receipt) -> Result<()> {
    let mut directories = std::collections::BTreeSet::<PathBuf>::new();
    for path in receipt
        .packages
        .iter()
        .flat_map(|package| package.files.keys())
    {
        for parent in Path::new(path)
            .ancestors()
            .skip(1)
            .filter(|path| !path.as_os_str().is_empty())
        {
            directories.insert(parent.into());
        }
    }
    for directory in directories.into_iter().rev() {
        let label = directory
            .to_str()
            .ok_or_else(|| Error::Invalid("invalid payload directory".into()))?;
        let (parent, name) = paths::parent_for(root, label, false)?;
        paths::directory(&parent, &name)?.sync_all()?;
    }
    root.sync_all()?;
    Ok(())
}
