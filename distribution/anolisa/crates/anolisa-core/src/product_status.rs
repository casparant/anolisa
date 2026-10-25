//! Offline product discovery separates installed evidence, usable commands and live service state.

mod collect;
mod native;
pub use native::RpmInventoryRunner;
#[cfg(test)]
mod tests;

use crate::{
    deployment::{Scope as ServiceScope, Systemctl},
    release::Environment,
    state_store::StateStore,
};
use anolisa_platform::{fs_layout::FsLayout, pkg_query::PackageQuery};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Installation visibility requested by a human operator, independent of write privileges.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// Inspect both current-user and publicly observable system installations.
    #[default]
    All,
    /// Inspect only the invoking user's installation evidence.
    User,
    /// Inspect system evidence without attempting privilege elevation.
    System,
}

impl Visibility {
    fn includes(self, scope: &str) -> bool {
        self == Self::All
            || matches!(
                (self, scope),
                (Self::User, "user") | (Self::System, "system")
            )
    }
}

/// One recorded installation, with provenance kept distinct from runtime health.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InstalledEvidence {
    /// Actual package or legacy component identifier.
    pub package_id: String,
    /// Recorded version; never inferred from the repository's component catalog.
    pub version: Option<String>,
    /// User or system installation boundary.
    pub scope: String,
    /// `package_database`, `public_product_summary`, `legacy_state`, or `rpmdb`.
    pub source: String,
    /// Private program root, when discovered from a recorded product installation.
    pub prefix: Option<PathBuf>,
    /// Static resource root, when discovered from a recorded product installation.
    pub resources: Option<PathBuf>,
}

/// Facts about one known service, without treating active systemd state as endpoint health.
#[derive(Clone, Debug, Serialize)]
pub struct ServiceObservation {
    /// Native unit whose state was inspected.
    pub unit: String,
    /// Whether a real native-manager inspection was requested.
    pub checked: bool,
    /// Whether systemd reported an installed unit; unknown when inspection failed or was skipped.
    pub loaded: Option<bool>,
    /// Whether systemd reported active/transitioning state; this is not a health probe.
    pub active: Option<bool>,
    /// Bounded native-manager diagnostic when inspection was unavailable.
    pub error: Option<String>,
}

/// Human-facing operational state derived only from observed installation and filesystem facts.
#[derive(Clone, Debug, Serialize)]
pub struct ComponentOverview {
    /// Stable component name used for CLI guidance.
    pub component: String,
    /// Recorded package versions and their evidence sources.
    pub installations: Vec<InstalledEvidence>,
    /// An actual executable found in this installation or the invoking user's PATH.
    pub command: Option<PathBuf>,
    /// `not_observed`, `files_missing`, `needs_configuration`, `service_stopped`, or a qualified availability label.
    pub availability: String,
    /// Concrete reason installation does or does not imply immediate usability.
    pub detail: String,
    /// Privileged functionality that requires a separately configured system service or access.
    pub privilege_requirement: Option<String>,
    /// Real optional service-manager evidence.
    pub service: Option<ServiceObservation>,
    /// Installed AW Provider package identifiers, without claiming they are enabled.
    pub provider_packages: Vec<String>,
    /// Installed native integration package identifiers, without claiming they are connected.
    pub plugin_packages: Vec<String>,
    /// Explicit local next actions; collection never performs them.
    pub next_actions: Vec<String>,
}

/// Read-only product overview, including unavailable sources instead of inventing absence.
#[derive(Debug, Serialize)]
pub struct ProductOverview {
    /// Stable output version.
    pub schema_version: u32,
    /// Current effective identity; sudo identities are never guessed or adopted.
    pub uid: u32,
    /// Observed component and runtime facts.
    pub components: Vec<ComponentOverview>,
    /// Sources that could not be read or whose evidence is weaker than file verification.
    pub warnings: Vec<String>,
    /// Remote latest release is never inferred by offline discovery.
    pub latest: String,
    /// Optional comparison with a locally verified candidate bundle.
    pub candidate: Option<CandidateComparison>,
}

/// One locally verified candidate package compared with actual recorded versions.
#[derive(Debug, Serialize)]
pub struct CandidatePackage {
    /// Package identifier from the candidate bundle.
    pub package_id: String,
    /// Distinct recorded versions, which may include inactive side-by-side releases.
    pub observed_versions: Vec<String>,
    /// Exact candidate version, absent for packages omitted by the candidate.
    pub candidate_version: Option<String>,
    /// `add`, `unchanged`, `update`, `downgrade`, `version_differs`, or `not_in_candidate`.
    pub change: String,
}

/// An offline candidate comparison is separate from choosing or applying an upgrade.
#[derive(Debug, Serialize)]
pub struct CandidateComparison {
    /// Fully verified local bundle directory.
    pub bundle: PathBuf,
    /// Candidate product version.
    pub product_version: String,
    /// Package-level differences; no files have changed.
    pub packages: Vec<CandidatePackage>,
    /// Conservative supported installation or migration path.
    pub guidance: Vec<String>,
}

/// Compare a verified local candidate without guessing the remote latest release or upgrading files.
pub fn compare_candidate(report: &mut ProductOverview, bundle: &crate::release::Bundle) {
    use std::collections::{BTreeMap, BTreeSet};
    let mut observed: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for installation in report.components.iter().flat_map(|row| &row.installations) {
        if let Some(version) = &installation.version {
            observed
                .entry(installation.package_id.clone())
                .or_default()
                .insert(version.clone());
        }
    }
    let mut packages = Vec::new();
    for package in &bundle.packages {
        let versions = observed.remove(&package.package_id).unwrap_or_default();
        let change = if versions.is_empty() {
            "add"
        } else if versions.len() == 1 && versions.contains(&package.version) {
            "unchanged"
        } else if versions.len() == 1 {
            match (
                versions
                    .first()
                    .and_then(|version| semver::Version::parse(version).ok()),
                semver::Version::parse(&package.version).ok(),
            ) {
                (Some(old), Some(new)) if new > old => "update",
                (Some(old), Some(new)) if new < old => "downgrade",
                _ => "version_differs",
            }
        } else {
            "version_differs"
        };
        packages.push(CandidatePackage {
            package_id: package.package_id.clone(),
            observed_versions: versions.into_iter().collect(),
            candidate_version: Some(package.version.clone()),
            change: change.into(),
        });
    }
    packages.extend(observed.into_iter().map(|(id, versions)| CandidatePackage {
        package_id: id,
        observed_versions: versions.into_iter().collect(),
        candidate_version: None,
        change: "not_in_candidate".into(),
    }));
    let mut guidance = vec!["Product releases install side by side; review `anolisa install --bundle DIR --dry-run` before explicitly activating a candidate. Omitted packages are not automatically removed.".into()];
    if report
        .components
        .iter()
        .flat_map(|row| &row.installations)
        .any(|item| item.source == "legacy_state")
    {
        guidance.push("Legacy component records need `anolisa migrate plan --help`; `anolisa update --check` applies to the legacy component path, not the new product release database.".into());
    }
    report.candidate = Some(CandidateComparison {
        bundle: bundle.root.clone(),
        product_version: bundle.manifest.product_version.clone(),
        packages,
        guidance,
    });
}

/// Explicit process inputs for offline discovery; callers may supply isolated test roots.
pub struct OverviewRequest<'a> {
    /// Invoking-user XDG roots, resolved before discovery.
    pub environment: &'a Environment,
    /// Requested installation visibility.
    pub visibility: Visibility,
    /// Explicit PATH directories; no shell or remote lookup is used.
    pub search_path: Vec<PathBuf>,
    /// Product launcher context, when supplied by an installed launcher.
    pub product_prefix: Option<PathBuf>,
    /// Product launcher resource context.
    pub product_resources: Option<PathBuf>,
    /// Legacy layouts whose local state should also remain observable.
    pub legacy_layouts: Vec<FsLayout>,
    /// Whether live, read-only systemd inspection is requested.
    pub check_services: bool,
    /// Optional native package authority; callers supply a local-only query backend.
    pub native_packages: Option<&'a dyn PackageQuery>,
}

/// Collect installed versions and qualified usability without network access or filesystem writes.
pub fn overview(request: &OverviewRequest<'_>, manager: &impl Systemctl) -> ProductOverview {
    let mut warnings = Vec::new();
    let evidence = collect::evidence(request, &mut warnings);
    let components = DEFINITIONS
        .iter()
        .map(|definition| collect::component(definition, &evidence, request, manager))
        .collect();
    ProductOverview {
        schema_version: 1,
        uid: nix::unistd::geteuid().as_raw(),
        components,
        warnings,
        latest: "not_checked".into(),
        candidate: None,
    }
}

struct Definition {
    name: &'static str,
    backend_ids: &'static [&'static str],
    resource_ids: &'static [&'static str],
    provider_ids: &'static [&'static str],
    plugin_ids: &'static [&'static str],
    commands: &'static [&'static str],
    unit: Option<&'static str>,
    privilege: Option<&'static str>,
}

const DEFINITIONS: &[Definition] = &[
    Definition {
        name: "anolisa",
        backend_ids: &["anolisa", "anolisa-cli"],
        resource_ids: &[],
        provider_ids: &[],
        plugin_ids: &[],
        commands: &["anolisa"],
        unit: None,
        privilege: None,
    },
    Definition {
        name: "aw",
        backend_ids: &["aw", "aw-core"],
        resource_ids: &[],
        provider_ids: &[],
        plugin_ids: &[],
        commands: &["aw"],
        unit: None,
        privilege: None,
    },
    Definition {
        name: "cosh",
        backend_ids: &["cosh", "cosh-ng", "cosh-ng-core"],
        resource_ids: &["cosh-ng-skills"],
        provider_ids: &[],
        plugin_ids: &[],
        commands: &["cosh"],
        unit: None,
        privilege: None,
    },
    Definition {
        name: "sec-core",
        backend_ids: &["sec-core", "agent-sec-core", "agent-sec-cli"],
        resource_ids: &[
            "agent-sec-sandbox",
            "agent-sec-python-runtime",
            "agent-sec-skills",
        ],
        provider_ids: &["aw-provider-sec-core"],
        plugin_ids: &[
            "agent-sec-openclaw-hook",
            "agent-sec-hermes-hook",
            "agent-sec-codex-hook",
            "agent-sec-qoder-hook",
            "agent-sec-qwen-code-hook",
            "agent-sec-cosh-hook",
        ],
        commands: &["agent-sec-cli"],
        unit: Some("agent-sec-core.service"),
        privilege: Some(
            "Backend service installation and activation require administrator setup; ordinary users can use its configured CLI/socket.",
        ),
    },
    Definition {
        name: "agentsight",
        backend_ids: &["agentsight", "agent-sight"],
        resource_ids: &[],
        provider_ids: &[],
        plugin_ids: &[],
        commands: &["agentsight"],
        unit: Some("agentsight.service"),
        privilege: Some(
            "eBPF collection needs root or delegated kernel capabilities; trajectory collection and viewing have separate user modes.",
        ),
    },
    Definition {
        name: "tokenless",
        backend_ids: &["tokenless", "tokenless-core"],
        resource_ids: &[],
        provider_ids: &["aw-provider-tokenless"],
        plugin_ids: &["tokenless-plugins"],
        commands: &["tokenless"],
        unit: None,
        privilege: None,
    },
    Definition {
        name: "memory",
        backend_ids: &["memory", "agent-memory", "agent-memory-core"],
        resource_ids: &[],
        provider_ids: &[],
        plugin_ids: &["agent-memory-plugins"],
        commands: &["agent-memory", "memory"],
        unit: None,
        privilege: None,
    },
    Definition {
        name: "skillfs",
        backend_ids: &["skillfs", "skillfs-core"],
        resource_ids: &["skillfs-skills"],
        provider_ids: &[],
        plugin_ids: &[],
        commands: &["skillfs"],
        unit: None,
        privilege: Some(
            "Mounting needs a usable /dev/fuse and host FUSE permissions; installed files alone do not grant them.",
        ),
    },
    Definition {
        name: "ws-ckpt",
        backend_ids: &["ws-ckpt", "ws-ckpt-core"],
        resource_ids: &["ws-ckpt-skills"],
        provider_ids: &[],
        plugin_ids: &["ws-ckpt-plugins"],
        commands: &["ws-ckpt"],
        unit: Some("ws-ckpt.service"),
        privilege: Some(
            "Checkpoint backend service requires administrator setup; ordinary users use the CLI through its configured service.",
        ),
    },
    Definition {
        name: "os-skills",
        backend_ids: &["os-skills"],
        resource_ids: &[],
        provider_ids: &[],
        plugin_ids: &[],
        commands: &[],
        unit: None,
        privilege: None,
    },
    Definition {
        name: "ktuner",
        backend_ids: &["ktuner"],
        resource_ids: &[],
        provider_ids: &[],
        plugin_ids: &[],
        commands: &["ktuner"],
        unit: None,
        privilege: Some(
            "Applying host kernel tuning requires administrator privileges; inspection and recommendations are separate from applying changes.",
        ),
    },
];

impl Definition {
    fn packages(&self) -> impl Iterator<Item = &&'static str> {
        self.backend_ids
            .iter()
            .chain(self.resource_ids)
            .chain(self.provider_ids)
            .chain(self.plugin_ids)
    }
}

fn belongs(definition: &Definition, id: &str) -> bool {
    definition.packages().any(|known| *known == id)
}

fn command(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

fn legacy(store: &StateStore, layout: &FsLayout) -> Vec<InstalledEvidence> {
    store
        .installations
        .iter()
        .filter(|item| item.kind == crate::ObjectKind::Component)
        .map(|item| InstalledEvidence {
            package_id: item.name.clone(),
            version: item.binding.version().map(str::to_owned),
            scope: match item.scope {
                crate::domain::InstallationScope::System => "system",
                _ => "user",
            }
            .into(),
            source: "legacy_state".into(),
            prefix: Some(layout.bin_dir.parent().unwrap_or(&layout.prefix).to_owned()),
            resources: Some(layout.datadir.clone()),
        })
        .collect()
}
