#![forbid(unsafe_code)]

//! Bounded operational access to the COSH audit subsystem.

use std::time::Instant;

use clap::Parser;
use serde::Serialize;

#[path = "cosh-audit/command.rs"]
mod command;

/// Single-purpose command line for the COSH audit subsystem.
#[derive(Parser)]
#[command(
    name = "cosh-audit",
    version,
    about = "Inspect, export, and evaluate COSH audit records"
)]
struct Cli {
    #[command(subcommand)]
    command: command::AuditCommands,
}

/// Minimal platform identity retained in the structured response envelope.
pub(crate) struct Distro {
    id: String,
}

impl Distro {
    fn detect() -> Self {
        let id = if cfg!(target_os = "macos") {
            "macos".to_string()
        } else {
            linux_distribution_id().unwrap_or_else(|| std::env::consts::OS.to_string())
        };
        Self { id }
    }

    pub(crate) fn id_str(&self) -> &str {
        &self.id
    }
}

/// Stable JSON metadata emitted by the audit utility.
#[derive(Serialize)]
pub(crate) struct ResponseMeta {
    subsystem: String,
    duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    distro: Option<String>,
    dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
}

#[derive(Serialize)]
struct Response<T: Serialize> {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<cosh_types::error::CoshError>,
    meta: ResponseMeta,
}

fn main() {
    init_tracing();
    let cli = Cli::parse();
    let distro = Distro::detect();
    let exit_code = command::run(cli.command, &distro, Instant::now());
    std::process::exit(exit_code);
}

fn init_tracing() {
    let filter = std::env::var("COSH_LOG")
        .or_else(|_| std::env::var("RUST_LOG"))
        .unwrap_or_else(|_| "warn".to_string());
    let filter = tracing_subscriber::EnvFilter::try_new(&filter)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(true)
        .try_init();
}

pub(crate) fn print_success<T: Serialize>(data: T, meta: ResponseMeta) -> i32 {
    print_response(Response {
        ok: true,
        data: Some(data),
        error: None,
        meta,
    })
}

pub(crate) fn print_failure(error: cosh_types::error::CoshError, meta: ResponseMeta) -> i32 {
    let _ = print_response(Response::<()> {
        ok: false,
        data: None,
        error: Some(error),
        meta,
    });
    1
}

fn print_response<T: Serialize>(response: Response<T>) -> i32 {
    match serde_json::to_string_pretty(&response) {
        Ok(json) => {
            println!("{json}");
            0
        }
        Err(error) => {
            eprintln!("{{\"ok\":false,\"error\":\"serialization failed: {error}\"}}");
            1
        }
    }
}

pub(crate) fn build_meta(
    subsystem: &str,
    distro: &Distro,
    start: Instant,
    dry_run: bool,
) -> ResponseMeta {
    ResponseMeta {
        subsystem: subsystem.to_string(),
        duration_ms: start.elapsed().as_millis() as u64,
        distro: Some(distro.id_str().to_string()),
        dry_run,
        warning: None,
    }
}

pub(crate) fn build_meta_with_warning(
    subsystem: &str,
    distro: &Distro,
    start: Instant,
    dry_run: bool,
    warning: &str,
) -> ResponseMeta {
    ResponseMeta {
        warning: Some(warning.to_string()),
        ..build_meta(subsystem, distro, start, dry_run)
    }
}

fn linux_distribution_id() -> Option<String> {
    let contents = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .ok()?;
    contents.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "ID").then(|| normalize_distribution_id(value))
    })
}

fn normalize_distribution_id(value: &str) -> String {
    let id = value.trim().trim_matches(['\'', '"']).to_ascii_lowercase();
    match id.as_str() {
        "opensuse-leap" | "opensuse-tumbleweed" | "sles" => "opensuse".to_string(),
        _ => id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_response_keeps_the_stable_json_shape() {
        let response = Response {
            ok: true,
            data: Some(serde_json::json!({"mode": "best_effort"})),
            error: None,
            meta: ResponseMeta {
                subsystem: "audit".to_string(),
                duration_ms: 7,
                distro: Some("opensuse".to_string()),
                dry_run: false,
                warning: None,
            },
        };
        let encoded = serde_json::to_value(response).unwrap();
        assert_eq!(
            encoded,
            serde_json::json!({
                "ok": true,
                "data": {"mode": "best_effort"},
                "meta": {
                    "subsystem": "audit",
                    "duration_ms": 7,
                    "distro": "opensuse",
                    "dry_run": false
                }
            })
        );
    }

    #[test]
    fn failure_response_keeps_the_stable_json_shape() {
        let response = Response::<()> {
            ok: false,
            data: None,
            error: Some(cosh_types::error::CoshError::new(
                cosh_types::error::ErrorCode::AuditUnavailable,
                "audit store missing",
                "audit",
            )),
            meta: ResponseMeta {
                subsystem: "audit".to_string(),
                duration_ms: 11,
                distro: None,
                dry_run: false,
                warning: Some("degraded audit writer".to_string()),
            },
        };
        let encoded = serde_json::to_value(response).unwrap();
        assert_eq!(
            encoded,
            serde_json::json!({
                "ok": false,
                "error": {
                    "code": "AuditUnavailable",
                    "message": "audit store missing",
                    "recoverable": false,
                    "hint": null,
                    "subsystem": "audit",
                    "details": null
                },
                "meta": {
                    "subsystem": "audit",
                    "duration_ms": 11,
                    "dry_run": false,
                    "warning": "degraded audit writer"
                }
            })
        );
    }

    #[test]
    fn opensuse_family_keeps_the_legacy_wire_id() {
        for id in ["opensuse-leap", "opensuse-tumbleweed", "sles"] {
            assert_eq!(normalize_distribution_id(id), "opensuse");
        }
        assert_eq!(normalize_distribution_id("\"Ubuntu\""), "ubuntu");
    }
}
