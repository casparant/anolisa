//! Descriptor-anchored mutations of exclusively owned user prefixes.

use crate::{open_bundle::reader, require, Error, Result};
use std::{
    ffi::{CString, OsStr, OsString},
    fs::{self, File, Metadata, OpenOptions},
    os::{
        fd::AsRawFd,
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        },
    },
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) fn uid() -> u32 {
    // SAFETY: geteuid only reads the process identity.
    unsafe { libc::geteuid() }
}

pub(super) fn ordinary_user() -> Result<()> {
    require(
        uid() != 0,
        "format 2 raw installation requires a non-root user",
    )
}

pub(super) fn descriptor(directory: &File) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()))
}

pub(super) fn owned(directory: &File) -> Result<()> {
    let metadata = directory.metadata()?;
    require(
        metadata.is_dir() && metadata.uid() == uid() && metadata.mode() & 0o022 == 0,
        "installation directory must belong to the current user and not be group/world writable",
    )
}

pub(super) fn location(prefix: &Path) -> Result<(File, OsString)> {
    require(prefix.is_absolute(), "installation prefix must be absolute")?;
    let parent_path = prefix
        .parent()
        .ok_or_else(|| Error::Invalid("installation prefix has no parent".into()))?;
    let name = prefix
        .file_name()
        .ok_or_else(|| Error::Invalid("installation prefix has no final name".into()))?;
    let parent = reader::root(parent_path)?;
    owned(&parent)?;
    Ok((parent, name.into()))
}

pub(super) fn absent(parent: &File, name: &OsStr) -> Result<()> {
    match fs::symlink_metadata(descriptor(parent).join(name)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
        Ok(_) => Err(Error::Invalid(
            "format 2 requires a new prefix; existing prefixes are never overwritten or adopted"
                .into(),
        )),
    }
}

pub(super) fn directory(parent: &File, name: &OsStr) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(descriptor(parent).join(name))?;
    owned(&file)?;
    Ok(file)
}

pub(super) fn lock(directory: &File) -> Result<()> {
    // SAFETY: the borrowed descriptor remains open for this nonblocking operation.
    if unsafe { libc::flock(directory.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

pub(super) fn parent_for(root: &File, relative: &str, create: bool) -> Result<(File, OsString)> {
    crate::open_bundle::regular_relative(relative)?;
    let mut directory = root.try_clone()?;
    let mut parts = relative.split('/').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return Ok((directory, part.into()));
        }
        if create {
            match fs::DirBuilder::new()
                .mode(0o700)
                .create(descriptor(&directory).join(part))
            {
                Ok(()) => (),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(error) => return Err(error.into()),
            }
        }
        directory = self::directory(&directory, part.as_ref())?;
    }
    Err(Error::Invalid("empty installation path".into()))
}

pub(super) fn create(root: &File, relative: &str) -> Result<File> {
    let (parent, name) = parent_for(root, relative, true)?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(descriptor(&parent).join(name))?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

pub(super) fn rename(from: &File, from_name: &OsStr, to: &File, to_name: &OsStr) -> Result<()> {
    let from_name = CString::new(from_name.as_bytes())
        .map_err(|_| Error::Invalid("invalid source filename".into()))?;
    let to_name = CString::new(to_name.as_bytes())
        .map_err(|_| Error::Invalid("invalid destination filename".into()))?;
    // SAFETY: descriptors and NUL-terminated names remain valid throughout the syscall.
    // RENAME_NOREPLACE is required: unsupported filesystems fail without a fallback.
    if unsafe {
        libc::renameat2(
            from.as_raw_fd(),
            from_name.as_ptr(),
            to.as_raw_fd(),
            to_name.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

pub(super) fn same_object(before: &Metadata, after: &Metadata) -> bool {
    before.dev() == after.dev() && before.ino() == after.ino()
}

pub(super) fn same_payload(before: &Metadata, after: &Metadata) -> bool {
    same_object(before, after)
        && before.len() == after.len()
        && before.mode() == after.mode()
        && before.uid() == after.uid()
        && before.gid() == after.gid()
        && before.nlink() == after.nlink()
        && before.mtime() == after.mtime()
        && before.mtime_nsec() == after.mtime_nsec()
}

pub(super) fn still_named(parent: &File, name: &OsStr, directory: &File) -> Result<()> {
    let current = self::directory(parent, name)?;
    require(
        same_object(&current.metadata()?, &directory.metadata()?),
        "installation prefix changed; retry with its current location",
    )
}

pub(super) fn cancelled(cancel: &AtomicBool) -> Result<()> {
    require(
        !cancel.load(Ordering::Relaxed),
        "installation operation cancelled",
    )
}
