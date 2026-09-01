#![forbid(unsafe_code)]
//! Diagnostic command surface for the headless AW Provider Host.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use aw_contracts::ids::RequestId;
use aw_contracts::provider::{
    CapabilityInvocation, ProviderApiVersion, ProviderDisposition, ProviderResult,
    ProviderResultEnvelope,
};
use aw_provider_host::{
    ProviderAdmissionOptions, ProviderCatalog, ProviderManifestSource,
    MAX_PROVIDER_INVOCATION_BYTES,
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{json, Value};
use thiserror::Error;

const EXIT_INPUT: u8 = 10;
const EXIT_RUNTIME: u8 = 12;

#[derive(Debug, Parser)]
#[command(
    name = "aw-provider-host",
    version,
    about = "Inspect and invoke AW Capability Providers"
)]
struct Cli {
    /// Presentation format for graph, health, and invocation results.
    #[arg(long, value_enum, default_value_t = Output::Human)]
    output: Output,
    #[command(subcommand)]
    command: ProviderCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum ProviderCommand {
    /// List the deterministic Runtime Capability Graph.
    List(ProviderSourceArgs),
    /// Validate manifests, executable identities, schemas, and mappings.
    Doctor(ProviderSourceArgs),
    /// Invoke one Capability from a versioned JSON document.
    Invoke(ProviderInvokeArgs),
}

#[derive(Debug, Clone, Args)]
struct ProviderSourceArgs {
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
}

#[derive(Debug, Clone, Args)]
struct ProviderInvokeArgs {
    #[command(flatten)]
    source: ProviderSourceArgs,
    /// Read `CapabilityInvocation` JSON from this regular file; default is stdin.
    #[arg(long, value_name = "PATH")]
    invocation_file: Option<PathBuf>,
    /// Existing absolute state root, required only by stateful manifests.
    #[arg(long, value_name = "DIR")]
    state_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Output {
    /// Stable newline-delimited JSON for automation.
    Jsonl,
    /// Pretty JSON for an operator.
    Human,
}

#[derive(Debug, Error)]
enum CliError {
    /// Invocation input could not be read.
    #[error("failed to read Provider invocation: {0}")]
    Input(#[source] io::Error),
    /// Invocation or source arguments violate a boundary contract.
    #[error("invalid request: {0}")]
    InvalidInput(String),
    /// Provider discovery, admission, or execution failed.
    #[error("Provider Host failed: {0}")]
    Provider(String),
    /// Diagnostic output could not be written.
    #[error("failed to write output: {0}")]
    Output(#[source] io::Error),
}

impl CliError {
    fn exit_code(&self) -> u8 {
        match self {
            Self::Input(_) | Self::InvalidInput(_) => EXIT_INPUT,
            Self::Provider(_) | Self::Output(_) => EXIT_RUNTIME,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Input(_) => "invocation_read_failed",
            Self::InvalidInput(_) => "invalid_request",
            Self::Provider(_) => "provider_failed",
            Self::Output(_) => "output_failed",
        }
    }
}

struct Reporter {
    output: Output,
}

impl Reporter {
    fn event(&self, event: &str, fields: Value) -> Result<(), CliError> {
        let value = match self.output {
            Output::Jsonl => {
                let mut value = json!({"event": event});
                if let (Some(target), Some(source)) = (value.as_object_mut(), fields.as_object()) {
                    target.extend(source.clone());
                }
                value.to_string()
            }
            Output::Human => serde_json::to_string_pretty(&fields)
                .map_err(|error| CliError::Provider(error.to_string()))?,
        };
        println!("{value}");
        io::stdout().flush().map_err(CliError::Output)
    }

    fn error(&self, error: &CliError) {
        match self.output {
            Output::Human => eprintln!("Error [{}]: {error}", error.code()),
            Output::Jsonl => println!(
                "{}",
                json!({"event":"error", "code":error.code(), "message":error.to_string()})
            ),
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let reporter = Reporter { output: cli.output };
    match run(cli.command, &reporter) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            reporter.error(&error);
            ExitCode::from(error.exit_code())
        }
    }
}

fn run(command: ProviderCommand, reporter: &Reporter) -> Result<u8, CliError> {
    match command {
        ProviderCommand::List(source) => {
            let catalog = discover(source)?;
            reporter.event(
                "provider_list",
                serde_json::to_value(catalog.capability_graph())
                    .map_err(|error| CliError::Provider(error.to_string()))?,
            )?;
        }
        ProviderCommand::Doctor(source) => {
            let catalog = discover(source)?;
            reporter.event(
                "provider_doctor",
                json!({
                    "api_version": "diagnostics.agentic-os.sh/v1",
                    "status": "ready",
                    "graph": catalog.capability_graph(),
                }),
            )?;
        }
        ProviderCommand::Invoke(args) => {
            let catalog = discover(args.source)?;
            let invocation = read_invocation(args.invocation_file.as_ref())?;
            let invocation = catalog
                .invoke(&invocation, args.state_root.as_deref())
                .map_err(|error| CliError::Provider(error.to_string()))?;
            let exit_code = if matches!(
                invocation.receipt.disposition,
                ProviderDisposition::Denied
                    | ProviderDisposition::Failed
                    | ProviderDisposition::Uncertain
            ) {
                EXIT_RUNTIME
            } else {
                0
            };
            reporter.event(
                "provider_result",
                serde_json::to_value(ProviderResultEnvelope {
                    api_version: ProviderApiVersion::V1,
                    request_id: RequestId::new(),
                    result: ProviderResult::Invoked {
                        invocation: Box::new(invocation),
                    },
                })
                .map_err(|error| CliError::Provider(error.to_string()))?,
            )?;
            return Ok(exit_code);
        }
    }
    Ok(0)
}

fn discover(source: ProviderSourceArgs) -> Result<ProviderCatalog, CliError> {
    let ProviderSourceArgs {
        manifest,
        manifest_dir,
        executable_root,
    } = source;
    let source = match (manifest, manifest_dir) {
        (Some(path), None) => ProviderManifestSource::File(path),
        (None, Some(path)) => ProviderManifestSource::Directory(path),
        _ => {
            return Err(CliError::InvalidInput(
                "exactly one of --manifest or --manifest-dir is required".to_owned(),
            ));
        }
    };
    ProviderCatalog::discover(
        source,
        &ProviderAdmissionOptions {
            executable_roots: executable_root,
        },
    )
    .map_err(|error| CliError::Provider(error.to_string()))
}

fn read_invocation(path: Option<&PathBuf>) -> Result<CapabilityInvocation, CliError> {
    let bytes = match path {
        Some(path) => {
            let metadata = fs::symlink_metadata(path).map_err(CliError::Input)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(CliError::InvalidInput(format!(
                    "Provider invocation path is not a regular file: {}",
                    path.display()
                )));
            }
            read_bounded(File::open(path).map_err(CliError::Input)?)?
        }
        None => read_bounded(io::stdin().lock())?,
    };
    serde_json::from_slice(&bytes).map_err(|error| {
        CliError::InvalidInput(format!("invalid CapabilityInvocation JSON: {error}"))
    })
}

fn read_bounded(mut reader: impl Read) -> Result<Vec<u8>, CliError> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(MAX_PROVIDER_INVOCATION_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(CliError::Input)?;
    if bytes.len() > MAX_PROVIDER_INVOCATION_BYTES {
        return Err(CliError::InvalidInput(format!(
            "Provider invocation exceeds the {MAX_PROVIDER_INVOCATION_BYTES}-byte limit"
        )));
    }
    Ok(bytes)
}
