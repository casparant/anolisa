//! Journal each external resource directory and stage on its own filesystem.

use super::{
    fs,
    journal::{Journal, ResourceRoot},
    ChangeSet,
};
use crate::{database, require, Error, Result};

pub(super) fn prepare(changes: &ChangeSet) -> Result<Option<ResourceRoot>> {
    let Some(path) = &changes.resources else {
        return Ok(None);
    };
    let created = match fs::directory(path) {
        Ok(_) => {
            fs::owned(path)?;
            false
        }
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::owned(
                path.parent()
                    .ok_or_else(|| Error::Invalid("resource root has no parent".into()))?,
            )?;
            true
        }
        Err(error) => return Err(error),
    };
    require(
        changes.marker_before.is_some() || created,
        "unowned resource directory appeared after planning",
    )?;
    Ok(Some(ResourceRoot {
        path: path.clone(),
        created,
        identity: if created {
            None
        } else {
            Some(fs::directory_identity(path)?)
        },
        workspace_identity: None,
    }))
}

pub(super) fn begin(entry: &mut Journal) -> Result<()> {
    let Some(root) = &entry.resources else {
        return Ok(());
    };
    let path = root.path.clone();
    if root.created {
        fs::mkdir(&path, 0o755)?;
        let identity = fs::directory_identity(&path)?;
        if let Some(root) = &mut entry.resources {
            root.identity = Some(identity);
        }
        entry.write()?;
    }
    let workspace = entry.workspace_for(database::resources::LOGICAL_LOCATOR);
    fs::mkdir(&workspace, 0o700)?;
    let identity = fs::directory_identity(&workspace)?;
    if let Some(root) = &mut entry.resources {
        root.workspace_identity = Some(identity);
    }
    entry.write()
}

pub(super) fn recheck(changes: &ChangeSet) -> Result<()> {
    let Some(root) = &changes.resources else {
        return Ok(());
    };
    let locator = root.join(database::resources::LOCATOR);
    let reverse = changes.prefix.join(database::resources::REVERSE_LOCATOR);
    let registered = changes
        .database_before
        .as_ref()
        .map(|bytes| -> Result<bool> {
            let before: database::InstalledDatabase = database::decode(bytes)?;
            Ok(before.installations.iter().any(|installation| {
                installation.prefix == changes.prefix
                    && installation.resources.as_ref() == Some(root)
            }))
        })
        .transpose()?
        .unwrap_or(false);
    if registered {
        require(
            fs::read(&locator, 4096)? == changes.prefix.as_os_str().as_encoded_bytes(),
            "resource locator changed after planning",
        )?;
        require(
            fs::read(&reverse, 4096)? == root.as_os_str().as_encoded_bytes(),
            "reverse resource locator changed after planning",
        )?;
    } else {
        require(
            !fs::exists(&locator)? && !fs::exists(&reverse)?,
            "unowned resource locator appeared after planning",
        )?;
    }
    Ok(())
}

pub(super) fn metadata(entry: &Journal, name: &str) -> Option<Vec<u8>> {
    let root = entry.resources.as_ref()?;
    match name {
        database::resources::LOGICAL_LOCATOR => {
            Some(entry.prefix.as_os_str().as_encoded_bytes().to_vec())
        }
        database::resources::REVERSE_LOCATOR => {
            Some(root.path.as_os_str().as_encoded_bytes().to_vec())
        }
        _ => None,
    }
}
