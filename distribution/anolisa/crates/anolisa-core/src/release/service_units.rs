//! Publish verified component unit templates separately from service activation.

use super::{Bundle, Error, ReleaseLayout, Result, Scope, files, require, status};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

const LIMIT: u64 = 1024 * 1024;

/// One immutable component template and its system-manager publication.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ServiceUnitAction {
    /// Component service identity, independent of its source package name.
    pub service: String,
    /// Selected package that owns the template.
    pub package_id: String,
    /// Installed, real resource path whose bytes are verified before rendering.
    pub template: PathBuf,
    /// Unit basename accepted by the existing component contract.
    pub unit: String,
    /// Native manager destination; no administrator override is overwritten.
    pub destination: PathBuf,
    /// Exact source template digest.
    pub template_sha256: String,
    /// Digest after resolving the verified version's program directory.
    pub rendered_sha256: String,
    /// Previously recorded target bytes, absent for a fresh publication.
    pub previous_sha256: Option<String>,
    /// Content-addressed backup retained before replacing an owned unit.
    pub backup: Option<PathBuf>,
    /// Existing component configuration conventions; missing files use component defaults.
    pub configuration_paths: Vec<PathBuf>,
}

/// A read-only unit publication plan; it does not enable or start any service.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ServiceUnitPlan {
    /// Unit publication protocol version.
    pub schema_version: u32,
    /// Successfully installed product selection.
    pub release_id: String,
    /// Input composition binding retained by the installation record.
    pub manifest_sha256: String,
    /// Program directory whose version is fixed into the unit.
    pub prefix: PathBuf,
    /// Real, separately installed static-resource directory.
    pub resources: PathBuf,
    /// Durable operation journal supporting identical-operation retries.
    pub record_path: PathBuf,
    /// Normalized requested services, including native service dependencies.
    pub services: Vec<String>,
    /// Exact publications reviewed by the caller.
    pub units: Vec<ServiceUnitAction>,
}

/// Durable publication progress, including a reload failure after successful file writes.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceUnitReport {
    /// Fixed intent recorded before the first filesystem publication.
    pub plan: ServiceUnitPlan,
    /// Units whose installed bytes and ownership record have been committed.
    pub published: Vec<String>,
    /// `prepared`, `publishing`, `reload_pending`, or `complete`.
    pub phase: String,
    /// Whether the native manager accepted daemon-reload.
    pub daemon_reloaded: bool,
    /// Actual failure retained for diagnosis and retry.
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OwnedUnit {
    release_id: String,
    destination: PathBuf,
    sha256: String,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Ownership {
    units: BTreeMap<String, OwnedUnit>,
}

trait Manager {
    fn directory(&self) -> PathBuf;
    fn overrides(&self, unit: &str) -> Vec<PathBuf>;
    fn reload(&self) -> io::Result<()>;
}

struct Systemd;
impl Manager for Systemd {
    fn directory(&self) -> PathBuf {
        PathBuf::from("/usr/local/lib/systemd/system")
    }
    fn overrides(&self, unit: &str) -> Vec<PathBuf> {
        [
            "/etc/systemd/system",
            "/run/systemd/system",
            "/usr/lib/systemd/system",
            "/lib/systemd/system",
        ]
        .into_iter()
        .map(|directory| Path::new(directory).join(unit))
        .collect()
    }
    fn reload(&self) -> io::Result<()> {
        crate::deployment::SystemctlRunner
            .daemon_reload(crate::deployment::Scope::System)
            .map_err(|error| io::Error::other(error.to_string()))
    }
}

/// Plan root-owned system unit publication from a successfully installed release.
///
/// # Errors
/// Refuses user scope, unsupported hosts, unowned native units and changed payloads.
pub fn plan_service_units(
    bundle: &Bundle,
    layout: &ReleaseLayout,
    services: &[String],
) -> Result<ServiceUnitPlan> {
    host(layout)?;
    plan_with(bundle, layout, services, &Systemd)
}

/// Publish an unchanged reviewed plan and reload systemd without enabling or starting units.
///
/// # Errors
/// Refuses changed input or target ownership. Execution failures after journaling
/// are retained in the returned report and can be retried with the same plan.
pub fn apply_service_units(
    bundle: &Bundle,
    layout: &ReleaseLayout,
    plan: &ServiceUnitPlan,
) -> Result<ServiceUnitReport> {
    host(layout)?;
    apply_with(bundle, layout, plan, &Systemd)
}

fn host(layout: &ReleaseLayout) -> Result<()> {
    require(
        layout.scope == Scope::System,
        "component system units require explicit system scope",
    )?;
    require(
        std::env::consts::OS == "linux" && nix::unistd::geteuid().is_root(),
        "component system unit publication requires Linux and root",
    )
}

fn directory(layout: &ReleaseLayout) -> PathBuf {
    layout.state.join("service-units")
}
fn ownership_path(layout: &ReleaseLayout) -> PathBuf {
    directory(layout).join("ownership.json")
}

pub(super) fn removal_preflight(layout: &ReleaseLayout, release_id: &str) -> Result<()> {
    require(
        !ownership(layout)?
            .units
            .values()
            .any(|unit| unit.release_id == release_id),
        "this release still owns published system units; explicitly stop and retire those units before removal",
    )?;
    let operations = directory(layout).join("operations");
    let entries = match fs::read_dir(&operations) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(files::io(&operations, error)),
    };
    files::owned(&operations)?;
    for (index, entry) in entries.enumerate() {
        require(
            index < 1024,
            "too many service publication operations; inspect them before removal",
        )?;
        let path = entry.map_err(|error| files::io(&operations, error))?.path();
        let Some(report) = saved(&path)? else {
            continue;
        };
        require(
            report.plan.prefix != layout.prefix && report.plan.release_id != release_id,
            "a recorded service publication still refers to this release; explicitly retire its units and operation record before removal",
        )?;
    }
    Ok(())
}
fn ownership(layout: &ReleaseLayout) -> Result<Ownership> {
    let path = ownership_path(layout);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Ownership::default()),
        Err(error) => Err(files::io(&path, error)),
        Ok(_) => {
            files::control_file(&path)?;
            files::decode(&files::read(&path, LIMIT)?)
        }
    }
}
fn existing(path: &Path) -> Result<Option<String>> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(files::io(path, error)),
        Ok(_) => {
            files::control_file(path)?;
            Ok(Some(files::digest(path)?))
        }
    }
}
fn installed(bundle: &Bundle, layout: &ReleaseLayout) -> Result<()> {
    bundle.recheck()?;
    require(
        layout.scope == Scope::System,
        "system component units cannot be published in user scope",
    )?;
    let report = status(bundle, layout)?
        .ok_or_else(|| Error::Invalid("release has no successful installation record".into()))?;
    require(
        report.succeeded() && !report.dry_run && report.activated,
        "unit publication requires this successfully activated release",
    )?;
    require(
        bundle.packages.iter().all(|package| {
            report.packages.iter().any(|result| {
                result.package_id == package.package_id
                    && result.version == package.version
                    && result.committed
                    && result.verified
            })
        }),
        "release record does not verify the selected packages",
    )
}

fn selected(services: &[String]) -> Result<Vec<String>> {
    require(
        !services.is_empty(),
        "select at least one component service",
    )?;
    let mut names = BTreeSet::new();
    for name in services {
        require(names.insert(name.clone()), "duplicate component service")?;
        require(
            matches!(
                name.as_str(),
                "ws-ckpt" | "agent-sec-core" | "agentsight" | "agentsight-enforcer"
            ),
            format!(
                "service {name} has no supported system unit template; COSH requires an explicit instance/user and AW uses its own service lifecycle"
            ),
        )?;
    }
    if names.contains("agentsight") {
        names.insert("agentsight-enforcer".into());
    }
    Ok(names.into_iter().collect())
}

fn specification(service: &str) -> (&str, Vec<&str>, Vec<&str>, Vec<&str>) {
    match service {
        "ws-ckpt" => (
            "ws-ckpt.service",
            vec![
                "share/ws-ckpt/templates/ws-ckpt.service",
                "share/anolisa/ws-ckpt/ws-ckpt.service",
            ],
            vec![
                "ExecStart=\"{bindir}/ws-ckpt\" daemon",
                "ExecReload=\"{bindir}/ws-ckpt\" reload",
            ],
            vec!["bin/ws-ckpt"],
        ),
        "agent-sec-core" => (
            "agent-sec-core.service",
            vec![
                "share/agent-sec-core/templates/agent-sec-core.service.in",
                "share/anolisa/sec-core/agent-sec-core.service.in",
            ],
            vec!["ExecStart=\"{bindir}/agent-sec-daemon\" serve"],
            vec!["bin/agent-sec-daemon"],
        ),
        "agentsight" => (
            "agentsight.service",
            vec![
                "share/agentsight/templates/agentsight.service",
                "share/anolisa/agentsight/agentsight.service",
            ],
            vec!["ExecStart=\"{bindir}/agentsight-start\""],
            vec!["bin/agentsight-start", "bin/agentsight"],
        ),
        _ => (
            "agentsight-enforcer.service",
            vec![
                "share/agentsight/templates/agentsight-enforcer.service",
                "share/anolisa/agentsight/agentsight-enforcer.service",
            ],
            vec!["ExecStart=\"{bindir}/agentsight-enforcer\""],
            vec!["bin/agentsight-enforcer"],
        ),
    }
}

fn resource(layout: &ReleaseLayout, relative: &str) -> PathBuf {
    match relative.strip_prefix("share/") {
        Some(name) => layout.resources.join(name),
        None => layout.prefix.join(relative),
    }
}
fn verified(
    bundle: &Bundle,
    layout: &ReleaseLayout,
    relative: &str,
    mode: u32,
) -> Result<(String, PathBuf, String)> {
    let matches: Vec<_> = bundle
        .packages
        .iter()
        .filter_map(|package| {
            package
                .files
                .get(relative)
                .map(|payload| (package, payload))
        })
        .collect();
    require(
        matches.len() == 1,
        format!("unit publication needs one selected owner for {relative}"),
    )?;
    let (package, payload) = matches[0];
    let path = resource(layout, relative);
    let metadata = files::regular(&path)?;
    require(
        payload.mode == mode
            && metadata.mode() & 0o7777 == mode
            && files::digest(&path)? == payload.sha256,
        format!("installed component resource changed: {}", path.display()),
    )?;
    Ok((package.package_id.clone(), path, payload.sha256.clone()))
}
fn render(
    bundle: &Bundle,
    layout: &ReleaseLayout,
    service: &str,
) -> Result<(ServiceUnitAction, Vec<u8>)> {
    let (unit, templates, expected_lines, programs) = specification(service);
    for program in programs {
        verified(bundle, layout, program, 0o755)?;
    }
    let candidates: Vec<_> = templates
        .into_iter()
        .filter(|name| {
            bundle
                .packages
                .iter()
                .any(|package| package.files.contains_key(*name))
        })
        .collect();
    require(
        candidates.len() == 1,
        format!("service {service} needs one verified component template"),
    )?;
    let (package_id, path, expected_digest) = verified(bundle, layout, candidates[0], 0o644)?;
    let bytes = files::read(&path, LIMIT)?;
    require(
        files::digest_bytes(&bytes) == expected_digest,
        "unit template changed after verification",
    )?;
    let template = std::str::from_utf8(&bytes)
        .map_err(|error| Error::Invalid(format!("unit template is not UTF-8: {error}")))?;
    require(
        expected_lines
            .iter()
            .all(|line| template.lines().any(|actual| actual == *line)),
        format!(
            "service {service} template entrypoints changed; review the component deployment contract"
        ),
    )?;
    let bindir = layout.prefix.join("bin");
    let value = bindir
        .to_str()
        .ok_or_else(|| Error::Invalid("unit bindir is not UTF-8".into()))?;
    require(
        !value.contains(['\n', '\r', '"', '\\', '%']),
        "unit bindir contains unsupported systemd quoting characters",
    )?;
    let rendered = template.replace("{bindir}", value);
    require(
        !rendered.contains("{bindir}")
            && !rendered.contains("{libexecdir}")
            && !rendered.contains("{datadir}"),
        "unit contains unresolved component layout placeholders",
    )?;
    let configuration_paths = match service {
        "ws-ckpt" => vec![PathBuf::from("/etc/ws-ckpt/config.toml")],
        "agent-sec-core" => vec![PathBuf::from("/etc/agent-sec/skillsec.json")],
        _ => Vec::new(),
    };
    Ok((
        ServiceUnitAction {
            service: service.into(),
            package_id,
            template: path,
            unit: unit.into(),
            destination: PathBuf::new(),
            template_sha256: files::digest_bytes(&bytes),
            rendered_sha256: files::digest_bytes(rendered.as_bytes()),
            previous_sha256: None,
            backup: None,
            configuration_paths,
        },
        rendered.into_bytes(),
    ))
}

fn no_overrides(manager: &impl Manager, unit: &str) -> Result<()> {
    for path in manager.overrides(unit) {
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(files::io(&path, error)),
            Ok(_) => {
                return Err(Error::Invalid(format!(
                    "native unit already exists outside this product's ownership; preserve {}",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}
fn operation(
    layout: &ReleaseLayout,
    bundle: &Bundle,
    services: &[String],
    target: &Path,
) -> Result<PathBuf> {
    let intent = serde_json::to_vec(&(
        bundle.manifest.release_id.as_str(),
        bundle.manifest_sha256.as_str(),
        &layout.prefix,
        &layout.resources,
        services,
        target,
    ))?;
    Ok(directory(layout)
        .join("operations")
        .join(format!("{}.json", files::digest_bytes(&intent))))
}
fn saved(path: &Path) -> Result<Option<ServiceUnitReport>> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(files::io(path, error)),
        Ok(_) => {
            files::control_file(path)?;
            Ok(Some(files::decode(&files::read(path, 4 * LIMIT)?)?))
        }
    }
}
fn compatible(actual: &ServiceUnitAction, expected: &ServiceUnitAction) -> bool {
    actual.service == expected.service
        && actual.package_id == expected.package_id
        && actual.template == expected.template
        && actual.unit == expected.unit
        && actual.destination == expected.destination
        && actual.template_sha256 == expected.template_sha256
        && actual.rendered_sha256 == expected.rendered_sha256
        && actual.configuration_paths == expected.configuration_paths
}
fn plan_with(
    bundle: &Bundle,
    layout: &ReleaseLayout,
    services: &[String],
    manager: &impl Manager,
) -> Result<ServiceUnitPlan> {
    installed(bundle, layout)?;
    let names = selected(services)?;
    let target = manager.directory();
    files::absolute(&target)?;
    let record = operation(layout, bundle, &names, &target)?;
    let pending = saved(&record)?;
    let authority = ownership(layout)?;
    let mut units = Vec::new();
    for service in &names {
        let (mut action, _) = render(bundle, layout, service)?;
        action.destination = target.join(&action.unit);
        no_overrides(manager, &action.unit)?;
        let current = existing(&action.destination)?;
        if let Some(report) = &pending {
            let previous = report
                .plan
                .units
                .iter()
                .find(|unit| unit.service == *service)
                .ok_or_else(|| Error::Invalid("unit journal lacks a requested service".into()))?;
            require(
                compatible(&action, previous),
                "unit journal differs from the verified component template",
            )?;
            require(
                current == previous.previous_sha256
                    || current.as_ref() == Some(&action.rendered_sha256),
                "unit target changed since the publication journal was prepared",
            )?;
            action = previous.clone();
        } else if let Some(digest) = current {
            let owner = authority.units.get(&action.unit).ok_or_else(|| {
                Error::Invalid(format!(
                    "unit target has no product ownership: {}",
                    action.destination.display()
                ))
            })?;
            require(
                owner.destination == action.destination && owner.sha256 == digest,
                "owned unit was modified; preserve the administrator's changes",
            )?;
            action.previous_sha256 = Some(digest.clone());
            action.backup = Some(
                directory(layout)
                    .join("backups")
                    .join(format!("{}-{digest}", action.unit)),
            );
        }
        units.push(action);
    }
    let plan = ServiceUnitPlan {
        schema_version: 1,
        release_id: bundle.manifest.release_id.clone(),
        manifest_sha256: bundle.manifest_sha256.clone(),
        prefix: layout.prefix.clone(),
        resources: layout.resources.clone(),
        record_path: record,
        services: names,
        units,
    };
    if let Some(report) = pending {
        require(
            report.plan == plan,
            "unit operation record conflicts with the requested installed release",
        )?;
    }
    Ok(plan)
}

fn apply_with(
    bundle: &Bundle,
    layout: &ReleaseLayout,
    plan: &ServiceUnitPlan,
    manager: &impl Manager,
) -> Result<ServiceUnitReport> {
    let expected = plan_with(bundle, layout, &plan.services, manager)?;
    require(
        plan == &expected,
        "unit plan changed; preview the current installed release again",
    )?;
    files::owned(&layout.state)?;
    let _product_lock = super::install::ProductLock::acquire(&layout.state.join("release.lock"))?;
    let directory = directory(layout);
    files::tree(&directory, 0o700)?;
    let lock_path = directory.join("publication.lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(&lock_path)
        .map_err(|error| files::io(&lock_path, error))?;
    files::control_file(&lock_path)?;
    lock.try_lock_exclusive()
        .map_err(|error| files::io(&lock_path, error))?;
    require(
        plan == &plan_with(bundle, layout, &plan.services, manager)?,
        "unit plan changed while acquiring publication lock",
    )?;
    files::tree(
        plan.record_path
            .parent()
            .ok_or_else(|| Error::Invalid("unit journal has no directory".into()))?,
        0o700,
    )?;
    let mut report = saved(&plan.record_path)?.unwrap_or(ServiceUnitReport {
        plan: plan.clone(),
        published: Vec::new(),
        phase: "prepared".into(),
        daemon_reloaded: false,
        error: None,
    });
    if report.phase == "complete" && report.daemon_reloaded && report.error.is_none() {
        return Ok(report);
    }
    report.error = None;
    files::json(&plan.record_path, &report)?;
    let mut execute = || -> Result<()> {
        let mut authority = ownership(layout)?;
        files::tree(&manager.directory(), 0o755)?;
        for action in &plan.units {
            no_overrides(manager, &action.unit)?;
            let (_, bytes) = render(bundle, layout, &action.service)?;
            let current = existing(&action.destination)?;
            require(
                current == action.previous_sha256
                    || current.as_ref() == Some(&action.rendered_sha256),
                "unit target changed during publication",
            )?;
            if current != Some(action.rendered_sha256.clone()) {
                if let Some(backup) = &action.backup {
                    let original = files::read(&action.destination, LIMIT)?;
                    require(
                        Some(files::digest_bytes(&original)) == action.previous_sha256,
                        "unit changed before backup",
                    )?;
                    files::tree(
                        backup
                            .parent()
                            .ok_or_else(|| Error::Invalid("unit backup has no directory".into()))?,
                        0o700,
                    )?;
                    match existing(backup)? {
                        Some(digest) => require(
                            Some(digest) == action.previous_sha256,
                            "unit backup differs from the recorded original",
                        )?,
                        None => files::publish(backup, &original, 0o600)?,
                    }
                }
                files::publish(&action.destination, &bytes, 0o644)?;
            }
            authority.units.insert(
                action.unit.clone(),
                OwnedUnit {
                    release_id: plan.release_id.clone(),
                    destination: action.destination.clone(),
                    sha256: action.rendered_sha256.clone(),
                },
            );
            files::json(&ownership_path(layout), &authority)?;
            if !report.published.contains(&action.unit) {
                report.published.push(action.unit.clone());
            }
            report.phase = "publishing".into();
            files::json(&plan.record_path, &report)?;
        }
        report.phase = "reload_pending".into();
        files::json(&plan.record_path, &report)?;
        manager.reload().map_err(|error| {
            Error::Invalid(format!(
                "units are published but native daemon-reload failed: {error}"
            ))
        })?;
        report.daemon_reloaded = true;
        report.phase = "complete".into();
        Ok(())
    };
    if let Err(error) = execute() {
        report.error = Some(error.to_string());
    }
    files::json(&plan.record_path, &report)?;
    Ok(report)
}

#[cfg(test)]
mod tests;
