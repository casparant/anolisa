//! Offline human operations view over product, legacy state and local native package evidence.

use crate::{
    context::CliContext,
    response::{self, CliError},
};
use anolisa_core::{
    deployment::SystemctlRunner,
    product_status::{self, OverviewRequest, ProductOverview, RpmInventoryRunner, Visibility},
    release::Environment,
};
use anolisa_platform::{fs_layout::FsLayout, rpm_query::RpmPackageQuery};
use clap::{Args, ValueEnum};
use std::{collections::BTreeSet, path::PathBuf};

/// Installation visibility; selecting system evidence never requests sudo.
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum OverviewScope {
    /// Show both invoking-user and observable system installations.
    #[default]
    All,
    /// Show invoking-user installations.
    User,
    /// Show observable system installations.
    System,
}

impl From<OverviewScope> for Visibility {
    fn from(scope: OverviewScope) -> Self {
        match scope {
            OverviewScope::All => Self::All,
            OverviewScope::User => Self::User,
            OverviewScope::System => Self::System,
        }
    }
}

/// Local evidence collection options, shared by the first-run setup guide.
#[derive(Args, Default)]
pub struct OverviewArgs {
    /// Installation evidence to show; does not select a write or service scope.
    #[arg(long, value_enum, default_value = "all")]
    pub scope: OverviewScope,
    /// Inspect current systemd states with bounded local queries; no services are started.
    #[arg(long)]
    pub check_services: bool,
    /// Verify a local unpacked candidate and compare package versions without downloading or upgrading.
    #[arg(long, value_name = "ABS_DIRECTORY")]
    pub candidate_bundle: Option<PathBuf>,
}

/// Build one request-scoped view without querying repositories or executing component binaries.
pub(crate) fn collect(
    args: &OverviewArgs,
    ctx: &CliContext,
) -> Result<(Environment, ProductOverview), CliError> {
    let environment = Environment::detect().map_err(|error| failure("overview", error))?;
    let mut search_path: Vec<_> =
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .filter(|path| path.is_absolute())
            .collect();
    search_path.extend([
        environment.home.join(".local/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
    ]);
    let native = RpmPackageQuery::with_runner(RpmInventoryRunner);
    let mut layouts = vec![
        FsLayout::user(environment.home.clone()),
        ctx.visible_system_layout().clone(),
    ];
    layouts.dedup_by(|left, right| left.state_dir == right.state_dir);
    let request = OverviewRequest {
        environment: &environment,
        visibility: args.scope.into(),
        search_path,
        product_prefix: std::env::var_os("ANOLISA_RELEASE_PREFIX").map(PathBuf::from),
        product_resources: std::env::var_os("ANOLISA_RELEASE_RESOURCES").map(PathBuf::from),
        legacy_layouts: layouts,
        check_services: args.check_services,
        native_packages: std::path::Path::new("/usr/bin/rpm")
            .is_file()
            .then_some(&native as &dyn anolisa_platform::pkg_query::PackageQuery),
    };
    let mut report = product_status::overview(&request, &SystemctlRunner);
    if let Some(path) = &args.candidate_bundle {
        let bundle = anolisa_core::release::Bundle::read(path)
            .map_err(|error| failure("overview", error))?;
        product_status::compare_candidate(&mut report, &bundle);
    }
    Ok((environment, report))
}

/// Render installed versions and qualified runtime readiness for the invoking human.
pub fn handle(args: OverviewArgs, ctx: &CliContext) -> Result<(), CliError> {
    let (_, report) = collect(&args, ctx)?;
    if ctx.json {
        return response::render_json("overview", report);
    }
    if !ctx.quiet {
        render(&report);
    }
    Ok(())
}

pub(crate) fn render(report: &ProductOverview) {
    println!(
        "{:<12} {:<20} {:<12} Availability",
        "Component", "Recorded version", "Scope"
    );
    for row in &report.components {
        let versions: BTreeSet<_> = row
            .installations
            .iter()
            .filter_map(|item| item.version.as_deref())
            .collect();
        let scopes: BTreeSet<_> = row
            .installations
            .iter()
            .map(|item| item.scope.as_str())
            .collect();
        let versions = if versions.is_empty() {
            "unknown".into()
        } else {
            versions.into_iter().collect::<Vec<_>>().join(",")
        };
        println!(
            "{:<12} {:<20} {:<12} {}",
            row.component,
            versions,
            scopes.into_iter().collect::<Vec<_>>().join(","),
            row.availability.replace('_', " ")
        );
        println!("  {}", row.detail);
        if let Some(command) = &row.command {
            println!("  Command: {}", command.display());
        }
        if let Some(requirement) = &row.privilege_requirement {
            println!("  Access: {requirement}");
        }
        if !row.provider_packages.is_empty() || !row.plugin_packages.is_empty() {
            println!(
                "  Provider packages: {}; native plugins: {}. Installation does not enable them.",
                row.provider_packages.join(", "),
                row.plugin_packages.join(", ")
            );
        }
    }
    println!(
        "Latest release: not checked (offline). Compare a verified local bundle with --candidate-bundle DIR."
    );
    if let Some(candidate) = &report.candidate {
        println!("Candidate product {}:", candidate.product_version);
        for package in &candidate.packages {
            println!(
                "  {}: {} -> {} ({})",
                package.package_id,
                package.observed_versions.join(","),
                package
                    .candidate_version
                    .as_deref()
                    .unwrap_or("not included"),
                package.change
            );
        }
        for guidance in &candidate.guidance {
            println!("  {guidance}");
        }
    }
    for warning in &report.warnings {
        eprintln!("Note: {warning}");
    }
}

pub(crate) fn failure(command: &str, error: impl std::fmt::Display) -> CliError {
    CliError::Runtime {
        command: command.into(),
        reason: error.to_string(),
    }
}
