//! Offline product-bundle commands keep explicit release scope separate from legacy layout flags.

use crate::{
    context::CliContext,
    response::{self, CliError},
};
use anolisa_core::release::{self, Bundle, Environment, ReleaseLayout, Scope};
use clap::{Args, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Offline product release operations.
#[derive(Args)]
pub struct ReleaseArgs {
    /// Product operation, independent of component installation.
    #[command(subcommand)]
    pub command: ReleaseCommand,
}

/// Explicit release scope, without UID-based inference.
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum ReleaseScope {
    /// Install for the invoking user.
    #[default]
    User,
    /// Install the root-managed independent suite.
    System,
}

impl From<ReleaseScope> for Scope {
    fn from(value: ReleaseScope) -> Self {
        match value {
            ReleaseScope::User => Self::User,
            ReleaseScope::System => Self::System,
        }
    }
}

/// The installed product and its verified unpacked input.
#[derive(Subcommand)]
pub enum ReleaseCommand {
    /// Remove an inactive immutable release; switch the current release before cleanup.
    Uninstall {
        /// Original unpacked bundle for the installed release.
        #[arg(long, value_name = "ABS_DIRECTORY")]
        bundle: PathBuf,
        /// Explicit product scope; system cleanup requires root.
        #[arg(long, value_enum, default_value = "user")]
        scope: ReleaseScope,
        /// The profile used by the installation being removed.
        #[arg(long)]
        profile: Option<String>,
        /// The component selection used by that installation.
        #[arg(long = "component", conflicts_with = "profile")]
        components: Vec<String>,
        /// Confirm removal of recorded package files; no effect on conflict checks.
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// List raw product package records; system records require administrator access.
    List {
        /// Explicit database scope, independent of effective UID.
        #[arg(long, value_enum, default_value = "user")]
        scope: ReleaseScope,
    },
    /// Check the selected product's installed files without repairing them.
    Doctor {
        /// Original unpacked bundle for this immutable product selection.
        #[arg(long, value_name = "ABS_DIRECTORY")]
        bundle: PathBuf,
        /// Explicit product scope.
        #[arg(long, value_enum, default_value = "user")]
        scope: ReleaseScope,
        /// The profile used by this installation.
        #[arg(long)]
        profile: Option<String>,
        /// The component selection used by this installation.
        #[arg(long = "component", conflicts_with = "profile")]
        components: Vec<String>,
    },
    /// Recover interrupted ANOLISA raw package operations without activating a release.
    Recover {
        /// Explicit package scope; system recovery requires root.
        #[arg(long, value_enum, default_value = "user")]
        scope: ReleaseScope,
    },
    /// Install or resume one immutable unpacked release; does not enable Providers or services.
    Install {
        /// Absolute symlink-free directory containing release.json and packages/.
        #[arg(long, value_name = "ABS_DIRECTORY")]
        bundle: PathBuf,
        /// Explicit package and product scope; defaults to user even under sudo.
        #[arg(long, value_enum, default_value = "user")]
        scope: ReleaseScope,
        /// Product package combination; defaults to the bundle's scope-specific profile.
        #[arg(long)]
        profile: Option<String>,
        /// Restrict the product to these components plus bootstrap and dependencies.
        #[arg(long = "component", conflicts_with = "profile")]
        components: Vec<String>,
    },
    /// Verify all release descriptors and payloads without writing or running AW.
    Inspect {
        /// Absolute unpacked release directory.
        #[arg(long, value_name = "ABS_DIRECTORY")]
        bundle: PathBuf,
    },
    /// Review or explicitly publish verified system unit templates without starting services.
    Services {
        /// Original unpacked bundle for the successfully installed product selection.
        #[arg(long, value_name = "ABS_DIRECTORY")]
        bundle: PathBuf,
        /// Native system units require explicit system scope and root.
        #[arg(long, value_enum, default_value = "system")]
        scope: ReleaseScope,
        /// The profile used to install the product.
        #[arg(long)]
        profile: Option<String>,
        /// The component selection used to install the product.
        #[arg(long = "component", conflicts_with = "profile")]
        components: Vec<String>,
        /// Component service identity; dependent units are included.
        #[arg(long = "service", required = true)]
        services: Vec<String>,
        /// Publish the reviewed templates and reload the manager; never enable or start.
        #[arg(long)]
        apply: bool,
    },
    /// Read this release's durable product progress without changing it.
    Status {
        /// Absolute unpacked release directory identifying the immutable release.
        #[arg(long, value_name = "ABS_DIRECTORY")]
        bundle: PathBuf,
        /// Explicit product scope.
        #[arg(long, value_enum, default_value = "user")]
        scope: ReleaseScope,
        /// The profile used by the installation being inspected.
        #[arg(long)]
        profile: Option<String>,
        /// The component selection used by that installation.
        #[arg(long = "component", conflicts_with = "profile")]
        components: Vec<String>,
    },
}

/// Run an offline product operation using a separately resolved release layout.
pub fn handle(args: ReleaseArgs, ctx: &CliContext) -> Result<(), CliError> {
    match args.command {
        ReleaseCommand::Uninstall {
            bundle,
            scope,
            profile,
            components,
            yes,
        } => {
            if !ctx.dry_run && !yes {
                return Err(error(release::Error::Invalid(
                    "release uninstall requires --yes; review with --dry-run first".into(),
                )));
            }
            let bundle = Bundle::read(&bundle)
                .and_then(|bundle| bundle.select(scope.into(), profile.as_deref(), &components))
                .map_err(error)?;
            let layout = layout(&bundle, scope.into())?;
            let report = release::remove_release(&bundle, &layout, ctx.dry_run).map_err(error)?;
            let succeeded = report.error.is_none() && report.conflicts.is_empty();
            if ctx.json {
                response::render_json_with_status("release uninstall", succeeded, &report)?;
            } else {
                println!("Release {}: {}", report.plan.release_id, report.phase);
                for warning in &report.warnings {
                    eprintln!("warning: {warning}");
                }
                if let Some(error) = &report.error {
                    eprintln!("error: {error}");
                }
            }
            if succeeded {
                Ok(())
            } else {
                Err(CliError::BatchPartial {
                    command: "release uninstall".into(),
                })
            }
        }
        ReleaseCommand::List { scope } => {
            let environment = Environment::detect().map_err(error)?;
            let inventory = release::list_packages(scope.into(), &environment).map_err(error)?;
            if ctx.json {
                response::render_json("release list", &inventory)
            } else {
                for installation in inventory["installations"].as_array().into_iter().flatten() {
                    for package in installation["packages"].as_array().into_iter().flatten() {
                        println!(
                            "{} {} ({})",
                            package["package_id"].as_str().unwrap_or("?"),
                            package["version"].as_str().unwrap_or("?"),
                            installation["prefix"].as_str().unwrap_or("?")
                        );
                    }
                }
                Ok(())
            }
        }
        ReleaseCommand::Doctor {
            bundle,
            scope,
            profile,
            components,
        } => {
            let bundle = Bundle::read(&bundle)
                .and_then(|bundle| bundle.select(scope.into(), profile.as_deref(), &components))
                .map_err(error)?;
            let layout = layout(&bundle, scope.into())?;
            let report = release::verify_packages(&bundle, &layout).map_err(error)?;
            if ctx.json {
                response::render_json("release doctor", report)
            } else {
                println!(
                    "Installed product files verified: {}",
                    layout.prefix.display()
                );
                Ok(())
            }
        }
        ReleaseCommand::Recover { scope } => {
            if ctx.dry_run {
                return Err(error(release::Error::Invalid(
                    "release recover does not support --dry-run; no recovery was started".into(),
                )));
            }
            let environment = Environment::detect().map_err(error)?;
            let report = release::recover_packages(scope.into(), &environment).map_err(error)?;
            let succeeded = report.conflicts.is_empty();
            if ctx.json {
                response::render_json_with_status("release recover", succeeded, &report)?;
            } else {
                println!(
                    "Package recovery: {} committed, {} rolled back, {} conflicts",
                    report.committed,
                    report.rolled_back,
                    report.conflicts.len()
                );
                for path in &report.conflicts {
                    eprintln!("Retained recovery evidence: {}", path.display());
                }
            }
            if succeeded {
                Ok(())
            } else {
                Err(CliError::BatchPartial {
                    command: "release recover".into(),
                })
            }
        }
        ReleaseCommand::Services {
            bundle,
            scope,
            profile,
            components,
            services,
            apply,
        } => {
            let bundle = Bundle::read(&bundle)
                .and_then(|bundle| bundle.select(scope.into(), profile.as_deref(), &components))
                .map_err(error)?;
            let layout = layout(&bundle, scope.into())?;
            let plan = release::plan_service_units(&bundle, &layout, &services).map_err(error)?;
            if !apply || ctx.dry_run {
                if ctx.json {
                    return response::render_json("release services", &plan);
                }
                for unit in &plan.units {
                    println!(
                        "{}: {} -> {}",
                        unit.service,
                        unit.template.display(),
                        unit.destination.display()
                    );
                }
                println!("Publish with --apply; this does not enable or start services");
                return Ok(());
            }
            let report = release::apply_service_units(&bundle, &layout, &plan).map_err(error)?;
            let succeeded = report.phase == "complete" && report.error.is_none();
            if ctx.json {
                response::render_json_with_status("release services", succeeded, &report)?;
            } else {
                println!("Service unit publication: {}", report.phase);
                for unit in &report.published {
                    println!("  {unit}");
                }
                if let Some(error) = report.error {
                    eprintln!("error: {error}");
                }
            }
            if succeeded {
                Ok(())
            } else {
                Err(CliError::BatchPartial {
                    command: "release services".into(),
                })
            }
        }
        ReleaseCommand::Inspect { bundle } => {
            let bundle = Bundle::read(&bundle).map_err(error)?;
            if ctx.json {
                response::render_json("release inspect", &bundle)
            } else {
                println!(
                    "Verified release {}: {} packages, {} payload bytes",
                    bundle.manifest.release_id,
                    bundle.packages.len(),
                    bundle.payload_bytes
                );
                Ok(())
            }
        }
        ReleaseCommand::Status {
            bundle,
            scope,
            profile,
            components,
        } => {
            let bundle = Bundle::read(&bundle)
                .and_then(|bundle| bundle.select(scope.into(), profile.as_deref(), &components))
                .map_err(error)?;
            let layout = layout(&bundle, scope.into())?;
            let report = release::status(&bundle, &layout).map_err(error)?;
            if ctx.json {
                response::render_json("release status", &report)
            } else {
                match report {
                    Some(report) => println!(
                        "Release {}: {} (current: {})",
                        report.release_id, report.phase, report.activated
                    ),
                    None => println!("No product operation is recorded for this release"),
                };
                Ok(())
            }
        }
        ReleaseCommand::Install {
            bundle,
            scope,
            profile,
            components,
        } => {
            let bundle = Bundle::read(&bundle)
                .and_then(|bundle| bundle.select(scope.into(), profile.as_deref(), &components))
                .map_err(error)?;
            let layout = layout(&bundle, scope.into())?;
            let report = release::install(&bundle, &layout, ctx.dry_run).map_err(error)?;
            let ok = report.succeeded();
            if ctx.json {
                response::render_json_with_status("release install", ok, &report)?;
            } else {
                println!(
                    "Release {}: {} at {}",
                    report.release_id,
                    report.phase,
                    report.prefix.display()
                );
                println!("Resources: {}", report.resources.display());
                println!("Commands: {}", layout.bin.display());
                for package in &report.packages {
                    println!(
                        "  {}@{}: committed={}, verified={}",
                        package.package_id, package.version, package.committed, package.verified
                    );
                }
                for warning in &report.warnings {
                    eprintln!("warning: {warning}");
                }
                if let Some(error) = &report.error {
                    eprintln!(
                        "error: {error}\nRetained product record: {}",
                        report.record_path.display()
                    );
                }
            }
            if ok {
                Ok(())
            } else {
                Err(CliError::BatchPartial {
                    command: "release install".into(),
                })
            }
        }
    }
}

pub(crate) fn layout(bundle: &Bundle, scope: Scope) -> Result<ReleaseLayout, CliError> {
    let environment = if scope == Scope::System {
        Environment::from_values(
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/root")),
            |name| std::env::var_os(name).map(PathBuf::from),
        )
    } else {
        Environment::detect()
    }
    .map_err(error)?;
    ReleaseLayout::resolve(&bundle.manifest, scope, &environment).map_err(error)
}

pub(crate) fn error(error: release::Error) -> CliError {
    let reason = error.to_string();
    match error {
        release::Error::Invalid(_) | release::Error::Json(_) => CliError::InvalidArgument {
            command: "release".into(),
            reason,
        },
        release::Error::Io { .. } => CliError::Runtime {
            command: "release".into(),
            reason,
        },
        #[cfg(target_os = "linux")]
        release::Error::Package(_) => CliError::Runtime {
            command: "release".into(),
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
    fn product_install_choices_do_not_change_legacy_component_arguments() {
        for arguments in [
            vec![
                "anolisa",
                "install",
                "--bundle",
                "/bundle",
                "--profile",
                "minimal",
                "--scope",
                "user",
            ],
            vec![
                "anolisa",
                "install",
                "tokenless",
                "--bundle",
                "/bundle",
                "--scope",
                "user",
            ],
            vec![
                "anolisa",
                "install",
                "tokenless",
                "--backend",
                "raw",
                "--install-mode",
                "user",
            ],
        ] {
            assert!(Cli::try_parse_from(arguments).is_ok());
        }
        for arguments in [
            vec!["anolisa", "install", "tokenless", "--all"],
            vec!["anolisa", "install", "--profile", "minimal"],
            vec!["anolisa", "install", "--bundle", "/bundle", "--all"],
            vec![
                "anolisa",
                "install",
                "--bundle",
                "/bundle",
                "--backend",
                "rpm",
            ],
            vec![
                "anolisa",
                "install",
                "--bundle",
                "/bundle",
                "tokenless",
                "--profile",
                "minimal",
            ],
        ] {
            assert!(Cli::try_parse_from(arguments).is_err());
        }
        assert!(
            Cli::try_parse_from(["anolisa", "release", "services", "--bundle", "/bundle"]).is_err()
        );
        assert!(
            Cli::try_parse_from([
                "anolisa",
                "release",
                "services",
                "--bundle",
                "/bundle",
                "--service",
                "ws-ckpt",
                "--apply",
                "--dry-run"
            ])
            .is_ok()
        );
    }

    #[test]
    fn release_scope_is_explicit_and_defaults_to_user_even_when_legacy_context_would_guess() {
        let cli = Cli::try_parse_from([
            "anolisa",
            "release",
            "install",
            "--bundle",
            "/bundle",
            "--json",
            "--dry-run",
        ])
        .unwrap();
        assert!(cli.json && cli.dry_run);
        assert!(matches!(
            cli.command,
            Commands::Management(ManagementCommands::Release(ReleaseArgs {
                command: ReleaseCommand::Install {
                    scope: ReleaseScope::User,
                    ..
                }
            }))
        ));
        let cli = Cli::try_parse_from([
            "anolisa", "release", "status", "--bundle", "/bundle", "--scope", "system",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Management(ManagementCommands::Release(ReleaseArgs {
                command: ReleaseCommand::Status {
                    scope: ReleaseScope::System,
                    ..
                }
            }))
        ));
    }

    #[test]
    fn release_never_interprets_legacy_prefix_or_install_mode_as_new_layout_parameters() {
        for option in ["--prefix=/", "--install-mode=user"] {
            let cli = Cli::try_parse_from([
                "anolisa",
                option,
                "release",
                "install",
                "--bundle",
                "/nonexistent-bundle",
            ])
            .unwrap();
            let context = CliContext::from_cli(&cli);
            let error = crate::commands::dispatch(cli, &context).unwrap_err();
            assert!(
                error
                    .reason()
                    .contains("legacy --prefix and --install-mode"),
                "{error}"
            );
        }
    }
}
