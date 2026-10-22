//! Descriptor-anchored, bounded reads for metadata-only package inspection.

use crate::{filesystem as paths, require, Result};
use std::{
    fs::{File, Metadata, OpenOptions},
    io::Read,
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path, PathBuf},
};

pub(crate) fn descriptor_path(directory: &File) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()))
}

pub(crate) fn directory(path: &Path) -> Result<Option<File>> {
    paths::absolute(path)?;
    let mut directory = File::open("/")?;
    for component in path.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        directory = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(descriptor_path(&directory).join(name))
        {
            Ok(directory) => directory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
    }
    safe_metadata(&directory.metadata()?, true)?;
    Ok(Some(directory))
}

pub(crate) fn child_directory(parent: &File, name: &str) -> Result<Option<File>> {
    let directory = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(descriptor_path(parent).join(name))
    {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    safe_metadata(&directory.metadata()?, true)?;
    Ok(Some(directory))
}

pub(crate) fn document(parent: &File, name: &std::ffi::OsStr, limit: usize) -> Result<Vec<u8>> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(descriptor_path(parent).join(name))?;
    let before = file.metadata()?;
    safe_metadata(&before, false)?;
    require(
        before.len() <= limit as u64,
        "package metadata exceeds byte limit",
    )?;
    let mut bytes = Vec::new();
    (&mut file).take(limit as u64 + 1).read_to_end(&mut bytes)?;
    require(bytes.len() <= limit, "package metadata exceeds byte limit")?;
    let after = file.metadata()?;
    require(
        before.len() == after.len()
            && before.mtime() == after.mtime()
            && before.mtime_nsec() == after.mtime_nsec()
            && before.ctime() == after.ctime()
            && before.ctime_nsec() == after.ctime_nsec(),
        "package metadata changed while reading; retry inspection",
    )?;
    Ok(bytes)
}

fn safe_metadata(metadata: &Metadata, directory: bool) -> Result<()> {
    // Root-owned metadata is readable inventory, not consent to mutate its prefix.
    // SAFETY: geteuid reads the effective identity without side effects.
    let current_user = unsafe { libc::geteuid() };
    let valid_type = if directory {
        metadata.is_dir()
    } else {
        metadata.is_file() && metadata.nlink() == 1
    };
    require(
        valid_type
            && (metadata.uid() == current_user || metadata.uid() == 0)
            && metadata.mode() & 0o022 == 0,
        "package metadata must be owned by the current user or root and not group/world writable",
    )
}
