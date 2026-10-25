//! Installer regression contracts, including source-open and rollback failures.

use super::*;
use crate::{digest, Component, Manifest, Payload};
use std::os::unix::fs::{symlink, MetadataExt};

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    prefix: PathBuf,
    state: PathBuf,
    core: PathBuf,
    provider: PathBuf,
    combined: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/preview");
        fs::create_dir_all(&target).unwrap();
        let directory = tempfile::Builder::new()
            .prefix("install-test-")
            .tempdir_in(target)
            .unwrap();
        let root = directory.path().canonicalize().unwrap();
        fs::set_permissions(&root, Permissions::from_mode(0o700)).unwrap();
        let core = bundle(&root.join("core"), &["aw-core"]);
        let provider = bundle(&root.join("provider"), &["aw-provider-sec-core"]);
        let combined = bundle(&root.join("combined"), &["aw-core", "aw-provider-sec-core"]);
        Self {
            _directory: directory,
            prefix: root.join("installed"),
            state: root.join("state"),
            root,
            core,
            provider,
            combined,
        }
    }
}
fn bundle(root: &Path, names: &[&str]) -> PathBuf {
    let mut components = Vec::new();
    for name in names {
        let entries = if *name == "aw-core" {
            vec!["bin/aw".into()]
        } else if *name == "aw-provider-tokenless" {
            ["aw-provider-tokenless", "tokenless", "rtk"]
                .iter()
                .map(|name| format!("libexec/aw/providers/tokenless/{name}"))
                .collect()
        } else {
            [
                "aw-provider-sec-core",
                "agent-sec-cli",
                "agent-sec-daemon",
                "agent-sec-daemon-health",
            ]
            .iter()
            .map(|name| format!("libexec/aw/providers/sec-core/{name}"))
            .collect()
        };
        let mut files = BTreeMap::new();
        for entry in entries {
            let path = root.join("payload").join(&entry);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, format!("fixture {entry}")).unwrap();
            files.insert(
                entry,
                Payload {
                    sha256: digest(&path).unwrap(),
                    mode: 0o755,
                },
            );
        }
        components.push(Component {
            format: 1,
            component: (*name).into(),
            version: "0.1.0-preview.1".into(),
            source_commit: "a".repeat(40),
            os: "linux".into(),
            arch: std::env::consts::ARCH.into(),
            provider_protocol: "aw-provider/v1alpha1".into(),
            requires: if *name == "aw-core" {
                BTreeMap::new()
            } else {
                BTreeMap::from([("aw-core".into(), "0.1.0-preview.1".into())])
            },
            files,
        });
    }
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec(&Manifest {
            format: 1,
            components,
        })
        .unwrap(),
    )
    .unwrap();
    root.into()
}
mod configuration;
mod standalone;
