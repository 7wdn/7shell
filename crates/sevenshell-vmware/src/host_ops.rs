//! The operations whose other end is a folder on this machine.
//!
//! Every function here takes a host path, and every caller resolves it from an opaque token the
//! host minted after a native dialog. These are the only paths in this crate by design, and they
//! are never printed, audited or sent anywhere: the agent and the interface only ever see the
//! token, the display name and — for a screenshot — the file name that was written.

use std::time::Duration;

use sevenshell_error::ShellError;

use crate::ops::GuestAuth;
use crate::vmcli::Vmcli;
use crate::vmrun::Vmrun;
use crate::{checked_id, resolve};

const OPERATION_TIMEOUT: Duration = Duration::from_secs(300);

/// Copies one host file into the guest. `host_path` came from a file token.
pub async fn copy_to_guest(
    vm_id: &str,
    host_path: &str,
    guest_path: &str,
    auth: &GuestAuth,
) -> Result<String, ShellError> {
    let vm_id = checked_id(vm_id, "copy-to")?;
    let vmrun = Vmrun::locate()?;
    let (path, _) = resolve(&vmrun, &vm_id, "copy-to").await?;
    vmrun
        .run_within(
            OPERATION_TIMEOUT,
            "copy-to",
            &[
                "-T",
                "ws",
                "-gu",
                auth.user.as_str(),
                "-gp",
                auth.password.as_str(),
                "CopyFileFromHostToGuest",
                &path,
                host_path,
                guest_path,
            ],
        )
        .await
}

/// Copies one guest file out to a host folder. `host_path` is the full destination path, built by
/// the core from a folder token and the guest file's own name.
pub async fn copy_from_guest(
    vm_id: &str,
    guest_path: &str,
    host_path: &str,
    auth: &GuestAuth,
) -> Result<String, ShellError> {
    let vm_id = checked_id(vm_id, "copy-from")?;
    let vmrun = Vmrun::locate()?;
    let (path, _) = resolve(&vmrun, &vm_id, "copy-from").await?;
    vmrun
        .run_within(
            OPERATION_TIMEOUT,
            "copy-from",
            &[
                "-T",
                "ws",
                "-gu",
                auth.user.as_str(),
                "-gp",
                auth.password.as_str(),
                "CopyFileFromGuestToHost",
                &path,
                guest_path,
                host_path,
            ],
        )
        .await
}

/// Clones one machine into a new `.vmx` document the core built from a folder token and a name.
pub async fn clone(
    vm_id: &str,
    dest_vmx: &str,
    clone_type: &str,
    snapshot: Option<&str>,
) -> Result<String, ShellError> {
    let vm_id = checked_id(vm_id, "clone")?;
    let vmrun = Vmrun::locate()?;
    let (path, _) = resolve(&vmrun, &vm_id, "clone").await?;
    let mut args = vec!["-T", "ws", "clone", path.as_str(), dest_vmx, clone_type];
    let snapshot_flag;
    if let Some(snapshot) = snapshot {
        snapshot_flag = format!("-snapshot={snapshot}");
        args.push(&snapshot_flag);
    }
    vmrun.run_within(OPERATION_TIMEOUT, "clone", &args).await
}

/// Captures the machine's screen into a file the core built from a folder token and a name.
pub async fn screenshot(vm_id: &str, output_path: &str) -> Result<String, ShellError> {
    let vm_id = checked_id(vm_id, "screenshot")?;
    let vmrun = Vmrun::locate()?;
    let (path, _) = resolve(&vmrun, &vm_id, "screenshot").await?;
    vmrun
        .run_within(
            OPERATION_TIMEOUT,
            "screenshot",
            &["-T", "ws", "captureScreen", &path, output_path],
        )
        .await
}

/// Points one shared folder at a host directory. `host_path` came from a folder token.
pub async fn hgfs_set_host_path(vm_id: &str, index: u32, host_path: &str) -> Result<String, ShellError> {
    let vm_id = checked_id(vm_id, "hgfs")?;
    let vmrun = Vmrun::locate()?;
    let (path, _) = resolve(&vmrun, &vm_id, "hgfs").await?;
    let vmcli = Vmcli::beside(vmrun.program())
        .or_else(Vmcli::locate)
        .ok_or_else(|| ShellError::vmware_unavailable("the vmcli command was not found; this operation needs it"))?;
    let index = index.to_string();
    vmcli
        .run(
            "hgfs",
            &[path.as_str(), "HGFS", "SetHostPath", "-i", &index, "-p", host_path],
            OPERATION_TIMEOUT,
        )
        .await
}

/// Adds a shared folder for a host directory. `host_path` came from a folder token.
pub async fn shared_add(vm_id: &str, name: &str, host_path: &str) -> Result<String, ShellError> {
    let vm_id = checked_id(vm_id, "shared-add")?;
    let vmrun = Vmrun::locate()?;
    let (path, _) = resolve(&vmrun, &vm_id, "shared-add").await?;
    vmrun
        .run_within(
            OPERATION_TIMEOUT,
            "shared-add",
            &["-T", "ws", "addSharedFolder", &path, name, host_path],
        )
        .await
}

/// Sets whether one shared folder is writable. `host_path` came from a folder token.
pub async fn shared_set_state(vm_id: &str, name: &str, host_path: &str, writable: bool) -> Result<String, ShellError> {
    let vm_id = checked_id(vm_id, "shared-set")?;
    let vmrun = Vmrun::locate()?;
    let (path, _) = resolve(&vmrun, &vm_id, "shared-set").await?;
    let mode = if writable { "writable" } else { "readonly" };
    vmrun
        .run_within(
            OPERATION_TIMEOUT,
            "shared-set",
            &["-T", "ws", "setSharedFolderState", &path, name, host_path, mode],
        )
        .await
}
