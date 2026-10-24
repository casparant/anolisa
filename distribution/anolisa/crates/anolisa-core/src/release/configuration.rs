//! Prepare explicit, disabled Provider definitions from the distribution's de-duplicated paths.

use super::{Bundle, Error, ReleaseLayout, Result, Scope, files, require};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Digest of the disabled, session-independent Provider template retained across retries.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfiguration {
    /// SHA-256 of the template's fixed program/resource paths and runtime resolution rules.
    pub sha256: String,
}

pub(super) fn document(bundle: &Bundle, layout: &ReleaseLayout) -> Result<Value> {
    let owned = bundle
        .packages
        .iter()
        .flat_map(|package| package.files.iter())
        .collect::<BTreeMap<_, _>>();
    let has = |name: &str| {
        bundle
            .packages
            .iter()
            .any(|package| package.package_id == name)
    };
    let executable = |name: &str| -> Result<std::path::PathBuf> {
        require(
            owned
                .get(&name.to_owned())
                .is_some_and(|payload| payload.mode == 0o755),
            format!("Provider binding requires uniquely owned executable {name}"),
        )?;
        Ok(layout.prefix.join(name))
    };
    let mut providers = BTreeMap::<String, Value>::new();
    if has("aw-provider-tokenless") {
        let program = executable("libexec/aw/providers/tokenless/aw-provider-tokenless")?;
        let cli = executable("bin/tokenless")?;
        let rtk = executable("libexec/anolisa/tokenless/rtk")?;
        let checksum = owned
            .get(&"libexec/anolisa/tokenless/rtk".to_owned())
            .ok_or_else(|| Error::Invalid("missing verified RTK metadata".into()))?;
        providers.insert("tokenless".into(), json!({"protocol":"aw-provider/v1alpha1","transport_template":{"type":"stdio","location":"agent","argv":[program,"--cli",cli,"--data-dir","{user_data_dir}/tokenless","--rtk",rtk,"--rtk-sha256",checksum.sha256]}}));
    }
    if has("aw-provider-sec-core") {
        let program = executable("libexec/aw/providers/sec-core/aw-provider-sec-core")?;
        let cli = executable("bin/agent-sec-cli")?;
        providers.insert("sec-core".into(), json!({"protocol":"aw-provider/v1alpha1","transport_template":{"type":"stdio","location":"agent","argv":[program,"--cli",cli,"--socket","{user_runtime_dir}/agent-sec-core.sock"]}}));
    }
    let value = json!({
        "schema_version":1,
        "kind":"anolisa-provider-template",
        "release_id":bundle.manifest.release_id,
        "prefix":layout.prefix,
        "resources":layout.resources,
        "enabled":false,
        "runtime_context":{
            "scope":"invoking-user",
            "user_data_dir":{"environment":"XDG_DATA_HOME","ignore_empty_or_relative":true,"home_fallback":".local/share","suffix":"anolisa"},
            "user_runtime_dir":{"environment":"XDG_RUNTIME_DIR","suffix":"anolisa","requires_absolute_owned_mode_0700_base":true,"fallback":null},
            "system_service":"requires explicit separate configuration; never infer it from installer UID"
        },
        "providers":providers,
        "note":"Unrendered template, not an AWConfiguration. Resolve placeholders for the invoking user's XDG data and secure session runtime before explicitly configuring Agent, events and policy. No Provider, daemon, adapter or system service was enabled."
    });
    Ok(value)
}

fn directory(layout: &ReleaseLayout) -> std::path::PathBuf {
    layout.config.join("releases").join(&layout.arch_id)
}

pub(super) fn prepare(bundle: &Bundle, layout: &ReleaseLayout) -> Result<ProviderConfiguration> {
    let bytes = serde_json::to_vec_pretty(&document(bundle, layout)?)?;
    Ok(ProviderConfiguration {
        sha256: files::digest_bytes(&bytes),
    })
}

fn bytes(
    bundle: &Bundle,
    layout: &ReleaseLayout,
    context: &ProviderConfiguration,
) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec_pretty(&document(bundle, layout)?)?;
    require(
        files::digest_bytes(&bytes) == context.sha256,
        "recorded Provider template differs from the initial installation context; preserve it for explicit recovery",
    )?;
    Ok(bytes)
}

pub(super) fn preflight(
    bundle: &Bundle,
    layout: &ReleaseLayout,
    context: &ProviderConfiguration,
) -> Result<()> {
    let bytes = bytes(bundle, layout, context)?;
    let path = directory(layout).join(format!("{}.providers.json", bundle.manifest.release_id));
    files::absolute(&path)?;
    if path.try_exists().map_err(|error| files::io(&path, error))? {
        files::control_file(&path)?;
        require(
            files::read(&path, 4 * 1024 * 1024)? == bytes,
            format!(
                "existing release Provider definitions changed; preserve {}",
                path.display()
            ),
        )?;
    }
    Ok(())
}

pub(super) fn publish(
    bundle: &Bundle,
    layout: &ReleaseLayout,
    context: &ProviderConfiguration,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let bytes = bytes(bundle, layout, context)?;
    let directory = directory(layout);
    let (directory_mode, file_mode) = match layout.scope {
        Scope::User => (0o700, 0o600),
        Scope::System => (0o755, 0o644),
    };
    files::tree(&directory, directory_mode)?;
    let path = directory.join(format!("{}.providers.json", bundle.manifest.release_id));
    match std::fs::symlink_metadata(&path) {
        Ok(_) => {
            files::control_file(&path)?;
            require(
                files::read(&path, 4 * 1024 * 1024)? == bytes,
                format!(
                    "existing release Provider definitions changed; preserve {}",
                    path.display()
                ),
            )?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            files::publish(&path, &bytes, file_mode)?
        }
        Err(error) => return Err(files::io(&path, error)),
    }
    warnings.push("Provider definitions are unrendered invoking-user-XDG templates; software installation does not enable Providers, connect native Agents, create service users, apply capabilities or activate systemd units".into());
    Ok(())
}
