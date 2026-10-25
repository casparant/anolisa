//! AW inventory API backed by the shared package database implementation.

pub(crate) use local_package::compatibility::payload_path;
#[cfg(test)]
pub(crate) use local_package::compatibility::safe_file;
pub use local_package::database::*;
pub(crate) mod resources {
    pub(crate) use local_package::compatibility::check_resources as check;
}
