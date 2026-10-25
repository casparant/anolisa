//! Explicit distribution templates used by compatibility acceptance fixtures.

use crate::Policy;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub(crate) fn policy(name: &str, prefix: &Path, socket: &Path) -> Policy<'static> {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../distribution/tooling/aw-preview/templates")
        .join(format!("{name}.json"));
    let definition = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    let hash = crate::digest(&prefix.join("libexec/aw/providers/tokenless/rtk"))
        .unwrap_or_else(|_| "0".repeat(64));
    Policy::Template {
        definition,
        variables: BTreeMap::from([
            ("SOCKET".into(), socket.display().to_string()),
            ("RTK_SHA256".into(), hash),
        ]),
    }
}
