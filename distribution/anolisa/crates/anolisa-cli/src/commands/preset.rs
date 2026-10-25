//! Native service activation is separate from package installation and defaults.

use crate::{
    context::CliContext,
    response::{self, CliError},
};
use anolisa_core::deployment::{self, DeploymentConfig, DeploymentError, Scope, SystemctlRunner};
use clap::{Args, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Native service deployment operations.
#[derive(Args)]
pub struct PresetArgs {
    /// Review the native plan or explicitly apply it.
    #[command(subcommand)]
    pub command: PresetCommand,
}

/// Explicit native manager scope, never selected from sudo environment variables.
#[derive(Clone, Copy, Default, ValueEnum)]
pub enum PresetScope {
    /// The actual invoking user's manager and state directory.
    #[default]
    User,
    /// The root-managed host manager and system state directory.
    System,
}

impl From<PresetScope> for Scope {
    fn from(value: PresetScope) -> Self {
        match value {
            PresetScope::User => Self::User,
            PresetScope::System => Self::System,
        }
    }
}

/// Shared catalog and selection for read-only and mutating operations.
#[derive(Args)]
pub struct PresetSelection {
    /// Installed product service catalog shared with the container entrypoint.
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,
    /// Product preset name from the shared catalog.
    #[arg(long, value_name = "PRESET")]
    pub preset: String,
    /// Manager scope; user application must run outside sudo as that user.
    #[arg(long, value_enum, default_value = "user")]
    pub scope: PresetScope,
    /// Initialize unrecorded units through administrator-aware systemd preset policy.
    #[arg(long)]
    pub initialize: bool,
}

/// Enable defaults and immediate starts remain separate reviewable actions.
#[derive(Subcommand)]
pub enum PresetCommand {
    /// Inspect current units and list actions without writing or starting services.
    Plan(PresetSelection),
    /// Apply explicitly selected defaults and starts; preserve prior enable choices.
    Apply(PresetSelection),
}

/// Run a native-manager operation using real caller identity and explicit scope.
pub fn handle(args: PresetArgs, ctx: &CliContext) -> Result<(), CliError> {
    let (selection, apply) = match args.command {
        PresetCommand::Plan(selection) => (selection, false),
        PresetCommand::Apply(selection) => (selection, !ctx.dry_run),
    };
    let path = catalog_path(selection.config)?;
    let config = DeploymentConfig::read(&path).map_err(error)?;
    let scope = selection.scope.into();
    let state = state_path(scope)?;
    if apply {
        let report = deployment::apply_preset(
            &config,
            &selection.preset,
            scope,
            &state,
            selection.initialize,
            &SystemctlRunner,
        )
        .map_err(error)?;
        if ctx.json {
            response::render_json("preset apply", &report)
        } else {
            println!(
                "Preset {}: {} actions completed",
                report.plan.preset,
                report.completed.len()
            );
            Ok(())
        }
    } else {
        let plan = deployment::plan_preset(
            &config,
            &selection.preset,
            scope,
            &state,
            selection.initialize,
            &SystemctlRunner,
        )
        .map_err(error)?;
        if ctx.json {
            response::render_json("preset plan", &plan)
        } else {
            println!("Preset {} ({:?}):", plan.preset, plan.scope);
            for action in &plan.actions {
                println!(
                    "  {}: {:?} (enabled={}, active={})",
                    action.unit, action.kind, action.before.enable_state, action.before.active
                );
            }
            Ok(())
        }
    }
}

fn state_path(scope: Scope) -> Result<PathBuf, CliError> {
    if scope == Scope::System {
        return Ok(PathBuf::from("/var/lib/anolisa/deployment/presets.json"));
    }
    let root =
        if let Some(value) = std::env::var_os("XDG_STATE_HOME").filter(|value| !value.is_empty()) {
            PathBuf::from(value)
        } else {
            let home = std::env::var_os("HOME").ok_or_else(|| CliError::InvalidArgument {
                command: "preset".into(),
                reason: "HOME is required for user deployment state".into(),
            })?;
            PathBuf::from(home).join(".local/state")
        };
    if !root.is_absolute() {
        return Err(CliError::InvalidArgument {
            command: "preset".into(),
            reason: "user deployment state root must be absolute".into(),
        });
    }
    Ok(root.join("anolisa/deployment/presets.json"))
}

pub(crate) fn catalog_path(explicit: Option<PathBuf>) -> Result<PathBuf, CliError> {
    resolve_catalog_path(
        explicit,
        std::env::var_os("ANOLISA_RELEASE_RESOURCES").map(PathBuf::from),
        &[
            PathBuf::from("/usr/share/anolisa/distribution/services.json"),
            PathBuf::from("/usr/local/share/anolisa/distribution/services.json"),
        ],
    )
}

fn resolve_catalog_path(
    explicit: Option<PathBuf>,
    release_resources: Option<PathBuf>,
    native_candidates: &[PathBuf],
) -> Result<PathBuf, CliError> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    if let Some(root) = release_resources {
        if !root.is_absolute() {
            return Err(CliError::InvalidArgument {
                command: "preset".into(),
                reason: "ANOLISA_RELEASE_RESOURCES must be an absolute resource root".into(),
            });
        }
        let path = root.join("anolisa/distribution/services.json");
        if !path.is_file() {
            return Err(CliError::InvalidArgument {
                command: "preset".into(),
                reason: format!(
                    "this product release has no service catalog at {}; specify --config for an explicit catalog",
                    path.display()
                ),
            });
        }
        return Ok(path);
    }
    native_candidates.iter().find(|path| path.is_file()).cloned().ok_or_else(|| CliError::InvalidArgument {
        command:"preset".into(), reason:"no installed ANOLISA service catalog found; install the product distribution or specify --config FILE".into(),
    })
}

fn error(error: DeploymentError) -> CliError {
    let reason = error.to_string();
    match error {
        DeploymentError::Invalid(_) | DeploymentError::Json(_) => CliError::InvalidArgument {
            command: "preset".into(),
            reason,
        },
        _ => CliError::Runtime {
            command: "preset".into(),
            reason,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{Cli, Commands, ManagementCommands};
    use clap::Parser;

    #[test]
    fn preset_apply_preserves_existing_defaults_and_has_an_explicit_scope() {
        let cli = Cli::try_parse_from([
            "anolisa",
            "preset",
            "apply",
            "--config",
            "/services.json",
            "--preset",
            "host",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Management(ManagementCommands::Preset(PresetArgs {
                command: PresetCommand::Apply(PresetSelection {
                    scope: PresetScope::User,
                    initialize: false,
                    ..
                })
            }))
        ));
        let cli = Cli::try_parse_from([
            "anolisa",
            "preset",
            "plan",
            "--config",
            "/services.json",
            "--preset",
            "host",
            "--scope",
            "system",
            "--initialize",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Management(ManagementCommands::Preset(PresetArgs {
                command: PresetCommand::Plan(PresetSelection {
                    scope: PresetScope::System,
                    initialize: true,
                    ..
                })
            }))
        ));
    }

    #[test]
    fn catalog_discovery_keeps_the_selected_release_and_honors_explicit_overrides() {
        let temp = tempfile::tempdir().unwrap();
        let release = temp.path().join("release");
        let catalog = release.join("anolisa/distribution/services.json");
        std::fs::create_dir_all(catalog.parent().unwrap()).unwrap();
        std::fs::write(&catalog, b"{}").unwrap();
        let native = temp.path().join("native.json");
        std::fs::write(&native, b"{}").unwrap();
        let candidates = [native.clone()];
        assert_eq!(
            resolve_catalog_path(None, Some(release), &candidates).unwrap(),
            catalog
        );
        assert_eq!(
            resolve_catalog_path(None, None, &candidates).unwrap(),
            native
        );
        assert!(
            resolve_catalog_path(None, Some(temp.path().join("missing-release")), &candidates)
                .is_err()
        );
        let explicit = temp.path().join("explicit.json");
        assert_eq!(
            resolve_catalog_path(
                Some(explicit.clone()),
                Some(PathBuf::from("relative-invalid")),
                &candidates
            )
            .unwrap(),
            explicit
        );
    }
}
