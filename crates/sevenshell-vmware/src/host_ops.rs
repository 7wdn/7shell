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
///
/// The console capture lives in `vmcli`'s MKS module; `vmrun captureScreen` is routed through the
/// guest-operations API and refuses a machine nobody is signed in to, which is exactly the
/// powered-off or unbootable machine a screenshot is most useful for.
pub async fn screenshot(vm_id: &str, output_path: &str) -> Result<String, ShellError> {
    let vm_id = checked_id(vm_id, "screenshot")?;
    let vmrun = Vmrun::locate()?;
    let (path, _) = resolve(&vmrun, &vm_id, "screenshot").await?;
    let vmcli = Vmcli::beside(vmrun.program())
        .or_else(Vmcli::locate)
        .ok_or_else(crate::vmcli::unavailable)?;
    vmcli
        .run(
            "screenshot",
            &[path.as_str(), "MKS", "captureScreenshot", output_path],
            OPERATION_TIMEOUT,
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
        .ok_or_else(crate::vmcli::unavailable)?;
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

/// Clones one snapshot of a machine into a new `.vmx` the core built from a folder token.
///
/// `vmcli` names a snapshot by its uid here, not by its display name; the help of this very
/// command says so.
pub async fn snapshot_clone(
    vm_id: &str,
    uid: &str,
    dest_vmx: &str,
    name: &str,
    linked: bool,
) -> Result<String, ShellError> {
    let vm_id = checked_id(vm_id, "snapshot-clone")?;
    let vmrun = Vmrun::locate()?;
    let (path, _) = resolve(&vmrun, &vm_id, "snapshot-clone").await?;
    let vmcli = Vmcli::beside(vmrun.program())
        .or_else(Vmcli::locate)
        .ok_or_else(crate::vmcli::unavailable)?;
    let mut args = vec![path.as_str(), "Snapshot", "Clone"];
    if linked {
        args.push("-l");
    }
    args.extend([uid, dest_vmx, name]);
    vmcli.run("snapshot-clone", &args, OPERATION_TIMEOUT).await
}

/// Creates a `.vmtx` template from a machine. Both paths came from a folder token.
pub async fn template_create(vm_id: &str, template_path: &str, display_name: &str) -> Result<String, ShellError> {
    let vm_id = checked_id(vm_id, "template-create")?;
    let vmrun = Vmrun::locate()?;
    let (path, _) = resolve(&vmrun, &vm_id, "template-create").await?;
    let vmcli = Vmcli::beside(vmrun.program())
        .or_else(Vmcli::locate)
        .ok_or_else(crate::vmcli::unavailable)?;
    vmcli
        .run(
            "template-create",
            &[
                path.as_str(),
                "VMTemplate",
                "Create",
                "-p",
                template_path,
                "-n",
                display_name,
            ],
            OPERATION_TIMEOUT,
        )
        .await
}

/// Deploys an existing `.vmtx` template. The template decides where the machine lands.
pub async fn template_deploy(template_path: &str) -> Result<String, ShellError> {
    let vmrun = Vmrun::locate()?;
    let vmcli = Vmcli::beside(vmrun.program())
        .or_else(Vmcli::locate)
        .ok_or_else(crate::vmcli::unavailable)?;
    vmcli
        .run(
            "template-deploy",
            &["VMTemplate", "Deploy", "-p", template_path],
            OPERATION_TIMEOUT,
        )
        .await
}
