//! Capture verified owned files before deletion; unknown prefix content is preserved.

use super::{filesystem as paths, receipt, Receipt};
use crate::{
    open_bundle::{reader, Payload},
    require, Error, Result,
};
use std::{
    collections::BTreeMap,
    fs::{self, File, Metadata},
    os::unix::fs::MetadataExt,
    path::Path,
    sync::atomic::AtomicBool,
};

/// Remove a whole user/raw bundle closure, preserving unrecorded prefix content.
///
/// Refuses root, modified or missing payloads, unsafe paths and a busy directory
/// lock. Verified files are captured with no-overwrite renames, then rechecked
/// before deletion. Pre-commit failures restore captured files; if another
/// writer blocks restoration, the error identifies a retained recovery directory.
/// Runtime references are not detected; the caller must stop users of the prefix.
pub fn uninstall(prefix: &Path, cancel: &AtomicBool) -> Result<()> {
    uninstall_using(prefix, cancel, |_, _| Ok(()))
}

fn uninstall_using(
    prefix: &Path,
    cancel: &AtomicBool,
    mut after_capture: impl FnMut(&str, &File) -> Result<()>,
) -> Result<()> {
    paths::ordinary_user()?;
    paths::cancelled(cancel)?;
    let (parent, name) = paths::location(prefix)?;
    let root = paths::directory(&parent, &name)?;
    paths::lock(&root)?;
    paths::still_named(&parent, &name, &root)?;
    let record = receipt::read(&root)?;
    require(
        record.scope.uid == paths::uid(),
        "installation belongs to a different user",
    )?;
    let (files, directories) = preflight(&root, &record, cancel)?;
    let record_bytes = reader::document(&root, receipt::NAME, receipt::LIMIT)?;
    require(
        serde_json::from_slice::<Receipt>(&record_bytes)? == record,
        "installation receipt changed during preflight",
    )?;
    let receipt_metadata = reader::payload(&root, receipt::NAME)?.metadata()?;
    paths::cancelled(cancel)?;
    let stage = tempfile::Builder::new()
        .prefix(".aw-uninstall-")
        .tempdir_in(paths::descriptor(&parent))?;
    let stage_name = stage
        .path()
        .file_name()
        .ok_or_else(|| Error::Invalid("temporary uninstall directory has no name".into()))?
        .to_owned();
    let stage_root = paths::directory(&parent, &stage_name)?;
    let mut captured = Vec::new();
    let result = (|| {
        for package in &record.packages {
            for (relative, expected) in &package.files {
                paths::cancelled(cancel)?;
                capture(&root, &stage_root, relative, &mut captured)?;
                after_capture(relative, &stage_root)?;
                let mut file = reader::payload(&stage_root, relative)?;
                require(
                    paths::same_payload(&files[relative], &file.metadata()?),
                    format!("owned file changed during uninstall: {relative:?}"),
                )?;
                reader::verify(&mut file, expected, relative)?;
            }
        }
        paths::still_named(&parent, &name, &root)?;
        capture(&root, &stage_root, receipt::NAME, &mut captured)?;
        require(
            paths::same_payload(
                &receipt_metadata,
                &reader::payload(&stage_root, receipt::NAME)?.metadata()?,
            ),
            "installation receipt was replaced during uninstall",
        )?;
        require(
            reader::document(&stage_root, receipt::NAME, receipt::LIMIT)? == record_bytes,
            "installation receipt changed during uninstall",
        )?;
        paths::cancelled(cancel)?;
        Ok(())
    })();
    if let Err(error) = result {
        let failures = restore(&root, &stage_root, &captured);
        if failures.is_empty() {
            return Err(error);
        }
        let _retained = stage.keep();
        let recovery = prefix
            .parent()
            .ok_or_else(|| Error::Invalid("installation parent disappeared".into()))?
            .join(stage_name);
        return Err(Error::Invalid(format!(
            "{error}; rollback could not restore {} path(s); recover retained files from {}",
            failures.len(),
            recovery.display()
        )));
    }
    // Only this invocation's private stage is recursively removed. Unrecorded
    // prefix paths were never moved and cannot be reached by this cleanup.
    let staged_path = stage.keep();
    if let Err(error) = fs::remove_dir_all(&staged_path) {
        let recovery = prefix
            .parent()
            .ok_or_else(|| Error::Invalid("installation parent disappeared".into()))?
            .join(stage_name);
        return Err(Error::Invalid(format!("owned files were removed from the prefix, but staged cleanup failed: {error}; retained directory: {}", recovery.display())));
    }
    prune_empty(&root, &directories);
    root.sync_all()?;
    if paths::still_named(&parent, &name, &root).is_ok() {
        match fs::remove_dir(paths::descriptor(&parent).join(&name)) {
            Ok(()) => (),
            Err(error) if matches!(error.raw_os_error(), Some(libc::ENOTEMPTY | libc::EEXIST)) => {}
            Err(error) => return Err(error.into()),
        }
    }
    parent.sync_all()?;
    Ok(())
}

type Entries = BTreeMap<String, Metadata>;

fn preflight(root: &File, receipt: &Receipt, cancel: &AtomicBool) -> Result<(Entries, Entries)> {
    let mut files = BTreeMap::new();
    let mut directories = BTreeMap::new();
    for package in &receipt.packages {
        for (relative, expected) in &package.files {
            paths::cancelled(cancel)?;
            let mut file = reader::payload(root, relative)?;
            verify_owned(&mut file, relative, expected)?;
            files.insert(relative.clone(), file.metadata()?);
            for path in Path::new(relative)
                .ancestors()
                .skip(1)
                .filter(|path| !path.as_os_str().is_empty())
            {
                let label = path
                    .to_str()
                    .ok_or_else(|| Error::Invalid("invalid recorded directory".into()))?;
                if !directories.contains_key(label) {
                    let (parent, name) = paths::parent_for(root, label, false)?;
                    directories.insert(label.into(), paths::directory(&parent, &name)?.metadata()?);
                }
            }
        }
    }
    Ok((files, directories))
}

fn verify_owned(file: &mut File, relative: &str, expected: &Payload) -> Result<()> {
    let metadata = file.metadata()?;
    require(
        metadata.uid() == paths::uid() && metadata.mode() & 0o7777 == expected.mode,
        format!("owned payload identity or mode changed: {relative:?}"),
    )?;
    reader::verify(file, expected, relative)
}

fn capture(root: &File, stage: &File, relative: &str, captured: &mut Vec<String>) -> Result<()> {
    let (from, name) = paths::parent_for(root, relative, false)?;
    let (to, target) = paths::parent_for(stage, relative, true)?;
    paths::rename(&from, &name, &to, &target)?;
    // Record immediately, including an unexpectedly substituted file: rollback
    // must preserve it rather than letting stage cleanup delete unknown content.
    captured.push(relative.into());
    Ok(())
}

fn restore(root: &File, stage: &File, captured: &[String]) -> Vec<String> {
    let mut failures = vec![];
    for relative in captured.iter().rev() {
        let result = (|| {
            let (from, name) = paths::parent_for(stage, relative, false)?;
            let (to, target) = paths::parent_for(root, relative, false)?;
            paths::rename(&from, &name, &to, &target)
        })();
        if result.is_err() {
            failures.push(relative.clone());
        }
    }
    failures
}

fn prune_empty(root: &File, directories: &Entries) {
    for (relative, expected) in directories.iter().rev() {
        let result = (|| -> Result<()> {
            let (parent, name) = paths::parent_for(root, relative, false)?;
            let current = paths::directory(&parent, &name)?;
            if paths::same_object(expected, &current.metadata()?) {
                // rmdir never removes nonempty or symlink-substituted directories.
                fs::remove_dir(paths::descriptor(&parent).join(name))?;
            }
            Ok(())
        })();
        // Empty directory cleanup is optional; unrecorded content stays intact.
        let _retained_directory = result;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{path::PathBuf, sync::atomic::Ordering};

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/open-uninstall");
        fs::create_dir_all(&target).unwrap();
        let directory = tempfile::tempdir_in(target).unwrap();
        let root = directory.path().canonicalize().unwrap();
        let bundle = root.join("bundle");
        let prefix = root.join("prefix");
        fs::create_dir_all(bundle.join("payload/bin")).unwrap();
        fs::write(bundle.join("payload/bin/mant"), b"owned bytes").unwrap();
        let manifest = serde_json::json!({
            "format": 2,
            "packages": [{
                "id": "example.mant", "version": "2.0", "kind": "tool",
                "platform": {"os":"linux", "arch":std::env::consts::ARCH},
                "files": {"bin/mant":{"size":11,"sha256":format!("{:x}",Sha256::digest(b"owned bytes")),"mode":493}}
            }]
        });
        fs::write(
            bundle.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        crate::open_install::install(&bundle, &prefix, &AtomicBool::new(false)).unwrap();
        (directory, prefix)
    }

    #[test]
    fn cancellation_after_capture_restores_receipt_and_owned_payloads() {
        let (_directory, prefix) = fixture();
        let cancel = AtomicBool::new(false);
        let result = uninstall_using(&prefix, &cancel, |_, _| {
            cancel.store(true, Ordering::Relaxed);
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(fs::read(prefix.join("bin/mant")).unwrap(), b"owned bytes");
        assert_eq!(
            crate::open_install::inspect(&prefix)
                .unwrap()
                .unhealthy_components,
            0
        );
        assert!(!fs::read_dir(prefix.parent().unwrap())
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".aw-uninstall-")));
    }

    #[test]
    fn blocked_rollback_preserves_new_user_file_and_retains_captured_owned_file() {
        let (_directory, prefix) = fixture();
        let result = uninstall_using(&prefix, &AtomicBool::new(false), |relative, _| {
            fs::write(prefix.join(relative), b"concurrent user content")?;
            Err(Error::Invalid("injected capture failure".into()))
        });
        let error = result.unwrap_err().to_string();
        assert!(error.contains("rollback could not restore"));
        assert_eq!(
            fs::read(prefix.join("bin/mant")).unwrap(),
            b"concurrent user content"
        );
        assert!(prefix.join(receipt::NAME).exists());
        let retained = fs::read_dir(prefix.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".aw-uninstall-")
            })
            .unwrap();
        assert_eq!(fs::read(retained.join("bin/mant")).unwrap(), b"owned bytes");
    }

    #[test]
    fn captured_file_changes_are_rechecked_and_restored_without_deletion() {
        let (_directory, prefix) = fixture();
        let result = uninstall_using(&prefix, &AtomicBool::new(false), |relative, stage| {
            fs::write(
                paths::descriptor(stage).join(relative),
                b"changed while captured",
            )?;
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(
            fs::read(prefix.join("bin/mant")).unwrap(),
            b"changed while captured"
        );
        assert!(prefix.join(receipt::NAME).exists());
    }
}
