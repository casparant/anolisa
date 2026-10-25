//! Probe the actual destination filesystems without publishing package payloads.

use super::{Bundle, ReleaseLayout, Result, files, require};
use std::path::Path;

pub(super) fn check(layout: &ReleaseLayout, bundle: &Bundle) -> Result<()> {
    let mut program_bytes = 0u64;
    let mut resource_bytes = 0u64;
    let mut resource_files = 0u64;
    for (reference, package) in bundle.manifest.packages.iter().zip(&bundle.packages) {
        for name in package.files.keys() {
            let path = bundle.root.join(&reference.path).join("payload").join(name);
            let bytes = files::regular(&path)?.len();
            if name.starts_with("share/") {
                resource_bytes = resource_bytes.saturating_add(bytes);
                resource_files += 1;
            } else {
                program_bytes = program_bytes.saturating_add(bytes);
            }
        }
    }
    destination(
        &layout.prefix,
        program_bytes
            .saturating_mul(2)
            .saturating_add(128 * 1024 * 1024),
    )?;
    destination(
        &layout.resources,
        resource_bytes
            .saturating_mul(2)
            .saturating_add(1024 * 1024)
            .saturating_add(resource_files.saturating_mul(4096)),
    )?;
    destination(&layout.package_database, 128 * 1024 * 1024)
}

fn destination(path: &Path, needed: u64) -> Result<()> {
    let ancestor = path
        .ancestors()
        .find(|candidate| candidate.is_dir())
        .ok_or_else(|| {
            super::Error::Invalid("release destination has no existing directory".into())
        })?;
    files::owned(ancestor)?;
    let stat = nix::sys::statvfs::statvfs(ancestor)
        .map_err(|error| files::io(ancestor, std::io::Error::from_raw_os_error(error as i32)))?;
    require(
        u128::from(stat.blocks_available()).saturating_mul(u128::from(stat.fragment_size()))
            >= u128::from(needed),
        format!(
            "insufficient free space at {}: need conservative {needed}-byte staging reserve",
            ancestor.display()
        ),
    )?;
    probe(ancestor)
}

#[cfg(target_os = "linux")]
fn probe(parent: &Path) -> Result<()> {
    use std::{
        ffi::CString,
        fs::{self, File},
        os::fd::{AsRawFd, FromRawFd},
        os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    };
    let temporary = tempfile::Builder::new()
        .prefix(".anolisa-fs-check-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(parent)
        .map_err(|error| files::io(parent, error))?;
    let root = temporary.path();
    let directory = File::open(root).map_err(|error| files::io(root, error))?;
    // SAFETY: the directory fd remains open and the literal is NUL terminated.
    let descriptor = unsafe {
        nix::libc::openat(
            directory.as_raw_fd(),
            c".".as_ptr(),
            nix::libc::O_TMPFILE | nix::libc::O_RDWR | nix::libc::O_CLOEXEC,
            0o600,
        )
    };
    if descriptor < 0 {
        return Err(files::io(parent, std::io::Error::last_os_error()));
    }
    // SAFETY: openat returned a newly owned descriptor.
    let anonymous = unsafe { File::from_raw_fd(descriptor) };
    anonymous
        .sync_all()
        .map_err(|error| files::io(root, error))?;
    let source = CString::new(format!("/proc/self/fd/{}", anonymous.as_raw_fd()))
        .map_err(|_| super::Error::Invalid("invalid descriptor path".into()))?;
    // SAFETY: both descriptors and the C string stay alive for the synchronous linkat call.
    if unsafe {
        nix::libc::linkat(
            nix::libc::AT_FDCWD,
            source.as_ptr(),
            directory.as_raw_fd(),
            c"anonymous".as_ptr(),
            nix::libc::AT_SYMLINK_FOLLOW,
        )
    } != 0
    {
        return Err(files::io(parent, std::io::Error::last_os_error()));
    }
    let left = root.join("left");
    let right = root.join("right");
    fs::write(&left, b"left").map_err(|error| files::io(&left, error))?;
    fs::hard_link(&left, root.join("linked")).map_err(|error| files::io(root, error))?;
    fs::write(&right, b"right").map_err(|error| files::io(&right, error))?;
    let rename = |source: &Path, target: &Path, flags| -> Result<()> {
        let source = CString::new(source.as_os_str().as_bytes())
            .map_err(|_| super::Error::Invalid("NUL in filesystem probe path".into()))?;
        let target = CString::new(target.as_os_str().as_bytes())
            .map_err(|_| super::Error::Invalid("NUL in filesystem probe path".into()))?;
        // SAFETY: both C strings stay alive for this synchronous renameat2 call.
        if unsafe {
            nix::libc::renameat2(
                nix::libc::AT_FDCWD,
                source.as_ptr(),
                nix::libc::AT_FDCWD,
                target.as_ptr(),
                flags,
            )
        } != 0
        {
            return Err(files::io(parent, std::io::Error::last_os_error()));
        }
        Ok(())
    };
    rename(&left, &right, nix::libc::RENAME_EXCHANGE)?;
    rename(&left, &root.join("moved"), nix::libc::RENAME_NOREPLACE)?;
    files::sync(root)?;
    drop(directory);
    drop(anonymous);
    temporary
        .close()
        .map_err(|error| files::io(parent, error))?;
    files::sync(parent)
}

#[cfg(not(target_os = "linux"))]
fn probe(_parent: &Path) -> Result<()> {
    // Only injected unit backends reach this on macOS; public install rejects non-Linux.
    Ok(())
}
