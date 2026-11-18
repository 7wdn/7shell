//! What a VMware power action could do, before it runs.
//!
//! These are not command-line classifications: by the time one of these is asked, the action is
//! already a fixed verb with a fixed argv. What is being decided is whether the interface may send
//! it without an explicit acknowledgement — the same gate container actions use.
//!
//! Starting is a change, and hanging or shutting a running machine down interrupts whatever it is
//! doing. Stopping is destructive in the same sense a container stop is: the guest may lose what
//! it has not written down.

use sevenshell_types::{RiskAssessment, RiskLevel, RiskReason};

/// Classifies one VM power operation. Unknown verbs are `Caution`, never `Safe`.
pub fn vm_action(action: &str) -> RiskAssessment {
    let action = action.trim().to_ascii_lowercase();
    let verdict = match action.as_str() {
        "start" => ("vm.start", "risk.vm.start", RiskLevel::Caution),
        "suspend" => ("vm.suspend", "risk.vm.suspend", RiskLevel::Caution),
        "pause" | "unpause" => ("vm.pause", "risk.vm.pause", RiskLevel::Caution),
        "stop" => ("vm.stop", "risk.vm.stop", RiskLevel::Destructive),
        // A reset discards whatever the guest has not written down.
        "reset" => ("vm.reset", "risk.vm.reset", RiskLevel::Destructive),
        // Removing a lock that belongs to a live machine would let two hypervisors write the same
        // disks, so taking ownership asks like any other destructive action.
        "take-ownership" => ("vm.ownership", "risk.vm.takeOwnership", RiskLevel::Destructive),
        _ => ("vm.unknown", "risk.vm.unknown", RiskLevel::Caution),
    };
    RiskAssessment::from_reasons(vec![RiskReason::new(verdict.0, verdict.1, verdict.2)])
}

/// Classifies one snapshot operation.
///
/// Taking one is a change; going back to it throws away everything since, and deleting one throws
/// away the state itself.
pub fn vm_snapshot(action: &str) -> RiskAssessment {
    let action = action.trim().to_ascii_lowercase();
    let verdict = match action.as_str() {
        "list" => return RiskAssessment::safe(),
        "take" => ("vm.snapshot-take", "risk.vm.snapshotTake", RiskLevel::Caution),
        "clone" => ("vm.snapshot-clone", "risk.vm.snapshotClone", RiskLevel::Caution),
        "revert" => ("vm.snapshot-revert", "risk.vm.snapshotRevert", RiskLevel::Destructive),
        "delete" => ("vm.snapshot-delete", "risk.vm.snapshotDelete", RiskLevel::Destructive),
        _ => ("vm.unknown", "risk.vm.unknown", RiskLevel::Caution),
    };
    RiskAssessment::from_reasons(vec![RiskReason::new(verdict.0, verdict.1, verdict.2)])
}

/// Classifies one guest operation.
///
/// Reading is safe; writing a file, making a directory or running a program is a change; deleting
/// something or killing a process destroys work inside the guest. Keys are destructive because they
/// answer whatever prompt is in front of the guest — including one that erases a disk.
pub fn vm_guest(action: &str) -> RiskAssessment {
    let action = action.trim().to_ascii_lowercase();
    let verdict = match action.as_str() {
        "ls" | "file-exists" | "directory-exists" | "processes" | "tools-state" => {
            return RiskAssessment::safe();
        }
        "mkdir" | "rename" | "run" | "tools-install" => ("vm.guest-change", "risk.vm.guestChange", RiskLevel::Caution),
        "type" | "send-key" => ("vm.guest-input", "risk.vm.guestInput", RiskLevel::Destructive),
        "rm" | "rmdir" | "kill" => ("vm.guest-remove", "risk.vm.guestRemove", RiskLevel::Destructive),
        _ => ("vm.unknown", "risk.vm.unknown", RiskLevel::Caution),
    };
    RiskAssessment::from_reasons(vec![RiskReason::new(verdict.0, verdict.1, verdict.2)])
}

/// Classifies one configuration change.
///
/// Resizing or creating is a change; purging an adapter or disconnecting a device takes something
/// away from a machine that may be running, so it asks first.
pub fn vm_config(action: &str) -> RiskAssessment {
    let action = action.trim().to_ascii_lowercase();
    let verdict = match action.as_str() {
        "disk-extend" | "ethernet-purge" | "serial-purge" | "sata-purge" | "nvme-purge" | "device-disconnect" => {
            ("vm.config-remove", "risk.vm.configRemove", RiskLevel::Destructive)
        }
        // A VProbes script runs inside the hypervisor and can observe everything the machine does,
        // so loading one is not an ordinary configuration change.
        "vprobes-load" => ("vm.vprobes-load", "risk.vm.vprobesLoad", RiskLevel::Destructive),
        "query" | "read" => return RiskAssessment::safe(),
        _ => ("vm.config-change", "risk.vm.configChange", RiskLevel::Caution),
    };
    RiskAssessment::from_reasons(vec![RiskReason::new(verdict.0, verdict.1, verdict.2)])
}

/// Classifies a change to this machine's virtual network (port forwards).
pub fn vm_host(_action: &str) -> RiskAssessment {
    RiskAssessment::from_reasons(vec![RiskReason::new(
        "vm.host-change",
        "risk.vm.hostChange",
        RiskLevel::Caution,
    )])
}

/// Classifies a whole-machine operation. Deleting a machine loses its files; upgrading it takes it
/// away from older Workstation versions. Both ask; a clone creates files and does not.
pub fn vm_image(action: &str) -> RiskAssessment {
    if action.trim().eq_ignore_ascii_case("clone") {
        return RiskAssessment::from_reasons(vec![RiskReason::new("vm.clone", "risk.vm.clone", RiskLevel::Caution)]);
    }
    RiskAssessment::from_reasons(vec![RiskReason::new(
        "vm.image",
        "risk.vm.imageChange",
        RiskLevel::Destructive,
    )])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopping_a_running_machine_asks_and_starting_it_does_not() {
        assert_eq!(vm_action("stop").level, RiskLevel::Destructive);
        assert_eq!(vm_action("start").level, RiskLevel::Caution);
        assert_eq!(vm_action("suspend").level, RiskLevel::Caution);
        assert_eq!(vm_action("take-ownership").level, RiskLevel::Destructive);
        assert_eq!(vm_config("disk-extend").level, RiskLevel::Destructive);
        assert_eq!(vm_config("vprobes-load").level, RiskLevel::Destructive);
        assert_eq!(vm_config("set-cpu").level, RiskLevel::Caution);
        assert_eq!(vm_guest("send-key").level, RiskLevel::Destructive);
        assert_eq!(vm_guest("type").level, RiskLevel::Destructive);
        assert_eq!(vm_image("delete").level, RiskLevel::Destructive);
        assert_eq!(
            vm_action("reboot").level,
            RiskLevel::Caution,
            "an unknown verb is looked at"
        );
    }
}
