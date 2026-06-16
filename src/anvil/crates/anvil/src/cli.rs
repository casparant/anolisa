// SPDX-License-Identifier: Apache-2.0
//! Clap-derived CLI surface for the `anvil` binary.
//!
//! Subcommands fall into two families:
//!
//! 1. `daemon` runs the long-lived process (handled in
//!    [`crate::daemon`]).
//! 2. Everything else is a CLI client that opens the daemon UDS socket
//!    via [`crate::client::Client`] and pretty-prints JSON.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use serde_json::Value;

use crate::client::Client;
use crate::error::{AnvilDaemonError, Result};

#[derive(Parser, Debug)]
#[command(
    name = "anvil",
    version,
    about = "ANOLISA per-host sandbox orchestrator",
    long_about = None
)]
pub struct Cli {
    /// UDS socket of the running anvil daemon (used by all client subcommands).
    #[arg(
        long,
        global = true,
        default_value = "/run/anvil/api.sock",
        env = "ANVIL_SOCKET"
    )]
    pub socket: PathBuf,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Daemon lifecycle (start/stop/reload/doctor).
    #[command(subcommand)]
    Daemon(DaemonAction),
    /// Show daemon health status.
    Status,
    /// List managed sandbox instances.
    Ps,
    /// Show detailed info on one instance.
    Inspect {
        /// Instance UUID.
        instance: String,
    },
    /// Tail per-instance trajectory log.
    Logs {
        /// Instance UUID.
        instance: String,
    },
    /// Policy library inspection / reload.
    #[command(subcommand)]
    Policy(PolicyAction),
    /// Kernel hook registry management.
    #[command(subcommand)]
    Hook(HookAction),
    /// Warm pool management.
    #[command(subcommand)]
    Pool(PoolAction),
    /// Template registry management.
    #[command(subcommand)]
    Template(TemplateAction),
    /// Per-instance trajectory log access.
    #[command(subcommand)]
    Trajectory(TrajectoryAction),
}

#[derive(Subcommand, Debug)]
pub enum DaemonAction {
    /// Run the daemon in the foreground (this process blocks).
    Start {
        /// Path to the daemon TOML config.
        #[arg(long, default_value = "/etc/anolisa/anvil/config.toml")]
        config: PathBuf,
        /// Run in foreground; v0.1 only supports foreground execution.
        #[arg(long)]
        foreground: bool,
    },
    /// Ask a running daemon to exit (admin endpoint).
    Stop,
    /// Ask a running daemon to reload its policy library.
    Reload,
    /// Print local diagnostics: config path, socket reachability.
    Doctor {
        #[arg(long, default_value = "/etc/anolisa/anvil/config.toml")]
        config: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
pub enum PolicyAction {
    /// List loaded policies.
    List,
    /// Show a single policy by name.
    Show { name: String },
    /// Reload the policy library from disk.
    Reload,
    /// Validate a single policy TOML file without contacting the daemon.
    Validate { file: PathBuf },
}

#[derive(Subcommand, Debug)]
pub enum HookAction {
    /// List all registered kernel hooks.
    List,
    /// Show status for a specific hook (e.g. `mm-template`).
    Status { hook: String },
}

#[derive(Subcommand, Debug)]
pub enum PoolAction {
    /// List all warm pools.
    List,
    /// Show pools matching `(backend, class)`.
    Status { backend: String, class: String },
    /// Drain pools matching `(backend, class)`.
    Drain { backend: String, class: String },
    /// Resize pool sizing config for `(backend, class)`.
    Resize {
        backend: String,
        class: String,
        #[arg(long)]
        min: u32,
        #[arg(long)]
        target: u32,
        #[arg(long)]
        max: u32,
        #[arg(long, default_value = "")]
        image_digest: String,
        #[arg(long, default_value_t = 1800)]
        warm_ttl_secs: u64,
    },
}

#[derive(Subcommand, Debug)]
pub enum TemplateAction {
    /// List all templates.
    List,
    /// Inspect one template by id.
    Inspect { id: String },
    /// Trigger a GC sweep on the template registry.
    Gc,
}

#[derive(Subcommand, Debug)]
pub enum TrajectoryAction {
    /// Read events for one instance (optionally bounded by sequence).
    Show {
        instance: String,
        #[arg(long)]
        from_seq: Option<u64>,
        #[arg(long)]
        to_seq: Option<u64>,
    },
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Run a CLI client command (everything except `daemon start`, which is
/// dispatched by `main.rs` directly so it does not need an async runtime
/// before tracing is configured).
pub async fn run_client(socket: PathBuf, command: Command) -> Result<()> {
    let client = Client::new(socket);
    match command {
        Command::Daemon(DaemonAction::Stop) => {
            println!("anvil daemon stop is not exposed over the API in v0.1.");
            println!("Send SIGTERM to the daemon process instead, e.g.:");
            println!("    kill -TERM $(pidof anvil)");
            Ok(())
        }
        Command::Daemon(DaemonAction::Reload) => post_print(&client, "/v1/admin/reload").await,
        Command::Daemon(DaemonAction::Doctor { config }) => doctor(&client, &config).await,
        Command::Daemon(DaemonAction::Start { .. }) => Err(AnvilDaemonError::Internal(
            "daemon start should be dispatched directly, not via run_client".into(),
        )),
        Command::Status => get_print(&client, "/v1/health").await,
        Command::Ps => get_print(&client, "/v1/instances").await,
        Command::Inspect { instance } => {
            get_print(&client, &format!("/v1/instances/{instance}")).await
        }
        Command::Logs { instance } => {
            get_print(&client, &format!("/v1/instances/{instance}/trajectory")).await
        }
        Command::Policy(action) => policy_dispatch(&client, action).await,
        Command::Hook(action) => hook_dispatch(&client, action).await,
        Command::Pool(action) => pool_dispatch(&client, action).await,
        Command::Template(action) => template_dispatch(&client, action).await,
        Command::Trajectory(action) => trajectory_dispatch(&client, action).await,
    }
}

async fn policy_dispatch(client: &Client, action: PolicyAction) -> Result<()> {
    match action {
        PolicyAction::List => get_print(client, "/v1/policies").await,
        PolicyAction::Show { name } => {
            // No dedicated daemon endpoint for single-policy lookup in v0.1;
            // pull the full list and filter client-side.
            let bytes = client.get("/v1/policies").await?;
            let v: Value = serde_json::from_slice(&bytes)?;
            let entry = v
                .as_array()
                .into_iter()
                .flat_map(|arr| arr.iter())
                .find(|p| p.get("name").and_then(|x| x.as_str()) == Some(name.as_str()));
            match entry {
                Some(e) => println!("{}", serde_json::to_string_pretty(e)?),
                None => return Err(AnvilDaemonError::NotFound(format!("policy {name}"))),
            }
            Ok(())
        }
        PolicyAction::Reload => post_print(client, "/v1/admin/reload").await,
        PolicyAction::Validate { file } => validate_policy_file(&file),
    }
}

async fn hook_dispatch(client: &Client, action: HookAction) -> Result<()> {
    match action {
        HookAction::List => get_print(client, "/v1/hooks").await,
        HookAction::Status { hook } => {
            let bytes = client.get("/v1/hooks").await?;
            let v: Value = serde_json::from_slice(&bytes)?;
            let entry = v
                .as_array()
                .into_iter()
                .flat_map(|arr| arr.iter())
                .find(|h| h.get("kind").and_then(|x| x.as_str()) == Some(hook.as_str()));
            match entry {
                Some(e) => println!("{}", serde_json::to_string_pretty(e)?),
                None => return Err(AnvilDaemonError::NotFound(format!("hook {hook}"))),
            }
            Ok(())
        }
    }
}

async fn pool_dispatch(client: &Client, action: PoolAction) -> Result<()> {
    match action {
        PoolAction::List => get_print(client, "/v1/pools").await,
        PoolAction::Status { backend, class } => {
            get_print(client, &format!("/v1/pools/{backend}/{class}")).await
        }
        PoolAction::Drain { backend, class } => {
            post_print(client, &format!("/v1/pools/{backend}/{class}/drain")).await
        }
        PoolAction::Resize {
            backend,
            class,
            min,
            target,
            max,
            image_digest,
            warm_ttl_secs,
        } => {
            let body = serde_json::json!({
                "min": min,
                "target": target,
                "max": max,
                "image_digest": image_digest,
                "warm_ttl_secs": warm_ttl_secs,
            });
            let bytes = client
                .put(
                    &format!("/v1/pools/{backend}/{class}/sizing"),
                    serde_json::to_vec(&body)?,
                )
                .await?;
            print_json_bytes(&bytes)
        }
    }
}

async fn template_dispatch(client: &Client, action: TemplateAction) -> Result<()> {
    match action {
        TemplateAction::List => get_print(client, "/v1/templates").await,
        TemplateAction::Inspect { id } => get_print(client, &format!("/v1/templates/{id}")).await,
        TemplateAction::Gc => post_print(client, "/v1/templates/gc").await,
    }
}

async fn trajectory_dispatch(client: &Client, action: TrajectoryAction) -> Result<()> {
    match action {
        TrajectoryAction::Show {
            instance,
            from_seq,
            to_seq,
        } => {
            let mut path = format!("/v1/instances/{instance}/trajectory");
            let mut query = Vec::new();
            if let Some(f) = from_seq {
                query.push(format!("from_seq={f}"));
            }
            if let Some(t) = to_seq {
                query.push(format!("to_seq={t}"));
            }
            if !query.is_empty() {
                path.push('?');
                path.push_str(&query.join("&"));
            }
            get_print(client, &path).await
        }
    }
}

async fn doctor(client: &Client, config: &Path) -> Result<()> {
    println!("anvil doctor");
    println!("  config        : {}", config.display());
    println!("  socket        : {}", client.socket().display());
    let parsed = match anvil_core::config::DaemonConfig::load(config) {
        Ok(_) => "ok",
        Err(e) => {
            println!("  config parse  : FAIL ({e})");
            return Err(e.into());
        }
    };
    println!("  config parse  : {parsed}");
    match client.get("/v1/health").await {
        Ok(bytes) => {
            let v: Value = serde_json::from_slice(&bytes)?;
            println!(
                "  daemon health : {}",
                v.get("status").and_then(|x| x.as_str()).unwrap_or("?")
            );
        }
        Err(AnvilDaemonError::SocketConnect { source, .. }) => {
            println!("  daemon health : not reachable ({source})");
        }
        Err(e) => {
            println!("  daemon health : error ({e})");
        }
    }
    Ok(())
}

fn validate_policy_file(file: &Path) -> Result<()> {
    let raw = std::fs::read_to_string(file)?;
    let parsed: anvil_core::policy::PolicyFile = toml::from_str(&raw)?;
    if parsed.manifest_version != 1 {
        return Err(AnvilDaemonError::BadRequest(format!(
            "unsupported manifest_version {}",
            parsed.manifest_version
        )));
    }
    println!(
        "ok: policy {} priority={} class={}",
        parsed.policy_name, parsed.priority, parsed.match_.workload_class
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn get_print(client: &Client, path: &str) -> Result<()> {
    let bytes = client.get(path).await?;
    print_json_bytes(&bytes)
}

async fn post_print(client: &Client, path: &str) -> Result<()> {
    let bytes = client.post(path, Vec::new()).await?;
    print_json_bytes(&bytes)
}

fn print_json_bytes(bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    match serde_json::from_slice::<Value>(bytes) {
        Ok(v) => println!("{}", serde_json::to_string_pretty(&v)?),
        Err(_) => {
            // Not JSON (e.g. Prometheus text) — print as-is.
            print!("{}", String::from_utf8_lossy(bytes));
        }
    }
    Ok(())
}
