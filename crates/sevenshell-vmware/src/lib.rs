//! VMware Workstation virtual machines discovered and driven from this machine.
//!
//! The only interface Workstation publishes for automation is its command line, so this crate is
//! an argument-vector builder and an output parser: `vmrun list` answers which machines are
//! running, Workstation's own `inventory.vmls` answers which machines exist at all, and the three
//! `.vmx` fields this build needs (a display name, the guest identifier, the machine's UUID) come
//! from the document itself.
//!
//! The UUID is the identity the interface receives. A `.vmx` path never leaves this crate: the
//! interface holds a token and every action resolves it against the inventory first, which is also
//! what keeps a saved session pointing at the right machine after somebody renames or moves it.
//! Nothing here reads or keeps a guest credential.

mod inventory;
mod vmrun;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sevenshell_error::ShellError;
use sevenshell_types::{VmInfo, VmPowerAction, VmStatus};

pub use inventory::inventory_path;
pub use vmrun::{Vmrun, parse_running};

/// Power operations wait for a guest to finish, which is not a question's budget.
const POWER_TIMEOUT: Duration = Duration::from_secs(120);

/// Every machine this build can act on, running or not.
pub async fn scan() -> Result<Vec<VmInfo>, ShellError> {
    let vmrun = Vmrun::locate()?;
    let running = vmrun.list_running().await?;
    let mut machines: BTreeMap<String, VmInfo> = BTreeMap::new();
    let mut known_paths: Vec<String> = Vec::new();

    for entry in inventory::load() {
        if !Path::new(&entry.path).is_file() {
            // The inventory keeps entries for machines on drives that are not attached. They are
            // not offered: every action on one would fail at vmrun, one click later.
            continue;
        }
        let Some(info) = inventory::read_vmx(&entry.path).await else {
            continue;
        };
        known_paths.push(entry.path.clone());
        // A machine with no readable UUID cannot be addressed by the token the interface holds,
        // so it is left out rather than offered as something the next click cannot resolve.
        let Some(id) = info.uuid else {
            continue;
        };
        let is_running = running.iter().any(|path| inventory::same_path(path, &entry.path));
        let machine = VmInfo {
            id: id.clone(),
            name: pick_name(Some(&entry.display_name), info.display_name.as_deref(), &entry.path),
            guest_os: info.guest_os,
            running: is_running,
        };
        machines
            .entry(id)
            .and_modify(|existing| existing.running |= is_running)
            .or_insert(machine);
    }

    for path in running {
        if known_paths.iter().any(|known| inventory::same_path(known, &path)) {
            continue;
        }
        let Some(info) = inventory::read_vmx(&path).await else {
            continue;
        };
        let Some(id) = info.uuid else {
            continue;
        };
        machines.entry(id.clone()).or_insert(VmInfo {
            id,
            name: pick_name(None, info.display_name.as_deref(), &path),
            guest_os: info.guest_os,
            running: true,
        });
    }

    let mut machines: Vec<VmInfo> = machines.into_values().collect();
    machines.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(machines)
}

/// What one machine is doing right now, including the address a guest reported.
pub async fn status(vm_id: &str) -> Result<VmStatus, ShellError> {
    let vm_id = checked_id(vm_id, "status")?;
    let vmrun = Vmrun::locate()?;
    let (path, running) = resolve(&vmrun, &vm_id, "status").await?;

    let mut status = VmStatus {
        id: vm_id,
        running,
        locked: lock_exists(&path),
        tools_state: None,
        ip: None,
    };
    if !running {
        return Ok(status);
    }

    let path = path.as_str();
    if let Ok(output) = vmrun
        .run("checkToolsState", &["-T", "ws", "checkToolsState", path])
        .await
    {
        status.tools_state = output
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_owned);
    }
    // The address is a courtesy: a guest that has not booted far enough to answer is not a
    // failure of the status call.
    if let Ok(output) = vmrun
        .run("getGuestIPAddress", &["-T", "ws", "getGuestIPAddress", path])
        .await
    {
        status.ip = first_ip(&output);
    }
    Ok(status)
}

/// Starts, stops or suspends one machine.
pub async fn power(vm_id: &str, action: VmPowerAction) -> Result<(), ShellError> {
    let vm_id = checked_id(vm_id, action.as_str())?;
    let vmrun = Vmrun::locate()?;
    let (path, _) = resolve(&vmrun, &vm_id, action.as_str()).await?;

    if action == VmPowerAction::Start {
        // Workstation hands a GUI start to its own window, and that handoff fails with a bare
        // "Unknown error" on machines where the UI is running under a different elevation (or
        // when it refuses to open another window). The machine itself starts perfectly well
        // without it, so the fallback keeps the power action working; the guest is reached over
        // SSH or RDP, not through that window.
        if vmrun
            .run_within(POWER_TIMEOUT, "start", &["-T", "ws", "start", path.as_str(), "gui"])
            .await
            .is_ok()
        {
            return Ok(());
        }
        vmrun
            .run_within(POWER_TIMEOUT, "start", &["-T", "ws", "start", path.as_str(), "nogui"])
            .await?;
        return Ok(());
    }

    let args = power_args(action, path.as_str());
    vmrun.run_within(POWER_TIMEOUT, action.as_str(), &args).await?;
    Ok(())
}

/// The argument vector for the actions that have one shape.
fn power_args(action: VmPowerAction, path: &str) -> Vec<&str> {
    match action {
        VmPowerAction::Start => vec!["-T", "ws", "start", path, "gui"],
        VmPowerAction::Stop => vec!["-T", "ws", "stop", path, "soft"],
        VmPowerAction::Suspend => vec!["-T", "ws", "suspend", path, "soft"],
    }
}

/// Takes ownership of a machine that is not running by removing the lock an unfinished run left.
///
/// Workstation's own window offers the same thing as "Take ownership". The lock is only removed
/// when `vmrun` says the machine is not running: a lock that belongs to a live machine is the
/// thing keeping two hypervisors from writing the same disks.
pub async fn take_ownership(vm_id: &str) -> Result<(), ShellError> {
    let vm_id = checked_id(vm_id, "take-ownership")?;
    let vmrun = Vmrun::locate()?;
    let (path, running) = resolve(&vmrun, &vm_id, "take-ownership").await?;
    if running {
        return Err(ShellError::vmware_command_failed(
            "take-ownership",
            "the machine is running; its lock is not stale",
        ));
    }
    remove_lock(&path)?;
    Ok(())
}

fn lock_path(vmx: &str) -> PathBuf {
    PathBuf::from(format!("{vmx}.lck"))
}

/// Whether Workstation's lock directory sits beside the document.
fn lock_exists(vmx: &str) -> bool {
    lock_path(vmx).exists()
}

/// Removes the lock directory, if there is one. A lock that is not there is already taken care
/// of, not a failure.
fn remove_lock(vmx: &str) -> Result<bool, ShellError> {
    let lock = lock_path(vmx);
    if !lock.exists() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&lock).map_err(|error| {
        ShellError::vmware_command_failed("take-ownership", format!("the lock could not be removed: {error}"))
    })?;
    Ok(true)
}

/// Turns a token into the document `vmrun` needs, and says whether it is running.
///
/// The inventory is consulted on every action rather than cached: a machine that was moved,
/// renamed or removed since the scan is then answered for what it is now, and a token the
/// interface holds stays useful across those changes.
async fn resolve(vmrun: &Vmrun, vm_id: &str, operation: &str) -> Result<(String, bool), ShellError> {
    let running = vmrun.list_running().await?;
    let mut paths: Vec<String> = inventory::load().into_iter().map(|entry| entry.path).collect();
    for path in &running {
        if !paths.iter().any(|known| inventory::same_path(known, path)) {
            paths.push(path.clone());
        }
    }

    for path in paths {
        if !Path::new(&path).is_file() {
            continue;
        }
        let Some(info) = inventory::read_vmx(&path).await else {
            continue;
        };
        if info.uuid.as_deref() == Some(vm_id) {
            let is_running = running.iter().any(|candidate| inventory::same_path(candidate, &path));
            return Ok((path, is_running));
        }
    }
    Err(ShellError::vmware_command_failed(
        operation,
        "no virtual machine with that id is known to this Workstation",
    ))
}

/// The first line that is an address. A failing `getGuestIPAddress` prints a sentence, and a
/// sentence is not an address.
fn first_ip(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|line| line.parse::<std::net::IpAddr>().is_ok())
        .map(str::to_owned)
}

fn pick_name(inventory_name: Option<&str>, vmx_name: Option<&str>, path: &str) -> String {
    inventory_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .or_else(|| vmx_name.map(str::trim).filter(|name| !name.is_empty()))
        .map(str::to_owned)
        .unwrap_or_else(|| file_stem(path))
}

fn file_stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

/// A token an action may be pointed at: a UUID-shaped identity, normalized.
fn checked_id(vm_id: &str, operation: &str) -> Result<String, ShellError> {
    inventory::normalize_uuid(vm_id.trim())
        .ok_or_else(|| ShellError::vmware_command_failed(operation, "the id is not a virtual machine identity"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_read_from_a_sentence_about_one() {
        assert_eq!(first_ip("192.168.254.130\n"), Some("192.168.254.130".to_owned()));
        assert_eq!(first_ip("Error: The guest has not reported an IP address\n"), None);
        assert_eq!(first_ip("fe80::1\n"), Some("fe80::1".to_owned()));
    }

    #[test]
    fn the_name_falls_back_from_the_inventory_to_the_document_to_the_file() {
        assert_eq!(pick_name(Some("uos_test"), Some("uos"), "e:\\uos\\uos.vmx"), "uos_test");
        assert_eq!(pick_name(None, Some("uos"), "e:\\uos\\uos.vmx"), "uos");
        assert_eq!(pick_name(None, None, "e:\\uos\\uos.vmx"), "uos");
    }

    #[test]
    fn a_token_that_is_not_a_identity_is_refused_before_anything_runs() {
        let error = checked_id("not-a-vm", "status").unwrap_err();
        assert_eq!(error.code(), sevenshell_error::ErrorCode::VmwareCommandFailed);
        assert!(
            checked_id("E:\\VMs\\uos\\uos.vmx", "status").is_err(),
            "a path is not a token"
        );
        assert!(checked_id("56 4d b5 f2 35 15 de 03-2c d9 33 8b a8 d8 31 b7", "status").is_ok());
    }

    #[test]
    fn each_power_action_builds_the_arguments_vmrun_documents() {
        let path = "e:\\vms\\a.vmx";
        assert_eq!(
            power_args(VmPowerAction::Start, path),
            ["-T", "ws", "start", path, "gui"]
        );
        assert_eq!(
            power_args(VmPowerAction::Stop, path),
            ["-T", "ws", "stop", path, "soft"]
        );
        assert_eq!(
            power_args(VmPowerAction::Suspend, path),
            ["-T", "ws", "suspend", path, "soft"]
        );
    }

    #[test]
    fn a_stale_lock_is_found_and_removed_and_a_missing_one_is_not_an_error() {
        let directory = std::env::temp_dir().join(format!("7shell-vmware-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("temp dir");
        let vmx = directory.join("machine.vmx");
        std::fs::write(&vmx, "displayName = \"machine\"\n").expect("fixture");
        let lock = directory.join("machine.vmx.lck");
        std::fs::create_dir_all(&lock).expect("lock dir");
        std::fs::write(lock.join("M1.lck"), "").expect("lock file");

        let vmx = vmx.to_string_lossy().into_owned();
        assert!(lock_exists(&vmx), "a lock beside the document is seen");
        assert!(remove_lock(&vmx).expect("the lock is removed"));
        assert!(!lock_exists(&vmx), "the lock is gone");
        assert!(
            !remove_lock(&vmx).expect("an absent lock is fine"),
            "nothing left to remove"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }
}
