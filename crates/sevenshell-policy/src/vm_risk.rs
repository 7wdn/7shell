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
        "stop" => ("vm.stop", "risk.vm.stop", RiskLevel::Destructive),
        // Removing a lock that belongs to a live machine would let two hypervisors write the same
        // disks, so taking ownership asks like any other destructive action.
        "take-ownership" => ("vm.ownership", "risk.vm.takeOwnership", RiskLevel::Destructive),
        _ => ("vm.unknown", "risk.vm.unknown", RiskLevel::Caution),
    };
    RiskAssessment::from_reasons(vec![RiskReason::new(verdict.0, verdict.1, verdict.2)])
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
        assert_eq!(
            vm_action("reboot").level,
            RiskLevel::Caution,
            "an unknown verb is looked at"
        );
    }
}
