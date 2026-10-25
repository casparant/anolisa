//! Explicit raw-to-product transfer preserves old files and retires only proven records.

use super::{Bundle, InstallReport, ReleaseLayout, Result, Scope, activate, files, require};
use crate::{
    deployment::{self, Systemctl},
    domain::ProviderBinding,
    facts::{JournalEvidence, pending_journal_for},
    lock::InstallLock,
    state::{FileOwner, ObjectKind},
    state_store::StateStore,
};
use anolisa_platform::fs_layout::{FsLayout, InstallMode};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

/// One explicitly accepted public command, backed up before replacement.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LegacyEntrypoint {
    /// Command name in the selected product's public bin directory.
    pub name: String,
    /// Exact old bytes accepted by the reviewed plan.
    pub sha256: String,
}

/// Fixed migration evidence; changing the source, target or choices requires a new plan.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationPlan {
    /// Migration format version, currently one.
    pub schema_version: u32,
    /// SHA-256 of this plan with an empty identity field.
    pub id: String,
    /// Real caller allowed to apply this plan.
    pub uid: u32,
    /// Verified bundle input, before product selection.
    pub bundle: PathBuf,
    /// Exact release.json digest.
    pub manifest_sha256: String,
    /// Product scope, which must match the old installation scope.
    pub scope: Scope,
    /// Explicit profile, or the bundle's scope default.
    pub profile: Option<String>,
    /// Explicit independent component selection.
    pub components: Vec<String>,
    /// Old state file; it remains the authority until the new release succeeds.
    pub source_state: PathBuf,
    /// Whether old managed state existed; CLI-only installs retain an absence proof.
    pub source_state_present: bool,
    /// Exact old-state bytes required on the first attempt.
    pub source_sha256: String,
    /// Old prefix, used to reject environment or layout drift.
    pub source_prefix: PathBuf,
    /// Final new program prefix, never registered under the old prefix.
    pub target_prefix: PathBuf,
    /// New resource location, outside the old component directories.
    pub target_resources: PathBuf,
    /// Explicit adoption of a historical CLI that has no persistent ownership receipt.
    pub legacy_cli_sha256: Option<String>,
    /// Old raw records retired only after successful product activation.
    pub retire: Vec<String>,
    /// Existing commands whose old bytes have proven or explicitly accepted ownership.
    pub entrypoints: Vec<LegacyEntrypoint>,
    /// Old files retained for rollback, configuration, data and existing references.
    pub retained_paths: Vec<PathBuf>,
    /// Conditions that must be resolved before creating a new applicable plan.
    pub blockers: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Progress {
    plan: MigrationPlan,
    phase: String,
    installation: Option<InstallReport>,
}

fn state_error(error: impl std::fmt::Display) -> super::Error {
    super::Error::Invalid(format!("legacy migration: {error}"))
}

fn backend_package(name: &str) -> &str {
    match name {
        "anolisa" | "anolisa-cli" => "anolisa-cli",
        "aw" | "aw-core" => "aw-core",
        "cosh" | "cosh-ng" => "cosh-ng-core",
        "sec-core" | "agent-sec-core" => "agent-sec-cli",
        "sight" | "agentsight" => "agentsight",
        "memory" | "agent-memory" => "agent-memory-core",
        "ckpt" | "ws-ckpt" => "ws-ckpt-core",
        "skillfs" => "skillfs-core",
        "tokenless" => "tokenless-core",
        "skill" => "os-skills",
        other => other,
    }
}

fn pending_legacy_work(
    bundle: &Bundle,
    source: &FsLayout,
    store: &StateStore,
) -> Result<Vec<String>> {
    let selected = |name: &str| {
        bundle
            .packages
            .iter()
            .any(|package| package.package_id == backend_package(name))
    };
    let mut blockers = Vec::new();
    let mut names: BTreeSet<&str> = [
        "anolisa",
        "anolisa-cli",
        "aw",
        "aw-core",
        "cosh",
        "cosh-ng",
        "sec-core",
        "agent-sec-core",
        "sight",
        "agentsight",
        "memory",
        "agent-memory",
        "ckpt",
        "ws-ckpt",
        "skillfs",
        "tokenless",
        "skill",
        "os-skills",
        "ktuner",
    ]
    .into_iter()
    .filter(|name| selected(name))
    .collect();
    for row in &store.installations {
        if row.kind == ObjectKind::Component && selected(&row.name) {
            names.insert(&row.name);
        }
    }
    for row in &store.quarantined {
        if row.record.kind == ObjectKind::Component && selected(&row.record.name) {
            names.insert(&row.record.name);
            blockers.push(format!("{} has quarantined legacy state; repair or explicitly forget it before transferring its files", row.record.name));
        }
    }
    let journals = source.state_dir.join("journal");
    for name in names {
        if let Some(path) =
            pending_journal_for(JournalEvidence::new(&journals, &store.operations), name)
                .map_err(state_error)?
        {
            blockers.push(format!(
                "{name} has pending legacy recovery at {}; finish recovery before migration",
                path.display()
            ));
        }
    }
    Ok(blockers)
}

const ABSENT_STATE: &[u8] = b"ANOLISA migration: installed.toml was absent\n";

fn source_inventory(path: &Path, source: &FsLayout) -> Result<(Option<Vec<u8>>, StateStore)> {
    files::absolute(path)?;
    match fs::symlink_metadata(path) {
        Ok(_) => {
            files::control_file(path)?;
            let bytes = files::read(path, 16 * 1024 * 1024)?;
            let store = StateStore::load_for_layout(path, nix::unistd::geteuid().as_raw(), source)
                .map_err(state_error)?;
            Ok((Some(bytes), store))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok((None, StateStore::empty_for_layout(source)))
        }
        Err(error) => Err(files::io(path, error)),
    }
}

impl MigrationPlan {
    /// Write a reviewed plan to an existing caller-owned directory.
    ///
    /// # Errors
    /// Refuses unsafe paths, ownership or publication failures.
    pub fn write(&self, path: &Path) -> Result<()> {
        self.validate()?;
        files::absolute(path)?;
        files::json(path, self)
    }

    /// Read a bounded, caller-owned plan without following symbolic links.
    ///
    /// # Errors
    /// Rejects modified plan contents, incompatible formats and unsafe control files.
    pub fn read(path: &Path) -> Result<Self> {
        files::control_file(path)?;
        let plan: Self = files::decode(&files::read(path, 4 * 1024 * 1024)?)?;
        plan.validate()?;
        Ok(plan)
    }

    fn validate(&self) -> Result<()> {
        let mut copy = self.clone();
        copy.id.clear();
        require(
            self.schema_version == 1 && self.id == files::digest_bytes(&serde_json::to_vec(&copy)?),
            "migration plan changed; generate and review a new plan",
        )?;
        require(
            self.uid == nix::unistd::geteuid().as_raw(),
            "migration plan belongs to a different caller",
        )
    }
}

/// Inspect a same-scope legacy installation without changing files or invoking AW.
///
/// RPM ownership is never transferred. Services must be inactive and native
/// adapters disabled before a raw component can change its managing installer.
/// Configuration, data and old resources remain in place for explicit rollback.
///
/// # Errors
/// Rejects unreadable ownership evidence and unsafe source paths.
pub fn plan_migration(
    bundle: &Bundle,
    target: &ReleaseLayout,
    source: &FsLayout,
    profile: Option<String>,
    components: Vec<String>,
    legacy_cli_sha256: Option<String>,
) -> Result<MigrationPlan> {
    verify_choices(bundle, target.scope, profile.as_deref(), &components)?;
    let source_state = source.state_dir.join("installed.toml");
    let (source_bytes, store) = source_inventory(&source_state, source)?;
    let mut plan = MigrationPlan {
        schema_version: 1,
        id: String::new(),
        uid: nix::unistd::geteuid().as_raw(),
        bundle: bundle.root.clone(),
        manifest_sha256: bundle.manifest_sha256.clone(),
        scope: target.scope,
        profile,
        components,
        source_state,
        source_state_present: source_bytes.is_some(),
        source_sha256: files::digest_bytes(source_bytes.as_deref().unwrap_or_default()),
        source_prefix: source.prefix.clone(),
        target_prefix: target.prefix.clone(),
        target_resources: target.resources.clone(),
        legacy_cli_sha256,
        retire: Vec::new(),
        entrypoints: Vec::new(),
        retained_paths: vec![
            source.etc_dir.clone(),
            source.state_dir.clone(),
            source.datadir.clone(),
        ],
        blockers: Vec::new(),
    };
    if (source.mode == InstallMode::System) != (target.scope == Scope::System) {
        plan.blockers.push("scope conversion requires separately deploying the target scope and migrating each user's access".into());
    }
    plan.blockers
        .extend(pending_legacy_work(bundle, source, &store)?);
    let selected = |id: &str| {
        bundle
            .packages
            .iter()
            .any(|package| package.package_id == id)
    };
    let mut owned = BTreeMap::new();
    for installation in store
        .installations
        .iter()
        .filter(|row| row.kind == ObjectKind::Component && selected(backend_package(&row.name)))
    {
        let ProviderBinding::Owned { artifact } = &installation.binding else {
            plan.blockers.push(format!("{} belongs to a native package manager; upgrade it through that manager instead of transferring its files to AW", installation.name));
            continue;
        };
        if artifact.files.is_empty() {
            plan.blockers.push(format!(
                "{} has no reliable old file inventory",
                installation.name
            ));
            continue;
        }
        if store
            .adapter_claims
            .iter()
            .any(|claim| claim.component == installation.name)
        {
            plan.blockers.push(format!("disable native adapters for {} before changing its managing installer; Agent configuration is not converted automatically", installation.name));
        }
        for service in &artifact.services {
            if service.manager != "systemd" {
                plan.blockers.push(format!(
                    "{} uses unsupported migration service manager {}",
                    service.name, service.manager
                ));
                continue;
            }
            let scope = match service.scope {
                crate::manifest::ServiceScope::User => deployment::Scope::User,
                crate::manifest::ServiceScope::System => deployment::Scope::System,
            };
            match deployment::SystemctlRunner.inspect(scope, &service.name) {
                Ok(status) if status.loaded && !status.active => {},
                Ok(_) => plan.blockers.push(format!("stop {} before changing its managing installer; its enable state and unit file will be preserved", service.name)),
                Err(error) => plan.blockers.push(format!("cannot prove {} is inactive: {error}", service.name)),
            }
        }
        let scoped = source
            .for_component(&installation.name)
            .map_err(state_error)?;
        for file in &artifact.files {
            plan.retained_paths.push(file.path.clone());
            if file.owner == FileOwner::Anolisa
                && file.sha256.is_some()
                && crate::path_safety::validate_owned_path(&scoped, &file.path).is_ok()
            {
                owned.insert(file.path.clone(), file.sha256.clone().unwrap_or_default());
            }
        }
        plan.retire.push(installation.name.clone());
    }
    for name in bundle.manifest.entrypoints.keys() {
        let path = target.bin.join(name);
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                files::control_file(&path)?;
                let digest = files::digest(&path)?;
                let proof = owned.get(&path).is_some_and(|hash| hash == &digest)
                    || (name == "anolisa" && plan.legacy_cli_sha256.as_ref() == Some(&digest));
                if proof {
                    plan.entrypoints.push(LegacyEntrypoint {
                        name: name.clone(),
                        sha256: digest,
                    });
                } else {
                    plan.blockers.push(format!("unproven existing command {}; a historical standalone anolisa CLI requires explicit --legacy-cli-sha256 adoption", path.display()));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(files::io(&path, error)),
        }
    }
    plan.retire.sort();
    plan.retained_paths.sort();
    plan.retained_paths.dedup();
    plan.id = files::digest_bytes(&serde_json::to_vec(&plan)?);
    Ok(plan)
}

fn verify_choices(
    bundle: &Bundle,
    scope: Scope,
    profile: Option<&str>,
    components: &[String],
) -> Result<()> {
    let selected = Bundle::read(&bundle.root)?.select(scope, profile, components)?;
    require(
        selected.manifest_sha256 == bundle.manifest_sha256
            && serde_json::to_vec(&selected.manifest)? == serde_json::to_vec(&bundle.manifest)?
            && serde_json::to_vec(&selected.packages)? == serde_json::to_vec(&bundle.packages)?,
        "migration bundle does not match the reviewed profile and component selection",
    )
}

fn quiescent(store: &StateStore, retired: &[String], runner: &impl Systemctl) -> Result<()> {
    for installation in &store.installations {
        if !retired.contains(&installation.name) || installation.kind != ObjectKind::Component {
            continue;
        }
        require(
            !store
                .adapter_claims
                .iter()
                .any(|claim| claim.component == installation.name),
            "native adapters became active after migration planning",
        )?;
        let ProviderBinding::Owned { artifact } = &installation.binding else {
            return Err(state_error("migration source is no longer raw-owned"));
        };
        for service in &artifact.services {
            require(
                service.manager == "systemd",
                "unsupported source service manager",
            )?;
            let scope = match service.scope {
                crate::manifest::ServiceScope::User => deployment::Scope::User,
                crate::manifest::ServiceScope::System => deployment::Scope::System,
            };
            let current = runner.inspect(scope, &service.name).map_err(state_error)?;
            require(
                current.loaded && !current.active,
                format!(
                    "{} became active after migration planning; stop it before retrying",
                    service.name
                ),
            )?;
        }
    }
    Ok(())
}

/// Apply fixed migration evidence and retire old raw records after successful activation.
///
/// # Errors
/// Rejects changed state, ownership, active services or a different caller/layout.
/// Failed installation retains old records and durable backup material for retry.
pub fn apply_migration(
    plan: &MigrationPlan,
    bundle: &Bundle,
    target: &ReleaseLayout,
    source: &FsLayout,
) -> Result<InstallReport> {
    require(
        std::env::consts::OS == "linux" && bundle.manifest.arch == std::env::consts::ARCH,
        "migration installation requires matching native Linux",
    )?;
    require(
        target.scope != Scope::System || nix::unistd::geteuid().is_root(),
        "system migration requires root",
    )?;
    apply_with(plan, bundle, target, source, || {
        super::install(bundle, target, false)
    })
}

pub(super) fn apply_with(
    plan: &MigrationPlan,
    bundle: &Bundle,
    target: &ReleaseLayout,
    source: &FsLayout,
    install: impl FnOnce() -> Result<InstallReport>,
) -> Result<InstallReport> {
    plan.validate()?;
    verify_choices(
        bundle,
        plan.scope,
        plan.profile.as_deref(),
        &plan.components,
    )?;
    require(
        plan.bundle == bundle.root
            && plan.manifest_sha256 == bundle.manifest_sha256
            && plan.target_prefix == target.prefix
            && plan.target_resources == target.resources
            && plan.scope == target.scope
            && plan.source_prefix == source.prefix
            && plan.source_state == source.state_dir.join("installed.toml"),
        "migration source or target differs from the reviewed plan",
    )?;
    require(
        plan.blockers.is_empty(),
        format!("migration has blockers: {}", plan.blockers.join("; ")),
    )?;
    files::absolute(&source.lock_file)?;
    if source.lock_file.exists() {
        files::control_file(&source.lock_file)?;
    }
    let _lock = InstallLock::acquire(&source.lock_file).map_err(state_error)?;
    let directory = target.state.join("migrations").join(&plan.id);
    let record_path = directory.join("record.json");
    let backup = directory.join(if plan.source_state_present {
        "installed.toml.before"
    } else {
        "installed.toml.absent"
    });
    let mut progress = if record_path
        .try_exists()
        .map_err(|error| files::io(&record_path, error))?
    {
        files::control_file(&record_path)?;
        let progress: Progress = files::decode(&files::read(&record_path, 8 * 1024 * 1024)?)?;
        require(
            matches!(
                progress.phase.as_str(),
                "prepared" | "retiring" | "complete"
            ),
            "unsupported migration progress phase",
        )?;
        require(
            serde_json::to_vec(&progress.plan)? == serde_json::to_vec(plan)?,
            "migration record belongs to another plan",
        )?;
        progress
    } else {
        let current = plan_migration(
            bundle,
            target,
            source,
            plan.profile.clone(),
            plan.components.clone(),
            plan.legacy_cli_sha256.clone(),
        )?;
        require(
            serde_json::to_vec(&current)? == serde_json::to_vec(plan)?,
            "migration evidence changed; generate a new plan",
        )?;
        files::tree(&directory, 0o700)?;
        let original_bytes = if plan.source_state_present {
            files::read(&plan.source_state, 16 * 1024 * 1024)?
        } else {
            ABSENT_STATE.to_vec()
        };
        files::publish(&backup, &original_bytes, 0o600)?;
        for entry in &plan.entrypoints {
            let path = target.bin.join(&entry.name);
            require(
                files::digest(&path)? == entry.sha256,
                "legacy command changed before backup",
            )?;
            let mode = files::regular(&path)?.permissions();
            use std::os::unix::fs::PermissionsExt;
            files::publish(
                &directory.join(format!("{}.before", entry.name)),
                &files::read(&path, 1024 * 1024 * 1024)?,
                mode.mode() & 0o777,
            )?;
        }
        let progress = Progress {
            plan: plan.clone(),
            phase: "prepared".into(),
            installation: None,
        };
        files::json(&record_path, &progress)?;
        progress
    };
    files::control_file(&backup)?;
    let backup_bytes = files::read(&backup, 16 * 1024 * 1024)?;
    let original_store = if plan.source_state_present {
        require(
            files::digest_bytes(&backup_bytes) == plan.source_sha256,
            "legacy state backup changed",
        )?;
        StateStore::load_for_layout(&backup, plan.uid, source).map_err(state_error)?
    } else {
        require(
            backup_bytes == ABSENT_STATE,
            "legacy state absence marker changed",
        )?;
        StateStore::empty_for_layout(source)
    };
    let (source_bytes, mut store) = source_inventory(&plan.source_state, source)?;
    require(
        source_bytes.is_some() == plan.source_state_present,
        "legacy state appeared or disappeared after migration planning",
    )?;
    let original =
        files::digest_bytes(source_bytes.as_deref().unwrap_or_default()) == plan.source_sha256;
    if !original {
        require(
            matches!(progress.phase.as_str(), "retiring" | "complete"),
            "legacy state changed after migration preparation",
        )?;
        let mut expected = original_store.clone();
        retire(&mut expected, plan);
        require(
            store.installations == expected.installations
                && store.quarantined == expected.quarantined
                && store.adapter_claims == expected.adapter_claims
                && store.adapter_trust_roots == expected.adapter_trust_roots
                && store.backups == expected.backups
                && store.operations == expected.operations
                && store.install_mode == expected.install_mode
                && store.prefix == expected.prefix,
            "legacy state changed beyond the recorded migration",
        )?;
    }
    if progress.phase == "complete" {
        let report = super::status(bundle, target)?
            .ok_or_else(|| state_error("completed migration lost its product record"))?;
        require(
            report.succeeded(),
            "completed migration no longer has a successful product record",
        )?;
        return Ok(report);
    }
    require(
        pending_legacy_work(bundle, source, &original_store)?.is_empty(),
        "legacy recovery appeared after migration planning; finish recovery before retrying",
    )?;
    quiescent(&original_store, &plan.retire, &deployment::SystemctlRunner)?;
    {
        let _product_lock =
            super::install::ProductLock::acquire(&target.state.join("release.lock"))?;
        activate::accept_migration_entries(bundle, target, &plan.entrypoints)?;
    }
    let report = install()?;
    progress.installation = Some(report.clone());
    if !report.succeeded() {
        files::json(&record_path, &progress)?;
        return Ok(report);
    }
    progress.phase = "retiring".into();
    files::json(&record_path, &progress)?;
    if original && plan.source_state_present {
        retire(&mut store, plan);
        store.save(&plan.source_state).map_err(state_error)?;
    }
    progress.phase = "complete".into();
    files::json(&record_path, &progress)?;
    Ok(report)
}

fn retire(store: &mut StateStore, plan: &MigrationPlan) {
    for name in &plan.retire {
        store.remove(ObjectKind::Component, name);
    }
}
