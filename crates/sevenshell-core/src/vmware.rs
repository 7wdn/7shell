//! Virtual machines on this machine: discovery, status, and the power actions a person approves.
//!
//! Power actions are decided here, before `vmrun` is started, so an unacknowledged stop fails the
//! same way whether or not the machine is running. The policy target is the global rule only: a
//! virtual machine is local, has no saved session id and no host layer to name, which is the same
//! bargain the container paths make.

use sevenshell_error::ShellError;
use sevenshell_policy::PolicyTarget;
use sevenshell_types::{PolicyAction, RiskAssessment, VmInfo, VmPowerAction, VmStatus};

use crate::Core;

impl Core {
    /// Every virtual machine this machine's Workstation can act on.
    pub async fn vmware_scan(&self) -> Result<Vec<VmInfo>, ShellError> {
        sevenshell_vmware::scan().await
    }

    /// What one machine is doing now: power, VMware Tools and the guest's address.
    pub async fn vmware_status(&self, vm_id: &str) -> Result<VmStatus, ShellError> {
        sevenshell_vmware::status(vm_id).await
    }

    /// Starts, stops or suspends one machine after the policy verdict for the action.
    pub async fn vmware_power(&self, vm_id: &str, action: VmPowerAction, acknowledged: bool) -> Result<(), ShellError> {
        let verdict = sevenshell_policy::vm_action(action.as_str());
        self.vm_gate(&format!("vmware.{}", action.as_str()), &verdict, acknowledged, vm_id)?;
        sevenshell_vmware::power(vm_id, action).await
    }

    /// Removes a stale lock from a machine that is not running, after the policy verdict.
    ///
    /// This is what Workstation's own window offers as "Take ownership"; it is gated like a
    /// destructive action because a lock that belongs to a live machine is not stale.
    pub async fn vmware_take_ownership(&self, vm_id: &str, acknowledged: bool) -> Result<(), ShellError> {
        let verdict = sevenshell_policy::vm_action("take-ownership");
        self.vm_gate("vmware.take-ownership", &verdict, acknowledged, vm_id)?;
        sevenshell_vmware::take_ownership(vm_id).await
    }

    fn vm_gate(
        &self,
        operation: &str,
        verdict: &RiskAssessment,
        acknowledged: bool,
        vm_id: &str,
    ) -> Result<(), ShellError> {
        let decision = self.policy_decision(operation, verdict, PolicyTarget::default(), None, Some(vm_id));
        match decision.action {
            PolicyAction::Deny => Err(ShellError::policy_denied(operation)),
            PolicyAction::Ask if !acknowledged => {
                let rule = verdict
                    .reasons
                    .first()
                    .map_or("vm.unknown", |reason| reason.rule_id.as_str());
                Err(ShellError::policy_approval_required(rule))
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sevenshell_types::{PolicyAction, PolicyMatrix, VmPowerAction};
    use sevenshell_vault::Vault;
    use tempfile::tempdir;

    use crate::Core;

    fn test_core() -> (tempfile::TempDir, Arc<Core>) {
        let directory = tempdir().expect("temp dir");
        let (core, _events) = Core::new(Arc::new(Vault::open_at(directory.path().join("vault.json"))));
        (directory, core)
    }

    #[tokio::test]
    async fn a_denied_stop_never_reaches_vmrun() {
        let (_directory, core) = test_core();
        let mut matrix = PolicyMatrix::default();
        // The global destructive rule says no, which must fail before any identity or process is
        // looked at — the id here is not a machine, and that must not be the error anybody sees.
        matrix.global.destructive = PolicyAction::Deny;
        core.set_policy(matrix, None);

        let error = core
            .vmware_power("not-a-vm", VmPowerAction::Stop, true)
            .await
            .unwrap_err();
        assert_eq!(error.code(), sevenshell_error::ErrorCode::PolicyDenied);
    }

    #[tokio::test]
    async fn a_stop_the_default_rule_raises_to_a_question_asks_before_running() {
        let (_directory, core) = test_core();
        // The default matrix asks about destructive actions; the missing machine must not be what
        // the call fails with before that question is answered.
        let error = core
            .vmware_power("not-a-vm", VmPowerAction::Stop, false)
            .await
            .unwrap_err();
        assert_eq!(
            error.code(),
            sevenshell_error::ErrorCode::PolicyApprovalRequired,
            "a destructive action must stop for an acknowledgement first"
        );

        let error = core
            .vmware_power("not-a-vm", VmPowerAction::Start, false)
            .await
            .unwrap_err();
        assert_eq!(
            error.code(),
            sevenshell_error::ErrorCode::VmwareCommandFailed,
            "starting is not gated, so the invalid identity is the failure"
        );
    }

    #[tokio::test]
    async fn taking_ownership_asks_like_a_destructive_action() {
        let (_directory, core) = test_core();
        let error = core.vmware_take_ownership("not-a-vm", false).await.unwrap_err();
        assert_eq!(
            error.code(),
            sevenshell_error::ErrorCode::PolicyApprovalRequired,
            "a lock is only removed after somebody says so"
        );
    }
}
