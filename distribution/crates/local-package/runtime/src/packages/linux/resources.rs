//! Bind separately installed resources to the same prefix lock without reading package schemas.

use super::{directory, exists, io, metadata, open_at, safe, same, LeaseError, Result};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, Metadata},
    io::Read,
    os::unix::{ffi::OsStringExt, fs::MetadataExt},
    path::{Component, Path, PathBuf},
};

const FORWARD: &str = ".aw-installation-prefix";
const REVERSE: &str = ".aw-resources-dir";
const MAX_POINTER_BYTES: u64 = 4096;

struct Pointer {
    root: PathBuf,
    name: &'static str,
    directory: Metadata,
    file: Metadata,
    bytes: Vec<u8>,
    target: PathBuf,
}

impl Pointer {
    fn read(root: &Path, name: &'static str) -> Result<Self> {
        let directory = directory(root)?;
        let owner = metadata(&directory, root)?;
        let path = root.join(name);
        let mut file = open_at(&directory, OsStr::new(name), false, &path)?;
        let before = metadata(&file, &path)?;
        if !before.is_file()
            || before.nlink() != 1
            || before.uid() != owner.uid()
            || before.mode() & 0o7777 != 0o644
            || !safe(&before, false)
            || !(1..=MAX_POINTER_BYTES).contains(&before.len())
        {
            return Err(LeaseError::Unsafe(path));
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_POINTER_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| io(&path, error))?;
        if bytes.len() as u64 != before.len() || !same(&before, &metadata(&file, &path)?) {
            return Err(LeaseError::Changed(path));
        }
        let target = PathBuf::from(OsString::from_vec(bytes.clone()));
        if !target.is_absolute()
            || target
                .components()
                .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
            || fs::canonicalize(&target)
                .map_err(|error| io(&target, error))?
                .as_os_str()
                != target.as_os_str()
        {
            return Err(LeaseError::Unsafe(path));
        }
        Ok(Self {
            root: root.to_owned(),
            name,
            directory: owner,
            file: before,
            bytes,
            target,
        })
    }

    fn matches(&self, fresh: &Self) -> bool {
        self.root == fresh.root
            && self.name == fresh.name
            && same(&self.directory, &fresh.directory)
            && same(&self.file, &fresh.file)
            && self.bytes == fresh.bytes
    }

    fn recheck(&self) -> Result<()> {
        if !self.matches(&Self::read(&self.root, self.name)?) {
            return Err(LeaseError::Changed(self.root.join(self.name)));
        }
        Ok(())
    }
}

pub(super) struct Reference(Pointer);

pub(super) fn discover(root: &Path) -> Result<Reference> {
    let real = fs::canonicalize(root).map_err(|error| io(root, error))?;
    Ok(Reference(Pointer::read(&real, FORWARD)?))
}

impl Reference {
    pub(super) fn prefix(&self) -> &Path {
        &self.0.target
    }
}

pub(super) fn verify(prefix: &Path, references: &[Reference]) -> Result<()> {
    let references: Vec<_> = references
        .iter()
        .filter(|reference| reference.prefix() == prefix)
        .collect();
    let reverse_path = prefix.join(REVERSE);
    if !exists(&reverse_path)? {
        return if references.is_empty() {
            // Historical combined prefixes have neither external-resource pointer.
            Ok(())
        } else {
            Err(LeaseError::Unsafe(reverse_path))
        };
    }
    let reverse = Pointer::read(prefix, REVERSE)?;
    let forward = Pointer::read(&reverse.target, FORWARD)?;
    if forward.target != prefix || forward.directory.uid() != reverse.directory.uid() {
        return Err(LeaseError::Unsafe(forward.root.join(FORWARD)));
    }
    for reference in references {
        if !reference.0.matches(&forward) {
            return Err(LeaseError::Changed(reference.0.root.join(FORWARD)));
        }
    }
    // A cooperative mutation cannot run while the caller owns the prefix lock.
    // Reopen both roots and files to reject replacements during discovery/acquisition.
    reverse.recheck()?;
    forward.recheck()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    #[test]
    fn a_replaced_resource_pointer_is_rejected_after_discovery_even_with_equal_bytes() {
        let id = fs::read_to_string("/proc/sys/kernel/random/uuid").unwrap();
        let root = Path::new("/tmp").join(format!("aw-resource-pointer-{}", id.trim()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let prefix = root.join("program");
        let resource = root.join("resources");
        for directory in [&prefix, &resource] {
            fs::create_dir(directory).unwrap();
            fs::set_permissions(directory, fs::Permissions::from_mode(0o755)).unwrap();
        }
        for (path, target) in [
            (prefix.join(REVERSE), &resource),
            (resource.join(FORWARD), &prefix),
        ] {
            fs::write(&path, target.as_os_str().as_encoded_bytes()).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        }
        let reference = discover(&resource).unwrap();
        let replacement = resource.join("replacement");
        fs::write(&replacement, prefix.as_os_str().as_encoded_bytes()).unwrap();
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o644)).unwrap();
        fs::rename(&replacement, resource.join(FORWARD)).unwrap();
        assert!(matches!(
            verify(&prefix, &[reference]),
            Err(LeaseError::Changed(_))
        ));
        fs::remove_dir_all(root).unwrap();
    }
}
