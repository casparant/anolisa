// SPDX-License-Identifier: Apache-2.0
//! `anvil` binary entry point. Acts as both the daemon (when invoked
//! as `anvil daemon start`) and the CLI client for every other
//! subcommand. See [`crate::cli`] for the command tree.

mod api;
mod cli;
mod client;
mod daemon;
mod error;
mod metrics;
mod state;

use std::process::ExitCode;

use clap::Parser;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::cli::{Cli, Command, DaemonAction};
use crate::error::Result;

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();

    let cli = Cli::parse();
    let outcome = run(cli).await;
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // Surface the error to the user with the standard Display
            // chain (via the underlying #[error("...")] format).
            eprintln!("anvil: {err}");
            ExitCode::from(1)
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Daemon(DaemonAction::Start { config, foreground }) => {
            if !foreground {
                tracing::warn!(
                    "anvil v0.1 only supports foreground execution; ignoring missing --foreground"
                );
            }
            daemon::run(&config).await
        }
        other => cli::run_client(cli.socket, other).await,
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let layer = fmt::layer()
        .json()
        .with_target(true)
        .with_current_span(false);
    tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .init();
}
