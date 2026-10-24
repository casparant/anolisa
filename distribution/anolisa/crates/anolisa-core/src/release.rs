//! Verified offline product bundles and recoverable, per-package release installation.

mod activate;
mod aw;
mod configuration;
mod files;
mod install;
mod layout;
mod model;
mod probe;
mod resources;
#[cfg(test)]
mod tests;

pub use configuration::ProviderConfiguration;
pub use install::{InstallReport, PackageResult, install, status};
pub use layout::{Environment, ReleaseLayout, Scope};
pub use model::{Bundle, LocalPackage, Manifest, PackageRef, Payload, Provider, Requirement};

use std::{io, path::PathBuf};

/// A release refusal preserves its filesystem or package-operation context.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Invalid or incompatible input, ownership, or recorded evidence.
    #[error("{0}")]
    Invalid(String),
    /// A filesystem operation failed at the specified path.
    #[error("release filesystem operation at {path}: {source}")]
    Io {
        /// Path involved in the failure.
        path: PathBuf,
        /// Original operating-system error.
        #[source]
        source: io::Error,
    },
    /// JSON is invalid or incompatible with the selected version.
    #[error("release JSON: {0}")]
    Json(#[from] serde_json::Error),
}

pub(super) type Result<T> = std::result::Result<T, Error>;

fn require(condition: bool, message: impl Into<String>) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Error::Invalid(message.into()))
    }
}
