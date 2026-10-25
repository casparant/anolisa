//! Compatibility entrypoint for distribution-owned Preview maintainer tooling.

use std::{os::unix::process::CommandExt, path::PathBuf, process::Command};
#[path = "support/arguments.rs"]
mod arguments;

fn main() {
    if let Err(error) = run() {
        eprintln!("aw-build: {error}");
        std::process::exit(1);
    }
}

fn run() -> aw_package::Result<()> {
    let arguments = arguments::decode_all(std::env::args_os().skip(1))?;
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../distribution/tooling/aw-preview/Cargo.toml");
    if !manifest.is_file() {
        return Err(aw_package::Error::Invalid(
            "Preview builds belong to distribution/tooling/aw-preview; use a complete source checkout".into(),
        ));
    }
    Err(Command::new("cargo")
        .args(["run", "--locked", "--release", "--manifest-path"])
        .arg(manifest)
        .args(["--bin", "anolisa-aw-preview", "--"])
        .args(arguments)
        .exec()
        .into())
}
