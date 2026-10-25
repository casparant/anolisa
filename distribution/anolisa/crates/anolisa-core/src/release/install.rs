//! Persist product progress before each package commit and activate only verified results.

use super::{
    Bundle, Error, ProviderConfiguration, ReleaseLayout, Result, Scope, activate, files, require,
    store::{self, Backend, Operation},
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

/// Observed state of one package operation, not an inferred release-wide rollback.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageResult {
    /// Exact package identity.
    pub package_id: String,
    /// Exact component version.
    pub version: String,
    /// Whether an actual AW commit was reported or observed in its database.
    pub committed: bool,
    /// `not_attempted`, `running`, `unknown`, `not_committed`, or `committed`.
    /// A false committed flag with unknown observation never proves rollback.
    pub observation: String,
    /// Whether installed bytes were checked before skipping or activating.
    pub verified: bool,
    /// Actual package mutation facts, retaining the historical field when reading old records.
    #[serde(alias = "aw")]
    pub package_operation: Option<Value>,
}

/// Durable product progress; packages remain independently committed in AW.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InstallReport {
    /// Product record version.
    pub schema_version: u32,
    /// Immutable release identity.
    pub release_id: String,
    /// Digest of the complete immutable release description.
    pub manifest_sha256: String,
    /// Persistent ID observed in AW, absent before its first successful package commit.
    #[serde(default)]
    pub installation_id: Option<String>,
    /// Initial session-independent Provider template digest, persisted before package writes.
    pub provider_configuration: ProviderConfiguration,
    /// Final real AW installation prefix.
    pub prefix: std::path::PathBuf,
    /// Final real resources destination recorded in AW's installation mapping.
    pub resources: std::path::PathBuf,
    /// Product-owned `prefix/share` alias, always targeting `resources`.
    pub resource_alias: std::path::PathBuf,
    /// Explicit scope, independent of effective UID.
    pub scope: Scope,
    /// Durable product record used to resume this same release.
    pub record_path: std::path::PathBuf,
    /// Last observed product phase.
    pub phase: String,
    /// This report describes a read-only preflight instead of writes.
    pub dry_run: bool,
    /// Per-package commit and verification facts.
    pub packages: Vec<PackageResult>,
    /// Whether the current pointer actually selected this prefix when observed.
    pub activated: bool,
    /// Primary failure, if the operation stopped.
    pub error: Option<String>,
    /// Recovery or feature boundaries requiring attention.
    pub warnings: Vec<String>,
}

impl InstallReport {
    /// Whether installation and activation completed without an outstanding failure.
    pub fn succeeded(&self) -> bool {
        self.error.is_none() && (self.dry_run || self.phase == "active")
    }
}

/// Read an existing matching product record without creating directories or locks.
///
/// # Errors
/// Rejects malformed records, changed release identities and unsafe state paths.
pub fn status(bundle: &Bundle, layout: &ReleaseLayout) -> Result<Option<InstallReport>> {
    let path = layout.record(&bundle.manifest.release_id);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(files::io(&path, error)),
        Ok(_) => {}
    }
    files::control_file(&path)?;
    let mut report: InstallReport = files::decode(&files::read(&path, 16 * 1024 * 1024)?)?;
    require(
        report.schema_version == 1
            && report.release_id == bundle.manifest.release_id
            && report.manifest_sha256 == bundle.manifest_sha256
            && report.prefix == layout.prefix
            && report.resources == layout.resources
            && report.resource_alias == layout.resource_alias()
            && report.scope == layout.scope
            && report.record_path == path,
        "existing product record conflicts with this immutable release; preserve it for explicit recovery",
    )?;
    report.activated = fs::read_link(layout.current()).is_ok_and(|prefix| prefix == layout.prefix);
    Ok(Some(report))
}

/// Install a verified Linux release, retaining partial results for identical-release resume.
///
/// # Errors
/// Refuses unsupported hosts, unsafe ownership, conflicting entries or invalid records.
/// Once product progress exists, execution failures are retained in the returned report.
pub fn install(bundle: &Bundle, layout: &ReleaseLayout, dry_run: bool) -> Result<InstallReport> {
    require(
        std::env::consts::OS == "linux" && bundle.manifest.arch == std::env::consts::ARCH,
        "product installation requires matching native Linux; inspect the bundle without installing on this host",
    )?;
    if layout.scope == Scope::System && !dry_run {
        require(
            nix::unistd::geteuid().is_root(),
            "system release installation requires root and explicit --scope system",
        )?;
    }
    let backend = store::Store { bundle, layout };
    execute(bundle, layout, dry_run, &backend)
}

pub(super) fn execute(
    bundle: &Bundle,
    layout: &ReleaseLayout,
    dry_run: bool,
    backend: &impl Backend,
) -> Result<InstallReport> {
    bundle.recheck()?;
    require(
        [&layout.prefix, &layout.resources]
            .into_iter()
            .all(|destination| {
                !bundle.root.starts_with(destination) && !destination.starts_with(&bundle.root)
            }),
        "release input, installed programs and resources must be separate trees",
    )?;
    activate::preflight(bundle, layout)?;
    super::resources::preflight(layout)?;
    let previous = status(bundle, layout)?;
    if previous.is_none() {
        require(
            !layout
                .prefix
                .try_exists()
                .map_err(|error| files::io(&layout.prefix, error))?,
            "final release prefix already exists without a matching product record; explicit adoption is required",
        )?;
        require(
            !layout
                .resources
                .try_exists()
                .map_err(|error| files::io(&layout.resources, error))?,
            "final resources directory already exists without a matching product record; explicit adoption is required",
        )?;
    }
    let mut report = match previous {
        Some(report) => report,
        None => InstallReport {
            schema_version: 1,
            release_id: bundle.manifest.release_id.clone(),
            manifest_sha256: bundle.manifest_sha256.clone(),
            installation_id: None,
            provider_configuration: super::configuration::prepare(bundle, layout)?,
            prefix: layout.prefix.clone(),
            resources: layout.resources.clone(),
            resource_alias: layout.resource_alias(),
            scope: layout.scope,
            record_path: layout.record(&bundle.manifest.release_id),
            phase: "planned".into(),
            dry_run,
            packages: bundle
                .packages
                .iter()
                .map(|package| PackageResult {
                    package_id: package.package_id.clone(),
                    version: package.version.clone(),
                    committed: false,
                    observation: "not_attempted".into(),
                    verified: false,
                    package_operation: None,
                })
                .collect(),
            activated: false,
            error: None,
            warnings: Vec::new(),
        },
    };
    require(
        report.packages.len() == bundle.packages.len()
            && report
                .packages
                .iter()
                .zip(&bundle.packages)
                .all(|(result, package)| {
                    result.package_id == package.package_id && result.version == package.version
                }),
        "product record package order differs from release",
    )?;
    super::configuration::preflight(bundle, layout, &report.provider_configuration)?;
    super::public_metadata::preflight(bundle, layout)?;
    report.dry_run = dry_run;
    if dry_run {
        report.phase = "planned".into();
        report.error = None;
        return Ok(report);
    }
    files::tree(&layout.state, 0o700)?;
    let _lock = ProductLock::acquire(&layout.state.join("release.lock"))?;
    // Another installer may have changed entries while this process acquired the product lock.
    activate::preflight(bundle, layout)?;
    if let Some(latest) = status(bundle, layout)? {
        report = latest;
    }
    super::resources::preflight(layout)?;
    super::configuration::preflight(bundle, layout, &report.provider_configuration)?;
    report.error = None;
    report.warnings.clear();
    report.phase = "installing".into();
    report.dry_run = false;
    let record_parent = report
        .record_path
        .parent()
        .ok_or_else(|| Error::Invalid("release record has no parent".into()))?;
    files::tree(record_parent, 0o700)?;
    files::json(&report.record_path, &report)?;
    let result = apply(bundle, layout, backend, &mut report);
    if let Err(error) = result {
        report.activated =
            fs::read_link(layout.current()).is_ok_and(|prefix| prefix == layout.prefix);
        report.error = Some(error.to_string());
        report.phase = "incomplete".into();
    }
    if let Err(error) = files::json(&report.record_path, &report) {
        report.warnings.push(format!(
            "could not durably save latest product result: {error}"
        ));
        if report.error.is_none() {
            report.error =
                Some("product result requires recovery before another installation".into());
        }
    }
    Ok(report)
}

fn apply(
    bundle: &Bundle,
    layout: &ReleaseLayout,
    backend: &impl Backend,
    report: &mut InstallReport,
) -> Result<()> {
    files::tree(
        layout
            .prefix
            .parent()
            .ok_or_else(|| Error::Invalid("release prefix has no parent".into()))?,
        0o755,
    )?;
    super::probe::check(layout, bundle)?;
    files::tree(
        layout
            .resources
            .parent()
            .ok_or_else(|| Error::Invalid("release resources have no parent".into()))?,
        0o755,
    )?;
    let installed = store::inventory(backend, layout)?;
    validate_installed(bundle, &installed)?;
    report.installation_id = installed.installation_id.clone();
    if !installed.packages.is_empty() {
        store::healthy(backend, layout)?;
    }
    for (index, reference) in bundle.manifest.packages.iter().enumerate() {
        if installed
            .packages
            .iter()
            .any(|package| package["package_id"] == reference.package_id)
        {
            report.packages[index].committed = true;
            report.packages[index].observation = "committed".into();
            report.packages[index].verified = true;
            files::json(&report.record_path, report)?;
            continue;
        }
        bundle.command_evidence()?;
        let directory = bundle.root.join(&reference.path);
        report.packages[index].observation = "running".into();
        files::json(&report.record_path, report)?;
        let output = match backend.invoke(Operation::Install(&directory)) {
            Ok(output) => output,
            Err(error) => {
                report.packages[index].observation = "unknown".into();
                report.warnings.push(format!(
                    "{} commit/recovery could not be observed; inspect package recovery before resuming",
                    reference.package_id
                ));
                return Err(error);
            }
        };
        let observed = output
            .report
            .pointer("/mutation/committed")
            .and_then(Value::as_bool);
        report.packages[index].observation = match observed {
            Some(true) => "committed",
            Some(false) => "not_committed",
            None => "unknown",
        }
        .into();
        report.packages[index].committed = output
            .report
            .pointer("/mutation/committed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        report.packages[index].package_operation = Some(compact(&output.report));
        files::json(&report.record_path, report)?;
        require(
            output.success && report.packages[index].committed,
            format!(
                "package installation stopped for {}; retain committed packages and inspect its reported recovery before resuming",
                reference.package_id
            ),
        )?;
    }
    store::healthy(backend, layout)?;
    let observed = store::inventory(backend, layout)?;
    validate_installed(bundle, &observed)?;
    require(
        observed.packages.len() == bundle.packages.len(),
        "package commit reports differ from the observed installed package set",
    )?;
    report.installation_id = observed.installation_id;
    for package in &mut report.packages {
        package.verified = true;
    }
    report.phase = "configuring".into();
    files::json(&report.record_path, report)?;
    super::resources::publish(layout)?;
    super::configuration::publish(
        bundle,
        layout,
        &report.provider_configuration,
        &mut report.warnings,
    )?;
    report.phase = "activating".into();
    super::public_metadata::publish(bundle, layout)?;
    files::json(&report.record_path, report)?;
    activate::publish(bundle, layout)?;
    report.activated = true;
    report.phase = "active".into();
    Ok(())
}

fn validate_installed(bundle: &Bundle, installed: &store::Inventory) -> Result<()> {
    let expected = bundle
        .manifest
        .packages
        .iter()
        .map(|package| (&package.package_id, package))
        .collect::<BTreeMap<_, _>>();
    for actual in &installed.packages {
        let id = actual["package_id"]
            .as_str()
            .ok_or_else(|| Error::Invalid("AW package lacks ID".into()))?;
        let reference = expected.get(&id.to_owned()).ok_or_else(|| {
            Error::Invalid(format!(
                "unexpected package already installed in immutable release: {id}"
            ))
        })?;
        let package = bundle
            .packages
            .iter()
            .find(|package| package.package_id == id)
            .ok_or_else(|| Error::Invalid("unknown release package".into()))?;
        require(
            actual["version"] == reference.version
                && actual
                    .pointer("/source/bundle_sha256")
                    .and_then(Value::as_str)
                    == Some(reference.description_sha256.as_str())
                && actual["files"] == serde_json::to_value(&package.files)?,
            format!("installed {id} differs from immutable release input"),
        )?;
        let dependencies = actual["dependencies"]
            .as_array()
            .ok_or_else(|| Error::Invalid("AW package lacks exact dependency records".into()))?;
        require(
            dependencies.len() == package.dependencies.len(),
            format!("installed {id} dependencies differ from its description"),
        )?;
        for requirement in &package.dependencies {
            require(
                dependencies.iter().any(|dependency| {
                    dependency["package_id"] == requirement.package_id
                        && dependency["version"] == requirement.version
                        && dependency["installation_id"].as_str()
                            == installed.installation_id.as_deref()
                }),
                format!("installed {id} has a different dependency target"),
            )?;
        }
    }
    Ok(())
}

fn compact(value: &Value) -> Value {
    let mut summary = serde_json::Map::new();
    for key in [
        "format", "command", "status", "mutation", "error", "warnings",
    ] {
        if let Some(value) = value.get(key) {
            summary.insert(key.into(), value.clone());
        }
    }
    if let Some(problems) = value.pointer("/result/plan/problems") {
        summary.insert("plan_problems".into(), problems.clone());
    }
    Value::Object(summary)
}

/// Serialize product activation and unit publication so both bind to the same active release.
pub(super) struct ProductLock(File);
impl ProductLock {
    /// Hold the existing product lock before changing files that name its active programs.
    pub(super) fn acquire(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC)
            .open(path)
            .map_err(|error| files::io(path, error))?;
        files::control_file(path)?;
        file.try_lock_exclusive()
            .map_err(|error| files::io(path, error))?;
        Ok(Self(file))
    }
}
impl Drop for ProductLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}
