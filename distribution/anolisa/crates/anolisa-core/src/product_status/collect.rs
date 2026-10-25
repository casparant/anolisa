//! Read actual package authorities and bounded public product summaries without privileged helpers.

use super::*;
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Summary {
    schema_version: u32,
    product_version: String,
    release_id: String,
    source_commit: String,
    os: String,
    arch: String,
    manifest_sha256: String,
    packages: Vec<SummaryPackage>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SummaryPackage {
    package_id: String,
    version: String,
}

pub(super) fn evidence(
    request: &OverviewRequest<'_>,
    warnings: &mut Vec<String>,
) -> Vec<InstalledEvidence> {
    let mut evidence = package_database(request, warnings);
    for layout in &request.legacy_layouts {
        let scope = if layout.mode == anolisa_platform::fs_layout::InstallMode::System {
            "system"
        } else {
            "user"
        };
        if !request.visibility.includes(scope) {
            continue;
        }
        let path = layout.state_dir.join("installed.toml");
        match StateStore::load_for_layout(&path, nix::unistd::geteuid().as_raw(), layout) {
            Ok(store) => evidence.extend(legacy(&store, layout)),
            Err(error) => warnings.push(format!(
                "{scope} legacy installation state is unavailable: {error}"
            )),
        }
    }
    let mut roots = Vec::new();
    if let (Some(prefix), Some(resources)) = (&request.product_prefix, &request.product_resources) {
        let scope = if prefix.starts_with("/usr") {
            "system"
        } else {
            "user"
        };
        roots.push((prefix.clone(), resources.clone(), scope));
    }
    for (current, scope) in [
        (
            request.environment.home.join(format!(
                ".local/lib/{}-linux-gnu/anolisa/current",
                std::env::consts::ARCH
            )),
            "user",
        ),
        (PathBuf::from("/usr/local/lib/anolisa/current"), "system"),
    ] {
        if !request.visibility.includes(scope) {
            continue;
        }
        match fs::canonicalize(&current) {
            Ok(prefix) => {
                if let Ok(resources) = fs::canonicalize(prefix.join("share")) {
                    roots.push((prefix, resources, scope));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => warnings.push(format!("{scope} current product is unavailable: {error}")),
        }
    }
    let mut seen = BTreeSet::new();
    for (prefix, resources, scope) in roots {
        if !request.visibility.includes(scope) || !seen.insert(prefix.clone()) {
            continue;
        }
        match summary(&prefix, &resources, scope) {
            Ok(packages) => {
                for package in packages {
                    if !evidence.iter().any(|existing| {
                        existing.package_id == package.package_id
                            && existing.scope == package.scope
                            && existing.prefix == package.prefix
                    }) {
                        evidence.push(package);
                    }
                }
                warnings.push(format!("{scope} public product versions describe installed files; runtime health and private package integrity were not verified."));
            }
            Err(error) => warnings.push(format!(
                "{scope} public product summary is unavailable: {error}"
            )),
        }
    }
    if let Some(native) = request
        .native_packages
        .filter(|_| request.visibility != Visibility::User)
    {
        'inventory: for definition in DEFINITIONS {
            for name in definition.packages() {
                match native.query_installed(name) {
                    Ok(Some(package)) => evidence.push(InstalledEvidence {
                        package_id: name.to_string(),
                        version: Some(package.version.to_string()),
                        scope: "system".into(),
                        source: "rpmdb".into(),
                        prefix: None,
                        resources: None,
                    }),
                    Ok(None) => {}
                    Err(error) => {
                        warnings.push(format!("local RPM inventory is unavailable: {error}"));
                        break 'inventory;
                    }
                }
            }
        }
    }
    evidence
}

#[cfg(target_os = "linux")]
fn package_database(
    request: &OverviewRequest<'_>,
    warnings: &mut Vec<String>,
) -> Vec<InstalledEvidence> {
    use local_package::database::{DatabasePaths, Inventory, PackageManager};
    let mut result = Vec::new();
    for scope in ["user", "system"] {
        if !request.visibility.includes(scope) {
            continue;
        }
        let paths = if scope == "system" {
            Ok(DatabasePaths::system_for(PackageManager::Anolisa))
        } else {
            DatabasePaths::user_for(
                PackageManager::Anolisa,
                Some(&request.environment.state),
                None,
            )
        };
        match paths.and_then(|paths| Inventory::read(&paths, None)) {
            Ok(inventory) => {
                for installation in inventory.installations {
                    for package in installation.packages {
                        result.push(InstalledEvidence {
                            package_id: package.package_id,
                            version: Some(package.version),
                            scope: scope.into(),
                            source: "package_database".into(),
                            prefix: Some(installation.prefix.clone()),
                            resources: installation.resources.clone(),
                        });
                    }
                }
            }
            Err(error) => warnings.push(format!(
                "{scope} product package database is unavailable: {error}"
            )),
        }
    }
    result
}

#[cfg(not(target_os = "linux"))]
fn package_database(_: &OverviewRequest<'_>, _: &mut Vec<String>) -> Vec<InstalledEvidence> {
    Vec::new()
}

fn trusted(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("product summary path must be absolute".into());
    }
    let uid = nix::unistd::geteuid().as_raw();
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor).map_err(|error| error.to_string())?;
        let sticky = ancestor != path && metadata.uid() == 0 && metadata.mode() & 0o1000 != 0;
        if metadata.file_type().is_symlink()
            || !(metadata.uid() == uid || metadata.uid() == 0)
            || metadata.mode() & 0o022 != 0 && !sticky
        {
            return Err(format!(
                "unsafe product summary path: {}",
                ancestor.display()
            ));
        }
    }
    Ok(())
}

fn summary(prefix: &Path, resources: &Path, scope: &str) -> Result<Vec<InstalledEvidence>, String> {
    trusted(prefix)?;
    let path = resources.join("anolisa/installed-product.json");
    trusted(&path)?;
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(&path)
        .map_err(|error| error.to_string())?;
    let before = file.metadata().map_err(|error| error.to_string())?;
    if !before.is_file() || before.nlink() != 1 || before.len() > 1024 * 1024 {
        return Err("public product summary must be one bounded regular file".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 != before.len() {
        return Err("public product summary changed while reading".into());
    }
    let value: Summary = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let valid = |value: &str| {
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-+".contains(&byte))
    };
    if value.schema_version != 1
        || value.os != "linux"
        || value.arch != std::env::consts::ARCH
        || !valid(&value.release_id)
        || !valid(&value.product_version)
        || value.source_commit.len() != 40
        || value.manifest_sha256.len() != 64
        || value.packages.len() > 1024
    {
        return Err("unsupported or malformed public product summary".into());
    }
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for package in value.packages {
        if !valid(&package.package_id)
            || !valid(&package.version)
            || !seen.insert(package.package_id.clone())
        {
            return Err("invalid or duplicate public package identity".into());
        }
        result.push(InstalledEvidence {
            package_id: package.package_id,
            version: Some(package.version),
            scope: scope.into(),
            source: "public_product_summary".into(),
            prefix: Some(prefix.to_owned()),
            resources: Some(resources.to_owned()),
        });
    }
    Ok(result)
}

pub(super) fn component(
    definition: &Definition,
    evidence: &[InstalledEvidence],
    request: &OverviewRequest<'_>,
    manager: &impl Systemctl,
) -> ComponentOverview {
    let installations: Vec<_> = evidence
        .iter()
        .filter(|item| belongs(definition, &item.package_id))
        .cloned()
        .collect();
    let backend: Vec<_> = installations
        .iter()
        .filter(|item| definition.backend_ids.contains(&item.package_id.as_str()))
        .cloned()
        .collect();
    let mut directories: Vec<_> = request
        .product_prefix
        .iter()
        .map(|root| root.join("bin"))
        .collect();
    for (current, scope) in [
        (
            request.environment.home.join(format!(
                ".local/lib/{}-linux-gnu/anolisa/current/bin",
                std::env::consts::ARCH
            )),
            "user",
        ),
        (
            PathBuf::from("/usr/local/lib/anolisa/current/bin"),
            "system",
        ),
    ] {
        if request.visibility.includes(scope) {
            directories.push(current);
        }
    }
    directories.extend(request.search_path.clone());
    directories.extend(
        backend
            .iter()
            .filter_map(|item| item.prefix.as_ref().map(|root| root.join("bin"))),
    );
    let executable = directories
        .iter()
        .flat_map(|root| definition.commands.iter().map(move |name| root.join(name)))
        .find(|path| command(path));
    let provider_packages = installations
        .iter()
        .filter(|item| definition.provider_ids.contains(&item.package_id.as_str()))
        .map(|item| item.package_id.clone())
        .collect();
    let plugin_packages = installations
        .iter()
        .filter(|item| definition.plugin_ids.contains(&item.package_id.as_str()))
        .map(|item| item.package_id.clone())
        .collect();
    let config = request.environment.config.join("anolisa/aw.yaml");
    let mut result = ComponentOverview {
        component: definition.name.into(),
        installations,
        command: executable,
        availability: "not_observed".into(),
        detail:
            "No installation or executable was observed; unavailable sources are listed separately."
                .into(),
        privilege_requirement: definition.privilege.map(str::to_owned),
        service: None,
        provider_packages,
        plugin_packages,
        next_actions: Vec::new(),
    };
    if result.command.is_some() {
        result.availability = "command_available".into();
        result.detail = "Executable is present; application configuration and runtime health have not been certified.".into();
    } else if !backend.is_empty() {
        result.availability = "files_missing".into();
        result.detail =
            "Installation is recorded, but its command is not executable at an observed location."
                .into();
    }
    if definition.name == "os-skills" && !backend.is_empty() {
        let present = backend
            .iter()
            .filter_map(|item| item.resources.as_ref())
            .any(|root| root.join("os-skills").is_dir() || root.join("skills").is_dir());
        result.availability = if present {
            "resources_available"
        } else {
            "recorded_resources_unverified"
        }
        .into();
        result.detail = "Skill files require an Agent integration or AW configuration; installation does not connect them automatically.".into();
    }
    if definition.name == "aw" && result.command.is_some() && !config.is_file() {
        result.availability = "needs_configuration".into();
        result.detail = "AW executable is available; create an invoking-user Agent configuration with anolisa setup.".into();
    }
    if let Some(unit) = definition.unit.filter(|_| result.command.is_some()) {
        let mut service = ServiceObservation {
            unit: unit.into(),
            checked: request.check_services,
            loaded: None,
            active: None,
            error: None,
        };
        if request.check_services {
            match manager.inspect(ServiceScope::System, unit) {
                Ok(state) => {
                    service.loaded = Some(state.loaded);
                    service.active = Some(state.active);
                    if !state.active {
                        result.availability = "service_stopped".into();
                        result.detail = "The native service is not active; a present CLI does not make the backend usable.".into();
                    } else {
                        result.availability = "service_active_health_unverified".into();
                        result.detail = "systemd reports an active unit; protocol health and invoking-user access are not checked.".into();
                    }
                }
                Err(error) => service.error = Some(error.to_string()),
            }
        } else {
            result
                .detail
                .push_str(" Backend service state was not checked; use overview --check-services.");
        }
        result.service = Some(service);
    }
    if result.availability == "not_observed" {
        result.next_actions.push("anolisa install --help".into());
    } else {
        result
            .next_actions
            .push(format!("anolisa doctor {}", definition.name));
    }
    result
}
