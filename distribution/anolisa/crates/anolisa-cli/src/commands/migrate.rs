//! Fixed, reviewable ownership transfer from legacy raw installs to product releases.

use super::release::{self, ReleaseScope};
use crate::{
    context::CliContext,
    response::{self, CliError},
};
use anolisa_core::release::{self as product, Bundle, MigrationPlan, Scope};
use anolisa_platform::fs_layout::FsLayout;
use clap::{Args, Subcommand};
use std::path::PathBuf;

/// Review or execute a recorded legacy installation migration.
#[derive(Args)]
pub struct MigrateArgs {
    /// Plan creation and application are separate operations.
    #[command(subcommand)]
    pub command: MigrateCommand,
}

/// Evidence required before changing a component's managing installer.
#[derive(Subcommand)]
pub enum MigrateCommand {
    /// Inspect old ownership and retain a fixed plan; never install packages or stop services.
    Plan {
        /// Unpacked product directory; every delivered payload is verified.
        #[arg(long)]
        bundle: PathBuf,
        /// Explicit source and destination scope; cross-scope takeover is not implicit.
        #[arg(long, value_enum, default_value = "user")]
        scope: ReleaseScope,
        /// Product package profile, independent of service activation.
        #[arg(long)]
        profile: Option<String>,
        /// Select independent components instead of a product profile.
        #[arg(long = "component", conflicts_with = "profile")]
        components: Vec<String>,
        /// Historical system prefix; user migration uses the invoking user's XDG roots.
        #[arg(long)]
        source_prefix: Option<PathBuf>,
        /// Explicitly accept these exact historical standalone anolisa CLI bytes.
        #[arg(long)]
        legacy_cli_sha256: Option<String>,
        /// Save the fixed plan here for a subsequent apply; parent must already exist.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Apply exactly a reviewed plan, retaining backups and failed progress for retry.
    Apply {
        /// Caller-owned plan saved by migrate plan --output.
        #[arg(long)]
        plan: PathBuf,
    },
}

/// Dispatch migration without interpreting legacy global layout options.
pub fn handle(args: MigrateArgs, ctx: &CliContext) -> Result<(), CliError> {
    match args.command {
        MigrateCommand::Plan {
            bundle,
            scope,
            profile,
            components,
            source_prefix,
            legacy_cli_sha256,
            output,
        } => {
            let scope = scope.into();
            let source = source_layout(scope, source_prefix)?;
            let bundle = Bundle::read(&bundle)
                .and_then(|bundle| bundle.select(scope, profile.as_deref(), &components))
                .map_err(release::error)?;
            let target = release::layout(&bundle, scope)?;
            let plan = product::plan_migration(
                &bundle,
                &target,
                &source,
                profile,
                components,
                legacy_cli_sha256,
            )
            .map_err(release::error)?;
            if let Some(path) = output.filter(|_| !ctx.dry_run) {
                plan.write(&path).map_err(release::error)?;
            }
            render_plan(&plan, ctx)
        }
        MigrateCommand::Apply { plan } => {
            let plan = MigrationPlan::read(&plan).map_err(release::error)?;
            let source = source_layout(
                plan.scope,
                (plan.scope == Scope::System).then(|| plan.source_prefix.clone()),
            )?;
            let bundle = Bundle::read(&plan.bundle)
                .and_then(|bundle| {
                    bundle.select(plan.scope, plan.profile.as_deref(), &plan.components)
                })
                .map_err(release::error)?;
            let target = release::layout(&bundle, plan.scope)?;
            if ctx.dry_run {
                let current = product::plan_migration(
                    &bundle,
                    &target,
                    &source,
                    plan.profile.clone(),
                    plan.components.clone(),
                    plan.legacy_cli_sha256.clone(),
                )
                .map_err(release::error)?;
                if current.id != plan.id {
                    return Err(CliError::InvalidArgument {
                        command: "migrate apply".into(),
                        reason: "migration evidence changed; generate and review a new plan".into(),
                    });
                }
                return render_plan(&current, ctx);
            }
            let report = product::apply_migration(&plan, &bundle, &target, &source)
                .map_err(release::error)?;
            let succeeded = report.succeeded();
            if ctx.json {
                response::render_json_with_status("migrate apply", succeeded, &report)?;
            } else {
                println!(
                    "Migration {}: {} at {}",
                    plan.id,
                    report.phase,
                    report.prefix.display()
                );
                if let Some(error) = report.error {
                    eprintln!("error: {error}");
                }
                println!(
                    "Rollback material: {}",
                    target.state.join("migrations").join(&plan.id).display()
                );
            }
            if succeeded {
                Ok(())
            } else {
                Err(CliError::BatchPartial {
                    command: "migrate apply".into(),
                })
            }
        }
    }
}

fn source_layout(scope: Scope, prefix: Option<PathBuf>) -> Result<FsLayout, CliError> {
    if scope == Scope::System {
        return Ok(FsLayout::system(prefix));
    }
    if prefix.is_some() {
        return Err(CliError::InvalidArgument {
            command: "migrate".into(),
            reason: "--source-prefix only applies to system migration; user roots follow XDG"
                .into(),
        });
    }
    let home =
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| CliError::InvalidArgument {
                command: "migrate".into(),
                reason: "HOME is required for user migration".into(),
            })?;
    Ok(FsLayout::user(home))
}

fn render_plan(plan: &MigrationPlan, ctx: &CliContext) -> Result<(), CliError> {
    if ctx.json {
        return response::render_json("migrate plan", plan);
    }
    println!("Migration {}", plan.id);
    println!("Source: {}", plan.source_state.display());
    println!("Target: {}", plan.target_prefix.display());
    println!(
        "Retire old raw records after success: {}",
        plan.retire.join(", ")
    );
    println!(
        "Retain {} old paths for configuration, data and rollback",
        plan.retained_paths.len()
    );
    for entry in &plan.entrypoints {
        println!(
            "  replace verified command {} ({})",
            entry.name, entry.sha256
        );
    }
    for blocker in &plan.blockers {
        println!("  blocked: {blocker}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::Cli;
    use clap::Parser;

    #[test]
    fn migration_requires_a_fixed_plan_to_apply() {
        assert!(Cli::try_parse_from(["anolisa", "migrate", "apply"]).is_err());
        assert!(
            Cli::try_parse_from([
                "anolisa",
                "migrate",
                "apply",
                "--plan",
                "/plan.json",
                "--dry-run"
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "anolisa",
                "migrate",
                "plan",
                "--bundle",
                "/bundle",
                "--scope",
                "system",
                "--profile",
                "standard-system"
            ])
            .is_ok()
        );
        assert!(source_layout(Scope::User, Some(PathBuf::from("/other"))).is_err());
    }
}
