//! First-run guidance creates only explicit invoking-user configuration and read-only service previews.

use super::overview::{self, OverviewArgs};
use crate::{
    context::CliContext,
    response::{self, CliError},
};
use anolisa_core::{
    deployment::{self, DeploymentConfig, PresetPlan, SystemctlRunner},
    onboarding::{self, SetupRequest},
    product_status::ProductOverview,
    release::Environment,
};
use clap::{Args, ValueEnum};
use serde::Serialize;
use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
};

/// Service-manager scope is explicit and independent of installation visibility.
#[derive(Clone, Copy, Default, ValueEnum)]
pub enum ServiceScope {
    /// Preview the invoking user's service manager.
    #[default]
    User,
    /// Preview the system manager without elevating privileges or applying actions.
    System,
}

/// Human setup stays read-only until a configuration destination is explicitly selected.
#[derive(Args)]
pub struct SetupArgs {
    #[command(flatten)]
    pub overview: OverviewArgs,
    /// Always show unattended guidance; do not prompt even when stdin is a terminal.
    #[arg(long)]
    pub non_interactive: bool,
    /// Create this new invoking-user AW configuration; preserve every existing file.
    #[arg(long, value_name = "ABS_FILE", requires_all = ["agent", "agent_bin"])]
    pub write_config: Option<PathBuf>,
    /// Native adapter from the installed `aw agent list --json` catalog.
    #[arg(long, requires = "write_config")]
    pub agent: Option<String>,
    /// Absolute native Agent executable; SDK adapters may also require --agent-arg values.
    #[arg(long, value_name = "ABS_EXECUTABLE", requires = "write_config")]
    pub agent_bin: Option<PathBuf>,
    /// Append one literal native argument, without shell parsing.
    #[arg(
        long = "agent-arg",
        allow_hyphen_values = true,
        requires = "write_config"
    )]
    pub agent_arguments: Vec<String>,
    /// Copy this explicit Provider into a separate disabled definitions file.
    #[arg(long = "provider", requires_all = ["write_config", "provider_template"])]
    pub providers: Vec<String>,
    /// Disabled Provider template published by the installed product or explicitly supplied.
    #[arg(long, value_name = "ABS_JSON", requires = "providers")]
    pub provider_template: Option<PathBuf>,
    /// Prepare a real native service preset plan; never apply or start it.
    #[arg(long)]
    pub preset: Option<String>,
    /// Installed service catalog used for --preset; omitted catalogs use installed product resources.
    #[arg(long, value_name = "FILE", requires = "preset")]
    pub services_config: Option<PathBuf>,
    /// Service manager whose existing units should be previewed.
    #[arg(long, value_enum, default_value = "user")]
    pub service_scope: ServiceScope,
    /// Confirm explicitly requested user configuration creation for unattended use.
    #[arg(short = 'y', long, requires = "write_config")]
    pub yes: bool,
}

#[derive(Serialize)]
struct SetupOutput {
    overview: ProductOverview,
    configuration: Option<onboarding::SetupReport>,
    service_plan: Option<PresetPlan>,
    next_actions: Vec<String>,
}

/// Guide a human or print an unattended summary without changing installation or service defaults.
pub fn handle(mut args: SetupArgs, ctx: &CliContext) -> Result<(), CliError> {
    let (environment, report) =
        overview::collect(&args.overview, ctx).map_err(|error| error.with_command("setup"))?;
    if !ctx.json && !ctx.quiet {
        overview::render(&report);
    }
    let service_plan = preset(&args, &environment, &report)?;
    if args.write_config.is_none()
        && !args.non_interactive
        && !ctx.json
        && !ctx.quiet
        && !ctx.dry_run
        && io::stdin().is_terminal()
        && io::stdout().is_terminal()
    {
        wizard(&mut args, &environment, &report)?;
    }
    let configuration = if let Some(config) = &args.write_config {
        if !ctx.dry_run && !args.yes {
            return Err(argument(
                "user configuration creation requires --yes; use --dry-run to inspect it",
            ));
        }
        let aw = report
            .components
            .iter()
            .find(|row| row.component == "aw")
            .and_then(|row| row.command.as_ref())
            .ok_or_else(|| {
                argument(
                    "AW executable is unavailable; install the AW runtime before configuring it",
                )
            })?;
        let adapters = onboarding::agent_catalog(aw).map_err(setup_error)?;
        let mut argv = vec![
            args.agent_bin
                .as_ref()
                .ok_or_else(|| argument("select --agent-bin"))?
                .display()
                .to_string(),
        ];
        argv.extend(args.agent_arguments.clone());
        Some(
            onboarding::setup(
                &SetupRequest {
                    environment: &environment,
                    config: config.clone(),
                    adapter: args
                        .agent
                        .clone()
                        .ok_or_else(|| argument("select --agent"))?,
                    supported_adapters: adapters,
                    agent_argv: argv,
                    provider_template: args.provider_template.clone(),
                    providers: args.providers.clone(),
                },
                ctx.dry_run,
            )
            .map_err(setup_error)?,
        )
    } else {
        None
    };
    let next_actions = vec![
        "Use `anolisa setup --write-config ABS_FILE --agent NAME --agent-bin ABS_EXECUTABLE --yes` for unattended user AW configuration.".into(),
        "Use `anolisa overview --check-services` to inspect service states; privileged backends require explicit administrator service setup.".into(),
        "For product upgrades, compare `anolisa overview --candidate-bundle DIR`; for existing component installations, review `anolisa migrate plan --help`.".into(),
    ];
    let succeeded = configuration
        .as_ref()
        .is_none_or(|report| report.error.is_none());
    let output = SetupOutput {
        overview: report,
        configuration,
        service_plan,
        next_actions,
    };
    if ctx.json {
        response::render_json_with_status("setup", succeeded, &output)?;
    } else if !ctx.quiet {
        if let Some(config) = &output.configuration {
            println!(
                "User configuration {}: {}",
                config.plan.config.display(),
                if config.dry_run {
                    "planned"
                } else if config.error.is_none() {
                    "created"
                } else {
                    "incomplete"
                }
            );
            for path in &config.created {
                println!("  Created {}", path.display());
            }
            if let Some(error) = &config.error {
                eprintln!("Setup stopped: {error}");
            }
            for action in &config.plan.next_actions {
                println!("  {action}");
            }
        }
        if let Some(plan) = &output.service_plan {
            println!(
                "Service preset {} is a preview; no service was changed:",
                plan.preset
            );
            for action in &plan.actions {
                println!("  {}: {:?}", action.unit, action.kind);
            }
        }
        for action in &output.next_actions {
            println!("{action}");
        }
    }
    if succeeded {
        Ok(())
    } else {
        Err(CliError::BatchPartial {
            command: "setup".into(),
        })
    }
}

fn wizard(
    args: &mut SetupArgs,
    environment: &Environment,
    report: &ProductOverview,
) -> Result<(), CliError> {
    let Some(aw) = report
        .components
        .iter()
        .find(|row| row.component == "aw")
        .and_then(|row| row.command.as_ref())
    else {
        println!("AW is not available; install the product suite before configuring an Agent.");
        return Ok(());
    };
    let adapters = match onboarding::agent_catalog(aw) {
        Ok(adapters) => adapters,
        Err(error) => {
            println!("Guided Agent configuration is unavailable: {error}");
            return Ok(());
        }
    };
    if !matches!(
        prompt("Create a new user AW configuration? [y/N] ")?.as_str(),
        "y" | "Y" | "yes"
    ) {
        return Ok(());
    }
    println!("Installed AW adapters: {}", adapters.join(", "));
    let adapter = prompt("Adapter name: ")?;
    if !adapters.contains(&adapter) {
        return Err(argument(
            "select a name from the installed AW adapter catalog",
        ));
    }
    let executable = prompt(
        "Absolute Agent executable (use explicit --agent-arg flags for SDK launch arguments): ",
    )?;
    if executable.is_empty() {
        return Ok(());
    }
    println!("Add any required native or SDK startup arguments individually; Enter finishes.");
    loop {
        let value = prompt("Next literal argument: ")?;
        if value.is_empty() {
            break;
        }
        if args.agent_arguments.len() >= 127 {
            return Err(argument("Agent argv exceeds 128 entries"));
        }
        args.agent_arguments.push(value);
    }
    let default = environment.config.join("anolisa/aw.yaml");
    let destination = prompt(&format!("New configuration file [{}]: ", default.display()))?;
    args.write_config = Some(if destination.is_empty() {
        default
    } else {
        PathBuf::from(destination)
    });
    args.agent = Some(adapter);
    args.agent_bin = Some(PathBuf::from(executable));
    args.yes = true;
    Ok(())
}

fn prompt(message: &str) -> Result<String, CliError> {
    print!("{message}");
    io::stdout()
        .flush()
        .map_err(|error| overview::failure("setup", error))?;
    let mut text = String::new();
    io::stdin()
        .read_line(&mut text)
        .map_err(|error| overview::failure("setup", error))?;
    Ok(text.trim().to_owned())
}

fn preset(
    args: &SetupArgs,
    environment: &Environment,
    report: &ProductOverview,
) -> Result<Option<PresetPlan>, CliError> {
    let Some(name) = &args.preset else {
        return Ok(None);
    };
    let path = args
        .services_config
        .clone()
        .or_else(|| {
            report
                .components
                .iter()
                .flat_map(|row| &row.installations)
                .filter_map(|item| item.resources.as_ref())
                .map(|root| root.join("anolisa/distribution/services.json"))
                .find(|path| path.is_file())
        })
        .ok_or_else(|| {
            argument("no installed service catalog was observed; select --services-config FILE")
        })?;
    let config =
        DeploymentConfig::read(&path).map_err(|error| overview::failure("setup", error))?;
    let (scope, state) = match args.service_scope {
        ServiceScope::User => (
            deployment::Scope::User,
            environment.state.join("anolisa/deployment/presets.json"),
        ),
        ServiceScope::System => (
            deployment::Scope::System,
            PathBuf::from("/var/lib/anolisa/deployment/presets.json"),
        ),
    };
    deployment::plan_preset(&config, name, scope, &state, false, &SystemctlRunner)
        .map(Some)
        .map_err(|error| overview::failure("setup", error))
}

fn argument(reason: impl Into<String>) -> CliError {
    CliError::InvalidArgument {
        command: "setup".into(),
        reason: reason.into(),
    }
}
fn setup_error(error: onboarding::SetupError) -> CliError {
    match error {
        onboarding::SetupError::Invalid(reason) => argument(reason),
        other => overview::failure("setup", other),
    }
}

#[cfg(test)]
mod tests {
    use crate::commands::{Cli, Commands, ManagementCommands};
    use clap::Parser;

    #[test]
    fn setup_is_safe_when_piped_and_explicit_configuration_requires_complete_choices() {
        let cli =
            Cli::try_parse_from(["anolisa", "setup", "--non-interactive", "--scope", "system"])
                .unwrap();
        assert!(
            matches!(cli.command, Commands::Management(ManagementCommands::Setup(args)) if args.write_config.is_none() && args.non_interactive)
        );
        assert!(Cli::try_parse_from(["anolisa", "setup", "--write-config", "/aw.yaml"]).is_err());
        assert!(Cli::try_parse_from(["anolisa", "setup", "--agent", "qoder"]).is_err());
        let cli = Cli::try_parse_from([
            "anolisa",
            "setup",
            "--write-config",
            "/aw.yaml",
            "--agent",
            "qoder",
            "--agent-bin",
            "/qoder",
            "--yes",
            "--dry-run",
        ])
        .unwrap();
        assert!(cli.dry_run);
    }
}
