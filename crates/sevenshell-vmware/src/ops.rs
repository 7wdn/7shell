//! The operations an agent or the workspace asks for beyond power: snapshots and guest access.
//!
//! Snapshots are `vmcli`'s own module; guest operations are `vmrun`'s, because only that command
//! can sign in to a guest. The guest sign-in is the one place a password reaches a child process's
//! argument vector — `vmrun` offers no other form — so it is never put on an audit line, never
//! returned, and the session the call names is the only place it is read from.
//!
//! No operation here takes a local path: snapshot clone and host file copies are left out until
//! there is a token to name a host location with, which is the same rule the interface follows.

use std::time::Duration;

use sevenshell_error::ShellError;

use crate::vmcli::Vmcli;
use crate::vmrun::Vmrun;
use crate::{checked_id, resolve};

/// Reverting and deleting a snapshot can take a while; the budget is a backstop, not a poll.
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(120);
/// A program started in a guest may run for a while; the command itself bounds the wait.
const GUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// What may be done with snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotAction {
    List,
    Take(String),
    Revert(String),
    Delete { name: String, delete_children: bool },
}

/// The account a guest operation signs in as. Held only for the length of one call.
#[derive(Clone, PartialEq, Eq)]
pub struct GuestAuth {
    pub user: String,
    pub password: String,
}

impl std::fmt::Debug for GuestAuth {
    /// Deliberately without the password: a `Debug` that prints it is a log line waiting to happen.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuestAuth")
            .field("user", &self.user)
            .field("password", &"…")
            .finish()
    }
}

/// What may be done inside a guest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestOp {
    ListDirectory(String),
    FileExists(String),
    DirectoryExists(String),
    MakeDirectory(String),
    RemoveDirectory(String),
    RemoveFile(String),
    Rename {
        from: String,
        to: String,
    },
    Run {
        program: String,
        args: Vec<String>,
        no_wait: bool,
        interactive: bool,
    },
    RunScript {
        interpreter: String,
        script: String,
        no_wait: bool,
        interactive: bool,
    },
    Type(String),
    TempFile,
    Processes,
    Kill(u64),
    ToolsState,
    ToolsInstall,
}

impl GuestOp {
    /// The name the policy table and the audit line use.
    pub const fn action(&self) -> &'static str {
        match self {
            Self::ListDirectory(_) => "ls",
            Self::FileExists(_) => "file-exists",
            Self::DirectoryExists(_) => "directory-exists",
            Self::MakeDirectory(_) => "mkdir",
            Self::RemoveDirectory(_) => "rmdir",
            Self::RemoveFile(_) => "rm",
            Self::Rename { .. } => "rename",
            Self::Run { .. } => "run",
            Self::RunScript { .. } => "script",
            Self::Type(_) => "type",
            Self::TempFile => "temp-file",
            Self::Processes => "processes",
            Self::Kill(_) => "kill",
            Self::ToolsState => "tools-state",
            Self::ToolsInstall => "tools-install",
        }
    }
}

/// One snapshot action on one machine, as `vmcli` prints it.
pub async fn snapshot(vm_id: &str, action: &SnapshotAction) -> Result<String, ShellError> {
    let vm_id = checked_id(vm_id, "snapshot")?;
    let vmrun = Vmrun::locate()?;
    let (path, _) = resolve(&vmrun, &vm_id, "snapshot").await?;
    let vmcli = Vmcli::beside(vmrun.program()).or_else(Vmcli::locate).ok_or_else(|| {
        ShellError::vmware_unavailable("the vmcli command was not found; snapshot operations need it")
    })?;
    let args = snapshot_args(&path, action);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    vmcli.run("snapshot", &refs, SNAPSHOT_TIMEOUT).await
}

fn snapshot_args(path: &str, action: &SnapshotAction) -> Vec<String> {
    let base = |command: &str| vec![path.to_owned(), "Snapshot".to_owned(), command.to_owned()];
    match action {
        SnapshotAction::List => base("query"),
        SnapshotAction::Take(name) => {
            let mut args = base("Take");
            args.extend(["-n".to_owned(), name.clone()]);
            args
        }
        SnapshotAction::Revert(name) => {
            let mut args = base("Revert");
            args.extend(["-n".to_owned(), name.clone()]);
            args
        }
        SnapshotAction::Delete { name, delete_children } => {
            let mut args = base("Delete");
            args.extend(["-n".to_owned(), name.clone()]);
            if *delete_children {
                args.push("--andDeleteChildren".to_owned());
            }
            args
        }
    }
}

/// One guest operation on one machine, signed in with the session's guest account.
pub async fn guest(vm_id: &str, op: &GuestOp, auth: &GuestAuth) -> Result<String, ShellError> {
    let vm_id = checked_id(vm_id, "guest")?;
    let vmrun = Vmrun::locate()?;
    let (path, _) = resolve(&vmrun, &vm_id, "guest").await?;
    let args = guest_args(&path, op, auth);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    vmrun.run_within(GUEST_TIMEOUT, "guest", &refs).await
}

/// `vmrun` wants its authentication flags before the command, and each argument on its own: a
/// program argument with a space is one argument here because the process API says so.
fn guest_args(path: &str, op: &GuestOp, auth: &GuestAuth) -> Vec<String> {
    let mut args = vec![
        "-T".to_owned(),
        "ws".to_owned(),
        "-gu".to_owned(),
        auth.user.clone(),
        "-gp".to_owned(),
        auth.password.clone(),
    ];
    let command = |args: &mut Vec<String>, name: &str| {
        args.push(name.to_owned());
        args.push(path.to_owned());
    };
    match op {
        GuestOp::ListDirectory(directory) => {
            command(&mut args, "listDirectoryInGuest");
            args.push(directory.clone());
        }
        GuestOp::FileExists(path_in_guest) => {
            command(&mut args, "fileExistsInGuest");
            args.push(path_in_guest.clone());
        }
        GuestOp::DirectoryExists(path_in_guest) => {
            command(&mut args, "directoryExistsInGuest");
            args.push(path_in_guest.clone());
        }
        GuestOp::MakeDirectory(directory) => {
            command(&mut args, "createDirectoryInGuest");
            args.push(directory.clone());
        }
        GuestOp::RemoveDirectory(directory) => {
            command(&mut args, "deleteDirectoryInGuest");
            args.push(directory.clone());
        }
        GuestOp::RemoveFile(file) => {
            command(&mut args, "deleteFileInGuest");
            args.push(file.clone());
        }
        GuestOp::Rename { from, to } => {
            command(&mut args, "renameFileInGuest");
            args.push(from.clone());
            args.push(to.clone());
        }
        GuestOp::Run {
            program,
            args: program_args,
            no_wait,
            interactive,
        } => {
            command(&mut args, "runProgramInGuest");
            if *no_wait {
                args.push("-noWait".to_owned());
            }
            if *interactive {
                args.push("-interactive".to_owned());
            }
            args.push(program.clone());
            args.extend(program_args.iter().cloned());
        }
        GuestOp::RunScript {
            interpreter,
            script,
            no_wait,
            interactive,
        } => {
            command(&mut args, "runScriptInGuest");
            if *no_wait {
                args.push("-noWait".to_owned());
            }
            if *interactive {
                args.push("-interactive".to_owned());
            }
            args.push(interpreter.clone());
            args.push(script.clone());
        }
        GuestOp::Type(keystrokes) => {
            command(&mut args, "typeKeystrokesInGuest");
            args.push(keystrokes.clone());
        }
        GuestOp::TempFile => command(&mut args, "CreateTempfileInGuest"),
        GuestOp::Processes => command(&mut args, "listProcessesInGuest"),
        GuestOp::Kill(pid) => {
            command(&mut args, "killProcessInGuest");
            args.push(pid.to_string());
        }
        GuestOp::ToolsState => command(&mut args, "checkToolsState"),
        GuestOp::ToolsInstall => command(&mut args, "installTools"),
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth() -> GuestAuth {
        GuestAuth {
            user: "z".to_owned(),
            password: "hunter2".to_owned(),
        }
    }

    #[test]
    fn a_snapshot_argument_vector_names_the_machine_and_the_snapshot() {
        let path = "e:\\vms\\a.vmx";
        assert_eq!(snapshot_args(path, &SnapshotAction::List), [path, "Snapshot", "query"]);
        assert_eq!(
            snapshot_args(path, &SnapshotAction::Take("before upgrade".to_owned())),
            [path, "Snapshot", "Take", "-n", "before upgrade"]
        );
        assert_eq!(
            snapshot_args(
                path,
                &SnapshotAction::Delete {
                    name: "old".to_owned(),
                    delete_children: true
                }
            ),
            [path, "Snapshot", "Delete", "-n", "old", "--andDeleteChildren"]
        );
    }

    #[test]
    fn guest_arguments_put_the_auth_flags_before_the_command() {
        let args = guest_args("e:\\vms\\a.vmx", &GuestOp::ListDirectory("/home/z".to_owned()), &auth());
        assert_eq!(
            args,
            [
                "-T",
                "ws",
                "-gu",
                "z",
                "-gp",
                "hunter2",
                "listDirectoryInGuest",
                "e:\\vms\\a.vmx",
                "/home/z"
            ]
        );
    }

    #[test]
    fn a_program_and_each_of_its_arguments_stay_separate() {
        let args = guest_args(
            "e:\\vms\\a.vmx",
            &GuestOp::Run {
                program: "/bin/echo".to_owned(),
                args: vec!["two words".to_owned(), "third".to_owned()],
                no_wait: true,
                interactive: false,
            },
            &auth(),
        );
        assert_eq!(
            &args[6..],
            [
                "runProgramInGuest",
                "e:\\vms\\a.vmx",
                "-noWait",
                "/bin/echo",
                "two words",
                "third"
            ]
        );
    }

    #[test]
    fn the_debug_form_of_a_guest_auth_keeps_the_password_out() {
        let text = format!("{:?}", auth());
        assert!(text.contains("z"));
        assert!(!text.contains("hunter2"));
    }
}
