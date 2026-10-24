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
        });
    }
    files::control_file(&path)?;
    let entries: Entries = files::decode(&files::read(&path, 1024 * 1024)?)?;
    require(
        entries.schema_version == 1,
        "unsupported entrypoint registry schema",
    )?;
    Ok(entries)
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
        // An interruption can always recognize an exact newly generated launcher on retry.
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

#[cfg(test)]
pub(super) fn script(layout: &ReleaseLayout, target: &str) -> String {
    launcher(layout, target)
}
