//! Virtual machines on this machine: discovery, status, and the power actions a person approves.
//!
//! Power actions are decided here, before `vmrun` is started, so an unacknowledged stop fails the
//! same way whether or not the machine is running. The policy target is the global rule only: a
//! virtual machine is local, has no saved session id and no host layer to name, which is the same
//! bargain the container paths make.

use sevenshell_error::ShellError;
use sevenshell_policy::PolicyTarget;
use std::path::Path;

use sevenshell_types::{HostLocationKind, PolicyAction, Protocol, RiskAssessment, VmInfo, VmPowerAction, VmStatus};
use sevenshell_vault::SessionSecret;
use sevenshell_vmware::{
    ConfigRead, ConfigWrite, GuestAuth, GuestOp, HostRead, HostWrite, ImageAction, SnapshotAction,
};

use crate::Core;

impl Core {
    /// The machine a saved VM session names, for a caller that has the session id.
    pub fn vmware_session_vm_id(&self, session_id: &str) -> Result<String, ShellError> {
        let session = self.session(session_id)?;
        if session.protocol != Protocol::Vmware {
            return Err(ShellError::app_unsupported(
                "this session is not a virtual machine, so it has no guest",
            ));
        }
        let vm_id = session.vmware.vm_id.trim();
        if vm_id.is_empty() {
            return Err(ShellError::app_unsupported(
                "this session names no virtual machine, so there is nothing to act on",
            ));
        }
        Ok(vm_id.to_owned())
    }

    /// The guest account and password a VM session carries.
    ///
    /// The account is the session's own guest account, and the password is the one secret the
    /// vault keeps beside a session — the same slot the editor's password field writes. A guest
    /// operation without one is refused here rather than sent to `vmrun` to fail without saying
    /// what is missing.
    pub fn vmware_guest_auth(&self, session_id: &str) -> Result<GuestAuth, ShellError> {
        let session = self.session(session_id)?;
        if session.protocol != Protocol::Vmware {
            return Err(ShellError::app_unsupported(
                "this session is not a virtual machine, so it has no guest",
            ));
        }
        let user = session.vmware.ssh_user.as_deref().unwrap_or("").trim().to_owned();
        if user.is_empty() {
            return Err(ShellError::vmware_guest_credentials(
                "the session has no guest account; set the SSH user on it first",
            ));
        }
        let Some(secret) = self.vault.secret_for_session(session_id)? else {
            return Err(ShellError::vmware_guest_credentials(
                "the session has no guest password stored",
            ));
        };
        let password = match &secret {
            SessionSecret::Password(password) => password.clone(),
            _ => {
                return Err(ShellError::vmware_guest_credentials(
                    "the session's stored secret is not a guest password",
                ));
            }
        };
        if password.is_empty() {
            return Err(ShellError::vmware_guest_credentials(
                "the session's stored guest password is empty",
            ));
        }
        Ok(GuestAuth { user, password })
    }

    /// One snapshot action on the machine a saved session names.
    pub async fn vmware_snapshot(&self, session_id: &str, action: &SnapshotAction) -> Result<String, ShellError> {
        let vm_id = self.vmware_session_vm_id(session_id)?;
        sevenshell_vmware::snapshot(&vm_id, action).await
    }

    /// One guest operation on the machine a saved session names, with its stored guest account.
    pub async fn vmware_guest(&self, session_id: &str, op: &GuestOp) -> Result<String, ShellError> {
        let vm_id = self.vmware_session_vm_id(session_id)?;
        let auth = self.vmware_guest_auth(session_id)?;
        sevenshell_vmware::guest(&vm_id, op, &auth).await
    }

    /// One configuration module of the machine a saved session names.
    pub async fn vmware_config_read(&self, session_id: &str, action: &ConfigRead) -> Result<String, ShellError> {
        let vm_id = self.vmware_session_vm_id(session_id)?;
        sevenshell_vmware::config_read(&vm_id, action).await
    }

    /// One configuration change on the machine a saved session names.
    pub async fn vmware_config_write(&self, session_id: &str, action: &ConfigWrite) -> Result<String, ShellError> {
        let vm_id = self.vmware_session_vm_id(session_id)?;
        sevenshell_vmware::config_write(&vm_id, action).await
    }

    /// Sends one key sequence to the console of the machine a saved session names.
    pub async fn vmware_send_key(&self, session_id: &str, sequence: &str) -> Result<String, ShellError> {
        let vm_id = self.vmware_session_vm_id(session_id)?;
        sevenshell_vmware::send_key(&vm_id, sequence).await
    }

    /// Loads the VProbes script a file token names into the machine's hypervisor.
    pub async fn vmware_vprobes_load(&self, session_id: &str, token: &str) -> Result<String, ShellError> {
        let script = self.host_location_path(token, HostLocationKind::File)?;
        let vm_id = self.vmware_session_vm_id(session_id)?;
        sevenshell_vmware::config_write(&vm_id, &ConfigWrite::VprobesLoad(host_path(&script))).await
    }

    /// The host's own virtual networks, or one network's port forwards.
    pub async fn vmware_host_read(&self, session_id: &str, action: &HostRead) -> Result<String, ShellError> {
        let vm_id = self.vmware_session_vm_id(session_id)?;
        sevenshell_vmware::host_read(&vm_id, action).await
    }

    /// One port-forward change on the host.
    pub async fn vmware_host_write(&self, session_id: &str, action: &HostWrite) -> Result<String, ShellError> {
        let vm_id = self.vmware_session_vm_id(session_id)?;
        sevenshell_vmware::host_write(&vm_id, action).await
    }

    /// Deletes or upgrades one machine.
    pub async fn vmware_image(&self, session_id: &str, action: ImageAction) -> Result<String, ShellError> {
        let vm_id = self.vmware_session_vm_id(session_id)?;
        sevenshell_vmware::image(&vm_id, action).await
    }

    /// The guest's environment, which only `vmcli` prints.
    pub async fn vmware_guest_env(&self, session_id: &str) -> Result<String, ShellError> {
        let vm_id = self.vmware_session_vm_id(session_id)?;
        let auth = self.vmware_guest_auth(session_id)?;
        sevenshell_vmware::guest_env(&vm_id, &auth).await
    }

    /// Copies one host file into the guest, named by a file token.
    pub async fn vmware_guest_copy_to(
        &self,
        session_id: &str,
        token: &str,
        guest_path: &str,
    ) -> Result<String, ShellError> {
        let path = self.host_location_path(token, HostLocationKind::File)?;
        let vm_id = self.vmware_session_vm_id(session_id)?;
        let auth = self.vmware_guest_auth(session_id)?;
        sevenshell_vmware::copy_to_guest(&vm_id, &host_path(&path), guest_path, &auth).await
    }

    /// Copies one guest file into the folder a folder token names. The host file keeps the guest
    /// file's own name.
    pub async fn vmware_guest_copy_from(
        &self,
        session_id: &str,
        token: &str,
        guest_path: &str,
    ) -> Result<String, ShellError> {
        let folder = self.host_location_path(token, HostLocationKind::Folder)?;
        let name = Path::new(guest_path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                ShellError::vmware_command_failed("copy-from", "the guest path has no file name to save under")
            })?;
        let destination = folder.join(&name);
        let vm_id = self.vmware_session_vm_id(session_id)?;
        let auth = self.vmware_guest_auth(session_id)?;
        sevenshell_vmware::copy_from_guest(&vm_id, guest_path, &host_path(&destination), &auth).await
    }

    /// Clones one machine into a subfolder of the folder a token names.
    pub async fn vmware_clone(
        &self,
        session_id: &str,
        token: &str,
        name: &str,
        clone_type: &str,
        snapshot: Option<&str>,
    ) -> Result<String, ShellError> {
        let name = safe_component(name, "clone")?;
        let clone_type = match clone_type.trim().to_ascii_lowercase().as_str() {
            "full" => "full",
            "linked" => "linked",
            _ => {
                return Err(ShellError::vmware_command_failed(
                    "clone",
                    "cloneType must be `full` or `linked`",
                ));
            }
        };
        let folder = self.host_location_path(token, HostLocationKind::Folder)?;
        let destination = folder.join(&name).join(format!("{name}.vmx"));
        // `vmrun clone` writes the document at the path it is given but not the folder above it.
        std::fs::create_dir_all(destination.parent().expect("the clone has a folder"))
            .map_err(|error| ShellError::store_io(format!("the clone folder could not be created: {error}")))?;
        let vm_id = self.vmware_session_vm_id(session_id)?;
        sevenshell_vmware::clone(&vm_id, &host_path(&destination), clone_type, snapshot).await
    }

    /// Captures the machine's screen into the folder a token names, and answers the file name.
    pub async fn vmware_screenshot(&self, session_id: &str, token: &str) -> Result<String, ShellError> {
        let folder = self.host_location_path(token, HostLocationKind::Folder)?;
        let stem = safe_component(&self.session(session_id)?.name, "screenshot")?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        let file_name = format!("{stem}-{stamp}.png");
        let destination = folder.join(&file_name);
        let vm_id = self.vmware_session_vm_id(session_id)?;
        sevenshell_vmware::screenshot(&vm_id, &host_path(&destination)).await?;
        Ok(file_name)
    }

    /// Points one shared folder at the folder a token names.
    pub async fn vmware_hgfs_set_host_path(
        &self,
        session_id: &str,
        index: u32,
        token: &str,
    ) -> Result<String, ShellError> {
        let folder = self.host_location_path(token, HostLocationKind::Folder)?;
        let vm_id = self.vmware_session_vm_id(session_id)?;
        sevenshell_vmware::hgfs_set_host_path(&vm_id, index, &host_path(&folder)).await
    }

    /// Adds a shared folder for the folder a token names.
    pub async fn vmware_shared_add(&self, session_id: &str, name: &str, token: &str) -> Result<String, ShellError> {
        let name = safe_component(name, "shared-add")?;
        let folder = self.host_location_path(token, HostLocationKind::Folder)?;
        let vm_id = self.vmware_session_vm_id(session_id)?;
        sevenshell_vmware::shared_add(&vm_id, &name, &host_path(&folder)).await
    }

    /// Sets whether one shared folder is writable, still pointing at the folder a token names.
    pub async fn vmware_shared_set_state(
        &self,
        session_id: &str,
        name: &str,
        token: &str,
        writable: bool,
    ) -> Result<String, ShellError> {
        let name = safe_component(name, "shared-set")?;
        let folder = self.host_location_path(token, HostLocationKind::Folder)?;
        let vm_id = self.vmware_session_vm_id(session_id)?;
        sevenshell_vmware::shared_set_state(&vm_id, &name, &host_path(&folder), writable).await
    }

    /// Clones one snapshot into a subfolder of the folder a token names.
    pub async fn vmware_snapshot_clone(
        &self,
        session_id: &str,
        token: &str,
        uid: &str,
        name: &str,
        linked: bool,
    ) -> Result<String, ShellError> {
        let uid = uid.trim();
        if uid.is_empty() {
            return Err(ShellError::vmware_command_failed(
                "snapshot-clone",
                "a snapshot uid is required",
            ));
        }
        let name = safe_component(name, "snapshot-clone")?;
        let folder = self.host_location_path(token, HostLocationKind::Folder)?;
        let destination = folder.join(&name).join(format!("{name}.vmx"));
        // `vmcli Snapshot Clone` answers "Could not find the file" when the folder above the
        // destination is missing; it creates the document, not the directory.
        std::fs::create_dir_all(destination.parent().expect("the clone has a folder"))
            .map_err(|error| ShellError::store_io(format!("the clone folder could not be created: {error}")))?;
        let vm_id = self.vmware_session_vm_id(session_id)?;
        sevenshell_vmware::snapshot_clone(&vm_id, uid, &host_path(&destination), &name, linked).await
    }

    /// Creates a `.vmtx` template in the folder a token names, under `<name>.vmtx`.
    pub async fn vmware_template_create(
        &self,
        session_id: &str,
        token: &str,
        name: &str,
    ) -> Result<String, ShellError> {
        let name = safe_component(name, "template-create")?;
        let folder = self.host_location_path(token, HostLocationKind::Folder)?;
        let template = folder.join(format!("{name}.vmtx"));
        let vm_id = self.vmware_session_vm_id(session_id)?;
        sevenshell_vmware::template_create(&vm_id, &host_path(&template), &name).await
    }

    /// Deploys the `.vmtx` template a file token names.
    pub async fn vmware_template_deploy(&self, session_id: &str, token: &str) -> Result<String, ShellError> {
        let template = self.host_location_path(token, HostLocationKind::File)?;
        self.vmware_session_vm_id(session_id)?;
        sevenshell_vmware::template_deploy(&host_path(&template)).await
    }

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

/// A host path as the command line takes it.
fn host_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// A name that is one component: no separators, no control characters, no "..". A name is what the
/// agent typed, and a name that can walk out of the folder it was given is a path.
fn safe_component(name: &str, operation: &str) -> Result<String, ShellError> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed
            .chars()
            .any(|character| character == '/' || character == '\\' || character.is_control())
    {
        return Err(ShellError::vmware_command_failed(
            operation,
            "the name must be a single file or folder name",
        ));
    }
    Ok(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sevenshell_types::{PolicyAction, PolicyMatrix, VmPowerAction};
    use sevenshell_vault::Vault;
    use tempfile::tempdir;

    use super::safe_component;
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

    #[test]
    fn a_name_that_can_walk_out_of_its_folder_is_refused() {
        assert!(safe_component("..", "clone").is_err());
        assert!(safe_component("a/b", "clone").is_err());
        assert!(safe_component(r"a\b", "clone").is_err());
        assert_eq!(safe_component("ubuntu-2", "clone").expect("a name"), "ubuntu-2");
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
