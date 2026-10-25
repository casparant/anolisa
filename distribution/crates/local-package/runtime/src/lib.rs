//! Cooperating runtime leases for managed package files, independent of installation databases.

#[cfg(target_os = "linux")]
mod package_lock;
mod packages;

#[cfg(target_os = "linux")]
pub use package_lock::PackageLock;
pub use packages::{CommandReference, LeaseError, PackageLeases};
