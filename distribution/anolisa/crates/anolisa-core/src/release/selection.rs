//! Product selections resolve installed files independently of service activation.

use super::{Bundle, Error, Manifest, Result, Scope, files, model::identifier, require};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Product composition choices bound to the release's exact package inventory.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Distribution {
    /// Product-choice schema version, currently one.
    pub schema_version: u32,
    /// Explicit defaults keyed by installation scope.
    pub default_profiles: BTreeMap<String, String>,
    /// Named file selections; their presets do not run during package installation.
    pub profiles: BTreeMap<String, ProductProfile>,
    /// Independent component aliases mapped to their real local subpackages.
    pub components: BTreeMap<String, Vec<String>>,
    /// Known service presets exposed by the shared runtime description.
    pub presets: BTreeMap<String, ServicePreset>,
}

/// A product's exact install set and independently chosen service policy.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProductProfile {
    /// Local package identities selected by this profile.
    pub packages: Vec<String>,
    /// Scopes supported by this specific product selection.
    pub scopes: Vec<Scope>,
    /// Runtime policy name; installation does not automatically apply it.
    pub preset: String,
}

/// Service identifiers associated with an explicit deployment policy.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServicePreset {
    /// Stable service identities from the delivered runtime description.
    pub services: Vec<String>,
}

impl Distribution {
    pub(super) fn validate(&self, manifest: &Manifest) -> Result<()> {
        require(
            self.schema_version == 1,
            "unsupported distribution schema_version",
        )?;
        require(
            !self.profiles.is_empty() && self.profiles.len() <= 128,
            "invalid product profile count",
        )?;
        let available: BTreeSet<_> = manifest
            .packages
            .iter()
            .map(|package| package.package_id.as_str())
            .collect();
        let validate_packages = |packages: &[String]| -> Result<()> {
            require(!packages.is_empty(), "empty product package selection")?;
            let mut unique = BTreeSet::new();
            for name in packages {
                identifier(name)?;
                require(
                    available.contains(name.as_str()) && unique.insert(name),
                    format!("unknown or duplicate product package: {name}"),
                )?;
            }
            Ok(())
        };
        for (name, profile) in &self.profiles {
            identifier(name)?;
            validate_packages(&profile.packages)?;
            require(
                profile.packages.iter().any(|name| name == "anolisa-cli"),
                format!("profile lacks bootstrap packages: {name}"),
            )?;
            require(
                !profile.scopes.is_empty() && profile.scopes.len() <= 2,
                "invalid profile scopes",
            )?;
            require(
                profile
                    .scopes
                    .iter()
                    .map(|scope| scope.as_str())
                    .collect::<BTreeSet<_>>()
                    .len()
                    == profile.scopes.len(),
                "duplicate profile scope",
            )?;
            require(
                self.presets.contains_key(&profile.preset),
                format!("unknown profile preset: {}", profile.preset),
            )?;
        }
        for (name, packages) in &self.components {
            identifier(name)?;
            validate_packages(packages)?;
        }
        for (name, preset) in &self.presets {
            identifier(name)?;
            let mut unique = BTreeSet::new();
            for service in &preset.services {
                identifier(service)?;
                require(unique.insert(service), "duplicate preset service")?;
            }
        }
        require(
            self.default_profiles.len() == 2,
            "both user and system product defaults are required",
        )?;
        for scope in [Scope::User, Scope::System] {
            let default = self
                .default_profiles
                .get(scope.as_str())
                .and_then(|name| self.profiles.get(name));
            require(
                default.is_some_and(|profile| profile.scopes.contains(&scope)),
                format!(
                    "missing or incompatible default profile: {}",
                    scope.as_str()
                ),
            )?;
        }
        Ok(())
    }
}

impl Bundle {
    /// Select a dependency-complete product profile or independent component set.
    ///
    /// # Errors
    /// Rejects unknown profiles/components, incompatible scopes and legacy bundles
    /// that do not declare choices. Call after `read` verifies the complete input.
    pub fn select(
        &self,
        scope: Scope,
        profile: Option<&str>,
        components: &[String],
    ) -> Result<Self> {
        let Some(distribution) = &self.manifest.distribution else {
            require(
                profile.is_none() && components.is_empty(),
                "legacy release does not declare profile or component choices",
            )?;
            return Ok(self.clone());
        };
        let selected_profile = if let Some(name) = profile {
            Some(name)
        } else if components.is_empty() {
            distribution
                .default_profiles
                .get(scope.as_str())
                .map(String::as_str)
        } else {
            None
        };
        let mut selected = BTreeSet::from(["anolisa-cli".to_owned()]);
        if let Some(name) = selected_profile {
            let choice = distribution
                .profiles
                .get(name)
                .ok_or_else(|| Error::Invalid(format!("unknown product profile: {name}")))?;
            require(
                choice.scopes.contains(&scope),
                format!(
                    "profile {name} does not support {} installation",
                    scope.as_str()
                ),
            )?;
            selected.extend(choice.packages.iter().cloned());
        }
        let mut component_ids = BTreeSet::new();
        for name in components {
            require(
                component_ids.insert(name),
                format!("duplicate component selection: {name}"),
            )?;
            selected.extend(
                distribution
                    .components
                    .get(name)
                    .ok_or_else(|| Error::Invalid(format!("unknown product component: {name}")))?
                    .iter()
                    .cloned(),
            );
        }
        loop {
            let previous = selected.len();
            for package in &self.packages {
                if selected.contains(&package.package_id) {
                    selected.extend(
                        package
                            .dependencies
                            .iter()
                            .map(|dependency| dependency.package_id.clone()),
                    );
                }
            }
            if selected.len() == previous {
                break;
            }
        }
        let mut result = self.clone();
        result
            .packages
            .retain(|package| selected.contains(&package.package_id));
        result
            .manifest
            .packages
            .retain(|package| selected.contains(&package.package_id));
        require(
            result.packages.len() == selected.len(),
            "selected package is not in the verified input",
        )?;
        let owned: BTreeSet<_> = result
            .packages
            .iter()
            .flat_map(|package| package.files.keys().cloned())
            .collect();
        result
            .manifest
            .entrypoints
            .retain(|_, target| owned.contains(target));
        result.payload_bytes = 0;
        for reference in &result.manifest.packages {
            let package = result
                .packages
                .iter()
                .find(|package| package.package_id == reference.package_id)
                .ok_or_else(|| Error::Invalid("selected package metadata is missing".into()))?;
            for name in package.files.keys() {
                result.payload_bytes +=
                    files::regular(&result.root.join(&reference.path).join("payload").join(name))?
                        .len();
            }
        }
        let identity = serde_json::to_vec(&(
            self.manifest_sha256.as_str(),
            scope.as_str(),
            selected_profile,
            component_ids,
            &selected,
        ))?;
        let digest = files::digest_bytes(&identity);
        let base = &self.manifest.release_id[..self.manifest.release_id.len().min(110)];
        result.manifest.release_id = format!("{base}-s{}", &digest[..16]);
        // The source digest still binds every delivered byte; the derived release ID
        // keeps different package selections out of each other's resumable prefix.
        Ok(result)
    }
}
