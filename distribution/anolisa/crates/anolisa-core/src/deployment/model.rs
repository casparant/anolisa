//! Versioned service descriptions shared by product presets and container plans.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Failures retain the service or operation responsible for partial deployment.
#[derive(Debug, thiserror::Error)]
pub enum DeploymentError {
    /// A description or requested scope violates the deployment contract.
    #[error("invalid deployment: {0}")]
    Invalid(String),
    /// A description could not be decoded without discarding unknown fields.
    #[error("invalid deployment JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// An operating-system operation failed before its result was established.
    #[error("{operation}: {source}")]
    Io {
        /// Context identifying the affected path or service.
        operation: String,
        /// Original operating-system error.
        #[source]
        source: std::io::Error,
    },
    /// A systemd command returned a nonzero status.
    #[error("systemctl {operation} failed: {detail}")]
    Systemd {
        /// Exact operation attempted, without shell interpretation.
        operation: String,
        /// Exit status and captured diagnostics.
        detail: String,
    },
    /// A required service exited or failed readiness; siblings have been stopped.
    #[error("service {service} failed: {detail}")]
    Service {
        /// Stable service identity from the catalog.
        service: String,
        /// Observed failure, including the exit status when available.
        detail: String,
    },
    /// TERM or INT requested shutdown after every direct child was reaped.
    #[error("service group interrupted by signal {0}")]
    Interrupted(i32),
}

pub(super) fn io(operation: impl Into<String>, source: std::io::Error) -> DeploymentError {
    DeploymentError::Io {
        operation: operation.into(),
        source,
    }
}

/// A service belongs to exactly one systemd manager scope.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    /// The invoking user's manager; identity is never inferred from sudo variables.
    User,
    /// The host manager, requiring an explicit root invocation when applying.
    System,
}

/// Descriptions never ask the supervisor to change identity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunAs {
    /// Foreground processes inherit the invoking user's UID and GID.
    SameUser,
    /// Privileged backend managed separately by the system manager.
    Root,
}

/// The communication contract determines whether a process can be shared.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Communication {
    /// A Unix-domain socket with independently configured authorization.
    Socket,
    /// An explicitly configured HTTP endpoint.
    Http,
    /// A process without a public IPC endpoint.
    None,
    /// An Agent-owned MCP stdio session, excluded from shared supervision.
    McpStdio,
}

/// Bounded readiness checks never execute a shell or arbitrary health command.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Readiness {
    /// Require the process to survive a startup interval.
    Process {
        /// Minimum observed lifetime before dependents may start.
        delay_ms: u64,
        /// Startup deadline including the survival interval.
        timeout_ms: u64,
    },
    /// Require a connectable local Unix socket; stale files alone are insufficient.
    UnixSocket {
        /// Absolute socket path, optionally using supported XDG variables.
        path: PathBuf,
        /// Maximum startup time.
        timeout_ms: u64,
    },
    /// Require a connectable loopback TCP endpoint.
    Tcp {
        /// Explicit loopback address and port, without DNS lookup.
        address: std::net::SocketAddr,
        /// Maximum startup time.
        timeout_ms: u64,
    },
}

impl Readiness {
    pub(super) fn timeout(&self) -> u64 {
        match self {
            Self::Process { timeout_ms, .. }
            | Self::UnixSocket { timeout_ms, .. }
            | Self::Tcp { timeout_ms, .. } => *timeout_ms,
        }
    }
}

/// Component-owned execution facts; product presets separately choose instances.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Service {
    /// Stable identity, retained across package path changes.
    pub id: String,
    /// Actual backend package owning the executable and service protocol.
    pub package: String,
    /// Absolute executable or command resolved within the selected release,
    /// falling back to PATH only outside a versioned product installation.
    pub program: String,
    /// Literal argv entries; supported variables expand without shell evaluation.
    pub args: Vec<String>,
    /// Native service-manager scope.
    pub scope: Scope,
    /// Required identity; the supervisor never elevates or switches user.
    pub run_as: RunAs,
    /// IPC ownership contract, including exclusion of MCP stdio sessions.
    pub communication: Communication,
    /// Explicit allowlist admission for the same-UID container supervisor.
    pub container_allowed: bool,
    /// Native unit name, absent when no corresponding unit is delivered.
    pub unit: Option<String>,
    /// Required service IDs; the preset must explicitly select every dependency.
    pub depends_on: Vec<String>,
    /// Pre-existing configuration, data, or device paths required at startup.
    pub required_paths: Vec<PathBuf>,
    /// Startup condition checked before any dependent process begins.
    pub readiness: Readiness,
    /// TERM grace period before killing and reaping the entire process group.
    pub stop_timeout_ms: u64,
}

/// One explicit deployment decision; enable and start have different effects.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PresetService {
    /// Catalog service selected by this preset.
    pub id: String,
    /// Desired first-install enable default, applied through systemd preset policy.
    pub enable: bool,
    /// Explicit request to start now, separate from persistent enable defaults.
    pub start: bool,
}

/// A named product deployment selection, independent of installed package sets.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    /// Explicit decisions; an empty preset intentionally starts nothing.
    pub services: Vec<PresetService>,
}

/// Shared product service catalog, shipped as one source across distribution formats.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentConfig {
    /// Required format gate; older installers must reject unknown schemas.
    pub schema_version: u32,
    /// Component execution descriptions, never automatically activated on install.
    pub services: Vec<Service>,
    /// Product selections that refer to catalog IDs without duplicating descriptions.
    pub presets: BTreeMap<String, Preset>,
}

impl DeploymentConfig {
    /// Load and validate the complete catalog before planning any operation.
    ///
    /// # Errors
    /// Rejects unknown fields, unsupported versions, invalid identities and references.
    pub fn read(path: &Path) -> Result<Self, DeploymentError> {
        let data = std::fs::read(path).map_err(|e| io(format!("read {}", path.display()), e))?;
        Self::parse(&data)
    }

    /// Decode the strict versioned wire format without filesystem mutation.
    ///
    /// # Errors
    /// Rejects unsupported or inconsistent service and preset descriptions.
    pub fn parse(data: &[u8]) -> Result<Self, DeploymentError> {
        let config: Self = serde_json::from_slice(data)?;
        config.validate()?;
        Ok(config)
    }

    /// Verify version, IDs, privilege boundaries and dependency consistency.
    ///
    /// # Errors
    /// Rejects duplicate identities, cycles, unsafe unit names or invalid timeouts.
    pub fn validate(&self) -> Result<(), DeploymentError> {
        if self.schema_version != 1 {
            return Err(DeploymentError::Invalid(format!(
                "unsupported service schema {}",
                self.schema_version
            )));
        }
        let mut ids = BTreeSet::new();
        let mut units = BTreeSet::new();
        for service in &self.services {
            if !identifier(&service.id) || !identifier(&service.package) || !ids.insert(&service.id)
            {
                return Err(DeploymentError::Invalid(format!(
                    "invalid or duplicate service {}",
                    service.id
                )));
            }
            if service.program.is_empty()
                || (service.program.contains('/') && !Path::new(&service.program).is_absolute())
                || service.program.contains('\0')
                || service.args.iter().any(|arg| arg.contains('\0'))
            {
                return Err(DeploymentError::Invalid(format!(
                    "invalid executable or argv for {}",
                    service.id
                )));
            }
            if service.run_as == RunAs::Root && service.scope != Scope::System {
                return Err(DeploymentError::Invalid(format!(
                    "root service {} must use system scope",
                    service.id
                )));
            }
            if service.container_allowed
                && (service.scope != Scope::User
                    || service.run_as != RunAs::SameUser
                    || service.communication == Communication::McpStdio)
            {
                return Err(DeploymentError::Invalid(format!(
                    "service {} cannot enter the container allowlist",
                    service.id
                )));
            }
            if let Some(unit) = &service.unit
                && (!valid_unit(unit) || !units.insert((service.scope as u8, unit)))
            {
                return Err(DeploymentError::Invalid(format!(
                    "invalid or duplicate unit {unit}"
                )));
            }
            if !(1..=300_000).contains(&service.stop_timeout_ms)
                || !(1..=300_000).contains(&service.readiness.timeout())
            {
                return Err(DeploymentError::Invalid(format!(
                    "invalid bounded timeout for {}",
                    service.id
                )));
            }
            match &service.readiness {
                Readiness::Process {
                    delay_ms,
                    timeout_ms,
                } if delay_ms > timeout_ms => {
                    return Err(DeploymentError::Invalid(format!(
                        "startup delay exceeds deadline for {}",
                        service.id
                    )));
                }
                Readiness::Tcp { address, .. }
                    if !address.ip().is_loopback() || address.port() == 0 =>
                {
                    return Err(DeploymentError::Invalid(format!(
                        "readiness endpoint for {} must be loopback",
                        service.id
                    )));
                }
                _ => {}
            }
        }
        for service in &self.services {
            let mut dependencies = BTreeSet::new();
            for dependency in &service.depends_on {
                if !ids.contains(dependency) || !dependencies.insert(dependency) {
                    return Err(DeploymentError::Invalid(format!(
                        "invalid dependency {dependency} of {}",
                        service.id
                    )));
                }
                if self.service(dependency)?.scope != service.scope {
                    return Err(DeploymentError::Invalid(format!(
                        "service {} dependency {dependency} crosses manager scopes; describe it as a separately managed external backend",
                        service.id
                    )));
                }
            }
        }
        self.ordered(self.services.iter().map(|s| s.id.as_str()))?;
        for (name, preset) in &self.presets {
            if !identifier(name) {
                return Err(DeploymentError::Invalid(format!(
                    "invalid preset name {name}"
                )));
            }
            let mut selected = BTreeSet::new();
            for item in &preset.services {
                if !ids.contains(&item.id) || !selected.insert(&item.id) {
                    return Err(DeploymentError::Invalid(format!(
                        "invalid or duplicate service {} in {name}",
                        item.id
                    )));
                }
            }
        }
        Ok(())
    }

    pub(super) fn selected(&self, preset: &str) -> Result<&Preset, DeploymentError> {
        self.presets
            .get(preset)
            .ok_or_else(|| DeploymentError::Invalid(format!("unknown preset {preset}")))
    }

    pub(super) fn service(&self, id: &str) -> Result<&Service, DeploymentError> {
        self.services
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| DeploymentError::Invalid(format!("unknown service {id}")))
    }

    pub(super) fn ordered<'a>(
        &'a self,
        selected: impl IntoIterator<Item = &'a str>,
    ) -> Result<Vec<&'a Service>, DeploymentError> {
        fn visit<'a>(
            config: &'a DeploymentConfig,
            id: &'a str,
            selected: &BTreeSet<&str>,
            visiting: &mut BTreeSet<&'a str>,
            done: &mut BTreeSet<&'a str>,
            result: &mut Vec<&'a Service>,
        ) -> Result<(), DeploymentError> {
            if done.contains(id) {
                return Ok(());
            }
            if !visiting.insert(id) {
                return Err(DeploymentError::Invalid(format!(
                    "service dependency cycle at {id}"
                )));
            }
            let service = config.service(id)?;
            for dependency in &service.depends_on {
                if !selected.contains(dependency.as_str()) {
                    return Err(DeploymentError::Invalid(format!(
                        "service {id} requires explicit selection of {dependency}"
                    )));
                }
                visit(config, dependency, selected, visiting, done, result)?;
            }
            visiting.remove(id);
            done.insert(id);
            result.push(service);
            Ok(())
        }
        let selected: BTreeSet<_> = selected.into_iter().collect();
        let mut visiting = BTreeSet::new();
        let mut done = BTreeSet::new();
        let mut result = Vec::new();
        for id in &selected {
            visit(self, id, &selected, &mut visiting, &mut done, &mut result)?;
        }
        Ok(result)
    }
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(&c))
}

fn valid_unit(value: &str) -> bool {
    value.ends_with(".service")
        && !value.starts_with('-')
        && !value.contains("@.")
        && value.len() <= 200
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._@:-".contains(&c))
}

pub(super) fn expand(value: &str) -> Result<String, DeploymentError> {
    let mut expanded = value.to_owned();
    for name in [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_STATE_HOME",
        "XDG_DATA_HOME",
        "XDG_RUNTIME_DIR",
    ] {
        let marker = format!("${{{name}}}");
        if !expanded.contains(&marker) {
            continue;
        }
        let home = || {
            std::env::var("HOME")
                .map_err(|_| DeploymentError::Invalid("HOME is required for XDG expansion".into()))
        };
        let actual = match std::env::var(name) {
            Ok(value) if !value.is_empty() => value,
            _ => match name {
                "XDG_CONFIG_HOME" => format!("{}/.config", home()?),
                "XDG_STATE_HOME" => format!("{}/.local/state", home()?),
                "XDG_DATA_HOME" => format!("{}/.local/share", home()?),
                _ => {
                    return Err(DeploymentError::Invalid(format!(
                        "{name} is required by the selected service"
                    )));
                }
            },
        };
        if !Path::new(&actual).is_absolute() {
            return Err(DeploymentError::Invalid(format!("{name} must be absolute")));
        }
        expanded = expanded.replace(&marker, &actual);
    }
    if expanded.contains("${") {
        return Err(DeploymentError::Invalid(format!(
            "unsupported variable in {value}"
        )));
    }
    Ok(expanded)
}
