//! The `vmrun` command line, and the few questions this build asks it.
//!
//! One binary, one argument vector per operation, no shell: a path with a space in it is one
//! argument because the process API says so, not because a quoting rule was remembered. The
//! binary is found by an environment override first — Workstation may be installed anywhere —
//! then by the paths Workstation itself uses, then on `PATH`.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use sevenshell_error::ShellError;
use tokio::process::Command;

/// How long one question may take. Listing is the only call a scan waits on, and a machine with a
/// stalled hypervisor should make the scan say so rather than hang the window.
const QUERY_TIMEOUT: Duration = Duration::from_secs(20);

pub struct Vmrun {
    program: PathBuf,
}

impl Vmrun {
    /// Finds the command line, or explains that this machine has no Workstation this build can see.
    pub fn locate() -> Result<Self, ShellError> {
        if let Some(path) = std::env::var_os("VMRUN_PATH").map(PathBuf::from)
            && path.is_file()
        {
            return Ok(Self { program: path });
        }
        if let Some(path) = candidates().into_iter().find(|path| path.is_file()) {
            return Ok(Self { program: path });
        }
        if let Some(path) = find_on_path() {
            return Ok(Self { program: path });
        }
        Err(ShellError::vmware_unavailable(
            "the vmrun command was not found; install VMware Workstation Pro or set VMRUN_PATH",
        ))
    }

    pub fn program(&self) -> &Path {
        &self.program
    }

    /// The machines that are running right now.
    pub async fn list_running(&self) -> Result<Vec<String>, ShellError> {
        let output = self.run("list", &["-T", "ws", "list"]).await?;
        Ok(parse_running(&output))
    }

    /// Runs one command and answers its standard output, or the failure its own words.
    pub(crate) async fn run(&self, operation: &str, args: &[&str]) -> Result<String, ShellError> {
        self.run_within(QUERY_TIMEOUT, operation, args).await
    }

    /// The same, with a budget of its own. Power operations get longer: suspending a guest that
    /// is writing its memory to disk is not a query.
    pub(crate) async fn run_within(
        &self,
        budget: Duration,
        operation: &str,
        args: &[&str],
    ) -> Result<String, ShellError> {
        let mut command = Command::new(&self.program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // A call somebody stopped — or a window that closed while a guest was answering
            // slowly — must not leave a child holding a VM open.
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

/// Every line that names a `.vmx` is a running machine; the header line is not.
pub fn parse_running(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| line.to_ascii_lowercase().ends_with(".vmx"))
        .map(str::to_owned)
        .collect()
}

fn candidates() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let mut paths = Vec::new();
        if let Some(install) = registry_install_path() {
            paths.push(install.join("vmrun.exe"));
        }
        paths.extend(install_folders(
            std::env::var_os("ProgramFiles").map(PathBuf::from),
            std::env::var_os("ProgramFiles(x86)").map(PathBuf::from),
            "vmrun.exe",
        ));
        paths
    }
    #[cfg(not(windows))]
    {
        vec![
            PathBuf::from("/usr/bin/vmrun"),
            PathBuf::from("/usr/lib/vmware/bin/vmrun"),
            PathBuf::from("/usr/local/bin/vmrun"),
        ]
    }
}

/// The two default install folders, 64-bit first.
///
/// Workstation 26H1 is a 64-bit application installed under `Program Files`; an upgrade can leave
/// the old `Program Files (x86)` tree behind, so the new location is asked first.
#[cfg(windows)]
fn install_folders(programs: Option<PathBuf>, programs_x86: Option<PathBuf>, name: &str) -> Vec<PathBuf> {
    [programs, programs_x86]
        .into_iter()
        .flatten()
        .map(|root| root.join("VMware").join("VMware Workstation").join(name))
        .collect()
}

/// Where Workstation says it is installed. A machine with the application on another drive is a
/// normal install, not an exotic one; the default folder only covers the other case.
#[cfg(windows)]
fn registry_install_path() -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY};

    for flags in [KEY_READ | KEY_WOW64_64KEY, KEY_READ | KEY_WOW64_32KEY] {
        for key in [
            "SOFTWARE\\VMware, Inc.\\VMware Workstation",
            "SOFTWARE\\WOW6432Node\\VMware, Inc.\\VMware Workstation",
        ] {
            let Ok(opened) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(key, flags) else {
                continue;
            };
            let Ok(path) = opened.get_value::<String, _>("InstallPath") else {
                continue;
            };
            let path = PathBuf::from(path.trim());
            // A key left pointing at an upgraded-away folder is not an install: the folder may
            // remain with stale files while the binary this build runs now lives elsewhere.
            if path.join("vmrun.exe").is_file() {
                return Some(path);
            }
        }
    }
    None
}

fn find_on_path() -> Option<PathBuf> {
    let name = if cfg!(windows) { "vmrun.exe" } else { "vmrun" };
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
    fn the_running_list_is_every_line_that_names_a_document() {
        let output = "Total running VMs: 2\nE:\\VMs\\uos\\uos.vmx\nE:\\VMs\\win10\\win10.vmx\n";
        assert_eq!(
            parse_running(output),
            ["E:\\VMs\\uos\\uos.vmx", "E:\\VMs\\win10\\win10.vmx"]
        );
    }

    #[test]
    fn an_empty_running_list_is_an_empty_answer_not_an_error() {
        assert!(parse_running("Total running VMs: 0\n").is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn the_64_bit_install_folder_is_asked_before_the_legacy_one() {
        let folders = install_folders(
            Some(PathBuf::from(r"C:\Program Files")),
            Some(PathBuf::from(r"C:\Program Files (x86)")),
            "vmrun.exe",
        );
        assert_eq!(
            folders,
            [
                PathBuf::from(r"C:\Program Files\VMware\VMware Workstation\vmrun.exe"),
                PathBuf::from(r"C:\Program Files (x86)\VMware\VMware Workstation\vmrun.exe"),
            ]
        );
    }
}
