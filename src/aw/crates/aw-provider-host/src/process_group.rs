//! Process-group isolation and signalling for one-shot Provider processes.

use std::fmt;
use std::io;
use std::os::unix::process::CommandExt;
use std::process::Command;

use nix::errno::Errno;
use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;

/// Lifecycle operations used to isolate and terminate a Provider process group.
pub(crate) trait ProcessGroupLifecycle: fmt::Debug + Send + Sync {
    /// Configures a command so the child leads a dedicated process group.
    fn configure(&self, command: &mut Command);

    /// Sends an unconditional kill signal to the complete process group.
    fn kill(&self, process_group: u32) -> io::Result<()>;
}

/// Native process-group implementation used by the Provider driver.
#[derive(Debug, Default)]
pub(crate) struct PlatformProcessGroup;

impl ProcessGroupLifecycle for PlatformProcessGroup {
    fn configure(&self, command: &mut Command) {
        command.process_group(0);
    }

    fn kill(&self, process_group: u32) -> io::Result<()> {
        signal_group(process_group, Signal::SIGKILL)
    }
}

fn signal_group(process_group: u32, signal: Signal) -> io::Result<()> {
    match killpg(Pid::from_raw(process_group as i32), signal) {
        Ok(()) | Err(Errno::ESRCH) => Ok(()),
        Err(error) => Err(io::Error::from_raw_os_error(error as i32)),
    }
}
