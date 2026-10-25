//! Native inventory uses a bounded local query without repositories or inherited RPM executables.

use anolisa_platform::command::{CommandOutput, CommandRunner};
use std::{path::Path, process::Command, time::Duration};

/// Local RPM query runner with bounded output, a deadline and descendant cleanup.
pub struct RpmInventoryRunner;

impl CommandRunner for RpmInventoryRunner {
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<CommandOutput> {
        if program != "rpm" {
            return Err(std::io::Error::other(
                "overview only permits local RPM queries",
            ));
        }
        query(Path::new("/usr/bin/rpm"), args, Duration::from_secs(5))
    }
}

fn query(program: &Path, args: &[&str], timeout: Duration) -> std::io::Result<CommandOutput> {
    let mut command = Command::new(program);
    command.args(args).env("LC_ALL", "C");
    let output =
        crate::deployment::bounded_command(command, "inspect local RPM inventory", timeout)
            .map_err(std::io::Error::other)?;
    Ok(CommandOutput {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn local_inventory_query_cannot_wait_forever_for_a_descendants_pipe() {
        let began = Instant::now();
        let error = query(
            Path::new("/bin/sh"),
            &["-c", "sleep 30 & exit 0"],
            Duration::from_millis(100),
        )
        .unwrap_err();
        assert!(error.to_string().contains("deadline exceeded"));
        assert!(began.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn inventory_retains_only_bounded_output_and_rejects_other_tools() {
        let output = query(
            Path::new("/bin/sh"),
            &["-c", "head -c 131072 /dev/zero"],
            Duration::from_secs(3),
        )
        .unwrap();
        assert_eq!(output.code, Some(0));
        assert_eq!(output.stdout.len(), 64 * 1024);
        assert!(RpmInventoryRunner.run("dnf", &["list"]).is_err());
    }
}
