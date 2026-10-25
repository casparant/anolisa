//! AW extension installation and offline framework configuration.
//! Distribution software installation uses the shared local-package library directly.

pub mod config_edit;
mod configuration;
pub mod database;
mod filesystem;
mod install;
pub mod package;
pub mod plan;

pub use configuration::{configure, document, Policy, Settings};
pub use local_package::{
    description, digest, inspect, install, install_cancellable, install_reported, open_bundle,
    transaction, uninstall, uninstall_cancellable, uninstall_reported, Component, Error, Manifest,
    MutationFailure, MutationNotice, MutationReport, MutationResult, NoticeCode, Payload,
    RecoveryState, Result,
};

pub(crate) fn require(condition: bool, message: impl Into<String>) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Error::Invalid(message.into()))
    }
}
