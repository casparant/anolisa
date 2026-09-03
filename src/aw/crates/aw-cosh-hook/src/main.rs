#![forbid(unsafe_code)]
//! Standalone COSH hook process for AW context projection.

use std::fs::OpenOptions;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use aw_contracts::common::BoundedName;
use aw_core::CapabilityPreferences;
use aw_cosh_hook::{local_host_target, run_cosh_post_tool_use, CoshHookConfig};
use aw_provider_host::{ProviderAdmissionOptions, ProviderManifestSource};
use clap::Parser;

const EXIT_FAILURE: u8 = 12;

#[derive(Debug, Parser)]
#[command(
    name = "aw-cosh-hook",
    version,
    about = "Bridge COSH PostToolUse results to AW"
)]
struct Cli {
    /// Load exactly one absolute Provider manifest path.
    #[arg(long, value_name = "PATH", required_unless_present = "manifest_dir")]
    manifest: Option<PathBuf>,
    /// Load `<provider-id>/provider.toml` packages below one absolute directory.
    #[arg(
        long,
        value_name = "DIR",
        required_unless_present = "manifest",
        conflicts_with = "manifest"
    )]
    manifest_dir: Option<PathBuf>,
    /// Explicit absolute root searched for a bare installed executable name.
    #[arg(long, value_name = "DIR")]
    executable_root: Vec<PathBuf>,
    /// Stable target identifier in the local host authority.
    #[arg(long, value_name = "ID")]
    target_id: String,
    /// Pin one Provider for one Capability, as `CAPABILITY=PROVIDER`.
    ///
    /// Applies only to Capabilities the Core plan routes to a single
    /// implementation. Repeat the flag to pin several Capabilities.
    #[arg(long, value_name = "CAPABILITY=PROVIDER")]
    preferred_provider: Vec<String>,
    /// Maximum time Core grants one Provider invocation, in milliseconds.
    ///
    /// The effective limit is the smallest of this value, the Provider's own
    /// declared limit, and the remaining invocation deadline.
    #[arg(long, value_name = "MS")]
    provider_wall_time_ms: Option<u64>,
    /// Trust this Provider even though its network and filesystem declarations
    /// are not yet enforced by an OS sandbox.
    #[arg(long)]
    allow_unenforced_provider: bool,
    /// Append content-free Provider receipts and replacement requests as JSONL.
    #[arg(long, value_name = "PATH")]
    receipt_log: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("aw-cosh-hook: {error}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let provider_source = match (cli.manifest, cli.manifest_dir) {
        (Some(path), None) => ProviderManifestSource::File(path),
        (None, Some(path)) => ProviderManifestSource::Directory(path),
        _ => return Err("exactly one Provider manifest source is required".into()),
    };
    let mut preferences = CapabilityPreferences::default();
    for pair in cli.preferred_provider {
        let (capability, provider) = pair
            .split_once('=')
            .ok_or("--preferred-provider expects CAPABILITY=PROVIDER")?;
        preferences
            .preferred_providers
            .insert(BoundedName::new(capability)?, BoundedName::new(provider)?);
    }
    let config = CoshHookConfig {
        provider_source,
        provider_admission: ProviderAdmissionOptions {
            executable_roots: cli.executable_root,
        },
        target: local_host_target(cli.target_id)?,
        preferences,
        provider_wall_time_ms: cli.provider_wall_time_ms,
        allow_unenforced_provider: cli.allow_unenforced_provider,
    };
    let result =
        run_cosh_post_tool_use(std::io::stdin().lock(), std::io::stdout().lock(), &config)?;
    if let Some(path) = cli.receipt_log.as_deref() {
        append_receipt_log(path, &result)?;
    }
    Ok(())
}

fn append_receipt_log(
    path: &Path,
    run: &aw_cosh_hook::CoshHookRun,
) -> Result<(), Box<dyn std::error::Error>> {
    if run.receipts.is_empty() {
        return Ok(());
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    let mut line = serde_json::to_vec(run)?;
    line.push(b'\n');
    file.write_all(&line)?;
    file.sync_data()?;
    Ok(())
}
