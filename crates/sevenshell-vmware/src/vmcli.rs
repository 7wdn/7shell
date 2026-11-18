//! The `vmcli` command line, asked one question: what is the power state?
//!
//! `vmrun list` names only machines that are powered on. A suspended machine is neither on nor
//! off there — and that distinction is exactly what the interface has to draw, because starting a
//! suspended machine resumes it rather than booting it. `vmcli <vmx> Power query` answers `on`,
//! `off` or `suspended`, which is what Workstation's own window shows.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use sevenshell_error::ShellError;
use tokio::process::Command;

const QUERY_TIMEOUT: Duration = Duration::from_secs(15);

pub struct Vmcli {
    program: PathBuf,
}

impl Vmcli {
    /// The command beside the `vmrun` this build already found. Workstation installs the two
    /// together, and a machine with the application on another drive is found through the first.
    pub fn beside(vmrun: &Path) -> Option<Self> {
        let name = if cfg!(windows) { "vmcli.exe" } else { "vmcli" };
        let path = vmrun.with_file_name(name);
        path.is_file().then_some(Self { program: path })
    }

    pub fn locate() -> Option<Self> {
        if let Some(path) = std::env::var_os("VMCLI_PATH").map(PathBuf::from)
            && path.is_file()
        {
            return Some(Self { program: path });
        }
        if let Some(path) = candidates().into_iter().find(|path| path.is_file()) {
            return Some(Self { program: path });
        }
        find_on_path().map(|program| Self { program })
    }

    /// `on`, `off` or `suspended`; nothing when the command could not answer.
    pub async fn power_state(&self, vmx: &str) -> Option<String> {
        let output = self.run("power", &[vmx, "Power", "query"], QUERY_TIMEOUT).await.ok()?;
        parse_power_state(&output)
    }

    /// Runs one module command and answers its standard output, or the failure its own words.
    pub(crate) async fn run(&self, operation: &str, args: &[&str], budget: Duration) -> Result<String, ShellError> {
        let mut command = Command::new(&self.program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let output = tokio::time::timeout(budget, command.output())
            .await
            .map_err(|_| {
                ShellError::vmware_command_failed(
                    operation,
                    format!("the command did not finish within {} seconds", budget.as_secs()),
                )
            })?
            .map_err(|error| {
                ShellError::vmware_unavailable(format!("`{}` could not be started: {error}", self.program.display()))
            })?;
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let detail = if stderr.is_empty() { stdout } else { stderr };
            return Err(ShellError::vmware_command_failed(operation, detail));
        }
        Ok(stdout)
    }
}

/// The `PowerState: suspended` line, lower-cased. Anything else is not an answer.
pub fn parse_power_state(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("PowerState") {
            continue;
        }
        let value = value.trim().to_ascii_lowercase();
        if matches!(value.as_str(), "on" | "off" | "suspended") {
            return Some(value);
        }
    }
    None
}

fn candidates() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let mut paths = Vec::new();
        for variable in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(root) = std::env::var_os(variable) {
                paths.push(
                    PathBuf::from(root)
                        .join("VMware")
                        .join("VMware Workstation")
                        .join("vmcli.exe"),
                );
            }
        }
        paths
    }
    #[cfg(not(windows))]
    {
        vec![
            PathBuf::from("/usr/bin/vmcli"),
            PathBuf::from("/usr/lib/vmware/bin/vmcli"),
            PathBuf::from("/usr/local/bin/vmcli"),
        ]
    }
}

fn find_on_path() -> Option<PathBuf> {
    let name = if cfg!(windows) { "vmcli.exe" } else { "vmcli" };
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(if cfg!(windows) { ';' } else { ':' })
        .filter(|entry| !entry.trim().is_empty())
        .map(|entry| Path::new(entry).join(name))
        .find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_power_line_is_the_answer_and_a_sentence_is_not() {
        assert_eq!(
            parse_power_state("PowerState: suspended\ncleanShutdown: true\n").as_deref(),
            Some("suspended")
        );
        assert_eq!(parse_power_state("PowerState: ON\n").as_deref(), Some("on"));
        assert_eq!(parse_power_state("Error: unknown\n"), None);
        assert_eq!(parse_power_state("PowerState: rebooting\n"), None);
    }
}
