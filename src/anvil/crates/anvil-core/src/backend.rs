// SPDX-License-Identifier: Apache-2.0
//! Sandbox backend kinds + selection / fallback.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::config::OciSection;
use crate::error::{AnvilError, Result};

/// All backends that anvil v0.1 knows about. The OCI shim is responsible
/// for delegating to the corresponding `containerd-shim-*` binary
/// configured in [`OciSection::delegate_shims`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendKind {
    LinuxSandbox,
    Runc,
    Rund,
    KataFc,
    KataClh,
    KataQemu,
    Firecracker,
    Gvisor,
    GvisorSubstrate,
    Landlock,
}

impl BackendKind {
    /// Stable string label used in policy files / metrics / config keys.
    pub const fn as_str(&self) -> &'static str {
        match self {
            BackendKind::LinuxSandbox => "linux-sandbox",
            BackendKind::Runc => "runc",
            BackendKind::Rund => "rund",
            BackendKind::KataFc => "kata-fc",
            BackendKind::KataClh => "kata-clh",
            BackendKind::KataQemu => "kata-qemu",
            BackendKind::Firecracker => "firecracker",
            BackendKind::Gvisor => "gvisor",
            BackendKind::GvisorSubstrate => "gvisor-substrate",
            BackendKind::Landlock => "landlock",
        }
    }

    /// Look up the delegated containerd shim path for this backend in the
    /// daemon `[oci]` section. Returns `None` when the backend has no
    /// shim configured (the daemon should reject scheduling onto it).
    pub fn shim_path<'a>(&self, config: &'a OciSection) -> Option<&'a PathBuf> {
        config.delegate_shims.get(self.as_str())
    }
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for BackendKind {
    type Err = AnvilError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "linux-sandbox" => Ok(BackendKind::LinuxSandbox),
            "runc" => Ok(BackendKind::Runc),
            "rund" => Ok(BackendKind::Rund),
            "kata-fc" => Ok(BackendKind::KataFc),
            "kata-clh" => Ok(BackendKind::KataClh),
            "kata-qemu" => Ok(BackendKind::KataQemu),
            "firecracker" => Ok(BackendKind::Firecracker),
            "gvisor" => Ok(BackendKind::Gvisor),
            "gvisor-substrate" => Ok(BackendKind::GvisorSubstrate),
            "landlock" => Ok(BackendKind::Landlock),
            other => Err(AnvilError::PolicyEvalError {
                reason: format!("unknown backend kind: {other}"),
            }),
        }
    }
}

/// Probed availability of a single backend on this host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendStatus {
    pub kind: BackendKind,
    pub available: bool,
    #[serde(default)]
    pub version: Option<String>,
}

/// Walk `priority` in order and return the first backend that is marked
/// available. Returns [`AnvilError::BackendUnavailable`] when no entry in
/// `priority` is available.
pub fn select_backend(
    priority: &[BackendKind],
    available: &[BackendStatus],
) -> Result<BackendKind> {
    for kind in priority {
        if available
            .iter()
            .any(|status| status.kind == *kind && status.available)
        {
            tracing::info!(backend = %kind, "selected backend");
            return Ok(*kind);
        }
        tracing::warn!(backend = %kind, "backend not available, falling back");
    }

    let requested = priority.iter().map(|b| b.as_str().to_string()).collect();
    let available = available
        .iter()
        .filter(|s| s.available)
        .map(|s| s.kind.as_str().to_string())
        .collect();
    Err(AnvilError::BackendUnavailable {
        requested,
        available,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_str() {
        for kind in [
            BackendKind::LinuxSandbox,
            BackendKind::Runc,
            BackendKind::Rund,
            BackendKind::KataFc,
            BackendKind::KataClh,
            BackendKind::KataQemu,
            BackendKind::Firecracker,
            BackendKind::Gvisor,
            BackendKind::GvisorSubstrate,
            BackendKind::Landlock,
        ] {
            let s = kind.as_str();
            let parsed: BackendKind = s.parse().expect("round-trip");
            assert_eq!(parsed, kind);
        }
    }

    #[test]
    fn select_picks_first_available() {
        let priority = vec![BackendKind::KataFc, BackendKind::KataClh, BackendKind::Rund];
        let available = vec![
            BackendStatus {
                kind: BackendKind::KataFc,
                available: false,
                version: None,
            },
            BackendStatus {
                kind: BackendKind::KataClh,
                available: true,
                version: Some("3.2".into()),
            },
            BackendStatus {
                kind: BackendKind::Rund,
                available: true,
                version: None,
            },
        ];
        let chosen = select_backend(&priority, &available).expect("selects");
        assert_eq!(chosen, BackendKind::KataClh);
    }

    #[test]
    fn select_errors_when_none_available() {
        let priority = vec![BackendKind::KataFc];
        let available = vec![BackendStatus {
            kind: BackendKind::KataFc,
            available: false,
            version: None,
        }];
        let err = select_backend(&priority, &available).expect_err("must fail");
        assert!(matches!(err, AnvilError::BackendUnavailable { .. }));
    }
}
