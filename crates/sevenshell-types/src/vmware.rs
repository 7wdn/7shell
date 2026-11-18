//! VMware Workstation virtual machines: what a scan found, what one is doing now, and what a saved
//! VM session carries.
//!
//! Nothing here is a credential. The `.vmx` document is an identity the host scanned and the
//! session keeps by reference; the guest account and its port are what an SSH or RDP connection to
//! the running machine would use, and the password for one lives in the saved session that owns
//! it, never here.

use serde::{Deserialize, Serialize};

/// The VMware half of a saved session.
///
/// `vm_id` is the identity Workstation assigns the machine (`uuid.bios`, normalized). It is
/// deliberately not the `.vmx` path: the interface never holds a local path, so the host resolves
/// this token against its own inventory each time it acts. A VM may be renamed or moved and keeps
/// answering to the same id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmwareSettings {
    /// The machine's Workstation identity, as a scan reported it.
    #[serde(default)]
    pub vm_id: String,
    /// The account an SSH connection to the guest signs in as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_user: Option<String>,
    /// The guest's SSH port. Twenty-two unless somebody changed it.
    #[serde(default = "default_vm_ssh_port")]
    pub ssh_port: u16,
    /// The account a remote desktop session signs in as. Empty means the system client asks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rdp_user: Option<String>,
    /// The guest's remote-desktop port. 3389 unless somebody changed it.
    #[serde(default = "default_vm_rdp_port")]
    pub rdp_port: u16,
    /// The saved SSH session this VM's "open SSH" keeps current, once one has been made.
    ///
    /// A reference, not a copy: the server, the account and the stored credential belong to that
    /// session, and the VM workspace only rewrites its host and port to the address the guest has
    /// today. Deleting that session leaves this pointing at nothing, which the workspace repairs
    /// by making a new one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_session: Option<String>,
    /// The saved remote-desktop session this VM's "open RDP" keeps current, once one has been made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rdp_session: Option<String>,
}

pub fn default_vm_ssh_port() -> u16 {
    22
}

pub fn default_vm_rdp_port() -> u16 {
    3389
}

impl Default for VmwareSettings {
    /// The same values `serde` fills in for a document that omits them. A derived `Default` would
    /// answer zero and make an empty block serialize differently from one read back in.
    fn default() -> Self {
        Self {
            vm_id: String::new(),
            ssh_user: None,
            ssh_port: default_vm_ssh_port(),
            rdp_user: None,
            rdp_port: default_vm_rdp_port(),
            ssh_session: None,
            rdp_session: None,
        }
    }
}

impl VmwareSettings {
    /// Whether this block says anything. A session of another protocol carries an empty one, and
    /// an empty block is left out of the document the same way every other default is.
    pub fn is_empty(&self) -> bool {
        self.vm_id.trim().is_empty()
            && self.ssh_user.as_deref().unwrap_or("").trim().is_empty()
            && self.rdp_user.as_deref().unwrap_or("").trim().is_empty()
            && self.ssh_session.as_deref().unwrap_or("").trim().is_empty()
            && self.rdp_session.as_deref().unwrap_or("").trim().is_empty()
    }
}

/// One virtual machine a scan found.
///
/// The name is the one Workstation shows, which may differ from the file name; `guest_os` is the
/// `.vmx` identifier (`debian12-64`, `windows11-64`), which is a hint for the interface and never a
/// promise about what is installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmInfo {
    /// Workstation's identity for the machine, normalized (`uuid.bios` without spacing).
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_os: Option<String>,
    /// Whether `vmrun list` named this machine as running when the scan ran.
    pub running: bool,
    /// The guest's address when it is running, Tools answered and the guest has one. A scan
    /// reports it; the address is never stored with the session, because a DHCP guest changes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
}

/// What one virtual machine is doing right now.
///
/// Status is asked for when somebody looks at a VM, not stored: a power state and an address go
/// stale the moment they are written down.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmStatus {
    pub id: String,
    pub running: bool,
    /// Suspended with its memory saved: `vmrun list` does not name it, but starting it is a
    /// resume rather than a boot, and the interface says so.
    #[serde(default)]
    pub suspended: bool,
    /// The `.vmx` guest identifier (`debian12-64`, `windows9-64`). Read from the document, so it
    /// is known before the machine is powered on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_os: Option<String>,
    /// A lock file exists for the machine. While it is running that is the normal state; when it
    /// is neither running nor suspended, the lock is what `vmrun` refuses to start over — a run
    /// that did not finish, or a Workstation window this build cannot see.
    #[serde(default)]
    pub locked: bool,
    /// What VMware Tools said: `running`, `installed`, `notInstalled`, `notRunning`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools_state: Option<String>,
    /// The guest's address, when it is running, Tools answered and the guest has one yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
}

/// The power actions this build performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VmPowerAction {
    /// Boot the machine, or resume it when it is suspended, showing Workstation's window.
    Start,
    /// Ask the guest to shut down through VMware Tools.
    Stop,
    /// Suspend the machine, keeping its state.
    Suspend,
    /// Reset a running machine, which discards what the guest has not written down.
    Reset,
    /// Pause a running machine without saving its state.
    Pause,
    /// Resume a paused machine.
    Unpause,
}

impl VmPowerAction {
    pub const ALL: &'static [VmPowerAction] = &[
        VmPowerAction::Start,
        VmPowerAction::Stop,
        VmPowerAction::Suspend,
        VmPowerAction::Reset,
        VmPowerAction::Pause,
        VmPowerAction::Unpause,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Suspend => "suspend",
            Self::Reset => "reset",
            Self::Pause => "pause",
            Self::Unpause => "unpause",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|action| action.as_str() == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_block_is_left_out_of_the_document() {
        let settings = VmwareSettings::default();
        assert!(settings.is_empty());
        let json = serde_json::to_value(&settings).expect("the block serializes");
        assert_eq!(json, serde_json::json!({ "vmId": "", "sshPort": 22, "rdpPort": 3389 }));
    }

    #[test]
    fn a_vm_id_alone_makes_the_block_worth_writing() {
        let settings = VmwareSettings {
            vm_id: "564db5f23515de032cd9338ba8d831b7".to_owned(),
            ..VmwareSettings::default()
        };
        assert!(!settings.is_empty());
    }

    #[test]
    fn ports_have_the_defaults_an_ssh_and_a_desktop_session_use() {
        let settings: VmwareSettings = serde_json::from_value(serde_json::json!({ "vmId": "abc" }))
            .expect("a document written before the ports existed still reads");
        assert_eq!(settings.ssh_port, 22);
        assert_eq!(settings.rdp_port, 3389);
    }

    #[test]
    fn power_actions_round_trip_through_their_names() {
        for action in VmPowerAction::ALL {
            assert_eq!(VmPowerAction::from_id(action.as_str()), Some(*action));
        }
        assert_eq!(VmPowerAction::from_id("reboot"), None);
    }
}
