//! AW admission policy over shared read-only file installation plans.

use crate::{
    database::{DatabasePaths, Inventory},
    Result,
};
use std::path::Path;

pub use local_package::plan::{
    execute, Action, DependencySummary, FileChange, PackagePlan, PackageSummary, Problem,
};

/// Plan an AW extension installation while preserving historical runtime admission.
///
/// # Errors
/// Rejects malformed bundles, unsafe paths and inconsistent package records.
pub fn install(paths: &DatabasePaths, bundle: &Path, prefix: &Path) -> Result<PackagePlan> {
    install_with_resources(paths, bundle, prefix, None)
}

/// Plan an installation with an explicit external resource directory.
///
/// # Errors
/// Rejects invalid layouts, malformed bundles and unsafe or inconsistent package records.
pub fn install_with_resources(
    paths: &DatabasePaths,
    bundle: &Path,
    prefix: &Path,
    resources: Option<&Path>,
) -> Result<PackagePlan> {
    admit(
        paths,
        prefix,
        local_package::plan::install_with_resources(paths, bundle, prefix, resources)?,
    )
}

/// Plan selective AW extension removal and check the executable that owns historical records.
///
/// # Errors
/// Rejects unsafe paths, inconsistent package records and duplicate package IDs.
pub fn uninstall(paths: &DatabasePaths, prefix: &Path, ids: &[String]) -> Result<PackagePlan> {
    admit(
        paths,
        prefix,
        local_package::plan::uninstall(paths, prefix, ids)?,
    )
}

fn admit(paths: &DatabasePaths, prefix: &Path, mut plan: PackagePlan) -> Result<PackagePlan> {
    if plan.installation_id.is_none() && !plan.register_legacy {
        return Ok(plan);
    }
    let inventory = Inventory::read(paths, Some(prefix))?;
    let Some(installation) = inventory.installations.first() else {
        return Ok(plan);
    };
    let Some(core) = installation
        .packages
        .iter()
        .find(|package| package.package_id == "aw-core")
    else {
        return Ok(plan);
    };
    let known = core.files.get("bin/aw").is_some_and(|metadata| {
        std::env::current_exe()
            .ok()
            .and_then(|path| crate::digest(&path).ok())
            .is_some_and(|digest| digest == metadata.sha256)
    });
    if !known {
        let message = match plan.action {
            Action::Install => "this prefix contains an older AW executable whose running references cannot be checked; registration would disable its historical installer without a supported removal path; preserve this prefix and install the new build into a separate empty prefix",
            Action::Uninstall => "this prefix contains an older AW executable whose running references cannot be checked; preserve it and install the new build into a separate empty prefix until runtime migration is explicitly supported",
        };
        plan.problems.push(Problem {
            code: "runtime_unknown".into(),
            message: message.into(),
            package_id: Some("aw-core".into()),
            path: Some(prefix.to_owned()),
        });
    }
    Ok(plan)
}
