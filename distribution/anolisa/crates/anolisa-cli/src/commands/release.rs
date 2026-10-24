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
#[derive(Clone, Copy, Default, ValueEnum)]
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
    /// Install or resume one immutable unpacked release; does not enable Providers or services.
    Install {
        /// Absolute symlink-free directory containing release.json and packages/.
        #[arg(long, value_name = "ABS_DIRECTORY")]
        bundle: PathBuf,
        /// Explicit package and product scope; defaults to user even under sudo.
        #[arg(long, value_enum, default_value = "user")]
        scope: ReleaseScope,
    },
    /// Verify all release descriptors and payloads without writing or running AW.
    Inspect {
        /// Absolute unpacked release directory.
        #[arg(long, value_name = "ABS_DIRECTORY")]
        bundle: PathBuf,
    },
    /// Read this release's durable product progress without changing it.
    Status {
        /// Absolute unpacked release directory identifying the immutable release.
        #[arg(long, value_name = "ABS_DIRECTORY")]
        bundle: PathBuf,
        /// Explicit product scope.
        #[arg(long, value_enum, default_value = "user")]
        scope: ReleaseScope,
    },
}

/// Run an offline product operation using a separately resolved release layout.
pub fn handle(args: ReleaseArgs, ctx: &CliContext) -> Result<(), CliError> {
    match args.command {
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
        ReleaseCommand::Status { bundle, scope } => {
            let bundle = Bundle::read(&bundle).map_err(error)?;
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
        ReleaseCommand::Install { bundle, scope } => {
            let bundle = Bundle::read(&bundle).map_err(error)?;
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

fn layout(bundle: &Bundle, scope: Scope) -> Result<ReleaseLayout, CliError> {
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

fn error(error: release::Error) -> CliError {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{Cli, Commands, ManagementCommands};
    use clap::Parser;

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
