//! Installation-integrity presentation; diagnostics do not probe or repair runtime state.

use crate::cli::{Exit, Result};
use aw_package::{integrity::IntegrityReport, InventoryState};
use serde::Serialize;
use std::path::Path;

const NOTICE: &str = "Read-only installation integrity. Runtime health is not checked.";

#[derive(Serialize)]
struct Report<'a> {
    scope: &'static str,
    #[serde(flatten)]
    installation: &'a IntegrityReport,
}

/// Inspect one explicit prefix without starting, repairing or installing anything.
pub(crate) fn dispatch(arguments: &[String]) -> Result<Exit> {
    if arguments.len() == 1 && arguments[0] == "--help" {
        println!("aw doctor --prefix ABS_DIR [--json]\n\n{NOTICE}");
        return Ok(Exit::Code(0));
    }
    let mut prefix = None;
    let mut machine = false;
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--prefix" if prefix.is_none() => {
                prefix = Some(
                    arguments
                        .next()
                        .filter(|value| !value.is_empty() && !value.starts_with("--"))
                        .ok_or("--prefix requires an absolute directory")?,
                );
            }
            "--json" if !machine => machine = true,
            _ => return Err("unknown or duplicate doctor option".into()),
        }
    }
    let prefix = Path::new(prefix.ok_or("missing --prefix")?);
    if !prefix.is_absolute() {
        return Err("--prefix requires an absolute directory".into());
    }
    let report = if aw_package::installed_format(prefix)
        .map_err(|error| display_text(&error.to_string()))?
        == 2
    {
        aw_package::open_install::inspect(prefix)
    } else {
        aw_package::integrity::inspect(prefix)
    }
    .map_err(|error| display_text(&error.to_string()))?;
    if machine {
        println!(
            "{}",
            serde_json::to_string_pretty(&Report {
                scope: "installation_integrity",
                installation: &report,
            })?
        );
    } else {
        present(&report);
    }
    Ok(Exit::Code(
        if report.state == InventoryState::Installed && report.unhealthy_components == 0 {
            0
        } else {
            1
        },
    ))
}

fn present(report: &IntegrityReport) {
    println!("{NOTICE}");
    println!("Prefix: {}", display_text(&report.prefix.to_string_lossy()));
    match report.state {
        InventoryState::Absent => {
            println!("State: absent\nNo installation prefix found; nothing checked.");
            return;
        }
        InventoryState::Unmanaged => {
            println!("State: unmanaged\nNo AW receipts found; nothing checked.");
            return;
        }
        InventoryState::Installed => println!("State: installed"),
    }
    println!("\nCOMPONENT\tVERSION\tINTEGRITY\tFILES CHECKED");
    for component in &report.components {
        let status = match component.status {
            aw_package::integrity::IntegrityStatus::Checked => "checked",
            aw_package::integrity::IntegrityStatus::Unhealthy => "unhealthy",
        };
        println!(
            "{}\t{}\t{status}\t{}/{}",
            display_text(&component.component),
            display_text(&component.version),
            component.checked_files,
            component.recorded_files,
        );
        for finding in &component.findings {
            let path = finding
                .path
                .as_deref()
                .map(|path| format!(" {}", display_text(path)))
                .unwrap_or_default();
            let related = finding
                .related_component
                .as_deref()
                .map(|name| format!(" (component: {})", display_text(name)))
                .unwrap_or_default();
            println!(
                "  {:?}{path}: {}{related}",
                finding.code,
                display_text(&finding.message),
            );
        }
    }
    println!(
        "\nChecked: {} component(s), {} file(s); unhealthy: {} component(s).",
        report.checked_components, report.checked_files, report.unhealthy_components,
    );
}

fn display_text(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        if character.is_control()
            || matches!(character, '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        {
            escaped.extend(character.escape_default());
        } else {
            escaped.push(character);
        }
    }
    escaped
}
