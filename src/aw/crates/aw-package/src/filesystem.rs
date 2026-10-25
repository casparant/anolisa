//! Filesystem compatibility helpers shared with the local installation engine.

#[cfg(test)]
pub(crate) use local_package::compatibility::lock;
pub(crate) use local_package::compatibility::{
    absolute, owned_directory, parent, shared_lock, write_private,
};
