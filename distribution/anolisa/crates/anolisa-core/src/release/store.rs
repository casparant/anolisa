//! Product file operations call the shared library without launching the AW runtime.

use super::{Bundle, Environment, Error, ReleaseLayout, Result, Scope, require};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub(super) struct Output {
    pub success: bool,
    pub report: Value,
}

pub(super) enum Operation<'a> {
    Inventory,
    Doctor,
    Install(&'a Path),
}

pub(super) trait Backend {
    fn invoke(&self, operation: Operation<'_>) -> Result<Output>;
}

pub(super) struct Store<'a> {
    pub bundle: &'a Bundle,
    pub layout: &'a ReleaseLayout,
}

#[cfg(target_os = "linux")]
impl Store<'_> {
    fn paths(&self) -> Result<local_package::database::DatabasePaths> {
        use local_package::database::{PackageManager, RegistrationPointer};
        let default_paths = paths(self.layout, PackageManager::Anolisa)?;
        let Some(pointer) = RegistrationPointer::read(&self.layout.prefix)? else {
            return Ok(default_paths);
        };
        let recorded = pointer.database_paths()?;
        if recorded.manager() == PackageManager::Anolisa {
            require(
                recorded.root == default_paths.root && recorded.scope() == default_paths.scope(),
                "product prefix belongs to another ANOLISA database or scope",
            )?;
            return Ok(default_paths);
        }
        // Existing product records retain their original authority. Resuming one
        // release must never register its files in a second package database.
        let legacy = paths(self.layout, PackageManager::Aw)?;
        require(
            recorded.root == legacy.root
                && recorded.scope() == legacy.scope()
                && super::install::status(self.bundle, self.layout)?.is_some(),
            "AW-owned files require their matching existing product record; automatic adoption is refused",
        )?;
        Ok(recorded)
    }
}

#[cfg(target_os = "linux")]
fn paths(
    layout: &ReleaseLayout,
    manager: local_package::database::PackageManager,
) -> Result<local_package::database::DatabasePaths> {
    use local_package::database::DatabasePaths;
    Ok(match layout.scope {
        Scope::User => DatabasePaths::user_for(manager, Some(&layout.xdg_state), None)?,
        Scope::System => DatabasePaths::system_for(manager),
    })
}

#[cfg(target_os = "linux")]
impl Backend for Store<'_> {
    fn invoke(&self, operation: Operation<'_>) -> Result<Output> {
        use local_package::{database::Inventory, plan};
        use serde_json::json;
        use std::sync::atomic::AtomicBool;
        require(
            self.layout.scope != Scope::System || nix::unistd::geteuid().is_root(),
            "system product package records require administrator access",
        )?;
        self.bundle.command_evidence()?;
        let paths = self.paths()?;
        match operation {
            Operation::Inventory => {
                let inventory = Inventory::read(&paths, None)?;
                Ok(Output {
                    success: true,
                    report: json!({"format":1,"status":"succeeded","result":{"inventory":inventory}}),
                })
            }
            Operation::Doctor => {
                let inventory = Inventory::read(&paths, Some(&self.layout.prefix))?;
                let report = inventory.doctor()?;
                Ok(Output {
                    success: report.healthy(),
                    report: json!({"format":1,"status":if report.healthy() {"succeeded"} else {"failed"},"result":{"report":report}}),
                })
            }
            Operation::Install(directory) => {
                let plan = plan::install_with_resources(
                    &paths,
                    directory,
                    &self.layout.prefix,
                    Some(&self.layout.resources),
                )?;
                require(
                    !plan.register_legacy,
                    "unregistered historical AW installations require explicit migration",
                )?;
                if !plan.ready() {
                    return Ok(Output {
                        success: false,
                        report: json!({"format":1,"status":"failed","result":{"plan":plan}}),
                    });
                }
                let (success, report, error) =
                    match plan::execute(&paths, &plan, &AtomicBool::new(false)) {
                        Ok(report) => (true, report, None),
                        Err(failure) => (false, failure.report, Some(failure.error.to_string())),
                    };
                let recovery = match report.recovery {
                    local_package::RecoveryState::NotAttempted => "not_attempted",
                    local_package::RecoveryState::Complete => "complete",
                    local_package::RecoveryState::Incomplete => "incomplete",
                };
                let warnings: Vec<_> = report.notices.iter().map(|notice| {
                    json!({"code":format!("{:?}",notice.code),"path":notice.path,"io_kind":notice.io_kind.map(|kind|format!("{kind:?}"))})
                }).collect();
                Ok(Output {
                    success,
                    report: json!({"format":1,"status":if success {"succeeded"} else {"failed"},"mutation":{"committed":report.committed,"recovery":recovery},"error":error,"warnings":warnings}),
                })
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
impl Backend for Store<'_> {
    fn invoke(&self, operation: Operation<'_>) -> Result<Output> {
        let _ = (self.bundle, self.layout);
        if let Operation::Install(directory) = operation {
            let _ = directory;
        }
        Err(Error::Invalid(
            "local package installation requires Linux".into(),
        ))
    }
}

pub(super) struct Inventory {
    pub installation_id: Option<String>,
    pub packages: Vec<Value>,
}

pub(super) fn inventory(backend: &impl Backend, layout: &ReleaseLayout) -> Result<Inventory> {
    let output = backend.invoke(Operation::Inventory)?;
    require(
        output.success,
        format!("package inventory blocked installation: {}", output.report),
    )?;
    let installations = output
        .report
        .pointer("/result/inventory/installations")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Invalid("package inventory lacks installation records".into()))?;
    let matching = installations
        .iter()
        .filter(|installation| installation["prefix"] == layout.prefix.display().to_string())
        .collect::<Vec<_>>();
    require(
        matching.len() <= 1,
        "package inventory contains duplicate selected prefixes",
    )?;
    match matching.first() {
        None => Ok(Inventory {
            installation_id: None,
            packages: Vec::new(),
        }),
        Some(installation) => {
            require(
                installation["resources"].as_str() == layout.resources.to_str(),
                "selected installation has a different real resources directory",
            )?;
            require(
                installation["manager"] == "raw"
                    && installation["registration"] == "registered"
                    && installation["scope"] == layout.scope.as_str(),
                "selected prefix is not a registered raw installation in the requested scope",
            )?;
            let id = installation["installation_id"]
                .as_str()
                .ok_or_else(|| Error::Invalid("installation lacks its persistent ID".into()))?;
            super::model::identifier(id)?;
            let packages = installation["packages"]
                .as_array()
                .cloned()
                .ok_or_else(|| Error::Invalid("inventory lacks package records".into()))?;
            Ok(Inventory {
                installation_id: Some(id.to_owned()),
                packages,
            })
        }
    }
}

pub(super) fn healthy(backend: &impl Backend, _layout: &ReleaseLayout) -> Result<Value> {
    let output = backend.invoke(Operation::Doctor)?;
    require(
        output.success,
        format!("installed-file verification failed: {}", output.report),
    )?;
    let issues = output
        .report
        .pointer("/result/report/issues")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Invalid("file verification report lacks issues array".into()))?;
    require(
        issues.is_empty(),
        "package verification reported damaged files",
    )?;
    Ok(output.report)
}

/// Observed replay results for the selected ANOLISA raw package database.
#[derive(Debug, Serialize)]
pub struct PackageRecovery {
    /// Database whose interrupted file operations were examined.
    pub database_path: PathBuf,
    /// Committed transactions whose cleanup completed.
    pub committed: usize,
    /// Uncommitted transactions whose changes were restored.
    pub rolled_back: usize,
    /// Conflicting paths whose recovery evidence remains on disk.
    pub conflicts: Vec<PathBuf>,
}

/// Recover interrupted ANOLISA package operations without installing or activating a release.
///
/// # Errors
/// Refuses unsupported hosts, non-root system writes, unsafe records and active file leases.
pub fn recover_packages(scope: Scope, environment: &Environment) -> Result<PackageRecovery> {
    #[cfg(target_os = "linux")]
    {
        use local_package::database::{DatabasePaths, PackageManager};
        require(
            scope != Scope::System || nix::unistd::geteuid().is_root(),
            "system package recovery requires root",
        )?;
        let paths = match scope {
            Scope::User => {
                DatabasePaths::user_for(PackageManager::Anolisa, Some(&environment.state), None)?
            }
            Scope::System => DatabasePaths::system_for(PackageManager::Anolisa),
        };
        let report = local_package::transaction::recover(
            &paths,
            &std::sync::atomic::AtomicBool::new(false),
        )?;
        Ok(PackageRecovery {
            database_path: paths.installed(),
            committed: report.committed,
            rolled_back: report.rolled_back,
            conflicts: report.conflicts,
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (scope, environment);
        Err(Error::Invalid(
            "local package recovery requires Linux".into(),
        ))
    }
}

/// Read product package records without creating directories or migrating older records.
///
/// # Errors
/// Refuses unsupported hosts, unsafe paths and interrupted package transactions.
pub fn list_packages(scope: Scope, environment: &Environment) -> Result<Value> {
    #[cfg(target_os = "linux")]
    {
        use local_package::database::{DatabasePaths, Inventory, PackageManager};
        require(
            scope != Scope::System || nix::unistd::geteuid().is_root(),
            "system product package records require administrator access",
        )?;
        let paths = match scope {
            Scope::User => {
                DatabasePaths::user_for(PackageManager::Anolisa, Some(&environment.state), None)?
            }
            Scope::System => DatabasePaths::system_for(PackageManager::Anolisa),
        };
        Ok(serde_json::to_value(Inventory::read(&paths, None)?)?)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (scope, environment);
        Err(Error::Invalid(
            "local package inventory requires Linux".into(),
        ))
    }
}

/// Verify product files independently of the saved installation progress.
///
/// # Errors
/// Refuses unsupported hosts, conflicting records, damaged files and pending recovery.
pub fn verify_packages(bundle: &Bundle, layout: &ReleaseLayout) -> Result<Value> {
    healthy(&Store { bundle, layout }, layout)
}
