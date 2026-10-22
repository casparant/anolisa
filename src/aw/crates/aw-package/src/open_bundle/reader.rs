//! Descriptor-relative reads reject links, special files and concurrent changes.

use super::{regular_relative, Payload, FILE_LIMIT};
use crate::{require, Error, Result};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, Metadata, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path, PathBuf},
};

pub(crate) fn root(path: &Path) -> Result<File> {
    require(
        path.is_absolute()
            && !path
                .components()
                .any(|part| matches!(part, Component::ParentDir)),
        "bundle directory must be absolute and contain no '..'",
    )?;
    let mut directory = File::open("/")?;
    for component in path.components() {
        if let Component::Normal(name) = component {
            directory = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(descriptor_path(&directory).join(name))?;
        }
    }
    safe(&directory.metadata()?, true)?;
    Ok(directory)
}

pub(crate) fn payload(root: &File, relative: &str) -> Result<File> {
    regular_relative(relative)?;
    safe(&root.metadata()?, true)?;
    let mut directory = root.try_clone()?;
    let mut parts = relative.split('/').peekable();
    while let Some(part) = parts.next() {
        let is_directory = parts.peek().is_some();
        let flags =
            libc::O_NOFOLLOW | libc::O_NONBLOCK | if is_directory { libc::O_DIRECTORY } else { 0 };
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(flags)
            .open(descriptor_path(&directory).join(part))?;
        safe(&file.metadata()?, is_directory)?;
        if !is_directory {
            return Ok(file);
        }
        directory = file;
    }
    Err(Error::Invalid("empty bundle payload path".into()))
}

pub(crate) fn document(root: &File, relative: &str, limit: usize) -> Result<Vec<u8>> {
    let mut file = payload(root, relative)?;
    let before = file.metadata()?;
    require(
        before.len() <= limit as u64,
        "bundle metadata exceeds byte limit",
    )?;
    let mut bytes = Vec::new();
    (&mut file).take(limit as u64 + 1).read_to_end(&mut bytes)?;
    require(
        bytes.len() <= limit && bytes.len() as u64 == before.len(),
        "bundle metadata size changed or exceeds byte limit",
    )?;
    unchanged(&before, &file.metadata()?, relative)?;
    Ok(bytes)
}

pub(crate) fn verify(source: &mut File, expected: &Payload, label: &str) -> Result<()> {
    copy_to(source, &mut std::io::sink(), expected, label)
}

pub(crate) fn copy_verified(
    source: &mut File,
    target: &mut File,
    expected: &Payload,
    label: &str,
) -> Result<()> {
    copy_to(source, target, expected, label)
}

fn copy_to(
    source: &mut File,
    target: &mut impl Write,
    expected: &Payload,
    label: &str,
) -> Result<()> {
    let before = source.metadata()?;
    safe(&before, false)?;
    require(
        expected.size <= FILE_LIMIT && before.len() == expected.size,
        format!("payload size mismatch or limit exceeded: {label:?}"),
    )?;
    source.seek(SeekFrom::Start(0))?;
    let mut remaining = expected.size;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65536];
    while remaining != 0 {
        let limit = remaining.min(buffer.len() as u64) as usize;
        let count = source.read(&mut buffer[..limit])?;
        require(
            count != 0,
            format!("payload truncated while reading: {label:?}"),
        )?;
        hash.update(&buffer[..count]);
        target.write_all(&buffer[..count])?;
        remaining -= count as u64;
    }
    require(
        source.read(&mut buffer[..1])? == 0,
        format!("payload grew while reading: {label:?}"),
    )?;
    unchanged(&before, &source.metadata()?, label)?;
    require(
        format!("{:x}", hash.finalize()) == expected.sha256,
        format!("payload checksum mismatch: {label:?}"),
    )
}

fn descriptor_path(directory: &File) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()))
}

fn safe(metadata: &Metadata, directory: bool) -> Result<()> {
    // SAFETY: geteuid only reads the process identity.
    let uid = unsafe { libc::geteuid() };
    let expected_type = if directory {
        metadata.is_dir()
    } else {
        metadata.is_file() && metadata.nlink() == 1
    };
    require(
        expected_type
            && (metadata.uid() == uid || metadata.uid() == 0)
            && metadata.mode() & 0o022 == 0,
        "bundle data must be regular, singly linked, owned by the current user or root, and not group/world writable",
    )
}

fn unchanged(before: &Metadata, after: &Metadata, label: &str) -> Result<()> {
    safe(after, false)?;
    require(
        before.dev() == after.dev()
            && before.ino() == after.ino()
            && before.len() == after.len()
            && before.uid() == after.uid()
            && before.gid() == after.gid()
            && before.mode() == after.mode()
            && before.nlink() == after.nlink()
            && before.mtime() == after.mtime()
            && before.mtime_nsec() == after.mtime_nsec()
            && before.ctime() == after.ctime()
            && before.ctime_nsec() == after.ctime_nsec(),
        format!("bundle data changed while reading: {label:?}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};

    struct MutatingWriter {
        source: PathBuf,
        grow: bool,
    }

    impl Write for MutatingWriter {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            if self.grow {
                OpenOptions::new()
                    .append(true)
                    .open(&self.source)?
                    .write_all(b"extra")?;
            } else {
                fs::set_permissions(&self.source, fs::Permissions::from_mode(0o444))?;
            }
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn mutation_case(grow: bool) -> Result<String> {
        let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/open-reader");
        fs::create_dir_all(&target)?;
        let directory = tempfile::tempdir_in(target)?;
        let path = directory.path().join("source");
        let bytes = b"verified bytes";
        fs::write(&path, bytes)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644))?;
        let mut source = File::open(&path)?;
        let expected = Payload {
            size: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
            mode: 0o644,
        };
        let mut writer = MutatingWriter { source: path, grow };
        Ok(copy_to(&mut source, &mut writer, &expected, "source")
            .unwrap_err()
            .to_string())
    }

    #[test]
    fn metadata_mutation_is_rejected_even_when_content_hash_matches() {
        assert!(mutation_case(false)
            .unwrap()
            .contains("changed while reading"));
    }

    #[test]
    fn source_growth_is_rejected_without_reading_beyond_declared_budget() {
        assert!(mutation_case(true).unwrap().contains("grew while reading"));
    }
}
