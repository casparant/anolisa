//! Read-only installation receipt summaries, independent of payload or runtime checks.

use crate::cli::Result;
use aw_package::{Inventory, InventoryState};
use serde_json::json;
use std::path::Path;

const NOTICE: &str =
    "Installation records only; payload integrity and runtime activation are not checked.";

/// Present bounded installation metadata without creating or changing a prefix.
pub(crate) fn dispatch(arguments: &[String]) -> Result<()> {
    if arguments.first().map(String::as_str) == Some("--help") {
        if arguments.len() != 1 {
            return Err("unexpected arguments after --help".into());
        }
        println!(
            "aw list --installed --prefix ABSOLUTE_DIR [--json]\n\
Read existing AW package receipts. No software is started or repaired.\n{NOTICE}"
        );
        return Ok(());
    }
    let mut installed = false;
    let mut machine = false;
    let mut prefix = None;
    let mut arguments = arguments.iter();
    while let Some(flag) = arguments.next() {
        match flag.as_str() {
            "--installed" if !installed => installed = true,
            "--json" if !machine => machine = true,
            "--prefix" if prefix.is_none() => {
                prefix = Some(
                    arguments
                        .next()
                        .filter(|value| !value.is_empty() && !value.starts_with("--"))
                        .ok_or("--prefix requires an absolute directory path")?,
                );
            }
            _ => return Err("unknown or duplicate list option".into()),
        }
    }
    if !installed {
        return Err("list requires --installed; source catalogs are not inspected".into());
    }
    let prefix = Path::new(prefix.ok_or("missing --prefix")?);
    if !prefix.is_absolute() {
        return Err("--prefix requires an absolute directory path".into());
    }
    let inventory = aw_package::inventory(prefix).map_err(|error| safe_text(&error.to_string()))?;
    if machine {
        let components = inventory
            .components
            .iter()
            .map(|component| {
                json!({
                    "name": component.component,
                    "version": component.version,
                    "os": component.os,
                    "arch": component.arch,
                    "source_commit": component.source_commit,
                    "file_count": component.files.len(),
                    "requires": component.requires,
                })
            })
            .collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "scope": "installation_records",
                "prefix": inventory.prefix,
                "state": inventory.state,
                "payload_verified": false,
                "runtime_checked": false,
                "components": components,
                "notice": NOTICE,
            }))?
        );
    } else {
        present(&inventory);
    }
    Ok(())
}

fn present(inventory: &Inventory) {
    println!("{NOTICE}");
    println!("Prefix: {}", safe_text(&inventory.prefix.to_string_lossy()));
    match inventory.state {
        InventoryState::Absent => {
            println!("State: absent\nThe prefix does not exist. No installation records found.");
        }
        InventoryState::Unmanaged => {
            println!(
                "State: unmanaged\nNo AW installation records found; existing files were not inspected."
            );
        }
        InventoryState::Installed => {
            println!("State: installed (recorded)\n\nNAME\tVERSION\tPLATFORM\tFILES");
            for component in &inventory.components {
                println!(
                    "{}\t{}\t{}/{}\t{}",
                    safe_text(&component.component),
                    safe_text(&component.version),
                    safe_text(&component.os),
                    safe_text(&component.arch),
                    component.files.len(),
                );
                println!("  Source: {}", safe_text(&component.source_commit));
                if !component.requires.is_empty() {
                    let dependencies = component
                        .requires
                        .iter()
                        .map(|(name, version)| {
                            format!("{}={}", safe_text(name), safe_text(version))
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!("  Requires: {dependencies}");
                }
            }
        }
    }
}

fn safe_text(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| {
            // Receipt metadata is not trusted terminal formatting or directionality.
            if character.is_control()
                || matches!(character, '\u{061c}' | '\u{200e}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            {
                character.escape_default().collect::<Vec<_>>()
            } else {
                vec![character]
            }
        })
        .collect()
}
