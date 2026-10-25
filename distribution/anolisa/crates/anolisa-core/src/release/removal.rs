//! Remove only inactive product-owned package files while retaining configuration and evidence.

#[cfg(any(target_os = "linux", test))]
use super::files;
use super::{Bundle, Error, InstallReport, ReleaseLayout, Result, Scope, require};
use serde::Serialize;
use serde_json::Value;
#[cfg(any(target_os = "linux", test))]
use std::fs;
use std::path::PathBuf;

/// Read-only evidence for retiring one inactive immutable product release.
#[derive(Clone, Debug, Serialize)]
pub struct RemovalPlan {
    /// Fixed output version.
    pub schema_version: u32,
    /// Immutable release selected by the matching product record.
    pub release_id: String,
    /// Exact product description digest.
    pub manifest_sha256: String,
    /// Registered program directory; registration metadata remains after removal.
    pub prefix: PathBuf,
    /// Registered static resource root; unknown files remain untouched.
    pub resources: PathBuf,
    /// Product progress record retained for retry and diagnosis.
    pub record_path: PathBuf,
    /// Explicit privilege boundary.
    pub scope: Scope,
    /// ANOLISA database owning the selected raw files.
    pub database_path: PathBuf,
    /// Persistent installation whose file ownership was verified.
    pub installation_id: String,
    /// Packages still installed in this immutable release.
    pub packages: Vec<String>,
    /// Shared library plan; execution always derives fresh evidence under the product lock.
    pub package_plan: Value,
}

/// Observed inactive-release removal, including committed failures that need package recovery.
#[derive(Debug, Serialize)]
pub struct RemovalReport {
    /// Reviewed product and package ownership evidence.
    pub plan: RemovalPlan,
    /// `planned`, `removed`, or `incomplete`.
    pub phase: String,
    /// Whether the package engine actually reported a committed file removal.
    pub committed: bool,
    /// Actual mutation facts; absent for a dry run or an already removed release.
    pub package_operation: Option<Value>,
    /// Primary refusal after product progress was durably recorded.
    pub error: Option<String>,
    /// Recovery evidence or cleanup paths requiring explicit attention.
    pub conflicts: Vec<PathBuf>,
    /// Metadata and mutable data deliberately retained after removing package payloads.
    pub warnings: Vec<String>,
}

fn preflight(bundle: &Bundle, layout: &ReleaseLayout) -> Result<InstallReport> {
    bundle.recheck()?;
    let report = super::status(bundle, layout)?.ok_or_else(|| {
        Error::Invalid("release removal requires its matching product record".into())
    })?;
    require(
        !report.dry_run,
        "a dry-run product record grants no package removal authority",
    )?;
    require(
        report.packages.len() == bundle.packages.len()
            && report
                .packages
                .iter()
                .zip(&bundle.packages)
                .all(|(actual, expected)| {
                    actual.package_id == expected.package_id && actual.version == expected.version
                }),
        "product package record differs from the selected immutable release",
    )?;
    super::activate::removal_preflight(bundle, layout)?;
    super::service_units::removal_preflight(layout, &bundle.manifest.release_id)?;
    super::resources::preflight(layout)?;
    Ok(report)
}

/// Inspect an inactive release and its exact owned files without creating records or locks.
///
/// # Errors
/// Refuses active releases, legacy AW ownership, published services, unsafe controls and damaged files.
pub fn plan_removal(bundle: &Bundle, layout: &ReleaseLayout) -> Result<RemovalPlan> {
    let report = preflight(bundle, layout)?;
    #[cfg(target_os = "linux")]
    {
        Ok(prepare(bundle, layout, &report)?.0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = report;
        Err(Error::Invalid(
            "local package removal requires Linux".into(),
        ))
    }
}

/// Remove one inactive product-owned release, preserving current, launchers and mutable data.
///
/// # Errors
/// Refuses active or externally managed releases, stale controls, unsafe ownership and concurrent writes.
/// Failures after the removing phase is saved are retained in the returned report for explicit recovery.
pub fn remove_release(
    bundle: &Bundle,
    layout: &ReleaseLayout,
    dry_run: bool,
) -> Result<RemovalReport> {
    let plan = plan_removal(bundle, layout)?;
    let result = RemovalReport {
        plan,
        phase: "planned".into(),
        committed: false,
        package_operation: None,
        error: None,
        conflicts: Vec::new(),
        warnings: Vec::new(),
    };
    if dry_run {
        return Ok(result);
    }
    #[cfg(target_os = "linux")]
    {
        let mut result = result;
        require(
            bundle.manifest.arch == std::env::consts::ARCH,
            "release removal requires matching native Linux",
        )?;
        require(
            layout.scope != Scope::System || nix::unistd::geteuid().is_root(),
            "system release removal requires root and explicit system scope",
        )?;
        files::owned(&layout.state)?;
        let _lock = super::install::ProductLock::acquire(&layout.state.join("release.lock"))?;
        let mut record = preflight(bundle, layout)?;
        let (fresh, package_plan) = prepare(bundle, layout, &record)?;
        result.plan = fresh;
        record.phase = "removing".into();
        record.error = None;
        record.activated = false;
        files::json(&record.record_path, &record)?;
        let operation = apply(layout, &package_plan, &mut result);
        if let Err(error) = operation {
            result.phase = "incomplete".into();
            result.error = Some(error.to_string());
            record.error = result.error.clone();
            record.phase = "removing".into();
        } else {
            record.phase = "removed".into();
            for package in &mut record.packages {
                package.committed = false;
                package.verified = false;
                package.observation = "not_committed".into();
            }
            result.phase = "removed".into();
            result.warnings.push("Registration metadata, unknown files, configuration and mutable data remain; current and public launchers are unchanged.".into());
        }
        if let Err(error) = files::json(&record.record_path, &record) {
            result.phase = "incomplete".into();
            result.error.get_or_insert_with(|| {
                format!("product removal result requires recovery: {error}")
            });
            result.conflicts.push(record.record_path);
        }
        Ok(result)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = result;
        Err(Error::Invalid(
            "local package removal requires Linux".into(),
        ))
    }
}

#[cfg(target_os = "linux")]
fn paths(layout: &ReleaseLayout) -> Result<local_package::database::DatabasePaths> {
    use local_package::database::{DatabasePaths, PackageManager, RegistrationPointer};
    let expected = match layout.scope {
        Scope::User => {
            DatabasePaths::user_for(PackageManager::Anolisa, Some(&layout.xdg_state), None)?
        }
        Scope::System => DatabasePaths::system_for(PackageManager::Anolisa),
    };
    let pointer = RegistrationPointer::read(&layout.prefix)?
        .ok_or_else(|| Error::Invalid("release has no registered package ownership".into()))?;
    let recorded = pointer.database_paths()?;
    require(
        recorded.manager() == PackageManager::Anolisa
            && recorded.root == expected.root
            && recorded.scope() == expected.scope(),
        "legacy AW-owned releases retain their original removal authority; use their AW installer instead of automatic ANOLISA adoption",
    )?;
    Ok(expected)
}

#[cfg(target_os = "linux")]
fn prepare(
    bundle: &Bundle,
    layout: &ReleaseLayout,
    report: &InstallReport,
) -> Result<(RemovalPlan, local_package::plan::PackagePlan)> {
    use local_package::{database::Inventory, plan};
    let paths = paths(layout)?;
    let inventory = Inventory::read(&paths, Some(&layout.prefix))?;
    let installation = inventory
        .installations
        .first()
        .ok_or_else(|| Error::Invalid("release is absent from its package database".into()))?;
    require(
        report
            .installation_id
            .as_ref()
            .is_none_or(|id| id == &installation.installation_id)
            && installation.resources.as_ref() == Some(&layout.resources),
        "product record differs from registered package ownership",
    )?;
    for package in &installation.packages {
        let reference = bundle
            .manifest
            .packages
            .iter()
            .find(|item| item.package_id == package.package_id)
            .ok_or_else(|| {
                Error::Invalid(format!(
                    "unexpected package in immutable release: {}",
                    package.package_id
                ))
            })?;
        let expected = bundle
            .packages
            .iter()
            .find(|item| item.package_id == package.package_id)
            .ok_or_else(|| Error::Invalid("package description is missing".into()))?;
        require(
            package.version == reference.version
                && package.os == expected.os
                && package.arch == expected.arch
                && package.source.bundle_sha256.as_ref() == Some(&reference.description_sha256)
                && serde_json::to_value(&package.files)? == serde_json::to_value(&expected.files)?,
            format!(
                "installed {} differs from the immutable release",
                package.package_id
            ),
        )?;
        require(
            package.dependencies.len() == expected.dependencies.len()
                && expected.dependencies.iter().all(|dependency| {
                    package.dependencies.iter().any(|actual| {
                        actual.package_id == dependency.package_id
                            && actual.version == dependency.version
                            && actual.installation_id == installation.installation_id
                    })
                }),
            "installed dependency ownership differs from the immutable release",
        )?;
    }
    let package_plan = plan::uninstall(&paths, &layout.prefix, &[])?;
    require(
        package_plan.ready(),
        format!(
            "package removal is blocked: {}",
            serde_json::to_string(&package_plan.problems)?
        ),
    )?;
    let removal = RemovalPlan {
        schema_version: 1,
        release_id: bundle.manifest.release_id.clone(),
        manifest_sha256: bundle.manifest_sha256.clone(),
        prefix: layout.prefix.clone(),
        resources: layout.resources.clone(),
        record_path: report.record_path.clone(),
        scope: layout.scope,
        database_path: paths.installed(),
        installation_id: installation.installation_id.clone(),
        packages: installation
            .packages
            .iter()
            .map(|package| package.package_id.clone())
            .collect(),
        package_plan: serde_json::to_value(&package_plan)?,
    };
    Ok((removal, package_plan))
}

#[cfg(target_os = "linux")]
fn apply(
    layout: &ReleaseLayout,
    plan: &local_package::plan::PackagePlan,
    report: &mut RemovalReport,
) -> Result<()> {
    if !report.plan.packages.is_empty() {
        let paths = paths(layout)?;
        let mutation =
            local_package::plan::execute(&paths, plan, &std::sync::atomic::AtomicBool::new(false));
        match mutation {
            Ok(facts) => {
                report.committed = facts.committed;
                report.package_operation = Some(mutation_facts(&facts));
            }
            Err(failure) => {
                report.committed = failure.report.committed;
                report.package_operation = Some(mutation_facts(&failure.report));
                report.conflicts.extend(
                    failure
                        .report
                        .notices
                        .iter()
                        .map(|notice| notice.path.clone()),
                );
                return Err(failure.error.into());
            }
        }
    }
    super::resources::preflight(layout)?;
    let alias = layout.resource_alias();
    match fs::symlink_metadata(&alias) {
        Ok(_) => {
            fs::remove_file(&alias).map_err(|error| files::io(&alias, error))?;
            files::sync(&layout.prefix)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(files::io(&alias, error)),
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn mutation_facts(report: &local_package::MutationReport) -> Value {
    let recovery = match report.recovery {
        local_package::RecoveryState::NotAttempted => "not_attempted",
        local_package::RecoveryState::Complete => "complete",
        local_package::RecoveryState::Incomplete => "incomplete",
    };
    let notices: Vec<_> = report
        .notices
        .iter()
        .map(|notice| {
            serde_json::json!({
                "code":format!("{:?}",notice.code),"path":notice.path,
                "io_kind":notice.io_kind.map(|kind|format!("{kind:?}"))
            })
        })
        .collect();
    serde_json::json!({"committed":report.committed,"recovery":recovery,"notices":notices})
}

#[cfg(test)]
mod tests;
