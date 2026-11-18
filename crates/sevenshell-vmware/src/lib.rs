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

mod config;
mod host_ops;
mod inventory;
mod ops;
mod vmcli;
mod vmrest;
mod vmrun;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sevenshell_error::ShellError;
use sevenshell_types::{VmInfo, VmPowerAction, VmStatus};

pub use config::{
    ConfigRead, ConfigWrite, HostRead, HostWrite, ImageAction, config_read, config_write, guest_env, host_read,
    host_write, image,
};
pub use host_ops::{
    clone, copy_from_guest, copy_to_guest, hgfs_set_host_path, screenshot, shared_add, shared_set_state,
    snapshot_clone, template_create, template_deploy,
};
pub use inventory::inventory_path;
pub use ops::{GuestAuth, GuestOp, SnapshotAction, guest, send_key, snapshot};
pub use vmrest::{Vmrest, locate as locate_vmrest};
pub use vmrun::{Vmrun, parse_running};

use vmcli::Vmcli;

/// Power operations wait for a guest to finish, which is not a question's budget.
const POWER_TIMEOUT: Duration = Duration::from_secs(120);

/// A scan asks every running machine for its address; one guest that cannot answer must not hold
/// the list back for the command's full budget.
const GUEST_IP_BUDGET: Duration = Duration::from_secs(3);

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
        let ip = if is_running {
            guest_ip(&vmrun, &entry.path).await
        } else {
            None
        };
        let machine = VmInfo {
            id: id.clone(),
            name: pick_name(Some(&entry.display_name), info.display_name.as_deref(), &entry.path),
            guest_os: info.guest_os,
            running: is_running,
            ip,
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
        let ip = guest_ip(&vmrun, &path).await;
        machines.entry(id.clone()).or_insert(VmInfo {
            id,
            name: pick_name(None, info.display_name.as_deref(), &path),
            guest_os: info.guest_os,
            running: true,
            ip,
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
    let (path, listed_running) = resolve(&vmrun, &vm_id, "status").await?;

    // Workstation's own answer first: `vmrun list` cannot tell a suspended machine from a
    // stopped one, and that is the distinction the interface draws. The `.vmss` file beside the
    // document is the fallback for an install with no `vmcli`.
    let queried = match Vmcli::beside(vmrun.program()).or_else(Vmcli::locate) {
        Some(vmcli) => vmcli.power_state(&path).await,
        None => None,
    };
    let (running, suspended) = derive_power(queried.as_deref(), listed_running, suspend_state_exists(&path));
    // The guest identifier lives in the document, not in the runtime: it is an answer whether the
    // machine is on, suspended or off.
    let guest_os = inventory::read_vmx(&path).await.and_then(|info| info.guest_os);

    let mut status = VmStatus {
        id: vm_id,
        running,
        suspended,
        guest_os,
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
    status.ip = guest_ip(&vmrun, path).await;
    Ok(status)
}

/// The guest's address, with the scan's short budget. A failing `getGuestIPAddress` prints a
/// sentence, and a sentence is not an address.
async fn guest_ip(vmrun: &Vmrun, vmx: &str) -> Option<String> {
    let output = vmrun
        .run_within(
            GUEST_IP_BUDGET,
            "getGuestIPAddress",
            &["-T", "ws", "getGuestIPAddress", vmx],
        )
        .await
        .ok()?;
    first_ip(&output)
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
        // A resume asked for a moment after a suspend can meet files the old process has not
        // finished releasing; vmrun answers "already in use" for a second or two. Retrying is
        // what makes "suspend, then resume" work without asking the person to wait on a timer.
        let mut last: Option<ShellError> = None;
        for attempt in 0..4u8 {
            match vmrun
                .run_within(POWER_TIMEOUT, "start", &["-T", "ws", "start", path.as_str(), "nogui"])
                .await
            {
                Ok(_) => return Ok(()),
                Err(error) => {
                    let retryable = error
                        .detail()
                        .is_some_and(|detail| detail.to_ascii_lowercase().contains("already in use"));
                    last = Some(error);
                    if !retryable || attempt == 3 {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(1_200)).await;
                }
            }
        }
        return Err(last.expect("the loop runs at least once"));
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
        VmPowerAction::Reset => vec!["-T", "ws", "reset", path, "hard"],
        VmPowerAction::Pause => vec!["-T", "ws", "pause", path],
        VmPowerAction::Unpause => vec!["-T", "ws", "unpause", path],
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

/// The state to draw from three sources: what `vmcli` answered, what `vmrun list` named, and
/// whether a suspended-state file is there.
///
/// Workstation's own answer wins whenever it has one. Without it, a machine the running list names
/// is running, and one that is not named but has a `.vmss` beside it is suspended rather than off.
fn derive_power(queried: Option<&str>, listed_running: bool, suspend_file: bool) -> (bool, bool) {
    match queried {
        Some("on") => (true, false),
        Some("suspended") => (false, true),
        Some("off") => (false, false),
        _ if listed_running => (true, false),
        _ => (false, suspend_file),
    }
}

/// Whether this machine's suspend state file is present.
///
/// Newer Workstation writes it as `<name>-<hash>.vmss` beside the document; older ones use the
/// document's own name. Both are checked, and neither is read — only its presence is the answer.
fn suspend_state_exists(vmx: &str) -> bool {
    let path = Path::new(vmx);
    let (Some(parent), Some(stem)) = (
        path.parent(),
        path.file_stem().map(|stem| stem.to_string_lossy().to_lowercase()),
    ) else {
        return false;
    };
    let plain = format!("{stem}.vmss");
    let hashed = format!("{stem}-");
    std::fs::read_dir(parent)
        .map(|entries| {
            entries.flatten().any(|entry| {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                name.ends_with(".vmss") && (name == plain || name.starts_with(&hashed))
            })
        })
        .unwrap_or(false)
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
pub(crate) async fn resolve(vmrun: &Vmrun, vm_id: &str, operation: &str) -> Result<(String, bool), ShellError> {
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
pub(crate) fn checked_id(vm_id: &str, operation: &str) -> Result<String, ShellError> {
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

    #[test]
    fn workstations_answer_wins_over_the_two_fallbacks() {
        assert_eq!(derive_power(Some("suspended"), true, false), (false, true));
        assert_eq!(derive_power(Some("on"), false, true), (true, false));
        assert_eq!(derive_power(Some("off"), true, true), (false, false));
        assert_eq!(derive_power(None, true, false), (true, false));
        assert_eq!(derive_power(None, false, true), (false, true));
        assert_eq!(derive_power(None, false, false), (false, false));
    }

    #[test]
    fn a_suspend_state_file_is_recognised_with_or_without_the_hash() {
        let directory = std::env::temp_dir().join(format!("7shell-vmware-vmss-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("temp dir");
        let vmx = directory.join("Machine.vmx");
        std::fs::write(&vmx, "").expect("fixture");
        let vmx = vmx.to_string_lossy().into_owned();

        assert!(!suspend_state_exists(&vmx), "nothing suspended yet");
        std::fs::write(directory.join("Machine-9f2e4bb1.vmss"), "").expect("state file");
        assert!(suspend_state_exists(&vmx), "the hashed name counts");
        std::fs::remove_file(directory.join("Machine-9f2e4bb1.vmss")).expect("remove");
        std::fs::write(directory.join("Machine.vmss"), "").expect("state file");
        assert!(suspend_state_exists(&vmx), "the plain name counts");
        std::fs::write(directory.join("Other-1234.vmss"), "").expect("other machine");
        std::fs::remove_file(directory.join("Machine.vmss")).expect("remove");
        assert!(!suspend_state_exists(&vmx), "another machine's state is not this one's");
        let _ = std::fs::remove_dir_all(&directory);
    }
}
