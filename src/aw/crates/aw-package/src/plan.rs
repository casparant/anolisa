//! Advisory installation snapshots reuse execution preflight without creating resources.

use crate::{filesystem as paths, inventory, read_only, require, InventoryState, Manifest, Result};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Local source whose manifest and payload checksums have been inspected.
#[derive(Clone, Debug, Serialize)]
pub struct PlanSource {
    /// Source mechanism; currently always `local_bundle`.
    pub kind: &'static str,
    /// Absolute unpacked bundle directory.
    pub path: PathBuf,
    /// Payload checksums match this bundle's manifest, not a trusted publisher.
    pub payload_verified: bool,
    /// This format has no publisher signature verification.
    pub publisher_verified: bool,
}

/// One incoming component and any already recorded version.
#[derive(Clone, Debug, Serialize)]
pub struct PlanComponent {
    /// Stable Preview component identity.
    pub name: String,
    /// Incoming version from the manifest.
    pub version: String,
    /// Existing receipt version; reinstalling into the same prefix is a conflict.
    pub installed_version: Option<String>,
}

/// Files the installer would own; planning does not create any of them.
#[derive(Clone, Debug, Serialize)]
pub struct PlanFiles {
    /// Final payload locations in manifest order.
    pub payloads: Vec<PathBuf>,
    /// Component ownership records produced by successful execution.
    pub receipts: Vec<PathBuf>,
    /// Installation lock location; its availability has not been checked.
    pub lock: PathBuf,
    /// Whether the prefix was absent when the snapshot was taken.
    pub creates_prefix: bool,
}

/// Identity used by the current raw-prefix executor, without automatic privilege changes.
#[derive(Clone, Debug, Serialize)]
pub struct PlanPrivileges {
    /// Effective user identity of this planning process.
    pub effective_uid: u32,
    /// The raw-prefix installer never obtains privileges automatically.
    pub automatic_elevation: bool,
}

/// Activation side effects of the current installer, all intentionally absent.
#[derive(Clone, Debug, Serialize)]
pub struct PlanActivation {
    /// Installing files does not start services.
    pub starts_services: bool,
    /// Installing files does not create Provider configuration instances.
    pub registers_providers: bool,
    /// Installing files does not modify native Agent integration.
    pub connects_agents: bool,
}

/// Read-only snapshot, not an executable transaction or authorization token.
#[derive(Clone, Debug, Serialize)]
pub struct InstallPlan {
    /// Requested absolute installation target.
    pub prefix: PathBuf,
    /// Receipt presence observed in the target prefix.
    pub existing: InventoryState,
    /// Verified local bundle and the publisher-verification boundary.
    pub source: PlanSource,
    /// Incoming component identities and installed versions.
    pub components: Vec<PlanComponent>,
    /// Intended file locations, without reservation or creation.
    pub files: PlanFiles,
    /// Identity and privilege escalation policy.
    pub privileges: PlanPrivileges,
    /// Explicitly absent activation actions.
    pub activation: PlanActivation,
    /// Known target/preflight conflicts; an empty list does not guarantee execution.
    pub conflicts: Vec<String>,
    /// Checks deferred to execution or outside the installer contract.
    pub not_checked: Vec<&'static str>,
}

/// Inspect a local Preview bundle and report known installation changes and conflicts.
///
/// Reads a bounded manifest, verifies payload checksums with the same implementation
/// as installation, and reuses the installer's pure preflight. No prefix, lock,
/// stage, receipt, service or Provider instance is created. The returned plan is
/// immediately staleable: execution must reacquire ownership and recheck all inputs.
///
/// # Errors
/// Rejects unsafe or malformed input metadata and invalid bundle payloads. Valid
/// inputs with target ownership, dependency or overwrite conflicts return a plan
/// whose `conflicts` is nonempty, not permission to proceed.
pub fn plan_install(bundle: &Path, prefix: &Path) -> Result<InstallPlan> {
    let directory = read_only::directory(bundle)?
        .ok_or_else(|| crate::Error::Invalid("local bundle directory is absent".into()))?;
    let bytes = read_only::document(&directory, "manifest.json".as_ref(), 4 * 1024 * 1024)?;
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    require(
        manifest.components.len() <= 64,
        "too many components in package manifest",
    )?;
    require(
        manifest
            .components
            .iter()
            .all(|component| component.files.len() <= 8192),
        "too many payload files in package manifest",
    )?;
    let manifest = crate::manifest::inspect_manifest(bundle, manifest)?;
    let snapshot = inventory(prefix)?;
    let installed: BTreeMap<_, _> = snapshot
        .components
        .into_iter()
        .map(|component| (component.component.clone(), component))
        .collect();
    let mut conflicts = Vec::new();
    let mut note = |result: Result<()>| {
        if let Err(error) = result {
            let message = error.to_string();
            if !conflicts.contains(&message) {
                conflicts.push(message);
            }
        }
    };
    note(paths::parent(prefix));
    if snapshot.state != InventoryState::Absent {
        note(paths::owned_directory(prefix));
        note(require(
            !installed.is_empty(),
            "refusing a prefix without AW package receipts",
        ));
    }
    note(crate::install::preflight(prefix, &manifest, &installed));
    let components = manifest
        .components
        .iter()
        .map(|component| PlanComponent {
            name: component.component.clone(),
            version: component.version.clone(),
            installed_version: installed
                .get(&component.component)
                .map(|record| record.version.clone()),
        })
        .collect();
    let files = PlanFiles {
        payloads: manifest
            .components
            .iter()
            .flat_map(|component| component.files.keys())
            .map(|name| prefix.join(name))
            .collect(),
        receipts: manifest
            .components
            .iter()
            .map(|component| {
                prefix
                    .join(".aw-packages")
                    .join(format!("{}.json", component.component))
            })
            .collect(),
        lock: prefix.join(".aw-install.lock"),
        creates_prefix: snapshot.state == InventoryState::Absent,
    };
    Ok(InstallPlan {
        prefix: prefix.into(),
        existing: snapshot.state,
        source: PlanSource {
            kind: "local_bundle",
            path: bundle.into(),
            payload_verified: true,
            publisher_verified: false,
        },
        components,
        files,
        privileges: PlanPrivileges {
            // SAFETY: geteuid only reads the current process identity.
            effective_uid: unsafe { libc::geteuid() },
            automatic_elevation: false,
        },
        activation: PlanActivation {
            starts_services: false,
            registers_providers: false,
            connects_agents: false,
        },
        conflicts,
        not_checked: vec![
            "installation lock ownership and availability",
            "disk capacity and effective write access",
            "changes to source or target after this snapshot; execution rechecks under its lock",
            "Provider, native Agent and service runtime readiness",
        ],
    })
}
