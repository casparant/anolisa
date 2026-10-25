//! Bind private programs to their separately installed resources without changing package bytes.

use super::{ReleaseLayout, Result, files, require};
use std::{fs, os::unix::fs::MetadataExt};

pub(super) fn preflight(layout: &ReleaseLayout) -> Result<()> {
    files::absolute(&layout.prefix)?;
    files::absolute(&layout.resources)?;
    let alias = layout.resource_alias();
    match fs::symlink_metadata(&alias) {
        Ok(metadata) => {
            require(
                metadata.file_type().is_symlink()
                    && metadata.uid() == nix::unistd::geteuid().as_raw()
                    && fs::read_link(&alias).map_err(|error| files::io(&alias, error))?
                        == layout.resources,
                format!(
                    "program resource alias has unknown ownership or target; preserve {}",
                    alias.display()
                ),
            )?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(files::io(&alias, error)),
    }
    Ok(())
}

pub(super) fn publish(layout: &ReleaseLayout) -> Result<()> {
    preflight(layout)?;
    files::owned(&layout.prefix)?;
    files::tree(&layout.resources, 0o755)?;
    let alias = layout.resource_alias();
    if fs::symlink_metadata(&alias).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        std::os::unix::fs::symlink(&layout.resources, &alias)
            .map_err(|error| files::io(&alias, error))?;
    }
    preflight(layout)?;
    files::sync(&layout.prefix)
}
