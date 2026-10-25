//! Native unit defaults and starts remain explicit, scoped and independently recorded.

use super::model::{DeploymentConfig, DeploymentError, Scope, io};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Machine-readable unit state, including masked and disabled administrator choices.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UnitState {
    /// Whether systemd knows the installed unit.
    pub loaded: bool,
    /// Native UnitFileState, preserved without collapsing masked into disabled.
    pub enable_state: String,
    /// Whether the unit is active, reloading or transitioning between states.
    pub active: bool,
}

/// Injectable native-manager boundary; implementations must never invoke sudo.
pub trait Systemctl {
    /// Inspect one unit without changing it.
    fn inspect(&self, scope: Scope, unit: &str) -> Result<UnitState, DeploymentError>;
    /// Apply administrator-aware native preset policy for one unit only.
    fn preset(&self, scope: Scope, unit: &str, enable: bool) -> Result<(), DeploymentError>;
    /// Start one installed unit without modifying persistent enable state.
    fn start(&self, scope: Scope, unit: &str) -> Result<(), DeploymentError>;
}

/// Real systemctl executor with an explicit manager scope and no privilege helper.
pub struct SystemctlRunner;

impl SystemctlRunner {
    /// Reload only the explicitly selected manager after publishing unit files.
    ///
    /// # Errors
    /// Returns bounded command diagnostics on timeout, spawn or manager failure.
    pub fn daemon_reload(&self, scope: Scope) -> Result<(), DeploymentError> {
        self.command(scope, &["daemon-reload"]).map(|_| ())
    }

    fn command(
        &self,
        scope: Scope,
        args: &[&str],
    ) -> Result<std::process::Output, DeploymentError> {
        let mut command = Command::new("systemctl");
        if scope == Scope::User {
            command.arg("--user");
        }
        command.arg("--no-ask-password").args(args);
        let output = bounded_command(command, &args.join(" "), Duration::from_secs(30))?;
        if !output.status.success() {
            return Err(DeploymentError::Systemd {
                operation: args.join(" "),
                detail: format!(
                    "{}: {}{}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr),
                    String::from_utf8_lossy(&output.stdout)
                ),
            });
        }
        Ok(output)
    }
}

impl Systemctl for SystemctlRunner {
    fn inspect(&self, scope: Scope, unit: &str) -> Result<UnitState, DeploymentError> {
        let out = self.command(
            scope,
            &[
                "show",
                unit,
                "--no-pager",
                "--property=LoadState,ActiveState,UnitFileState",
            ],
        )?;
        let text = String::from_utf8_lossy(&out.stdout);
        let property = |name: &str| {
            text.lines()
                .find_map(|line| line.strip_prefix(name))
                .unwrap_or_default()
                .to_owned()
        };
        let load = property("LoadState=");
        let active = property("ActiveState=");
        if load.is_empty() || property("UnitFileState=").is_empty() && load == "loaded" {
            return Err(DeploymentError::Invalid(format!(
                "systemctl returned incomplete state for {unit}"
            )));
        }
        Ok(UnitState {
            loaded: matches!(load.as_str(), "loaded" | "masked"),
            enable_state: property("UnitFileState="),
            active: matches!(
                active.as_str(),
                "active" | "reloading" | "activating" | "deactivating" | "refreshing"
            ),
        })
    }

    fn preset(&self, scope: Scope, unit: &str, enable: bool) -> Result<(), DeploymentError> {
        // Mode limits product defaults while allowing /etc/systemd/*-preset policy
        // to veto an enable or disable decision; no host-wide wildcard is applied.
        let mode = if enable {
            "--preset-mode=enable-only"
        } else {
            "--preset-mode=disable-only"
        };
        self.command(scope, &["preset", mode, unit]).map(|_| ())
    }

    fn start(&self, scope: Scope, unit: &str) -> Result<(), DeploymentError> {
        self.command(scope, &["start", unit]).map(|_| ())
    }
}

#[cfg(unix)]
pub(crate) fn bounded_command(
    mut command: Command,
    operation: &str,
    timeout: Duration,
) -> Result<std::process::Output, DeploymentError> {
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    fn nonblocking(pipe: &impl AsRawFd) -> Result<(), DeploymentError> {
        use nix::fcntl::{FcntlArg, OFlag, fcntl};
        let old = fcntl(pipe.as_raw_fd(), FcntlArg::F_GETFL)
            .map_err(|error| io("inspect systemctl capture flags", error.into()))?;
        fcntl(
            pipe.as_raw_fd(),
            FcntlArg::F_SETFL(OFlag::from_bits_truncate(old) | OFlag::O_NONBLOCK),
        )
        .map_err(|error| io("set nonblocking systemctl capture", error.into()))?;
        Ok(())
    }
    fn drain(pipe: &mut impl Read, captured: &mut Vec<u8>) -> Result<bool, DeploymentError> {
        let mut buffer = [0u8; 4096];
        // Bound each poll as well as retained bytes: an endless writer cannot
        // starve deadline checks or the other output stream.
        for _ in 0..16 {
            match pipe.read(&mut buffer) {
                Ok(0) => return Ok(true),
                Ok(count) => {
                    let keep = count.min((64 * 1024usize).saturating_sub(captured.len()));
                    captured.extend_from_slice(&buffer[..keep]);
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) => return Err(io("read systemctl capture", error)),
            }
        }
        Ok(false)
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command
        .spawn()
        .map_err(|error| io("spawn systemctl", error))?;
    let result =
        (|| {
            let mut stdout_pipe = child.stdout.take().ok_or_else(|| {
                DeploymentError::Invalid("systemctl stdout was not captured".into())
            })?;
            let mut stderr_pipe = child.stderr.take().ok_or_else(|| {
                DeploymentError::Invalid("systemctl stderr was not captured".into())
            })?;
            nonblocking(&stdout_pipe)?;
            nonblocking(&stderr_pipe)?;
            let began = Instant::now();
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            loop {
                let stdout_eof = drain(&mut stdout_pipe, &mut stdout)?;
                let stderr_eof = drain(&mut stderr_pipe, &mut stderr)?;
                if let Some(status) = child
                    .try_wait()
                    .map_err(|error| io("poll systemctl", error))?
                    && stdout_eof
                    && stderr_eof
                {
                    return Ok(std::process::Output {
                        status,
                        stdout,
                        stderr,
                    });
                }
                if began.elapsed() >= timeout {
                    return Err(DeploymentError::Systemd {
                        operation: operation.to_owned(),
                        detail: format!(
                            "deadline exceeded after {} ms: {}",
                            timeout.as_millis(),
                            String::from_utf8_lossy(&stderr)
                        ),
                    });
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        })();
    if let Ok(pid) = i32::try_from(child.id()) {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(-pid),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
    let _ = child.kill();
    let _ = child.wait();
    result
}

#[cfg(not(unix))]
pub(crate) fn bounded_command(
    _command: Command,
    _operation: &str,
    _timeout: Duration,
) -> Result<std::process::Output, DeploymentError> {
    Err(DeploymentError::Invalid(
        "systemd deployment requires a Unix host".into(),
    ))
}

/// One separately reviewable native deployment operation.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PresetAction {
    /// Stable catalog service identity.
    pub service: String,
    /// Exact installed unit to act on.
    pub unit: String,
    /// Enable-default and start operations are represented independently.
    pub kind: PresetActionKind,
    /// Observed state explaining preservation and initialization decisions.
    pub before: UnitState,
}

/// Native deployment operations never remove unit files or application data.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum PresetActionKind {
    /// Preserve existing administrator state, including masks and explicit disables.
    Preserve,
    /// First-install defaults through native preset policy, never enable --now.
    Initialize {
        /// Desired default, still subject to administrator preset policy.
        enable: bool,
    },
    /// Explicit immediate startup after a fresh mask check.
    Start,
}

/// Read-only native deployment plan; package installation has already completed.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PresetPlan {
    /// Required format version for saved or JSON-rendered plans.
    pub schema_version: u32,
    /// Product selection that produced the actions.
    pub preset: String,
    /// Native manager selected independently of UID inference.
    pub scope: Scope,
    /// Ordered reviewable actions; missing required units are errors.
    pub actions: Vec<PresetAction>,
}

/// Completed actions remain visible if a later service operation fails.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PresetReport {
    /// The applied selection and its observed starting conditions.
    pub plan: PresetPlan,
    /// Actions completed successfully, in execution order.
    pub completed: Vec<PresetAction>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct State {
    schema_version: u32,
    units: BTreeMap<String, SavedUnit>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SavedUnit {
    service: String,
    default_applied: bool,
}

/// Plan activation without writing state or changing native unit status.
///
/// Existing units retain enable state unless first-install initialization is
/// explicitly requested and no earlier default was recorded for that unit.
///
/// # Errors
/// Rejects incompatible scope, absent units, invalid saved state and manager errors.
pub fn plan_preset(
    config: &DeploymentConfig,
    preset: &str,
    scope: Scope,
    state_path: &Path,
    initialize: bool,
    systemctl: &impl Systemctl,
) -> Result<PresetPlan, DeploymentError> {
    config.validate()?;
    let state = read_state(state_path)?;
    let selected = config.selected(preset)?;
    let applicable = selected.services.iter().any(|item| {
        config
            .services
            .iter()
            .any(|service| service.id == item.id && service.scope == scope)
            && (item.enable || item.start)
    });
    if !applicable
        && selected
            .services
            .iter()
            .any(|item| item.enable || item.start)
    {
        return Err(DeploymentError::Invalid(format!(
            "preset {preset} selects no active services in {scope:?} scope; select the matching scope explicitly"
        )));
    }
    let mut actions = Vec::new();
    let ordered = config.ordered(
        selected
            .services
            .iter()
            .filter(|item| item.start)
            .map(|item| item.id.as_str()),
    )?;
    // Defaults for inactive services do not imply startup dependencies.
    let mut items: Vec<_> = selected
        .services
        .iter()
        .filter(|item| !item.start)
        .collect();
    for service in ordered {
        if let Some(item) = selected.services.iter().find(|item| item.id == service.id) {
            items.push(item);
        }
    }
    for item in items {
        let service = config.service(&item.id)?;
        if service.scope != scope {
            continue;
        }
        let Some(unit) = &service.unit else {
            if !item.enable && !item.start {
                continue;
            }
            return Err(DeploymentError::Invalid(format!(
                "service {} has no installed systemd unit",
                service.id
            )));
        };
        let before = systemctl.inspect(scope, unit)?;
        if !before.loaded && !item.enable && !item.start {
            continue;
        }
        if !before.loaded {
            return Err(DeploymentError::Invalid(format!(
                "required unit {unit} is not installed"
            )));
        }
        if item.start && before.enable_state.starts_with("masked") {
            return Err(DeploymentError::Invalid(format!(
                "unit {unit} is masked; administrator action is required"
            )));
        }
        if item.start {
            for required in &service.required_paths {
                let path = PathBuf::from(super::model::expand(&required.to_string_lossy())?);
                if !path.is_absolute() || !path.exists() {
                    return Err(DeploymentError::Invalid(format!(
                        "service {} requires existing absolute path {}",
                        service.id,
                        path.display()
                    )));
                }
            }
        }
        let key = format!("{scope:?}:{unit}");
        if let Some(saved) = state.units.get(&key)
            && saved.service != service.id
        {
            return Err(DeploymentError::Invalid(format!(
                "unit {unit} was recorded for a different service"
            )));
        }
        let kind = if initialize
            && !state
                .units
                .get(&key)
                .is_some_and(|unit| unit.default_applied)
            && !before.enable_state.starts_with("masked")
        {
            PresetActionKind::Initialize {
                enable: item.enable,
            }
        } else {
            PresetActionKind::Preserve
        };
        actions.push(PresetAction {
            service: service.id.clone(),
            unit: unit.clone(),
            kind,
            before: before.clone(),
        });
        if item.start && !before.active {
            actions.push(PresetAction {
                service: service.id.clone(),
                unit: unit.clone(),
                kind: PresetActionKind::Start,
                before,
            });
        }
    }
    Ok(PresetPlan {
        schema_version: 1,
        preset: preset.into(),
        scope,
        actions,
    })
}

/// Apply one explicit selection, recording successful steps before continuing.
///
/// # Errors
/// Rejects wrong identity, unsafe state paths and manager failures. Previously
/// completed changes remain recorded and are never presented as rolled back.
pub fn apply_preset(
    config: &DeploymentConfig,
    preset: &str,
    scope: Scope,
    state_path: &Path,
    initialize: bool,
    systemctl: &impl Systemctl,
) -> Result<PresetReport, DeploymentError> {
    check_scope(scope)?;
    prepare_state_parent(state_path)?;
    let lock_path = state_path.with_extension("lock");
    reject_symlink(&lock_path)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| io(format!("open {}", lock_path.display()), e))?;
    lock.lock_exclusive()
        .map_err(|e| io("lock deployment state", e))?;
    let plan = plan_preset(config, preset, scope, state_path, initialize, systemctl)?;
    let mut state = read_state(state_path)?;
    let mut completed = Vec::new();
    for action in &plan.actions {
        let key = format!("{scope:?}:{}", action.unit);
        match action.kind {
            PresetActionKind::Initialize { enable } => {
                systemctl.preset(scope, &action.unit, enable)?;
                state.units.insert(
                    key,
                    SavedUnit {
                        service: action.service.clone(),
                        default_applied: true,
                    },
                );
                write_state(state_path, &state)?;
            }
            PresetActionKind::Start => {
                let current = systemctl.inspect(scope, &action.unit)?;
                if !current.loaded || current.enable_state.starts_with("masked") {
                    return Err(DeploymentError::Invalid(format!(
                        "unit {} became unavailable or masked before startup",
                        action.unit
                    )));
                }
                if !current.active {
                    systemctl.start(scope, &action.unit)?;
                }
            }
            PresetActionKind::Preserve => {}
        }
        completed.push(action.clone());
    }
    Ok(PresetReport { plan, completed })
}

fn check_scope(scope: Scope) -> Result<(), DeploymentError> {
    #[cfg(unix)]
    {
        let uid = nix::unistd::Uid::effective().as_raw();
        let real_uid = nix::unistd::Uid::current().as_raw();
        if uid != real_uid || nix::unistd::Gid::effective() != nix::unistd::Gid::current() {
            return Err(DeploymentError::Invalid(
                "setuid deployment is unsupported".into(),
            ));
        }
        if scope == Scope::System && uid != 0 {
            return Err(DeploymentError::Invalid("system preset apply requires an explicit root invocation; sudo is never run automatically".into()));
        }
        if scope == Scope::User && (uid == 0 || std::env::var_os("SUDO_UID").is_some()) {
            return Err(DeploymentError::Invalid(
                "user preset apply must run as the actual target user outside sudo".into(),
            ));
        }
    }
    Ok(())
}

fn read_state(path: &Path) -> Result<State, DeploymentError> {
    reject_symlink(path)?;
    match fs::read(path) {
        Ok(bytes) => {
            let state: State = serde_json::from_slice(&bytes)?;
            if state.schema_version != 1 {
                return Err(DeploymentError::Invalid(
                    "unsupported preset state schema".into(),
                ));
            }
            Ok(state)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(State {
            schema_version: 1,
            ..State::default()
        }),
        Err(error) => Err(io(format!("read {}", path.display()), error)),
    }
}

fn reject_symlink(path: &Path) -> Result<(), DeploymentError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(DeploymentError::Invalid(
            format!("deployment state cannot follow symlink {}", path.display()),
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io(format!("inspect {}", path.display()), error)),
    }
}

fn prepare_state_parent(path: &Path) -> Result<(), DeploymentError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| part == std::path::Component::ParentDir)
    {
        return Err(DeploymentError::Invalid(
            "deployment state path must be absolute without parent traversal".into(),
        ));
    }
    reject_symlink(path)?;
    let parent = path.parent().ok_or_else(|| {
        DeploymentError::Invalid("deployment state needs a parent directory".into())
    })?;
    let mut current = PathBuf::new();
    for part in parent.components() {
        current.push(part);
        reject_symlink(&current)?;
    }
    fs::create_dir_all(parent).map_err(|e| io(format!("create {}", parent.display()), e))
}

fn write_state(path: &Path, state: &State) -> Result<(), DeploymentError> {
    let parent = path.parent().ok_or_else(|| {
        DeploymentError::Invalid("deployment state needs a parent directory".into())
    })?;
    let mut temp =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| io("prepare preset state", e))?;
    temp.write_all(&serde_json::to_vec_pretty(state)?)
        .map_err(|e| io("write preset state", e))?;
    temp.as_file()
        .sync_all()
        .map_err(|e| io("sync preset state", e))?;
    temp.persist(path)
        .map_err(|e| io("publish preset state", e.error))?;
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|e| io("sync preset state directory", e))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn systemctl_capture_is_bounded_even_when_a_child_keeps_pipes_open() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "printf diagnostic >&2; sleep 60 & exit 0"]);
        let began = Instant::now();
        let error = bounded_command(command, "fixture", Duration::from_secs(1)).unwrap_err();
        assert!(began.elapsed() < Duration::from_secs(3));
        assert!(error.to_string().contains("deadline exceeded"));
        assert!(error.to_string().contains("diagnostic"));
    }

    #[test]
    fn systemctl_capture_drains_both_streams_without_retaining_unbounded_output() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "i=0; while test $i -lt 12000; do printf 'output-line\n'; printf 'error-line\n' >&2; i=$((i + 1)); done"]);
        let output = bounded_command(command, "fixture", Duration::from_secs(5)).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 64 * 1024);
        assert_eq!(output.stderr.len(), 64 * 1024);
    }
}
