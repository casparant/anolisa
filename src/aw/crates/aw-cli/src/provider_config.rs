//! Provider shortcuts edit desired configuration through the shared publication boundary.

use crate::cli::Result;
use aw_package::config_edit::ConfigurationEditor;
use std::{fs::File, io::Read, os::unix::fs::OpenOptionsExt, path::Path};

/// Applies one offline Provider edit without installing software or reloading a service.
pub(crate) fn dispatch(arguments: &[String]) -> Result<()> {
    let action = arguments.first().map(String::as_str).unwrap_or("--help");
    if action == "--help" {
        if arguments.len() > 1 {
            return Err("unexpected arguments after --help".into());
        }
        help();
        return Ok(());
    }
    if !matches!(action, "add" | "remove") {
        return Err("unknown Provider configuration action".into());
    }
    if arguments.get(1).map(String::as_str) == Some("--help") {
        if arguments.len() > 2 {
            return Err("unexpected arguments after --help".into());
        }
        help();
        return Ok(());
    }
    let mut arguments = arguments[1..].iter().peekable();
    let mut name = if arguments.peek().is_some_and(|name| !name.starts_with("--")) {
        arguments.next().map(String::as_str)
    } else {
        None
    };
    let mut config = None;
    let mut source = None;
    while let Some(flag) = arguments.next() {
        let destination = match flag.as_str() {
            "--name" if name.is_none() => &mut name,
            "--config" if config.is_none() => &mut config,
            "--definition" if action == "add" && source.is_none() => &mut source,
            _ => return Err("unknown, duplicate or conflicting Provider option".into()),
        };
        let value = arguments
            .next()
            .filter(|value| !value.is_empty())
            .ok_or("Provider option requires a value")?;
        if flag != "--name" && value.starts_with("--") {
            return Err("Provider file option requires a path".into());
        }
        *destination = Some(value.as_str());
    }
    let name = name
        .filter(|name| !name.is_empty())
        .ok_or("missing Provider name")?;
    let path = Path::new(config.ok_or("missing --config")?);
    if !path.is_absolute() {
        return Err("Provider edits require --config ABS_FILE".into());
    }
    if action == "add" && source.is_none() {
        return Err("missing --definition".into());
    }
    // Parse every option before opening files; only the editor may publish changes.
    let mut editor = ConfigurationEditor::open(path)?;
    if action == "add" {
        let definition = definition(source.ok_or("missing --definition")?)?;
        editor.add_provider(name, definition)?;
    } else {
        editor.remove_provider(name)?;
    }
    if editor.save()? {
        println!("Updated configuration");
    } else {
        println!("Configuration unchanged");
    }
    println!("Offline only: no service reload or runtime activation was performed.");
    if action == "remove" {
        println!("Only the Provider declaration is removed; no software is uninstalled.");
    }
    Ok(())
}

fn help() {
    println!(
        "aw provider add NAME --config ABS_FILE --definition FILE\n\
aw provider remove NAME --config ABS_FILE\n\
Use --name NAME instead of positional NAME for names beginning with --.\n\
Do not combine positional NAME and --name. Definitions are complete YAML/JSON objects.\n\
These commands edit offline configuration only; they do not install, enable or reload Providers.\n\
Remove is not software uninstall; referencing Hooks must be removed first, including disabled Hooks.\n\
Changed files are rewritten as YAML; comments and formatting are not retained."
    );
}

fn definition(path: &str) -> Result<serde_json::Value> {
    // A FIFO must not block before the regular-file check can reject it.
    let file = File::options()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err("definition must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(aw_config::MAX_DOCUMENT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let value = aw_config::parse_value(&bytes)?;
    if !value.is_object() {
        return Err("definition must be a YAML/JSON object".into());
    }
    Ok(value)
}
