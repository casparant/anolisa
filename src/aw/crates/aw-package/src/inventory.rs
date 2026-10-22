//! Read-only snapshots of installed Preview receipts, separate from runtime health.

use crate::{filesystem as paths, read_only, require, Component, Result};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

const MAX_DIRECTORY_ENTRIES: usize = 64;
const MAX_RECEIPT_BYTES: usize = 4 * 1024 * 1024;
const MAX_RECEIPT_FILES: usize = 8192;

/// Whether a prefix has usable AW receipts; this is not payload or runtime health.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryState {
    /// The prefix does not exist, including when an ancestor is absent.
    Absent,
    /// The prefix exists but has no AW component receipts.
    Unmanaged,
    /// One or more structurally valid component receipts were read.
    Installed,
}

/// An unlocked, bounded snapshot of installed component metadata.
///
/// Reading root-owned metadata does not authorize changing it. No payload hash,
/// running service, native Agent integration or Provider health is checked.
#[derive(Clone, Debug, Serialize)]
pub struct Inventory {
    /// Absolute requested installation prefix.
    pub prefix: PathBuf,
    /// Filesystem/receipt presence, not runtime readiness.
    pub state: InventoryState,
    /// Recorded components in stable component-name order.
    pub components: Vec<Component>,
}

/// Read installed receipts without creating a prefix, lock or temporary file.
///
/// Limits directory entries, receipt bytes and recorded file counts. Metadata
/// must be regular, symlink-free and owned by the current user or root without
/// group/world write access. This unlocked snapshot can become stale immediately;
/// the installer must perform its own locked verification before mutation.
///
/// # Errors
/// Rejects unsafe paths, oversized or malformed receipts and unreadable metadata.
pub fn inventory(prefix: &Path) -> Result<Inventory> {
    let mut inventory = Inventory {
        prefix: prefix.into(),
        state: InventoryState::Absent,
        components: vec![],
    };
    let Some(directory) = read_only::directory(prefix)? else {
        return Ok(inventory);
    };
    inventory.state = InventoryState::Unmanaged;
    let Some(receipts) = read_only::child_directory(&directory, ".aw-packages")? else {
        return Ok(inventory);
    };
    let mut components = BTreeMap::new();
    for (index, entry) in fs::read_dir(read_only::descriptor_path(&receipts))?.enumerate() {
        require(
            index < MAX_DIRECTORY_ENTRIES,
            "too many package receipt directory entries",
        )?;
        let name = entry?.file_name();
        let path = Path::new(&name);
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let bytes = read_only::document(&receipts, &name, MAX_RECEIPT_BYTES)?;
        let component: Component = serde_json::from_slice(&bytes)?;
        require(
            component.format == 1
                && matches!(
                    component.component.as_str(),
                    "aw-core" | "aw-provider-sec-core" | "aw-provider-tokenless"
                )
                && path
                    .file_stem()
                    .is_some_and(|stem| stem == component.component.as_str())
                && !components.contains_key(&component.component),
            "invalid package receipt identity",
        )?;
        require(
            component.files.len() <= MAX_RECEIPT_FILES,
            "too many files in package receipt",
        )?;
        for (name, metadata) in &component.files {
            paths::relative(name)?;
            require(
                !name.contains('\0')
                    && matches!(metadata.mode, 0o644 | 0o755)
                    && metadata.sha256.len() == 64
                    && metadata
                        .sha256
                        .bytes()
                        .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value)),
                "invalid package receipt payload metadata",
            )?;
        }
        components.insert(component.component.clone(), component);
    }
    inventory.components = components.into_values().collect();
    if !inventory.components.is_empty() {
        inventory.state = InventoryState::Installed;
    }
    Ok(inventory)
}
