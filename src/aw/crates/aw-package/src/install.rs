//! Historical AW installer compatibility and framework configuration fixtures.

#[cfg(test)]
use crate::filesystem as paths;
pub(crate) use local_package::compatibility::selected_receipts;
#[cfg(test)]
use local_package::{install, uninstall};
#[cfg(test)]
use std::{
    collections::BTreeMap,
    fs::{self, Permissions},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

#[cfg(test)]
fn receipts(prefix: &Path) -> crate::Result<BTreeMap<String, crate::Component>> {
    selected_receipts(prefix, None)
}

#[cfg(test)]
mod tests;
