//! Create explicit invoking-user AW configuration while leaving services and policy activation separate.

#[cfg(test)]
mod tests;

use crate::release::Environment;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

/// Read native Adapter names from the installed AW runtime with a bounded local command.
///
/// # Errors
/// Refuses a missing executable, incompatible catalog output or a failed/timed-out query.
pub fn agent_catalog(program: &Path) -> Result<Vec<String>, SetupError> {
    lexical(program)?;
    let mut command = std::process::Command::new(program);
    command.args(["agent", "list", "--json"]);
    let output = crate::deployment::bounded_command(
        command,
        "inspect installed AW Agent catalog",
        std::time::Duration::from_secs(5),
    )
    .map_err(|error| invalid(error.to_string()))?;
    if !output.status.success() {
        return Err(invalid(format!(
            "installed AW could not report its Agent catalog: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let value: Value = serde_json::from_slice(&output.stdout)?;
    let adapters = value.get("adapters").or_else(|| value.pointer("/result/adapters")).and_then(Value::as_array)
        .ok_or_else(|| invalid("installed AW has no compatible Agent catalog; update AW before guided configuration"))?;
    let mut names = Vec::new();
    for adapter in adapters {
        let name = adapter
            .as_str()
            .or_else(|| adapter.get("name").and_then(Value::as_str))
            .ok_or_else(|| invalid("AW catalog entry has no Adapter name"))?;
        if name.is_empty()
            || name.len() > 128
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return Err(invalid("AW catalog contains an invalid Adapter name"));
        }
        names.push(name.to_owned());
    }
    names.sort();
    names.dedup();
    if names.is_empty() {
        return Err(invalid("installed AW reports no native Adapters"));
    }
    Ok(names)
}

/// Configuration preparation refuses ambiguity without changing system services or installation ownership.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    /// Explicit input is incomplete, unsafe or incompatible with the installed runtime.
    #[error("{0}")]
    Invalid(String),
    /// A user-owned configuration operation failed.
    #[error("setup at {path}: {source}")]
    Io {
        /// File or directory involved in the failed operation.
        path: PathBuf,
        /// Original operating-system error.
        #[source]
        source: std::io::Error,
    },
    /// An explicit Provider definition template is malformed.
    #[error("Provider template JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// Explicit user choices; no installed Provider is enabled merely because its files exist.
pub struct SetupRequest<'a> {
    /// Invoking-user XDG paths, never inferred from SUDO_USER.
    pub environment: &'a Environment,
    /// Absolute new user configuration destination.
    pub config: PathBuf,
    /// Adapter chosen from the installed AW catalog.
    pub adapter: String,
    /// Catalog names discovered from the installed AW command.
    pub supported_adapters: Vec<String>,
    /// Literal native Agent invocation; SDK integrations may require extra arguments.
    pub agent_argv: Vec<String>,
    /// Explicit disabled product Provider template to render for later review.
    pub provider_template: Option<PathBuf>,
    /// Explicit Provider names to copy into a separate disabled definitions file.
    pub providers: Vec<String>,
}

/// Concrete, read-only configuration preparation; serialized output omits document contents.
#[derive(Debug, Serialize)]
pub struct SetupPlan {
    /// Fixed output version.
    pub schema_version: u32,
    /// New AW configuration destination.
    pub config: PathBuf,
    /// Selected native adapter; the configured AW target is named `agent`.
    pub adapter: String,
    /// Invoking-user runtime state, separate from product installation records.
    pub state: PathBuf,
    /// Optional inert Provider definitions destination.
    pub provider_definitions: Option<PathBuf>,
    /// Rendered Provider names, without automatically adding events or policy.
    pub providers: Vec<String>,
    /// Explicit follow-up commands; none are executed by configuration publication.
    pub next_actions: Vec<String>,
    /// Full in-memory AW document, excluded from status output to avoid exposing private inputs.
    #[serde(skip)]
    pub document: Value,
    #[serde(skip)]
    definitions: Option<Value>,
}

/// Observed configuration publication, retaining partial progress when a later publication fails.
#[derive(Debug, Serialize)]
pub struct SetupReport {
    /// Exact selected configuration and disabled definitions.
    pub plan: SetupPlan,
    /// Files actually created; existing files are never overwritten.
    pub created: Vec<PathBuf>,
    /// Whether this invocation only prepared read-only output.
    pub dry_run: bool,
    /// A publication failure after any earlier completed files.
    pub error: Option<String>,
}

fn invalid(message: impl Into<String>) -> SetupError {
    SetupError::Invalid(message.into())
}
fn io(path: &Path, source: std::io::Error) -> SetupError {
    SetupError::Io {
        path: path.to_owned(),
        source,
    }
}

/// Prepare a valid empty-policy AW document and explicitly selected inert Provider definitions.
///
/// # Errors
/// Rejects unknown adapters, non-executable Agent commands, existing destinations and unsafe templates.
pub fn plan_setup(request: &SetupRequest<'_>) -> Result<SetupPlan, SetupError> {
    if !request.supported_adapters.contains(&request.adapter) {
        return Err(invalid(
            "select an adapter reported by the installed `aw agent list --json` catalog",
        ));
    }
    let Some(program) = request.agent_argv.first() else {
        return Err(invalid("select an explicit Agent executable"));
    };
    let program = Path::new(program);
    lexical(program)?;
    let metadata = program.metadata().map_err(|error| io(program, error))?;
    if !metadata.is_file()
        || metadata.mode() & 0o111 == 0
        || request.agent_argv.len() > 128
        || request
            .agent_argv
            .iter()
            .any(|argument| argument.contains('\0'))
    {
        return Err(invalid(
            "Agent argv must contain an executable and bounded literal arguments",
        ));
    }
    destination(&request.config)?;
    let state = request.environment.state.join("aw");
    absolute(&state)?;
    if state == request.config
        || state.starts_with(&request.config)
        || request.config.starts_with(&state)
    {
        return Err(invalid(
            "AW configuration and runtime state must be separate paths",
        ));
    }
    let document = json!({"apiVersion":"aw/v1alpha1","kind":"AWConfiguration","metadata":{"name":"anolisa-user"},
        "spec":{"daemon":{"startup":"on_demand","state_dir":state,"endpoint":"auto"},
        "execution":{"guarantee":"native_hook","default_event_budget_ms":5000},"audit":{"enabled":true,"payload":"metadata_only"},
        "agents":{"agent":{"adapter":request.adapter,"argv":request.agent_argv}},"providers":{},"events":{}}});
    let (provider_definitions, definitions) = if request.providers.is_empty() {
        if request.provider_template.is_some() {
            return Err(invalid(
                "select at least one --provider when rendering a template",
            ));
        }
        (None, None)
    } else {
        let path = request.provider_template.as_ref().ok_or_else(|| {
            invalid("--provider requires an explicit installed Provider template")
        })?;
        let definitions = render_definitions(request, path)?;
        let filename = request
            .config
            .file_name()
            .ok_or_else(|| invalid("configuration has no file name"))?
            .to_string_lossy();
        let target = request
            .config
            .with_file_name(format!("{filename}.providers.disabled.json"));
        destination(&target)?;
        (Some(target), Some(definitions))
    };
    Ok(SetupPlan { schema_version: 1, config: request.config.clone(), adapter: request.adapter.clone(), state,
        provider_definitions, providers: request.providers.clone(), document, definitions,
        next_actions: vec![format!("aw config validate --config {}", shell(&request.config)),
            format!("aw run --config {} --agent agent", shell(&request.config)),
            "Provider definitions remain separate and disabled; review them and explicitly configure policy events before importing them.".into()] })
}

/// Create private user configuration from explicit choices, retaining independently completed writes.
///
/// # Errors
/// Returns preparation failures before writes; later failures are recorded with the created paths.
pub fn setup(request: &SetupRequest<'_>, dry_run: bool) -> Result<SetupReport, SetupError> {
    let plan = plan_setup(request)?;
    let mut report = SetupReport {
        plan,
        created: Vec::new(),
        dry_run,
        error: None,
    };
    if dry_run {
        return Ok(report);
    }
    let result = (|| {
        if let (Some(path), Some(value)) =
            (&report.plan.provider_definitions, &report.plan.definitions)
        {
            publish(path, value)?;
            report.created.push(path.clone());
        }
        publish(&report.plan.config, &report.plan.document)?;
        report.created.push(report.plan.config.clone());
        Ok::<_, SetupError>(())
    })();
    if let Err(error) = result {
        report.error = Some(error.to_string());
    }
    Ok(report)
}

fn render_definitions(request: &SetupRequest<'_>, path: &Path) -> Result<Value, SetupError> {
    absolute(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|error| io(path, error))?;
    let uid = nix::unistd::geteuid().as_raw();
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.mode() & 0o022 != 0
        || !(metadata.uid() == uid || metadata.uid() == 0)
        || metadata.len() > 4 * 1024 * 1024
    {
        return Err(invalid(
            "Provider template must be a bounded, owned regular file",
        ));
    }
    let mut bytes = Vec::new();
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| io(path, error))?
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| io(path, error))?;
    if bytes.len() as u64 != metadata.len() {
        return Err(invalid("Provider template changed while reading"));
    }
    let value: Value = serde_json::from_slice(&bytes)?;
    if value["schema_version"] != 1
        || value["kind"] != "anolisa-provider-template"
        || value["enabled"] != false
    {
        return Err(invalid(
            "expected an explicitly disabled ANOLISA Provider template",
        ));
    }
    let mut variables =
        BTreeMap::from([("{user_data_dir}", request.environment.data.join("anolisa"))]);
    if let Some(runtime) = &request.environment.runtime {
        variables.insert("{user_runtime_dir}", runtime.join("anolisa"));
    }
    let mut definitions = serde_json::Map::new();
    for name in &request.providers {
        if name.is_empty()
            || name.len() > 128
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return Err(invalid("selected Provider name is invalid"));
        }
        if definitions.contains_key(name) {
            return Err(invalid("duplicate selected Provider"));
        }
        let definition = value["providers"][name].as_object().ok_or_else(|| {
            invalid(format!(
                "Provider is absent from the explicit template: {name}"
            ))
        })?;
        let transport = definition
            .get("transport_template")
            .ok_or_else(|| invalid("Provider template has no transport_template"))?;
        let mut transport = transport.clone();
        if definition.get("protocol").and_then(Value::as_str) != Some("aw-provider/v1alpha1")
            || transport["type"] != "stdio"
            || transport["location"] != "agent"
        {
            return Err(invalid(
                "Provider definitions require the supported local stdio protocol",
            ));
        }
        let argv = transport["argv"]
            .as_array_mut()
            .ok_or_else(|| invalid("Provider template argv must be an array"))?;
        for argument in argv.iter_mut() {
            let mut text = argument
                .as_str()
                .ok_or_else(|| invalid("Provider argv must contain strings"))?
                .to_owned();
            for (name, path) in &variables {
                text = text.replace(name, &path.to_string_lossy());
            }
            if text.contains('{') || text.contains('}') {
                return Err(invalid(
                    "Provider template needs a secure invoking-user XDG_RUNTIME_DIR or contains an unknown placeholder",
                ));
            }
            *argument = Value::String(text);
        }
        let executable = argv
            .first()
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("Provider argv cannot be empty"))?;
        if !Path::new(executable).is_absolute() || !Path::new(executable).is_file() {
            return Err(invalid(
                "Provider template entrypoint is not installed at its absolute path",
            ));
        }
        let config = definition
            .get("config")
            .cloned()
            .unwrap_or_else(|| json!({}));
        definitions.insert(
            name.clone(),
            json!({"protocol":definition.get("protocol"),"transport":transport,
            "timeout_ms":2000,"max_output_bytes":1048576,"config":config}),
        );
    }
    Ok(
        json!({"schema_version":1,"enabled":false,"providers":definitions,"note":"Definitions only. Add explicit reviewed policy events through AW before use."}),
    )
}

fn absolute(path: &Path) -> Result<(), SetupError> {
    lexical(path)?;
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(invalid(format!(
                    "symbolic link in setup path: {}",
                    ancestor.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io(ancestor, error)),
        }
    }
    Ok(())
}

fn lexical(path: &Path) -> Result<(), SetupError> {
    if !path.is_absolute()
        || path
            .as_os_str()
            .to_string_lossy()
            .split('/')
            .any(|part| part == "." || part == "..")
    {
        return Err(invalid("setup paths must be absolute without traversal"));
    }
    Ok(())
}

fn destination(path: &Path) -> Result<(), SetupError> {
    absolute(path)?;
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io(path, error)),
        Ok(_) => Err(invalid(format!(
            "configuration destination already exists; preserve {} and choose a new file",
            path.display()
        ))),
    }
}

fn private_tree(path: &Path) -> Result<(), SetupError> {
    absolute(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_dir()
                || metadata.uid() != nix::unistd::geteuid().as_raw()
                || metadata.mode() & 0o022 != 0
            {
                return Err(invalid(format!(
                    "setup directory must belong to the invoking user and not be publicly writable: {}",
                    path.display()
                )));
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            private_tree(
                path.parent()
                    .ok_or_else(|| invalid("setup destination has no parent"))?,
            )?;
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new()
                .mode(0o700)
                .create(path)
                .map_err(|error| io(path, error))
        }
        Err(error) => Err(io(path, error)),
    }
}

fn publish(path: &Path, document: &Value) -> Result<(), SetupError> {
    destination(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| invalid("setup destination has no parent"))?;
    private_tree(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|error| io(parent, error))?;
    file.write_all(&serde_json::to_vec_pretty(document)?)
        .map_err(|error| io(path, error))?;
    file.as_file().sync_all().map_err(|error| io(path, error))?;
    file.persist_noclobber(path)
        .map_err(|error| io(path, error.error))?;
    fs::File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|error| io(parent, error))
}

fn shell(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}
