//! Read-only summaries of declared Agents and Providers, without runtime discovery.

use crate::cli::Result;
use serde_json::{json, Value};
use std::{fs::File, io::Read, os::unix::fs::OpenOptionsExt};

const NOTICE: &str =
    "Configured and referenced describe this file only; runtime activation is not checked.";

/// Presents validated configuration metadata without executing configured programs.
pub(crate) fn dispatch(command: &str, arguments: &[String]) -> Result<()> {
    let action = arguments.first().map(String::as_str).unwrap_or("--help");
    let allowed = match command {
        "provider" => &["list", "show"][..],
        "agent" => &["list"][..],
        _ => return Err("unknown inventory command".into()),
    };
    if action == "--help" {
        if arguments.len() > 1 {
            return Err("unexpected arguments after --help".into());
        }
        help(command);
        return Ok(());
    }
    if !allowed.contains(&action) {
        return Err("unknown inventory action".into());
    }
    if arguments.get(1).map(String::as_str) == Some("--help") {
        if arguments.len() > 2 {
            return Err("unexpected arguments after --help".into());
        }
        help(command);
        return Ok(());
    }
    let (selected, flags) = if action == "show" {
        let name = arguments
            .get(1)
            .filter(|name| !name.is_empty() && !name.starts_with("--"))
            .ok_or("provider show requires a Provider name")?;
        (Some(name.as_str()), &arguments[2..])
    } else {
        (None, &arguments[1..])
    };
    let mut config = None;
    let mut machine = false;
    let mut flags = flags.iter();
    while let Some(flag) = flags.next() {
        match flag.as_str() {
            "--config" if config.is_none() => {
                config = Some(
                    flags
                        .next()
                        .filter(|value| !value.is_empty() && !value.starts_with("--"))
                        .ok_or("--config requires a file path")?,
                );
            }
            "--json" if !machine => machine = true,
            _ => return Err("unknown or duplicate inventory option".into()),
        }
    }
    let path = config.ok_or("missing --config")?;
    let configuration = load(path)?;
    let spec = &configuration.as_value()["spec"];
    let report = if command == "provider" {
        providers(spec, selected)?
    } else {
        agents(spec)?
    };
    if machine {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        present(command, &report)?;
    }
    Ok(())
}

fn help(command: &str) {
    if command == "provider" {
        println!(
            "aw provider list --config FILE [--json]\n\
aw provider show NAME --config FILE [--json]"
        );
    } else {
        println!("aw agent list --config FILE [--json]");
    }
    println!("Read-only configuration inventory. No program is started.\n{NOTICE}");
}

fn load(path: &str) -> Result<aw_config::Configuration> {
    // Reject FIFOs without waiting for a writer; byte limits alone cannot do that.
    let file = File::options()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err("configuration must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(aw_config::MAX_DOCUMENT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    Ok(aw_config::Validator::new()?.parse(&bytes)?)
}

fn providers(spec: &Value, selected: Option<&str>) -> Result<Value> {
    let declared = spec["providers"]
        .as_object()
        .ok_or(aw_config::Error::InvalidSchema)?;
    if selected.is_some_and(|name| !declared.contains_key(name)) {
        return Err("Provider is not declared in this configuration".into());
    }
    let mut providers = Vec::new();
    for (name, provider) in declared {
        if selected.is_some_and(|selected| selected != name) {
            continue;
        }
        let mut references = Vec::new();
        for (event_name, event) in spec["events"]
            .as_object()
            .ok_or(aw_config::Error::InvalidSchema)?
        {
            for step in event["steps"]
                .as_array()
                .ok_or(aw_config::Error::InvalidSchema)?
            {
                if step["provider"] == *name {
                    references.push(json!({
                        "event": event_name,
                        "step": step["id"],
                        "event_enabled": event["enabled"],
                        "step_enabled": step["enabled"] != false,
                    }));
                }
            }
        }
        providers.push(json!({
            "name": name,
            "protocol": provider["protocol"].as_str().unwrap_or(aw_config::HOOK_PROTOCOL),
            "protocol_source": if provider.get("protocol").is_some() { "explicit" } else { "default" },
            "transport": provider["transport"]["type"],
            "referenced": !references.is_empty(),
            "references": references,
        }));
    }
    Ok(json!({
        "scope": "configuration", "runtime_checked": false,
        "providers": providers, "notice": NOTICE,
    }))
}

fn agents(spec: &Value) -> Result<Value> {
    let agents = spec["agents"]
        .as_object()
        .ok_or(aw_config::Error::InvalidSchema)?
        .iter()
        .map(|(name, agent)| json!({"name": name, "adapter": agent["adapter"]}))
        .collect::<Vec<_>>();
    Ok(json!({
        "scope": "configuration", "runtime_checked": false,
        "agents": agents, "notice": NOTICE,
    }))
}

fn text(value: &Value) -> Result<&str> {
    value
        .as_str()
        .ok_or_else(|| "invalid inventory presentation field".into())
}

fn present(command: &str, report: &Value) -> Result<()> {
    println!("{NOTICE}");
    if command == "agent" {
        println!("\nConfigured Agents\nNAME\tADAPTER");
        for agent in report["agents"]
            .as_array()
            .ok_or("invalid Agent inventory")?
        {
            println!("{}\t{}", text(&agent["name"])?, text(&agent["adapter"])?);
        }
    } else {
        let providers = report["providers"]
            .as_array()
            .ok_or("invalid Provider inventory")?;
        if providers.is_empty() {
            println!("\nNo Providers configured.");
            return Ok(());
        }
        println!("\nConfigured Providers\nNAME\tPROTOCOL\tTRANSPORT\tREFERENCED");
        for provider in providers {
            println!(
                "{}\t{}{}\t{}\t{}",
                text(&provider["name"])?,
                text(&provider["protocol"])?,
                if provider["protocol_source"] == "default" {
                    " (default)"
                } else {
                    ""
                },
                text(&provider["transport"])?,
                if provider["referenced"] == true {
                    "yes"
                } else {
                    "no"
                },
            );
            for reference in provider["references"]
                .as_array()
                .ok_or("invalid Provider references")?
            {
                println!(
                    "  {} / {} (event {}, step {})",
                    text(&reference["event"])?,
                    text(&reference["step"])?,
                    if reference["event_enabled"] == true {
                        "enabled"
                    } else {
                        "disabled"
                    },
                    if reference["step_enabled"] == true {
                        "enabled"
                    } else {
                        "disabled"
                    },
                );
            }
        }
    }
    Ok(())
}
