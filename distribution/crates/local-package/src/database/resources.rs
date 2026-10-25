//! External resource roots retain a direct locator to their installation's runtime lock.

use super::{paths, Installation};
use crate::{require, Result};
use std::path::Path;

pub(crate) const LOCATOR: &str = ".aw-installation-prefix";
pub(crate) const LOGICAL_LOCATOR: &str = "share/.aw-installation-prefix";
pub(crate) const REVERSE_LOCATOR: &str = ".aw-resources-dir";

pub(crate) fn validate(prefix: &Path, resources: &Path) -> Result<()> {
    paths::absolute(resources)?;
    require(
        prefix.as_os_str().as_encoded_bytes().len() <= 4096
            && resources.as_os_str().as_encoded_bytes().len() <= 4096,
        "resource locator paths exceed 4096 bytes",
    )?;
    require(
        !prefix.starts_with(resources) && !resources.starts_with(prefix),
        "resource root must be separate from the program prefix",
    )?;
    require(
        resources
            .parent()
            .is_some_and(|parent| parent != Path::new("/")),
        "resource root must be an application-specific directory",
    )
}

pub(crate) fn check(installation: &Installation) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let Some(root) = &installation.resources else {
        return Ok(());
    };
    let metadata = paths::directory(root)?.metadata()?;
    require(
        metadata.uid() == installation.owner_uid && metadata.mode() & 0o022 == 0,
        "resource directory ownership or permissions changed",
    )?;
    let marker = root.join(LOCATOR);
    for path in [&marker, &installation.prefix.join(REVERSE_LOCATOR)] {
        let metadata = paths::regular(path)?.metadata()?;
        require(
            metadata.uid() == installation.owner_uid
                && metadata.mode() & 0o7777 == 0o644
                && metadata.nlink() == 1,
            "resource locator ownership, mode or link count changed",
        )?;
    }
    let bytes = paths::bytes(&marker, 4096)?;
    require(
        bytes == installation.prefix.as_os_str().as_encoded_bytes(),
        format!(
            "resource root locator differs from its installation: {}",
            marker.display()
        ),
    )?;
    require(
        paths::bytes(&installation.prefix.join(REVERSE_LOCATOR), 4096)?
            == root.as_os_str().as_encoded_bytes(),
        "installation's reverse resource locator changed",
    )
}
