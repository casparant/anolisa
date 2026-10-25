//! Foreground product services use an explicit preset without invoking systemd.

use crate::{
    context::CliContext,
    response::{self, CliError},
};
use anolisa_core::deployment::{self, DeploymentConfig, DeploymentError};
use clap::Args;
use std::path::PathBuf;

/// Explicit foreground service selection for containers and non-systemd hosts.
#[derive(Args)]
pub struct ServeArgs {
    /// Installed product service catalog; contains the shared versioned presets.
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,
    /// Named allowlisted same-UID service selection, for example container.
    #[arg(long, value_name = "PRESET")]
    pub preset: String,
}

/// Preflight or supervise a product group in the current identity.
pub fn handle(args: ServeArgs, ctx: &CliContext) -> Result<(), CliError> {
    let path =
        super::preset::catalog_path(args.config).map_err(|error| error.with_command("serve"))?;
    let config = DeploymentConfig::read(&path).map_err(error)?;
    let plan = deployment::plan_serve(&config, &args.preset).map_err(error)?;
    if ctx.dry_run {
        if ctx.json {
            return response::render_json("serve", &plan);
        }
        println!("Foreground preset {} as UID {}:", plan.preset, plan.uid);
        for service in &plan.services {
            println!("  {}: {} {:?}", service.id, service.program, service.args);
        }
        return Ok(());
    }
    if ctx.json {
        return Err(CliError::InvalidArgument {
            command: "serve".into(),
            reason: "foreground services inherit stdout/stderr; use --json with --dry-run to inspect the plan".into(),
        });
    }
    deployment::serve(&config, &args.preset)
        .map(|_| ())
        .map_err(error)
}

fn error(error: DeploymentError) -> CliError {
    let reason = error.to_string();
    match error {
        DeploymentError::Invalid(_) | DeploymentError::Json(_) => CliError::InvalidArgument {
            command: "serve".into(),
            reason,
        },
        DeploymentError::Interrupted(signal) => CliError::Interrupted {
            command: "serve".into(),
            signal: u8::try_from(signal).unwrap_or(15),
        },
        _ => CliError::Runtime {
            command: "serve".into(),
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
    fn foreground_entrypoint_requires_explicit_selection_and_allows_catalog_discovery() {
        assert!(Cli::try_parse_from(["anolisa", "serve"]).is_err());
        let cli = Cli::try_parse_from([
            "anolisa",
            "serve",
            "--config",
            "/services.json",
            "--preset",
            "container",
            "--json",
            "--dry-run",
        ])
        .unwrap();
        assert!(cli.json && cli.dry_run);
        assert!(
            matches!(cli.command, Commands::Management(ManagementCommands::Serve(ServeArgs { preset, .. })) if preset == "container")
        );
        let cli = Cli::try_parse_from(["anolisa", "serve", "--preset", "container"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Management(ManagementCommands::Serve(ServeArgs { config: None, .. }))
        ));
        assert_eq!(error(DeploymentError::Interrupted(2)).exit_code(), 130);
        assert_eq!(error(DeploymentError::Interrupted(15)).exit_code(), 143);
    }
}
