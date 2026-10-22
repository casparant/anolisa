//! Read-only legacy state inspection before any migration or ownership transfer.

use crate::cli::Result;
use aw_package::migration::{Inspection, Scope};
use std::path::PathBuf;

const NOTICE: &str = "Read-only migration inventory. No packages, Hooks or services are changed.";

/// Inspect explicit legacy state documents without following their recorded resources.
pub(crate) fn dispatch(arguments: &[String]) -> Result<()> {
    if arguments.is_empty() || arguments == ["--help"] || arguments == ["inspect", "--help"] {
        help();
        return Ok(());
    }
    if arguments.first().map(String::as_str) != Some("inspect") {
        return Err("migrate supports inspect only; use aw migrate --help".into());
    }
    let mut states = Vec::new();
    let mut machine = false;
    let mut flags = arguments[1..].iter();
    while let Some(flag) = flags.next() {
        match flag.as_str() {
            "--state" => {
                let path = flags
                    .next()
                    .filter(|value| !value.is_empty() && !value.starts_with("--"))
                    .ok_or("--state requires an absolute file path")?;
                let path = PathBuf::from(path);
                if !path.is_absolute() {
                    return Err("--state requires an absolute file path".into());
                }
                if states.contains(&path) {
                    return Err("duplicate --state path".into());
                }
                if states.len() == 8 {
                    return Err("inspect accepts at most 8 state files".into());
                }
                states.push(path);
            }
            "--json" if !machine => machine = true,
            _ => return Err("unknown or duplicate migration inspection option".into()),
        }
    }
    if states.is_empty() {
        return Err("inspect requires at least one --state ABSOLUTE_FILE".into());
    }
    let inspection =
        aw_package::migration::inspect(&states).map_err(|error| safe_text(&error.to_string()))?;
    if machine {
        println!("{}", serde_json::to_string_pretty(&inspection)?);
    } else {
        present(&inspection);
    }
    Ok(())
}

fn help() {
    println!(
        "aw migrate inspect --state ABSOLUTE_FILE [--state ABSOLUTE_FILE ...] [--json]\n\n\
{NOTICE}\n\
Read up to 8 explicit legacy installed.toml files. No automatic discovery or elevation.\n\
Records and overlap candidates do not establish runtime health or migration readiness."
    );
}

fn present(inspection: &Inspection) {
    let records: usize = inspection
        .sources
        .iter()
        .map(|source| source.records.len())
        .sum();
    let findings: usize = inspection
        .sources
        .iter()
        .flat_map(|source| &source.records)
        .map(|record| record.findings.len())
        .sum();
    println!("{NOTICE}");
    println!(
        "Sources: {}  Records: {records}  Findings: {findings}  Overlap candidates: {}",
        inspection.sources.len(),
        inspection.overlaps.len(),
    );
    for (index, source) in inspection.sources.iter().enumerate() {
        println!(
            "\n[{}] {} (schema {}, file owner uid {})",
            index + 1,
            safe_text(&source.path.to_string_lossy()),
            source.schema_version,
            source.file_owner_uid,
        );
        println!(
            "Recorded layout: {}, prefix {}",
            safe_text(&source.recorded_install_mode),
            safe_text(&source.recorded_prefix),
        );
        println!(
            "Adapter claims: {}  Trust roots: {}  Quarantined: {}  Legacy objects: {}",
            source.adapter_claim_count,
            source.adapter_trust_root_count,
            source.quarantined_count,
            source.legacy_object_count,
        );
        if source.records.is_empty() {
            println!("No installation records in this document.");
            continue;
        }
        println!("RECORD\tSCOPE\tBACKEND\tRELATION\tSTATUS (RECORDED)\tVERSION\tFILES\tSERVICES");
        for record in &source.records {
            println!(
                "{}/{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                safe_text(&record.kind),
                safe_text(&record.name),
                scope(&record.recorded_scope),
                safe_text(&record.recorded_backend),
                safe_text(&record.recorded_relation),
                safe_text(&record.recorded_status),
                record
                    .recorded_version
                    .as_deref()
                    .map(safe_text)
                    .unwrap_or_else(|| "unknown".into()),
                record.file_count,
                record.service_count,
            );
            if let Some(package) = &record.recorded_package {
                println!("  Recorded native package: {}", safe_text(package));
            }
            for finding in &record.findings {
                println!(
                    "  {}: {}",
                    safe_text(finding.code),
                    safe_text(finding.message)
                );
            }
        }
    }
    if !inspection.overlaps.is_empty() {
        println!("\nOverlap candidates (live precedence and conflicts are not checked)");
        for overlap in &inspection.overlaps {
            let sources = overlap
                .source_indices
                .iter()
                .map(|index| (index + 1).to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let scopes = overlap
                .scopes
                .iter()
                .map(scope)
                .collect::<Vec<_>>()
                .join(", ");
            println!(
                "  {}/{}: sources {sources}; {scopes}",
                safe_text(&overlap.kind),
                safe_text(&overlap.name)
            );
        }
    }
    println!(
        "\nChecked: {}",
        inspection
            .checked
            .iter()
            .map(|value| safe_text(&value.replace('_', " ")))
            .collect::<Vec<_>>()
            .join("; ")
    );
    println!(
        "Not checked: {}",
        inspection
            .not_checked
            .iter()
            .map(|value| safe_text(&value.replace('_', " ")))
            .collect::<Vec<_>>()
            .join("; ")
    );
    println!("Changes: none. This inspection is not a migration plan.");
}

fn scope(scope: &Scope) -> String {
    if scope.kind == "user" {
        match scope.uid {
            Some(uid) => format!("user:{uid}"),
            None => "user (uid unknown)".into(),
        }
    } else {
        safe_text(&scope.kind)
    }
}

fn safe_text(value: &str) -> String {
    value.chars().flat_map(|character| {
        if character.is_control()
            || matches!(character, '\u{061c}' | '\u{200e}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        {
            character.escape_default().collect::<Vec<_>>()
        } else {
            vec![character]
        }
    }).collect()
}
