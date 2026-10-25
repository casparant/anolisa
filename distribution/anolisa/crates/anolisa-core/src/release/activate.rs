//! Publish only recorded launchers, then switch the separate current-version reference.

use super::{Bundle, ReleaseLayout, Result, Scope, files, require};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entries {
    schema_version: u32,
    files: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    targets: BTreeMap<String, String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Active {
    schema_version: u32,
    release_id: String,
    prefix: PathBuf,
    manifest_sha256: String,
}

fn registry(layout: &ReleaseLayout) -> PathBuf {
    layout.state.join("entrypoints.json")
}
fn active_path(layout: &ReleaseLayout) -> PathBuf {
    layout.state.join(format!("active-{}.json", layout.arch_id))
}

fn entries(layout: &ReleaseLayout) -> Result<Entries> {
    let path = registry(layout);
    if !path.try_exists().map_err(|error| files::io(&path, error))? {
        return Ok(Entries {
            schema_version: 1,
            files: BTreeMap::new(),
            targets: BTreeMap::new(),
        });
    }
    files::control_file(&path)?;
    let entries: Entries = files::decode(&files::read(&path, 1024 * 1024)?)?;
    require(
        entries.schema_version == 1,
        "unsupported entrypoint registry schema",
    )?;
    for name in entries.files.keys() {
        super::model::identifier(name)?;
    }
    for (name, target) in &entries.targets {
        require(
            entries.files.contains_key(name),
            "launcher target has no ownership digest",
        )?;
        super::model::payload_path(target)?;
        require(
            target.starts_with("bin/"),
            "public launcher must target bin/",
        )?;
    }
    Ok(entries)
}

fn obsolete(bundle: &Bundle, layout: &ReleaseLayout, entries: &Entries) -> Vec<(String, String)> {
    entries
        .files
        .iter()
        .filter_map(|(name, digest)| {
            if bundle.manifest.entrypoints.contains_key(name) {
                return None;
            }
            if let Some(target) = entries.targets.get(name) {
                return Some((name.clone(), target.clone()));
            }
            // The old registry recorded only digests. Shipped legacy launchers all
            // used bin/<command>; explicitly adopted legacy executables stay intact.
            let target = format!("bin/{name}");
            (files::digest_bytes(launcher(layout, &target).as_bytes()) == *digest)
                .then(|| (name.clone(), target))
        })
        .collect()
}

fn quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn launcher(layout: &ReleaseLayout, target: &str) -> String {
    let base = match layout.scope {
        Scope::System => format!("base={}\n", quote(&layout.base)),
        Scope::User => format!(
            "case $(/usr/bin/uname -m) in\n  x86_64|amd64) arch=x86_64-linux-gnu ;;\n  aarch64|arm64) arch=aarch64-linux-gnu ;;\n  *) echo 'unsupported ANOLISA launcher architecture' >&2; exit 1 ;;\nesac\nbase={}/\"$arch\"/anolisa\n",
            quote(&layout.user_library)
        ),
    };
    format!(
        "#!/bin/sh\n# ANOLISA release launcher v1; controlled by the product entrypoint registry.\nset -eu\n{base}selected=$(CDPATH= cd -P -- \"$base/current\" && pwd -P) || exit 1\ncase \"$selected\" in \"$base/releases/\"*) ;; *) echo 'invalid ANOLISA current-version reference' >&2; exit 1 ;; esac\nresources=$(CDPATH= cd -P -- \"$selected/share\" && pwd -P) || exit 1\nexport ANOLISA_RELEASE_PREFIX=\"$selected\"\nexport ANOLISA_RELEASE_RESOURCES=\"$resources\"\nexec \"$selected\"/{} \"$@\"\n",
        quote(Path::new(target))
    )
}

/// Refuse every unknown executable or symlink instead of guessing legacy ownership.
pub(super) fn preflight(bundle: &Bundle, layout: &ReleaseLayout) -> Result<()> {
    let entries = entries(layout)?;
    files::absolute(&layout.bin)?;
    for (name, target) in &bundle.manifest.entrypoints {
        let path = layout.bin.join(name);
        let expected = files::digest_bytes(launcher(layout, target).as_bytes());
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                files::control_file(&path)?;
                let digest = files::digest(&path)?;
                require(
                    entries.files.get(name) == Some(&digest)
                        || (digest == expected
                            && super::install::status(bundle, layout)?.is_some()),
                    format!(
                        "existing entrypoint {} has unverified ownership; preserve the old tool and explicitly migrate it before activating this release",
                        path.display()
                    ),
                )?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(files::io(&path, error)),
        }
    }
    for (name, target) in obsolete(bundle, layout, &entries) {
        let path = layout.bin.join(&name);
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                files::control_file(&path)?;
                let expected = files::digest_bytes(launcher(layout, &target).as_bytes());
                require(
                    entries.files.get(&name) == Some(&expected)
                        && files::digest(&path)? == expected,
                    format!(
                        "obsolete product command {} was modified; preserve it before changing the selection",
                        path.display()
                    ),
                )?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(files::io(&path, error)),
        }
    }
    let current = layout.current();
    match fs::symlink_metadata(&current) {
        Ok(metadata) => {
            require(
                metadata.file_type().is_symlink(),
                "current-version reference is an unknown non-symlink file",
            )?;
            let actual = fs::read_link(&current).map_err(|error| files::io(&current, error))?;
            if actual != layout.prefix {
                let path = active_path(layout);
                files::control_file(&path)?;
                let active: Active = files::decode(&files::read(&path, 1024 * 1024)?)?;
                require(
                    active.schema_version == 1
                        && actual == active.prefix
                        && actual.starts_with(layout.base.join("releases")),
                    "current-version reference differs from its activation record",
                )?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(files::io(&current, error)),
    }
    Ok(())
}

pub(super) fn accept_migration_entries(
    bundle: &Bundle,
    layout: &ReleaseLayout,
    accepted: &[super::migration::LegacyEntrypoint],
) -> Result<()> {
    let mut registry_entries = entries(layout)?;
    for entry in accepted {
        super::model::identifier(&entry.name)?;
        let target = bundle
            .manifest
            .entrypoints
            .get(&entry.name)
            .ok_or_else(|| {
                super::Error::Invalid(format!("migration command is not selected: {}", entry.name))
            })?;
        let path = layout.bin.join(&entry.name);
        files::control_file(&path)?;
        let actual = files::digest(&path)?;
        let generated = files::digest_bytes(launcher(layout, target).as_bytes());
        let already_published = actual == generated
            && registry_entries.files.get(&entry.name) == Some(&generated)
            && super::install::status(bundle, layout)?.is_some();
        require(
            actual == entry.sha256 || already_published,
            format!(
                "migration command changed after planning: {}",
                path.display()
            ),
        )?;
        registry_entries.files.insert(entry.name.clone(), actual);
        if !already_published {
            registry_entries.targets.remove(&entry.name);
        }
    }
    files::tree(&layout.state, 0o700)?;
    files::json(&registry(layout), &registry_entries)
}

pub(super) fn publish(bundle: &Bundle, layout: &ReleaseLayout) -> Result<()> {
    preflight(bundle, layout)?;
    files::tree(&layout.bin, 0o755)?;
    let mut entries = entries(layout)?;
    for (name, target) in &bundle.manifest.entrypoints {
        let bytes = launcher(layout, target);
        files::publish(&layout.bin.join(name), bytes.as_bytes(), 0o755)?;
        entries
            .files
            .insert(name.clone(), files::digest_bytes(bytes.as_bytes()));
        entries.targets.insert(name.clone(), target.clone());
        // An interruption can always recognize an exact newly generated launcher on retry.
        files::json(&registry(layout), &entries)?;
    }
    for (name, _) in obsolete(bundle, layout, &entries) {
        let path = layout.bin.join(&name);
        if path.try_exists().map_err(|error| files::io(&path, error))? {
            require(
                entries.files.get(&name) == Some(&files::digest(&path)?),
                "obsolete launcher changed before removal",
            )?;
            fs::remove_file(&path).map_err(|error| files::io(&path, error))?;
            files::sync(&layout.bin)?;
        }
        entries.files.remove(&name);
        entries.targets.remove(&name);
        files::json(&registry(layout), &entries)?;
    }
    let directory = tempfile::Builder::new()
        .prefix(".activation-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(&layout.base)
        .map_err(|error| files::io(&layout.base, error))?;
    let candidate = directory.path().join("current");
    std::os::unix::fs::symlink(&layout.prefix, &candidate)
        .map_err(|error| files::io(&candidate, error))?;
    files::sync(directory.path())?;
    fs::rename(&candidate, layout.current())
        .map_err(|error| files::io(&layout.current(), error))?;
    files::sync(&layout.base)?;
    files::json(
        &active_path(layout),
        &Active {
            schema_version: 1,
            release_id: bundle.manifest.release_id.clone(),
            prefix: layout.prefix.clone(),
            manifest_sha256: bundle.manifest_sha256.clone(),
        },
    )
}

pub(super) fn removal_preflight(bundle: &Bundle, layout: &ReleaseLayout) -> Result<()> {
    let current = layout.current();
    match fs::symlink_metadata(&current) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            require(
                !active_path(layout)
                    .try_exists()
                    .map_err(|error| files::io(&active_path(layout), error))?,
                "current reference is missing while an activation record exists; repair activation before removal",
            )?;
        }
        Err(error) => return Err(files::io(&current, error)),
        Ok(metadata) => {
            require(
                metadata.file_type().is_symlink(),
                "current-version reference is an unknown non-symlink file",
            )?;
            let selected = fs::read_link(&current).map_err(|error| files::io(&current, error))?;
            files::control_file(&active_path(layout))?;
            let active: Active = files::decode(&files::read(&active_path(layout), 1024 * 1024)?)?;
            require(
                active.schema_version == 1
                    && active.prefix == selected
                    && selected.parent() == Some(layout.base.join("releases").as_path())
                    && selected.file_name().and_then(|name| name.to_str())
                        == Some(active.release_id.as_str()),
                "current-version reference differs from its activation record",
            )?;
            files::directory(&selected)?;
            require(
                selected != layout.prefix,
                "active releases cannot be removed; activate another release before removing this inactive build",
            )?;
        }
    }
    let entries = entries(layout)?;
    for name in entries
        .files
        .keys()
        .chain(bundle.manifest.entrypoints.keys())
    {
        let path = layout.bin.join(name);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(files::io(&path, error)),
            Ok(_) => {
                files::control_file(&path)?;
                require(
                    entries.files.get(name) == Some(&files::digest(&path)?),
                    format!(
                        "product entrypoint has unverified or modified ownership; preserve {} before removal",
                        path.display()
                    ),
                )?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn script(layout: &ReleaseLayout, target: &str) -> String {
    launcher(layout, target)
}
