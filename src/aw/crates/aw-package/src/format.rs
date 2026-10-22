//! Select versioned package contracts without mutating or trusting their contents.

use crate::{read_only, require, Error, Result};
use std::{fs, path::Path};

/// Read the bounded local manifest's version before calling its typed validator.
///
/// The version alone does not validate the bundle or authorize installation.
pub fn bundle_format(bundle: &Path) -> Result<u8> {
    let directory = read_only::directory(bundle)?
        .ok_or_else(|| Error::Invalid("local bundle directory is absent".into()))?;
    let bytes = read_only::document(&directory, "manifest.json".as_ref(), 4 * 1024 * 1024)?;
    let document: serde_json::Value = serde_json::from_slice(&bytes)?;
    match document.get("format").and_then(serde_json::Value::as_u64) {
        Some(1) => Ok(1),
        Some(2) => Ok(2),
        _ => Err(Error::Invalid("unsupported package manifest format".into())),
    }
}

/// Select a receipt reader without following markers or creating a prefix.
///
/// Absent and unmarked prefixes use the legacy inventory reader, which reports
/// absence or unmanaged state. A v2 marker must still pass typed validation.
/// Mixed ownership markers are refused rather than silently choosing a backend.
pub fn installed_format(prefix: &Path) -> Result<u8> {
    let Some(directory) = read_only::directory(prefix)? else {
        return Ok(1);
    };
    let root = read_only::descriptor_path(&directory);
    let exists = |name: &str| -> Result<bool> {
        match fs::symlink_metadata(root.join(name)) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    };
    let open = exists(".aw-installation.json")?;
    require(
        !(open && exists(".aw-packages")?),
        "mixed Preview and open-bundle installation markers",
    )?;
    Ok(if open { 2 } else { 1 })
}
