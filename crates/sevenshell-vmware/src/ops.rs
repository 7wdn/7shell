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
/// Keys go to a console, not to a guest account; there is nothing to wait on but the click.
const KEY_TIMEOUT: Duration = Duration::from_secs(30);

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
    let vmcli = Vmcli::beside(vmrun.program())
        .or_else(Vmcli::locate)
        .ok_or_else(crate::vmcli::unavailable)?;
    let args = match action {
        SnapshotAction::List => vec![path.clone(), "Snapshot".to_owned(), "query".to_owned()],
        SnapshotAction::Take(name) => vec![path.clone(), "Snapshot".to_owned(), "Take".to_owned(), name.clone()],
        SnapshotAction::Revert(name) => {
            let uid = snapshot_uid(&vmcli, &path, name).await?;
            vec![path.clone(), "Snapshot".to_owned(), "Revert".to_owned(), uid]
        }
        SnapshotAction::Delete { name, delete_children } => {
            let uid = snapshot_uid(&vmcli, &path, name).await?;
            let mut args = vec![path.clone(), "Snapshot".to_owned(), "Delete".to_owned()];
            if *delete_children {
                args.push("-d".to_owned());
            }
            args.push(uid);
            args
        }
    };
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    vmcli.run("snapshot", &refs, SNAPSHOT_TIMEOUT).await
}

/// The uid `vmcli` names a snapshot by, found in the query it prints.
///
/// The display name is what a person sees and what the tools take; the uid is what revert, delete
/// and clone insist on. The query lists each snapshot as `displayName` followed by `uid`.
async fn snapshot_uid(vmcli: &Vmcli, path: &str, name: &str) -> Result<String, ShellError> {
    let output = vmcli
        .run("snapshot", &[path, "Snapshot", "query"], SNAPSHOT_TIMEOUT)
        .await?;
    parse_snapshot_uid(&output, name).ok_or_else(|| {
        ShellError::vmware_command_failed("snapshot", format!("no snapshot named `{name}` is on this machine"))
    })
}

/// The uid of one snapshot, by display name.
fn parse_snapshot_uid(output: &str, name: &str) -> Option<String> {
    let mut current: Option<String> = None;
    for line in output.lines() {
        let line = line.trim().trim_start_matches('-').trim();
        if let Some(value) = line.strip_prefix("displayName:") {
            current = Some(value.trim().trim_matches('"').to_owned());
            continue;
        }
        if let Some(value) = line.strip_prefix("uid:") {
            let uid = value.trim();
            if current.as_deref() == Some(name) && !uid.is_empty() {
                return Some(uid.to_owned());
            }
        }
    }
    None
}

/// Sends one key sequence to the machine's console.
///
/// This is the console's own input, not a guest sign-in: it works while the guest has no account
/// or no VMware Tools, which is exactly when somebody needs to answer a boot prompt. The sequence
/// syntax is `vmcli`'s own (`esc`, `ctrl-alt-delete`, `a b c`, …), and the caller never builds it
/// from a path.
pub async fn send_key(vm_id: &str, sequence: &str) -> Result<String, ShellError> {
    let sequence = sequence.trim();
    if sequence.is_empty() {
        return Err(ShellError::vmware_command_failed(
            "send-key",
            "a key sequence is required",
        ));
    }
    if sequence.starts_with('-') {
        return Err(ShellError::vmware_command_failed(
            "send-key",
            "a key sequence may not start with `-`, which the command line would read as an option",
        ));
    }
    let vm_id = checked_id(vm_id, "send-key")?;
    let vmrun = Vmrun::locate()?;
    let (path, _) = resolve(&vmrun, &vm_id, "send-key").await?;
    let vmcli = Vmcli::beside(vmrun.program())
        .or_else(Vmcli::locate)
        .ok_or_else(crate::vmcli::unavailable)?;
    // `sendKeySequence` takes the sequence as a positional argument; there is no `-s` in this
    // Workstation's `vmcli` even though some documentation shows one.
    vmcli
        .run(
            "send-key",
            &[path.as_str(), "MKS", "sendKeySequence", sequence],
            KEY_TIMEOUT,
        )
        .await
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
    fn a_snapshot_uid_is_found_by_its_display_name() {
        let output = "currentUID: 1\nhelperUID: 0\nsnapshots: \n  - displayName: live\n    parentUID: 0\n    uid: 1\n  - displayName: older\n    parentUID: 0\n    uid: 2\n";
        assert_eq!(parse_snapshot_uid(output, "live").as_deref(), Some("1"));
        assert_eq!(parse_snapshot_uid(output, "older").as_deref(), Some("2"));
        assert_eq!(parse_snapshot_uid(output, "missing"), None);
    }

    #[tokio::test]
    async fn an_empty_key_sequence_never_reaches_vmcli() {
        let error = send_key("not-a-vm", "   ").await.unwrap_err();
        assert_eq!(
            error.code(),
            sevenshell_error::ErrorCode::VmwareCommandFailed,
            "an empty sequence is refused before the machine is even looked up"
        );
        let error = send_key("not-a-vm", "-x").await.unwrap_err();
        assert!(error.detail().unwrap_or_default().contains("option"));
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
