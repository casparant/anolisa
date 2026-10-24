//! Bounded invocations of the verified bundle's AW, retaining actual JSON commit facts.

use super::{Bundle, Error, ReleaseLayout, Result, files, require};
use serde_json::Value;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub(super) struct Output {
    pub success: bool,
    pub report: Value,
}

pub(super) trait Backend {
    fn invoke(&self, args: &[String]) -> Result<Output>;
}

pub(super) struct Aw<'a> {
    pub bundle: &'a Bundle,
    pub layout: &'a ReleaseLayout,
}

impl Backend for Aw<'_> {
    fn invoke(&self, args: &[String]) -> Result<Output> {
        self.bundle.command_evidence()?;
        let mut stdout =
            tempfile::tempfile().map_err(|error| files::io(&self.bundle.root, error))?;
        let mut stderr =
            tempfile::tempfile().map_err(|error| files::io(&self.bundle.root, error))?;
        let mut command = Command::new(self.bundle.aw());
        command
            .args(args)
            .env("XDG_STATE_HOME", &self.layout.xdg_state)
            .env_remove("ANOLISA_DATA_DIR")
            .stdin(Stdio::null())
            .stdout(
                stdout
                    .try_clone()
                    .map_err(|error| files::io(&self.bundle.root, error))?,
            )
            .stderr(
                stderr
                    .try_clone()
                    .map_err(|error| files::io(&self.bundle.root, error))?,
            );
        use std::os::unix::process::CommandExt;
        command.process_group(0);
        #[cfg(target_os = "linux")]
        {
            let parent = nix::unistd::getpid().as_raw();
            // SAFETY: the child hook only invokes Linux prctl/getppid/kill; no allocation
            // or inherited synchronization is used between fork and exec.
            unsafe {
                command.pre_exec(move || {
                    if nix::libc::prctl(nix::libc::PR_SET_PDEATHSIG, nix::libc::SIGTERM) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if nix::libc::getppid() != parent {
                        nix::libc::kill(nix::libc::getpid(), nix::libc::SIGTERM);
                    }
                    Ok(())
                });
            }
        }
        let child = crate::process::spawn_retry_etxtbsy(&mut command)
            .map_err(|error| files::io(&self.bundle.aw(), error))?;
        let mut guard = ChildGuard(child);
        let deadline = Instant::now() + Duration::from_secs(600);
        let status = loop {
            if let Some(status) = guard
                .0
                .try_wait()
                .map_err(|error| files::io(&self.bundle.aw(), error))?
            {
                break status;
            }
            require(
                Instant::now() < deadline,
                "AW operation exceeded its 600-second deadline; inspect AW recovery before resuming",
            )?;
            for capture in [&stdout, &stderr] {
                require(
                    capture
                        .metadata()
                        .map_err(|error| files::io(&self.bundle.root, error))?
                        .len()
                        <= 32 * 1024 * 1024,
                    "AW operation exceeded its output limit; inspect recovery before resuming",
                )?;
            }
            thread::sleep(Duration::from_millis(10));
        };
        let read = |capture: &mut File| -> Result<Vec<u8>> {
            capture
                .seek(SeekFrom::Start(0))
                .map_err(|error| files::io(&self.bundle.root, error))?;
            let mut bytes = Vec::new();
            capture
                .take(32 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| files::io(&self.bundle.root, error))?;
            require(
                bytes.len() <= 32 * 1024 * 1024,
                "AW report exceeds output limit",
            )?;
            Ok(bytes)
        };
        let diagnostics = read(&mut stderr)?;
        let diagnostic = String::from_utf8_lossy(&diagnostics[..diagnostics.len().min(4096)]);
        let report: Value = files::decode(&read(&mut stdout)?).map_err(|error| Error::Invalid(format!("AW did not produce a valid management report ({error}); recovery status is unknown; {diagnostic}")))?;
        require(
            report.get("format").and_then(Value::as_u64) == Some(1)
                && report.get("status").and_then(Value::as_str).is_some(),
            "unsupported AW management report",
        )?;
        Ok(Output {
            success: status.success() && report["status"] == "succeeded",
            report,
        })
    }
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        // Kill the owned group even if its leader already exited and left descendants.
        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(self.0.id() as i32),
            nix::sys::signal::Signal::SIGKILL,
        );
        let _ = self.0.wait();
    }
}

pub(super) fn arguments(operation: &str, layout: &ReleaseLayout) -> Vec<String> {
    let mut args = vec![operation.to_owned()];
    if operation == "list" {
        args.push("--installed".into());
    }
    if operation != "list" {
        args.extend(["--prefix".into(), layout.prefix.display().to_string()]);
    }
    if operation == "install" {
        args.extend([
            "--resources-dir".into(),
            layout.resources.display().to_string(),
        ]);
    }
    args.extend([
        "--scope".into(),
        layout.scope.as_str().into(),
        "--json".into(),
    ]);
    args
}

pub(super) struct Inventory {
    pub installation_id: Option<String>,
    pub packages: Vec<Value>,
}

pub(super) fn inventory(backend: &impl Backend, layout: &ReleaseLayout) -> Result<Inventory> {
    let output = backend.invoke(&arguments("list", layout))?;
    require(
        output.success,
        format!("AW inventory blocked installation: {}", output.report),
    )?;
    let installations = output
        .report
        .pointer("/result/inventory/installations")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Invalid("AW inventory lacks installation records".into()))?;
    let matching = installations
        .iter()
        .filter(|installation| installation["prefix"] == layout.prefix.display().to_string())
        .collect::<Vec<_>>();
    require(
        matching.len() <= 1,
        "AW inventory contains duplicate selected prefixes",
    )?;
    match matching.first() {
        None => Ok(Inventory {
            installation_id: None,
            packages: Vec::new(),
        }),
        Some(installation) => {
            require(
                installation["resources"].as_str() == layout.resources.to_str(),
                "selected AW installation has a different real resources directory",
            )?;
            require(
                installation["manager"] == "raw"
                    && installation["registration"] == "registered"
                    && installation["scope"] == layout.scope.as_str(),
                "selected prefix is not a registered AW raw installation in the requested scope",
            )?;
            let id = installation["installation_id"]
                .as_str()
                .ok_or_else(|| Error::Invalid("AW installation lacks its persistent ID".into()))?;
            super::model::identifier(id)?;
            let packages = installation["packages"]
                .as_array()
                .cloned()
                .ok_or_else(|| Error::Invalid("AW inventory lacks package records".into()))?;
            Ok(Inventory {
                installation_id: Some(id.to_owned()),
                packages,
            })
        }
    }
}

pub(super) fn healthy(backend: &impl Backend, layout: &ReleaseLayout) -> Result<Value> {
    let output = backend.invoke(&arguments("doctor", layout))?;
    require(
        output.success,
        format!("AW installed-file verification failed: {}", output.report),
    )?;
    let issues = output
        .report
        .pointer("/result/report/issues")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Invalid("AW doctor report lacks issues array".into()))?;
    require(issues.is_empty(), "AW doctor reported damaged files")?;
    Ok(output.report)
}
