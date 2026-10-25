//! Prevent the standalone CLI updater from modifying AW-owned payloads.

use std::fs;
use std::path::Path;

use super::SelfUpdateError;

/// Reject direct CLI replacement inside an AW installation.
///
/// Malformed or incomplete AW records still block the standalone updater:
/// repairing package ownership is the package manager's responsibility.
///
/// # Errors
/// Rejects unresolved executable paths, unreadable ownership evidence, and
/// any ancestor containing an AW registration directory or installation lock.
pub fn ensure_standalone_executable(executable: &Path) -> Result<(), SelfUpdateError> {
    let actual = fs::canonicalize(executable).map_err(|source| SelfUpdateError::Io {
        path: executable.to_path_buf(),
        source,
    })?;
    for prefix in actual.parent().into_iter().flat_map(Path::ancestors) {
        for name in [".aw-packages", ".aw-install.lock"] {
            let marker = prefix.join(name);
            match fs::symlink_metadata(&marker) {
                Ok(_) => {
                    return Err(SelfUpdateError::ManagedExecutable {
                        executable: actual.clone(),
                        prefix: prefix.to_path_buf(),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(SelfUpdateError::Io {
                        path: marker,
                        source,
                    });
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::self_update::{ReleaseArtifact, perform_update, self_update_lock_path};

    #[test]
    fn standalone_executable_is_allowed() {
        let temporary = tempfile::tempdir().unwrap();
        let executable = temporary.path().join("anolisa");
        fs::write(&executable, b"standalone").unwrap();
        ensure_standalone_executable(&executable).unwrap();
    }

    #[test]
    fn registration_and_partial_installation_block_download_and_writes() {
        for marker in [".aw-packages", ".aw-install.lock"] {
            let temporary = tempfile::tempdir().unwrap();
            let prefix = temporary.path().join("release");
            fs::create_dir_all(prefix.join("bin")).unwrap();
            let executable = prefix.join("bin/anolisa");
            fs::write(&executable, b"managed").unwrap();
            fs::write(prefix.join(marker), b"malformed ownership evidence").unwrap();
            let artifact = ReleaseArtifact {
                os: "linux".into(),
                arch: "x86_64".into(),
                url: "http://127.0.0.1:1/must-not-download".into(),
                sha256: "0".repeat(64),
                size: Some(1),
            };
            assert!(matches!(
                perform_update(&artifact, &executable, None),
                Err(SelfUpdateError::ManagedExecutable { .. })
            ));
            assert!(!self_update_lock_path(&executable).exists());
            assert_eq!(fs::read(&executable).unwrap(), b"managed");
        }
    }

    #[test]
    fn a_public_symlink_does_not_bypass_registration() {
        let temporary = tempfile::tempdir().unwrap();
        let prefix = temporary.path().join("release");
        fs::create_dir_all(prefix.join("bin")).unwrap();
        fs::create_dir(prefix.join(".aw-packages")).unwrap();
        let executable = prefix.join("bin/anolisa");
        fs::write(&executable, b"managed").unwrap();
        let public = temporary.path().join("anolisa");
        std::os::unix::fs::symlink(&executable, &public).unwrap();
        assert!(matches!(
            ensure_standalone_executable(&public),
            Err(SelfUpdateError::ManagedExecutable { .. })
        ));
    }
}
