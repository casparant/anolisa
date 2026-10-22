//! Unified installation preview; the legacy package frontend retains its output contract.

use crate::cli::Result;
use aw_package::InstallPlan;
use std::{collections::BTreeSet, path::Path};

pub(super) fn run(arguments: &[String]) -> Result<()> {
    if arguments == ["--help"] {
        println!(
            "aw install --prefix ABS_DIR --bundle ABS_DIR [--dry-run [--json]]\n\
Inspect and install an unpacked local Preview bundle.\n\
--dry-run reads inputs without creating a prefix, lock or receipt.\n\
--json requires --dry-run; conflicts produce a plan and exit 1.\n\
Checksums verify payload consistency, not publisher identity.\n\
No automatic elevation, service startup, Provider registration or Agent connection."
        );
        return Ok(());
    }
    let mut arguments = arguments.iter();
    let mut seen = BTreeSet::new();
    let (mut prefix, mut bundle) = (None, None);
    let (mut dry_run, mut json) = (false, false);
    while let Some(flag) = arguments.next() {
        if !seen.insert(flag) {
            return Err(format!("duplicate installation option: {flag}").into());
        }
        match flag.as_str() {
            "--dry-run" => dry_run = true,
            "--json" => json = true,
            "--prefix" | "--bundle" => {
                let value = arguments
                    .next()
                    .ok_or_else(|| format!("missing value for {flag}"))?;
                let path = Path::new(value);
                if !path.is_absolute() {
                    return Err(format!("{flag} requires an absolute path").into());
                }
                if flag == "--prefix" {
                    prefix = Some(path);
                } else {
                    bundle = Some(path);
                }
            }
            _ => return Err(format!("unknown installation option: {flag}").into()),
        }
    }
    if json && !dry_run {
        return Err("--json currently requires --dry-run".into());
    }
    let prefix = prefix.ok_or("missing --prefix")?;
    let bundle = bundle.ok_or("missing --bundle")?;
    let plan = aw_package::plan_install(bundle, prefix)?;
    if !dry_run && !plan.conflicts.is_empty() {
        return Err(format!("installation blocked: {}", plan.conflicts.join("; ")).into());
    }
    if json {
        println!("{}", serde_json::to_string(&plan)?);
    } else {
        render(&plan);
    }
    if !plan.conflicts.is_empty() {
        return Err("installation plan has conflicts; nothing was changed".into());
    }
    if dry_run {
        if !json {
            println!("Dry run: nothing was changed.");
        }
    } else {
        super::signals::register()?;
        aw_package::install_cancellable(bundle, prefix, &super::signals::CANCEL)?;
        println!(
            "Installed verified AW components in {}",
            display(&prefix.to_string_lossy())
        );
    }
    Ok(())
}

fn display(value: &str) -> String {
    value.chars().flat_map(char::escape_debug).collect()
}

fn render(plan: &InstallPlan) {
    println!("Install plan");
    println!(
        "  Source      local bundle {}",
        display(&plan.source.path.to_string_lossy())
    );
    println!("              payload checksums match; publisher identity not verified");
    println!(
        "  Components  {}",
        plan.components
            .iter()
            .map(|item| format!("{} {}", display(&item.name), display(&item.version)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "  Files       {} payloads, {} receipts in {}",
        plan.files.payloads.len(),
        plan.files.receipts.len(),
        display(&plan.prefix.to_string_lossy())
    );
    println!(
        "  Privileges  current uid {}; no automatic elevation",
        plan.privileges.effective_uid
    );
    println!("  Activation  none (services, Provider registration, Agent connection)");
    println!(
        "  Conflicts   {}",
        if plan.conflicts.is_empty() {
            "none observed".into()
        } else {
            plan.conflicts
                .iter()
                .map(|value| display(value))
                .collect::<Vec<_>>()
                .join("; ")
        }
    );
    println!("  Deferred    lock, write access and capacity; execution rechecks inputs");
    println!("              runtime readiness is not checked");
}
