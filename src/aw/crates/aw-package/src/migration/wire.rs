//! Read-only subsets of legacy TOML wire shapes; private payloads never become output.

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
pub(super) struct State {
    pub schema_version: u32,
    pub updated_at: String,
    pub install_mode: String,
    pub prefix: String,
    pub anolisa_version: String,
    #[serde(default)]
    pub objects: Vec<OldRecord>,
    #[serde(default)]
    pub installations: Vec<CurrentRecord>,
    #[serde(default)]
    pub quarantined: Vec<Quarantined>,
    #[serde(default)]
    pub adapter_claims: Vec<toml::Table>,
    #[serde(default)]
    pub adapter_trust_roots: Vec<toml::Table>,
    #[serde(default)]
    pub backups: Vec<toml::Table>,
    #[serde(default)]
    pub operations: Vec<toml::Table>,
}

#[derive(Deserialize)]
pub(super) struct OldRecord {
    pub kind: String,
    pub name: String,
    pub version: String,
    pub status: String,
    pub installed_at: String,
    pub install_backend: Option<String>,
    pub ownership: Option<String>,
    #[serde(default = "default_true")]
    pub managed: bool,
    #[serde(default)]
    pub adopted: bool,
    pub rpm_metadata: Option<RpmMetadata>,
    #[serde(default)]
    pub files: Vec<toml::Table>,
    #[serde(default)]
    pub services: Vec<toml::Table>,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
pub(super) struct RpmMetadata {
    pub package_name: String,
}

#[derive(Deserialize)]
pub(super) struct Quarantined {
    pub record: OldRecord,
    pub reason: toml::Table,
}

#[derive(Deserialize)]
pub(super) struct CurrentRecord {
    pub kind: String,
    pub name: String,
    pub status: String,
    pub installed_at: String,
    pub scope: WireScope,
    pub binding: Binding,
}

// The historical serde enum is externally tagged, unlike the new summary DTO.
#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum WireScope {
    System(String),
    User(BTreeMap<String, UserIdentity>),
}

#[derive(Deserialize)]
pub(super) struct UserIdentity {
    pub uid: u32,
}

#[derive(Deserialize)]
pub(super) struct Binding {
    pub authority: String,
    pub artifact: Option<Artifact>,
    pub pm: Option<String>,
    pub package: Option<Package>,
    pub relation: Option<Relation>,
    pub last_observed: Option<Observation>,
}

#[derive(Deserialize)]
pub(super) struct Artifact {
    pub version: String,
    #[serde(default)]
    pub files: Vec<toml::Table>,
    #[serde(default)]
    pub services: Vec<toml::Table>,
}

#[derive(Deserialize)]
pub(super) struct Package {
    pub resolution: String,
    pub name: Option<String>,
    pub component_hint: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct Relation {
    pub kind: String,
    pub since: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct Observation {
    pub version: String,
    pub observed_at: String,
}
