//! Publish only nonsecret product and package versions for ordinary-user discovery.

use super::{Bundle, ReleaseLayout, Result, files, require};
use serde_json::json;
use std::path::PathBuf;

const NAME: &str = "anolisa/installed-product.json";

fn path(layout: &ReleaseLayout) -> PathBuf {
    layout.resources.join(NAME)
}

fn document(bundle: &Bundle) -> Result<Vec<u8>> {
    let packages: Vec<_> = bundle
        .packages
        .iter()
        .map(|package| json!({"package_id":package.package_id,"version":package.version}))
        .collect();
    Ok(serde_json::to_vec_pretty(&json!({
        "schema_version":1,
        "product_version":bundle.manifest.product_version,
        "release_id":bundle.manifest.release_id,
        "source_commit":bundle.manifest.source_commit,
        "os":bundle.manifest.os,
        "arch":bundle.manifest.arch,
        "manifest_sha256":bundle.manifest_sha256,
        "packages":packages,
    }))?)
}

pub(super) fn preflight(bundle: &Bundle, layout: &ReleaseLayout) -> Result<()> {
    let name = format!("share/{NAME}");
    require(
        !bundle
            .packages
            .iter()
            .any(|package| package.files.contains_key(&name)),
        "package payload conflicts with the reserved product version summary",
    )?;
    let target = path(layout);
    match std::fs::symlink_metadata(&target) {
        Ok(_) => {
            files::control_file(&target)?;
            require(
                files::read(&target, 1024 * 1024)? == document(bundle)?,
                "existing public product summary has unknown or changed contents",
            )
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(files::io(&target, error)),
    }
}

pub(super) fn publish(bundle: &Bundle, layout: &ReleaseLayout) -> Result<()> {
    preflight(bundle, layout)?;
    let target = path(layout);
    files::tree(&layout.resources.join("anolisa"), 0o755)?;
    files::publish(&target, &document(bundle)?, 0o644)
}
