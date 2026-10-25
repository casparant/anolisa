//! Adapt AW process contexts to the shared installation lease protocol.

use crate::CommandSpec;
pub use local_package_runtime::LeaseError;
use std::path::PathBuf;

/// Shared locks retained until the corresponding owned runtime work stops.
#[derive(Debug)]
pub struct PackageLeases {
    _held: local_package_runtime::PackageLeases,
}

impl PackageLeases {
    /// Resolve commands from their explicit process context and lease all known package paths.
    ///
    /// # Errors
    /// Rejects unresolved commands, unsafe managed records, contention and unsupported platforms.
    pub fn acquire<'a>(
        commands: impl IntoIterator<Item = &'a mut CommandSpec>,
    ) -> Result<Self, LeaseError> {
        let commands =
            commands
                .into_iter()
                .map(|command| local_package_runtime::CommandReference {
                    program: &mut command.program,
                    args: &command.args,
                    cwd: &command.cwd,
                    environment: &command.environment,
                });
        Ok(Self {
            _held: local_package_runtime::PackageLeases::acquire(commands)?,
        })
    }

    /// Lease installations referenced by known absolute paths without scanning databases.
    ///
    /// # Errors
    /// Rejects unsafe records or lock metadata, contention and inaccessible path ancestors.
    pub fn for_paths(paths: impl IntoIterator<Item = PathBuf>) -> Result<Self, LeaseError> {
        Ok(Self {
            _held: local_package_runtime::PackageLeases::for_paths(paths)?,
        })
    }
}
