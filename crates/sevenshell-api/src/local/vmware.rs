//! Virtual machines, from an agent's point of view.
//!
//! A VM session names one machine; every tool here takes that session id, so the same access
//! policy, visibility rules and matrix layers that protect a host session also decide what an agent
//! may do to a machine on this desktop. The identity the tools carry is the session, not a `.vmx`
//! path: no tool in this file can name a local file.
//!
//! Guest operations sign in with the account and password the session already stores — the SSH
//! user and the session's password field — so no credential travels through tool arguments, a
//! model's context or an audit line. A session without them is refused before `vmrun` runs.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use sevenshell_types::VmPowerAction;
use sevenshell_vmware::{GuestOp, SnapshotAction};

use crate::context::ToolContext;
use crate::registry::Tool;
use crate::tool::{ToolAccess, ToolAudit, ToolDescriptor, ToolError, ToolInputSchema, ToolResult, parse_arguments};

use super::cache::saved_session;
use super::gate;
use super::session_id;

pub const VMS_LIST: &str = "sshell_vms_list";
pub const VM_STATUS: &str = "sshell_vm_status";
pub const VM_POWER: &str = "sshell_vm_power";
pub const VM_SNAPSHOTS: &str = "sshell_vm_snapshots";
pub const VM_SNAPSHOT_CHANGE: &str = "sshell_vm_snapshot_change";
pub const VM_GUEST_READ: &str = "sshell_vm_guest_read";
pub const VM_GUEST_WRITE: &str = "sshell_vm_guest_write";
pub const VM_SEND_KEY: &str = "sshell_vm_send_key";

/// Guest operations and snapshots wait on a hypervisor, not on a network round trip.
const VM_OPERATION_TIMEOUT: Duration = Duration::from_secs(120);
/// How many program arguments one run may carry. A command line is not a file.
const MAX_PROGRAM_ARGS: usize = 64;

// -- Inputs -------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmsListInput {}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmStatusInput {
    pub session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmPowerInput {
    pub session_id: String,
    pub action: VmPowerAction,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmSnapshotsInput {
    pub session_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SnapshotChange {
    Take,
    Revert,
    Delete,
    Clone,
}

impl SnapshotChange {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Take => "take",
            Self::Revert => "revert",
            Self::Delete => "delete",
            Self::Clone => "clone",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmSnapshotChangeInput {
    pub session_id: String,
    pub action: SnapshotChange,
    pub name: String,
    #[serde(default)]
    pub delete_children: bool,
    #[serde(default)]
    pub uid: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub linked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GuestReadAction {
    Ls,
    FileExists,
    DirectoryExists,
    Processes,
    ToolsState,
    Env,
}

impl GuestReadAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ls => "ls",
            Self::FileExists => "file-exists",
            Self::DirectoryExists => "directory-exists",
            Self::Processes => "processes",
            Self::ToolsState => "tools-state",
            Self::Env => "env",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmGuestReadInput {
    pub session_id: String,
    pub action: GuestReadAction,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GuestWriteAction {
    Mkdir,
    Rmdir,
    Rm,
    Rename,
    Run,
    Script,
    Type,
    TempFile,
    CopyTo,
    CopyFrom,
    Kill,
    ToolsInstall,
}

impl GuestWriteAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mkdir => "mkdir",
            Self::Rmdir => "rmdir",
            Self::Rm => "rm",
            Self::Rename => "rename",
            Self::Run => "run",
            Self::Script => "script",
            Self::Type => "type",
            Self::TempFile => "temp-file",
            Self::CopyTo => "copy-to",
            Self::CopyFrom => "copy-from",
            Self::Kill => "kill",
            Self::ToolsInstall => "tools-install",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmSendKeyInput {
    pub session_id: String,
    pub sequence: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmGuestWriteInput {
    pub session_id: String,
    pub action: GuestWriteAction,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub new_path: Option<String>,
    #[serde(default)]
    pub program: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub no_wait: bool,
    #[serde(default)]
    pub interactive: bool,
    #[serde(default)]
    pub interpreter: Option<String>,
    #[serde(default)]
    pub script: Option<String>,
    #[serde(default)]
    pub keystrokes: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub guest_path: Option<String>,
    #[serde(default)]
    pub pid: Option<u64>,
}

// -- Descriptors --------------------------------------------------------

fn string(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn string_enum(description: &str, values: &[&str]) -> Value {
    json!({ "type": "string", "enum": values, "description": description })
}

fn session_id_property() -> Value {
    string("The id of a saved virtual-machine session, from sshell_sessions_list.")
}

fn guest_path_property() -> Value {
    string("A path inside the guest, as the guest spells it (`/home/zuser`, `C:\\Temp`).")
}

pub fn vms_list_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: VMS_LIST.to_owned(),
        title: "List virtual machines".to_owned(),
        description: "Lists the virtual machines VMware Workstation on this machine knows about: \
                      id, name, guest system, whether it is running, and the guest's address when \
                      one was reported. Works whether or not the machine is powered on."
            .to_owned(),
        input_schema: ToolInputSchema::object(json!({}), &[]),
        access: ToolAccess::Read,
    }
}

pub fn vm_status_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: VM_STATUS.to_owned(),
        title: "Read a virtual machine".to_owned(),
        description: "Reads one virtual machine's power state (running, suspended or stopped), \
                      whether VMware Tools is answering, the guest's current address and the guest \
                      system from the .vmx document."
            .to_owned(),
        input_schema: ToolInputSchema::object(json!({ "sessionId": session_id_property() }), &["sessionId"]),
        access: ToolAccess::Read,
    }
}

pub fn vm_power_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: VM_POWER.to_owned(),
        title: "Act on a virtual machine's power".to_owned(),
        description: "Starts (or resumes) a virtual machine, stops it through VMware Tools, \
                      suspends it, resets it, or pauses/unpauses it. Stop and reset are graded \
                      destructive and stop for a person's approval on the desktop."
            .to_owned(),
        input_schema: ToolInputSchema::object(
            json!({
                "sessionId": session_id_property(),
                "action": string_enum(
                    "What to do with it.",
                    &["start", "stop", "suspend", "reset", "pause", "unpause"]
                )
            }),
            &["sessionId", "action"],
        ),
        access: ToolAccess::Write,
    }
}

pub fn vm_snapshots_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: VM_SNAPSHOTS.to_owned(),
        title: "List snapshots".to_owned(),
        description: "Lists the snapshots of one virtual machine as VMware Workstation prints them.".to_owned(),
        input_schema: ToolInputSchema::object(json!({ "sessionId": session_id_property() }), &["sessionId"]),
        access: ToolAccess::Read,
    }
}

pub fn vm_snapshot_change_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: VM_SNAPSHOT_CHANGE.to_owned(),
        title: "Change a snapshot".to_owned(),
        description: "Takes a snapshot, goes back to one, deletes one, or clones one into a new \
                      machine in a host folder a token names. Going back and deleting are graded \
                      destructive: both throw away state, and a revert stops for a person's \
                      approval on the desktop."
            .to_owned(),
        input_schema: ToolInputSchema::object(
            json!({
                "sessionId": session_id_property(),
                "action": string_enum("What to do with the snapshot.", &["take", "revert", "delete", "clone"]),
                "name": string("The snapshot's name."),
                "deleteChildren": {
                    "type": "boolean",
                    "description": "For delete: remove the snapshots taken under this one too."
                },
                "uid": string("For clone: the snapshot uid that sshell_vm_snapshots printed. The machine must be powered off."),
                "token": string("For clone: a folder token from sshell_host_locations_list; the clone lands in a subfolder named after `name`."),
                "linked": { "type": "boolean", "description": "For clone: create a linked clone." }
            }),
            &["sessionId", "action", "name"],
        ),
        access: ToolAccess::Write,
    }
}

pub fn vm_guest_read_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: VM_GUEST_READ.to_owned(),
        title: "Read inside a guest".to_owned(),
        description: "Lists a directory, checks whether a file or directory exists, lists processes \
                      or reads the VMware Tools state inside a running guest. Signs in with the \
                      account and password the VM session stores; without them the call is refused."
            .to_owned(),
        input_schema: ToolInputSchema::object(
            json!({
                "sessionId": session_id_property(),
                "action": string_enum(
                    "What to read. `path` is required for ls, fileExists and directoryExists.",
                    &["ls", "fileExists", "directoryExists", "processes", "toolsState", "env"]
                ),
                "path": guest_path_property()
            }),
            &["sessionId", "action"],
        ),
        access: ToolAccess::Read,
    }
}

pub fn vm_guest_write_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: VM_GUEST_WRITE.to_owned(),
        title: "Change something inside a guest".to_owned(),
        description: "Makes or removes directories, removes or renames files, runs a program, \
                      kills a process, or installs VMware Tools inside a running guest. Removing \
                      files, directories and processes is graded destructive and stops for a \
                      person's approval on the desktop. Signs in with the account and password the \
                      VM session stores."
            .to_owned(),
        input_schema: ToolInputSchema::object(
            json!({
                "sessionId": session_id_property(),
                "action": string_enum(
                    "What to do. `path` is the target for mkdir/rmdir/rm/rename; `newPath` is the \
                     new name for rename; `program` and `args` are for run; `pid` is for kill.",
                    &["mkdir", "rmdir", "rm", "rename", "run", "script", "type", "tempFile", "copyTo", "copyFrom", "kill", "toolsInstall"]
                ),
                "path": guest_path_property(),
                "newPath": string("For rename: the new path inside the guest."),
                "program": string("For run: the complete path of the program inside the guest."),
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "For run: the program's arguments, one entry each."
                },
                "noWait": {
                    "type": "boolean",
                    "description": "For run: return as soon as the program starts."
                },
                "interactive": {
                    "type": "boolean",
                    "description": "For run: run the program in the guest's interactive session."
                },
                "token": string("For copyTo and copyFrom: a host location token from sshell_host_locations_list. copyTo needs a file, copyFrom a folder."),
                "guestPath": string("For copyTo and copyFrom: the file's path inside the guest."),
                "pid": { "type": "integer", "minimum": 1, "description": "For kill: the process id." }
            }),
            &["sessionId", "action"],
        ),
        access: ToolAccess::Write,
    }
}

pub fn vm_send_key_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: VM_SEND_KEY.to_owned(),
        title: "Send keys to a virtual machine".to_owned(),
        description: "Sends one key sequence to the machine's console without signing in to the \
                      guest, so it works while the guest has no account or no VMware Tools. The \
                      syntax is vmcli's (`esc`, `ctrl-alt-delete`, `a b c`). Keys can answer any \
                      prompt the guest shows, including a destructive one, so this is graded \
                      destructive and stops for a person's approval on the desktop."
            .to_owned(),
        input_schema: ToolInputSchema::object(
            json!({
                "sessionId": session_id_property(),
                "sequence": string("The key sequence, as vmcli spells it.")
            }),
            &["sessionId", "sequence"],
        ),
        access: ToolAccess::Write,
    }
}

// -- Argument helpers ---------------------------------------------------

fn guest_path(value: Option<&str>, name: &str) -> Result<String, ToolError> {
    let trimmed = value.unwrap_or("").trim();
    if trimmed.is_empty() {
        return Err(ToolError::invalid_arguments(format!(
            "{name} is required for this action"
        )));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(ToolError::invalid_arguments(format!(
            "{name} contains a control character"
        )));
    }
    Ok(trimmed.to_owned())
}

fn vm_id(context: &ToolContext, session_id: &str) -> Result<String, ToolError> {
    context.core().vmware_session_vm_id(session_id).map_err(ToolError::from)
}

pub(super) fn output_result(session_id: &str, subject: Option<String>, output: String) -> ToolResult {
    let (text, truncated) = gate::clip(&output, gate::TEXT_LIMIT);
    let mut audit = ToolAudit::for_session(session_id);
    if let Some(subject) = subject {
        audit = audit.with_subject(subject);
    }
    ToolResult::new(json!({
        "sessionId": session_id,
        "output": text,
        "truncated": truncated,
    }))
    .with_audit(audit)
}

// -- The tools ----------------------------------------------------------

pub struct VmsListTool;

#[async_trait]
impl Tool for VmsListTool {
    fn descriptor(&self) -> ToolDescriptor {
        vms_list_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let _input: VmsListInput = parse_arguments(arguments)?;
        let machines = gate::bounded(context, gate::READ_TIMEOUT, async {
            context.core().vmware_scan().await.map_err(ToolError::from)
        })
        .await?;
        let (rows, total, truncated) = gate::rows(&machines);
        Ok(ToolResult::new(json!({
            "vms": rows,
            "total": total,
            "truncated": truncated,
        }))
        .with_audit(ToolAudit::default()))
    }
}

pub struct VmStatusTool;

#[async_trait]
impl Tool for VmStatusTool {
    fn descriptor(&self) -> ToolDescriptor {
        vm_status_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: VmStatusInput = parse_arguments(arguments)?;
        let session_id = session_id(&input.session_id)?;
        context.guard_session(session_id, ToolAccess::Read)?;
        let vm_id = vm_id(context, session_id)?;
        let status = gate::bounded(context, gate::READ_TIMEOUT, async {
            context.core().vmware_status(&vm_id).await.map_err(ToolError::from)
        })
        .await?;
        Ok(ToolResult::new(json!({
            "sessionId": session_id,
            "status": status,
        }))
        .with_audit(ToolAudit::for_session(session_id).with_subject(vm_id)))
    }
}

pub struct VmPowerTool;

#[async_trait]
impl Tool for VmPowerTool {
    fn descriptor(&self) -> ToolDescriptor {
        vm_power_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: VmPowerInput = parse_arguments(arguments)?;
        let session_id = session_id(&input.session_id)?;
        context.guard_session(session_id, ToolAccess::Write)?;
        let action = input.action;
        let vm_id = vm_id(context, session_id)?;
        let subject = gate::subject(&format!("{vm_id} {}", action.as_str()));
        let verdict = sevenshell_policy::vm_action(action.as_str());
        let name = saved_session(context.core(), session_id).map(|session| session.name);
        let approval =
            gate::approve_risk(context, VM_POWER, &verdict, Some((session_id, name)), None, &subject).await?;
        let acknowledged = approval.is_some();

        gate::bounded(context, VM_OPERATION_TIMEOUT, async {
            context
                .core()
                .vmware_power(&vm_id, action, acknowledged)
                .await
                .map_err(ToolError::from)
        })
        .await?;

        let mut audit = ToolAudit::for_session(session_id)
            .with_subject(subject)
            .with_risk(verdict);
        if let Some(record) = approval {
            audit = audit.with_approval(record);
        }
        Ok(ToolResult::new(json!({
            "sessionId": session_id,
            "action": action.as_str(),
            "acknowledged": acknowledged,
        }))
        .with_audit(audit))
    }
}

pub struct VmSnapshotsTool;

#[async_trait]
impl Tool for VmSnapshotsTool {
    fn descriptor(&self) -> ToolDescriptor {
        vm_snapshots_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: VmSnapshotsInput = parse_arguments(arguments)?;
        let session_id = session_id(&input.session_id)?;
        context.guard_session(session_id, ToolAccess::Read)?;
        let vm_id = vm_id(context, session_id)?;
        let output = gate::bounded(context, VM_OPERATION_TIMEOUT, async {
            context
                .core()
                .vmware_snapshot(session_id, &SnapshotAction::List)
                .await
                .map_err(ToolError::from)
        })
        .await?;
        Ok(output_result(session_id, Some(vm_id), output))
    }
}

pub struct VmSnapshotChangeTool;

#[async_trait]
impl Tool for VmSnapshotChangeTool {
    fn descriptor(&self) -> ToolDescriptor {
        vm_snapshot_change_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: VmSnapshotChangeInput = parse_arguments(arguments)?;
        let session_id = session_id(&input.session_id)?;
        context.guard_session(session_id, ToolAccess::Write)?;
        let name = input.name.trim();
        if name.is_empty() {
            return Err(ToolError::invalid_arguments("name is empty"));
        }
        if name.chars().any(char::is_control) {
            return Err(ToolError::invalid_arguments("name contains a control character"));
        }
        let action = input.action;
        if action == SnapshotChange::Clone {
            let token = input.token.as_deref().unwrap_or("").trim();
            if token.is_empty() {
                return Err(ToolError::invalid_arguments("token is required for clone"));
            }
            let uid = input.uid.as_deref().unwrap_or("").trim();
            if uid.is_empty() {
                return Err(ToolError::invalid_arguments("uid is required for clone"));
            }
            let vm_id = vm_id(context, session_id)?;
            let subject = gate::subject(&format!("{vm_id} clone {name}"));
            let verdict = sevenshell_policy::vm_snapshot("clone");
            let session_name = saved_session(context.core(), session_id).map(|session| session.name);
            let approval = gate::approve_risk(
                context,
                VM_SNAPSHOT_CHANGE,
                &verdict,
                Some((session_id, session_name)),
                None,
                &subject,
            )
            .await?;
            let output = gate::bounded(context, VM_OPERATION_TIMEOUT, async {
                context
                    .core()
                    .vmware_snapshot_clone(session_id, token, uid, name, input.linked)
                    .await
                    .map_err(ToolError::from)
            })
            .await?;
            let mut audit = ToolAudit::for_session(session_id)
                .with_subject(subject)
                .with_risk(verdict);
            if let Some(record) = approval {
                audit = audit.with_approval(record);
            }
            let (text, truncated) = gate::clip(&output, gate::TEXT_LIMIT);
            return Ok(ToolResult::new(json!({
                "sessionId": session_id,
                "output": text,
                "truncated": truncated,
            }))
            .with_audit(audit));
        }
        let op = match action {
            SnapshotChange::Clone => unreachable!("handled above"),
            SnapshotChange::Take => SnapshotAction::Take(name.to_owned()),
            SnapshotChange::Revert => SnapshotAction::Revert(name.to_owned()),
            SnapshotChange::Delete => SnapshotAction::Delete {
                name: name.to_owned(),
                delete_children: input.delete_children,
            },
        };
        let vm_id = vm_id(context, session_id)?;
        let subject = gate::subject(&format!("{vm_id} {} {name}", action.as_str()));
        let verdict = sevenshell_policy::vm_snapshot(action.as_str());
        let session_name = saved_session(context.core(), session_id).map(|session| session.name);
        let approval = gate::approve_risk(
            context,
            VM_SNAPSHOT_CHANGE,
            &verdict,
            Some((session_id, session_name)),
            None,
            &subject,
        )
        .await?;

        let output = gate::bounded(context, VM_OPERATION_TIMEOUT, async {
            context
                .core()
                .vmware_snapshot(session_id, &op)
                .await
                .map_err(ToolError::from)
        })
        .await?;

        let mut audit = ToolAudit::for_session(session_id)
            .with_subject(subject)
            .with_risk(verdict);
        if let Some(record) = approval {
            audit = audit.with_approval(record);
        }
        let (text, truncated) = gate::clip(&output, gate::TEXT_LIMIT);
        Ok(ToolResult::new(json!({
            "sessionId": session_id,
            "output": text,
            "truncated": truncated,
        }))
        .with_audit(audit))
    }
}

pub struct VmGuestReadTool;

#[async_trait]
impl Tool for VmGuestReadTool {
    fn descriptor(&self) -> ToolDescriptor {
        vm_guest_read_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: VmGuestReadInput = parse_arguments(arguments)?;
        let session_id = session_id(&input.session_id)?;
        context.guard_session(session_id, ToolAccess::Read)?;
        let action = input.action;
        let op = match action {
            GuestReadAction::Ls => Some(GuestOp::ListDirectory(guest_path(input.path.as_deref(), "path")?)),
            GuestReadAction::FileExists => Some(GuestOp::FileExists(guest_path(input.path.as_deref(), "path")?)),
            GuestReadAction::DirectoryExists => {
                Some(GuestOp::DirectoryExists(guest_path(input.path.as_deref(), "path")?))
            }
            GuestReadAction::Processes => Some(GuestOp::Processes),
            GuestReadAction::ToolsState => Some(GuestOp::ToolsState),
            GuestReadAction::Env => None,
        };
        let vm_id = vm_id(context, session_id)?;
        let subject = gate::subject(&format!(
            "{vm_id} {} {}",
            action.as_str(),
            input.path.unwrap_or_default()
        ));
        let output = gate::bounded(context, VM_OPERATION_TIMEOUT, async {
            let result = match &op {
                Some(op) => context.core().vmware_guest(session_id, op).await,
                None => context.core().vmware_guest_env(session_id).await,
            };
            result.map_err(ToolError::from)
        })
        .await?;
        Ok(output_result(session_id, Some(subject), output))
    }
}

pub struct VmGuestWriteTool;

#[async_trait]
impl Tool for VmGuestWriteTool {
    fn descriptor(&self) -> ToolDescriptor {
        vm_guest_write_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: VmGuestWriteInput = parse_arguments(arguments)?;
        let session_id = session_id(&input.session_id)?;
        context.guard_session(session_id, ToolAccess::Write)?;
        let action = input.action;
        if matches!(action, GuestWriteAction::CopyTo | GuestWriteAction::CopyFrom) {
            let token = input.token.as_deref().unwrap_or("").trim();
            if token.is_empty() {
                return Err(ToolError::invalid_arguments(
                    "token is required for copyTo and copyFrom",
                ));
            }
            let guest_path_value = guest_path(input.guest_path.as_deref(), "guestPath")?;
            let vm_id = vm_id(context, session_id)?;
            let subject = gate::subject(&format!("{vm_id} {} {guest_path_value}", action.as_str()));
            let verdict = sevenshell_policy::vm_guest(action.as_str());
            let session_name = saved_session(context.core(), session_id).map(|session| session.name);
            let approval = gate::approve_risk(
                context,
                VM_GUEST_WRITE,
                &verdict,
                Some((session_id, session_name)),
                None,
                &subject,
            )
            .await?;
            let output = gate::bounded(context, VM_OPERATION_TIMEOUT, async {
                let result = match action {
                    GuestWriteAction::CopyTo => {
                        context
                            .core()
                            .vmware_guest_copy_to(session_id, token, &guest_path_value)
                            .await
                    }
                    _ => {
                        context
                            .core()
                            .vmware_guest_copy_from(session_id, token, &guest_path_value)
                            .await
                    }
                };
                result.map_err(ToolError::from)
            })
            .await?;
            let mut audit = ToolAudit::for_session(session_id)
                .with_subject(subject)
                .with_risk(verdict);
            if let Some(record) = approval {
                audit = audit.with_approval(record);
            }
            let (text, truncated) = gate::clip(&output, gate::TEXT_LIMIT);
            return Ok(ToolResult::new(json!({
                "sessionId": session_id,
                "output": text,
                "truncated": truncated,
            }))
            .with_audit(audit));
        }
        let op = match action {
            GuestWriteAction::Mkdir => GuestOp::MakeDirectory(guest_path(input.path.as_deref(), "path")?),
            GuestWriteAction::Rmdir => GuestOp::RemoveDirectory(guest_path(input.path.as_deref(), "path")?),
            GuestWriteAction::Rm => GuestOp::RemoveFile(guest_path(input.path.as_deref(), "path")?),
            GuestWriteAction::Rename => GuestOp::Rename {
                from: guest_path(input.path.as_deref(), "path")?,
                to: guest_path(input.new_path.as_deref(), "newPath")?,
            },
            GuestWriteAction::Run => {
                let program = input.program.as_deref().unwrap_or("").trim();
                if program.is_empty() {
                    return Err(ToolError::invalid_arguments("program is required for run"));
                }
                if input.args.len() > MAX_PROGRAM_ARGS {
                    return Err(ToolError::invalid_arguments(format!(
                        "args carries {} entries; the limit is {MAX_PROGRAM_ARGS}",
                        input.args.len()
                    )));
                }
                GuestOp::Run {
                    program: program.to_owned(),
                    args: input.args,
                    no_wait: input.no_wait,
                    interactive: input.interactive,
                }
            }
            GuestWriteAction::Script => {
                let interpreter = input.interpreter.as_deref().unwrap_or("").trim();
                if interpreter.is_empty() {
                    return Err(ToolError::invalid_arguments("interpreter is required for script"));
                }
                let script = input.script.as_deref().unwrap_or("");
                if script.trim().is_empty() {
                    return Err(ToolError::invalid_arguments("script is required for script"));
                }
                GuestOp::RunScript {
                    interpreter: interpreter.to_owned(),
                    script: script.to_owned(),
                    no_wait: input.no_wait,
                    interactive: input.interactive,
                }
            }
            GuestWriteAction::Type => {
                let keystrokes = input.keystrokes.as_deref().unwrap_or("");
                if keystrokes.is_empty() {
                    return Err(ToolError::invalid_arguments("keystrokes is required for type"));
                }
                GuestOp::Type(keystrokes.to_owned())
            }
            GuestWriteAction::TempFile => GuestOp::TempFile,
            GuestWriteAction::CopyTo | GuestWriteAction::CopyFrom => {
                unreachable!("copy actions are handled before the guest operation is built")
            }
            GuestWriteAction::Kill => {
                let pid = input.pid.unwrap_or(0);
                if pid == 0 {
                    return Err(ToolError::invalid_arguments("pid is required for kill"));
                }
                GuestOp::Kill(pid)
            }
            GuestWriteAction::ToolsInstall => GuestOp::ToolsInstall,
        };
        let vm_id = vm_id(context, session_id)?;
        let detail = match &op {
            GuestOp::MakeDirectory(path)
            | GuestOp::RemoveDirectory(path)
            | GuestOp::RemoveFile(path)
            | GuestOp::ListDirectory(path) => format!("{} {path}", action.as_str()),
            GuestOp::Rename { from, to } => format!("rename {from} {to}"),
            GuestOp::Run { program, .. } => format!("run {program}"),
            GuestOp::RunScript { interpreter, .. } => format!("script {interpreter}"),
            GuestOp::Kill(pid) => format!("kill {pid}"),
            _ => action.as_str().to_owned(),
        };
        let subject = gate::subject(&format!("{vm_id} {detail}"));
        let verdict = sevenshell_policy::vm_guest(action.as_str());
        let session_name = saved_session(context.core(), session_id).map(|session| session.name);
        let approval = gate::approve_risk(
            context,
            VM_GUEST_WRITE,
            &verdict,
            Some((session_id, session_name)),
            None,
            &subject,
        )
        .await?;

        let output = gate::bounded(context, VM_OPERATION_TIMEOUT, async {
            context
                .core()
                .vmware_guest(session_id, &op)
                .await
                .map_err(ToolError::from)
        })
        .await?;

        let mut audit = ToolAudit::for_session(session_id)
            .with_subject(subject)
            .with_risk(verdict);
        if let Some(record) = approval {
            audit = audit.with_approval(record);
        }
        let (text, truncated) = gate::clip(&output, gate::TEXT_LIMIT);
        Ok(ToolResult::new(json!({
            "sessionId": session_id,
            "output": text,
            "truncated": truncated,
        }))
        .with_audit(audit))
    }
}

pub struct VmSendKeyTool;

#[async_trait]
impl Tool for VmSendKeyTool {
    fn descriptor(&self) -> ToolDescriptor {
        vm_send_key_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: VmSendKeyInput = parse_arguments(arguments)?;
        let session_id = session_id(&input.session_id)?;
        context.guard_session(session_id, ToolAccess::Write)?;
        let sequence = input.sequence.trim();
        if sequence.is_empty() {
            return Err(ToolError::invalid_arguments("sequence is empty"));
        }
        let vm_id = vm_id(context, session_id)?;
        let subject = gate::subject(&format!("{vm_id} send-key"));
        let verdict = sevenshell_policy::vm_guest("send-key");
        let session_name = saved_session(context.core(), session_id).map(|session| session.name);
        let approval = gate::approve_risk(
            context,
            VM_SEND_KEY,
            &verdict,
            Some((session_id, session_name)),
            None,
            &subject,
        )
        .await?;

        let output = gate::bounded(context, VM_OPERATION_TIMEOUT, async {
            context
                .core()
                .vmware_send_key(session_id, sequence)
                .await
                .map_err(ToolError::from)
        })
        .await?;

        let mut audit = ToolAudit::for_session(session_id)
            .with_subject(subject)
            .with_risk(verdict);
        if let Some(record) = approval {
            audit = audit.with_approval(record);
        }
        let (text, truncated) = gate::clip(&output, gate::TEXT_LIMIT);
        Ok(ToolResult::new(json!({
            "sessionId": session_id,
            "action": "send-key",
            "output": text,
            "truncated": truncated,
        }))
        .with_audit(audit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_destructive_guest_actions_are_the_ones_that_ask() {
        for action in [
            GuestWriteAction::Rm,
            GuestWriteAction::Rmdir,
            GuestWriteAction::Kill,
            GuestWriteAction::Type,
        ] {
            assert!(
                sevenshell_policy::vm_guest(action.as_str()).needs_approval(),
                "`{}` must stop for a person",
                action.as_str()
            );
        }
        for action in [GuestWriteAction::Mkdir, GuestWriteAction::Rename, GuestWriteAction::Run] {
            assert!(
                !sevenshell_policy::vm_guest(action.as_str()).needs_approval(),
                "`{}` is a change, not a destruction",
                action.as_str()
            );
        }
        assert!(
            sevenshell_policy::vm_guest("send-key").needs_approval(),
            "console keys can answer a prompt that erases a disk"
        );
    }

    #[test]
    fn the_destructive_snapshot_actions_are_the_ones_that_ask() {
        for action in [SnapshotChange::Revert, SnapshotChange::Delete] {
            assert!(sevenshell_policy::vm_snapshot(action.as_str()).needs_approval());
        }
        assert!(!sevenshell_policy::vm_snapshot(SnapshotChange::Take.as_str()).needs_approval());
    }

    #[test]
    fn a_guest_run_with_too_many_arguments_is_refused_before_vmrun() {
        let input = json!({
            "sessionId": "vm",
            "action": "run",
            "program": "/bin/true",
            "args": vec!["a"; MAX_PROGRAM_ARGS + 1]
        });
        let parsed = parse_arguments::<VmGuestWriteInput>(input).expect("the shape is valid");
        assert_eq!(
            parsed.args.len(),
            MAX_PROGRAM_ARGS + 1,
            "the limit is checked by the call"
        );
    }
}
