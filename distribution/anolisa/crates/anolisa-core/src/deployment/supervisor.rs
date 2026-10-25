//! Bounded same-UID service supervision for explicit container deployments.

use super::model::{
    Communication, DeploymentConfig, DeploymentError, Readiness, RunAs, Scope, Service, expand, io,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

static SIGNAL: AtomicI32 = AtomicI32::new(0);
static SUPERVISOR: Mutex<()> = Mutex::new(());

/// Fully preflighted foreground selection, including resolved executable identities.
#[derive(Clone, Debug, Serialize)]
pub struct ServePlan {
    /// Required report format gate.
    pub schema_version: u32,
    /// Explicit product preset, never inferred from installed daemon count.
    pub preset: String,
    /// Actual effective UID inherited by every child.
    pub uid: u32,
    /// Services in dependency order; all must become ready and remain running.
    pub services: Vec<Service>,
}

/// Terminal foreground outcome after every directly supervised child is reaped.
#[derive(Debug, Serialize)]
pub struct ServeReport {
    /// Selected product preset.
    pub preset: String,
    /// Services actually started before shutdown.
    pub started: Vec<String>,
}

/// Resolve commands and startup prerequisites without creating files or processes.
///
/// # Errors
/// Rejects privileged backends, stdio sessions, implicit dependencies, absent
/// executables or configuration, pre-existing listeners and unsupported hosts.
pub fn plan_serve(config: &DeploymentConfig, preset: &str) -> Result<ServePlan, DeploymentError> {
    config.validate()?;
    let selected = config.selected(preset)?;
    let ordered = config.ordered(
        selected
            .services
            .iter()
            .filter(|item| item.start)
            .map(|item| item.id.as_str()),
    )?;
    if ordered.is_empty() {
        return Err(DeploymentError::Invalid(format!(
            "preset {preset} selects no foreground services"
        )));
    }
    let uid = current_uid()?;
    let mut services = Vec::new();
    for description in ordered {
        if !description.container_allowed
            || description.scope != Scope::User
            || description.run_as != RunAs::SameUser
            || description.communication == Communication::McpStdio
        {
            return Err(DeploymentError::Invalid(format!(
                "service {} is outside the same-UID container allowlist; use its native service or dedicated sidecar",
                description.id
            )));
        }
        let mut service = description.clone();
        service.program = resolve_program(&expand(&service.program)?)?
            .to_string_lossy()
            .into_owned();
        service.args = service
            .args
            .iter()
            .map(|arg| expand(arg))
            .collect::<Result<_, _>>()?;
        for required in &mut service.required_paths {
            *required = expanded_path(required)?;
            if !required.exists() {
                return Err(DeploymentError::Invalid(format!(
                    "service {} requires existing path {}",
                    service.id,
                    required.display()
                )));
            }
        }
        if let Readiness::UnixSocket { path, .. } = &mut service.readiness {
            *path = expanded_path(path)?;
            #[cfg(unix)]
            nix::sys::socket::UnixAddr::new(path.as_path()).map_err(|error| {
                DeploymentError::Invalid(format!(
                    "invalid readiness socket path {}: {error}",
                    path.display()
                ))
            })?;
            let exists = match std::fs::symlink_metadata(&*path) {
                Ok(_) => true,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => {
                    return Err(io(
                        format!("inspect readiness socket {}", path.display()),
                        error,
                    ));
                }
            };
            if exists {
                return Err(DeploymentError::Invalid(format!(
                    "service {} socket {} already exists; refuse a second manager or stale socket takeover",
                    service.id,
                    path.display()
                )));
            }
        }
        if let Readiness::Tcp { address, .. } = &service.readiness
            && std::net::TcpStream::connect_timeout(address, Duration::from_millis(100)).is_ok()
        {
            return Err(DeploymentError::Invalid(format!(
                "service {} readiness endpoint {address} already has a listener",
                service.id
            )));
        }
        #[cfg(target_os = "linux")]
        if Path::new("/run/systemd/system").is_dir()
            && let Some(unit) = &service.unit
        {
            use super::preset::Systemctl;
            let existing = super::preset::SystemctlRunner.inspect(Scope::User, unit)?;
            if existing.active {
                return Err(DeploymentError::Invalid(format!(
                    "service {} is already managed by systemd unit {unit}",
                    service.id
                )));
            }
        }
        services.push(service);
    }
    Ok(ServePlan {
        schema_version: 1,
        preset: preset.into(),
        uid,
        services,
    })
}

/// Run a selected foreground group until interruption or a required child fails.
///
/// Children inherit stdout/stderr and run in separate process groups. TERM/INT
/// reaches every group; the stop deadline escalates to KILL and reaps children.
/// An outer OCI init remains responsible for unrelated adopted grandchildren.
///
/// # Errors
/// Returns the first startup, readiness, signal or child-exit failure after cleanup.
pub fn serve(config: &DeploymentConfig, preset: &str) -> Result<ServeReport, DeploymentError> {
    let plan = plan_serve(config, preset)?;
    let _exclusive = SUPERVISOR.try_lock().map_err(|_| {
        DeploymentError::Invalid("only one foreground supervisor may run per process".into())
    })?;
    SIGNAL.store(0, Ordering::SeqCst);
    let _signals = SignalGuard::install()?;
    let mut group = ProcessGroup {
        children: Vec::new(),
    };
    let mut report = ServeReport {
        preset: preset.into(),
        started: Vec::new(),
    };
    for service in &plan.services {
        check_signal()?;
        group.check_children()?;
        let mut command = Command::new(&service.program);
        command
            .args(&service.args)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let child = crate::process::spawn_retry_etxtbsy(&mut command)
            .map_err(|error| io(format!("spawn service {}", service.id), error))?;
        report.started.push(service.id.clone());
        group.children.push(ManagedChild {
            id: service.id.clone(),
            child,
            timeout: Duration::from_millis(service.stop_timeout_ms),
        });
        let started = Instant::now();
        loop {
            check_signal()?;
            group.check_children()?;
            if ready(&service.readiness, started)? {
                break;
            }
            if started.elapsed() >= Duration::from_millis(service.readiness.timeout()) {
                return Err(DeploymentError::Service {
                    service: service.id.clone(),
                    detail: "readiness deadline expired".into(),
                });
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    loop {
        check_signal()?;
        group.check_children()?;
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn expanded_path(path: &Path) -> Result<PathBuf, DeploymentError> {
    let path = PathBuf::from(expand(&path.to_string_lossy())?);
    if !path.is_absolute()
        || path
            .components()
            .any(|part| part == std::path::Component::ParentDir)
    {
        return Err(DeploymentError::Invalid(format!(
            "service prerequisite must be absolute: {}",
            path.display()
        )));
    }
    Ok(path)
}

fn resolve_program(program: &str) -> Result<PathBuf, DeploymentError> {
    resolve_program_with_prefix(
        program,
        std::env::var_os("ANOLISA_RELEASE_PREFIX").map(PathBuf::from),
    )
}

fn resolve_program_with_prefix(
    program: &str,
    release_prefix: Option<PathBuf>,
) -> Result<PathBuf, DeploymentError> {
    let candidate = if Path::new(program).is_absolute() {
        PathBuf::from(program)
    } else if let Some(prefix) = release_prefix {
        if !prefix.is_absolute()
            || prefix
                .components()
                .any(|part| part == std::path::Component::ParentDir)
        {
            return Err(DeploymentError::Invalid(
                "ANOLISA_RELEASE_PREFIX must identify an absolute product version directory".into(),
            ));
        }
        // Public launchers select one immutable product version. Resolving a
        // sibling from PATH could mix its catalog with another installed AW.
        prefix.join("bin").join(program)
    } else {
        let paths = std::env::var_os("PATH").unwrap_or_default();
        std::env::split_paths(&paths)
            .filter(|path| path.is_absolute())
            .map(|path| path.join(program))
            .find(|path| executable(path))
            .ok_or_else(|| {
                DeploymentError::Invalid(format!(
                    "service executable {program} is not installed in an absolute PATH directory"
                ))
            })?
    };
    if !executable(&candidate) {
        return Err(DeploymentError::Invalid(format!(
            "service executable {} is absent or not executable",
            candidate.display()
        )));
    }
    let resolved = candidate
        .canonicalize()
        .map_err(|error| io(format!("resolve {}", candidate.display()), error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = resolved
            .metadata()
            .map_err(|error| io(format!("inspect {}", resolved.display()), error))?;
        if metadata.permissions().mode() & 0o6000 != 0 {
            return Err(DeploymentError::Invalid(format!(
                "same-UID supervision rejects setuid/setgid executable {}",
                resolved.display()
            )));
        }
    }
    #[cfg(target_os = "linux")]
    if xattr::get(&resolved, "security.capability")
        .map_err(|error| {
            io(
                format!("inspect capabilities on {}", resolved.display()),
                error,
            )
        })?
        .is_some_and(|bytes| !bytes.is_empty())
    {
        return Err(DeploymentError::Invalid(format!(
            "same-UID supervision rejects file capabilities on {}",
            resolved.display()
        )));
    }
    Ok(resolved)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn selected_product_programs_cannot_fall_back_to_another_version_on_path() {
        let temp = tempfile::tempdir().unwrap();
        let program = temp.path().join("bin/fixture");
        std::fs::create_dir_all(program.parent().unwrap()).unwrap();
        std::fs::write(&program, b"#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            resolve_program_with_prefix("fixture", Some(temp.path().to_path_buf())).unwrap(),
            program.canonicalize().unwrap()
        );
        assert!(resolve_program_with_prefix("sh", Some(temp.path().to_path_buf())).is_err());
        assert!(resolve_program_with_prefix("fixture", Some(PathBuf::from("relative"))).is_err());
        assert!(resolve_program_with_prefix("/bin/sh", Some(temp.path().to_path_buf())).is_ok());
    }
}

fn executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn current_uid() -> Result<u32, DeploymentError> {
    #[cfg(unix)]
    {
        let uid = nix::unistd::Uid::effective().as_raw();
        if uid != nix::unistd::Uid::current().as_raw()
            || nix::unistd::Gid::effective() != nix::unistd::Gid::current()
        {
            return Err(DeploymentError::Invalid(
                "setuid foreground supervision is unsupported".into(),
            ));
        }
        Ok(uid)
    }
    #[cfg(not(unix))]
    {
        Err(DeploymentError::Invalid(
            "foreground supervision requires Unix process groups".into(),
        ))
    }
}

fn ready(readiness: &Readiness, started: Instant) -> Result<bool, DeploymentError> {
    match readiness {
        Readiness::Process { delay_ms, .. } => {
            Ok(started.elapsed() >= Duration::from_millis(*delay_ms))
        }
        Readiness::Tcp { address, .. } => {
            Ok(std::net::TcpStream::connect_timeout(address, Duration::from_millis(20)).is_ok())
        }
        Readiness::UnixSocket { path, .. } => {
            #[cfg(unix)]
            {
                use nix::sys::socket::{
                    AddressFamily, SockFlag, SockType, UnixAddr, connect, socket,
                };
                use std::os::fd::AsRawFd;
                let address = UnixAddr::new(path).map_err(|error| {
                    DeploymentError::Invalid(format!(
                        "invalid readiness socket {}: {error}",
                        path.display()
                    ))
                })?;
                let socket = socket(
                    AddressFamily::Unix,
                    SockType::Stream,
                    SockFlag::empty(),
                    None,
                )
                .map_err(|error| io("create readiness probe socket", error.into()))?;
                use nix::fcntl::{FcntlArg, FdFlag, OFlag, fcntl};
                fcntl(socket.as_raw_fd(), FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
                    .map_err(|error| io("protect readiness socket inheritance", error.into()))?;
                fcntl(socket.as_raw_fd(), FcntlArg::F_SETFL(OFlag::O_NONBLOCK))
                    .map_err(|error| io("set nonblocking readiness probe", error.into()))?;
                Ok(connect(socket.as_raw_fd(), &address).is_ok())
            }
            #[cfg(not(unix))]
            {
                let _ = path;
                Err(DeploymentError::Invalid(
                    "Unix socket readiness requires Unix".into(),
                ))
            }
        }
    }
}

fn check_signal() -> Result<(), DeploymentError> {
    let signal = SIGNAL.load(Ordering::SeqCst);
    if signal == 0 {
        Ok(())
    } else {
        Err(DeploymentError::Interrupted(signal))
    }
}

struct ManagedChild {
    id: String,
    child: Child,
    timeout: Duration,
}

struct ProcessGroup {
    children: Vec<ManagedChild>,
}

impl ProcessGroup {
    fn check_children(&mut self) -> Result<(), DeploymentError> {
        for managed in &mut self.children {
            if let Some(status) = managed
                .child
                .try_wait()
                .map_err(|error| io(format!("wait for {}", managed.id), error))?
            {
                return Err(DeploymentError::Service {
                    service: managed.id.clone(),
                    detail: format!("required foreground process exited with {status}"),
                });
            }
        }
        Ok(())
    }

    fn stop(&mut self) {
        let received = SIGNAL.load(Ordering::SeqCst);
        let signal = if received == 2 { 2 } else { 15 };
        for managed in self.children.iter_mut().rev() {
            signal_group(managed.child.id(), signal);
        }
        let began = Instant::now();
        for managed in self.children.iter_mut().rev() {
            loop {
                match managed.child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if began.elapsed() < managed.timeout => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    _ => break,
                }
            }
            // A leader may exit while a child ignores TERM. Kill the remaining
            // process group as well, rather than leaking descendants on shutdown.
            signal_group(managed.child.id(), 9);
            let _ = managed.child.wait();
        }
        self.children.clear();
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.stop();
    }
}

fn signal_group(pid: u32, signal: i32) {
    #[cfg(unix)]
    {
        use nix::sys::signal::{Signal, kill};
        if let (Ok(pid), Ok(signal)) = (i32::try_from(pid), Signal::try_from(signal)) {
            let _ = kill(nix::unistd::Pid::from_raw(-pid), signal);
        }
    }
}

#[cfg(unix)]
struct SignalGuard {
    term: nix::sys::signal::SigAction,
    int: nix::sys::signal::SigAction,
}

#[cfg(unix)]
extern "C" fn on_signal(signal: i32) {
    SIGNAL.store(signal, Ordering::SeqCst);
}

#[cfg(unix)]
impl SignalGuard {
    fn install() -> Result<Self, DeploymentError> {
        use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};
        let action = SigAction::new(
            SigHandler::Handler(on_signal),
            SaFlags::empty(),
            SigSet::empty(),
        );
        // SAFETY: on_signal only stores a lock-free atomic integer and has a
        // static lifetime. SUPERVISOR serializes installation and restoration.
        let term = unsafe { sigaction(Signal::SIGTERM, &action) }
            .map_err(|error| io("install TERM handler", error.into()))?;
        // SAFETY: same static signal-safe handler and serialization as above.
        let int = match unsafe { sigaction(Signal::SIGINT, &action) } {
            Ok(previous) => previous,
            Err(error) => {
                // SAFETY: restore the exact action returned by sigaction.
                let _ = unsafe { sigaction(Signal::SIGTERM, &term) };
                return Err(io("install INT handler", error.into()));
            }
        };
        Ok(Self { term, int })
    }
}

#[cfg(unix)]
impl Drop for SignalGuard {
    fn drop(&mut self) {
        use nix::sys::signal::{Signal, sigaction};
        // SAFETY: restore the exact prior handlers while SUPERVISOR is held.
        unsafe {
            let _ = sigaction(Signal::SIGTERM, &self.term);
            let _ = sigaction(Signal::SIGINT, &self.int);
        }
    }
}

#[cfg(not(unix))]
struct SignalGuard;
#[cfg(not(unix))]
impl SignalGuard {
    fn install() -> Result<Self, DeploymentError> {
        Err(DeploymentError::Invalid(
            "foreground supervision requires Unix".into(),
        ))
    }
}
