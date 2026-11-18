//! Configuration, host network and whole-machine tools.
//!
//! These are the fixed `vmcli` modules and the `vmrun` verbs behind them: chipset, disks, adapters,
//! shared folders, VProbes, the configuration key/value pairs, the host's virtual networks and
//! port forwards, and deleting or upgrading a machine.
//!
//! Every action is a closed verb; nothing takes a host path, so an agent cannot point a shared
//! folder, a disk or a clone at a file it chose. The operations whose destination is a folder on
//! this machine wait for a token that can name one without the interface ever holding it.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use sevenshell_vmware::{ConfigRead, ConfigWrite, HostRead, HostWrite, ImageAction};

use crate::context::ToolContext;
use crate::registry::Tool;
use crate::tool::{ToolAccess, ToolAudit, ToolDescriptor, ToolError, ToolInputSchema, ToolResult, parse_arguments};

use super::cache::saved_session;
use super::gate;
use super::session_id;
use super::vmware::output_result;

pub const VM_CONFIG_READ: &str = "sshell_vm_config_read";
pub const VM_CONFIG_WRITE: &str = "sshell_vm_config_write";
pub const VM_HOST_READ: &str = "sshell_vm_host_read";
pub const VM_HOST_WRITE: &str = "sshell_vm_host_write";
pub const VM_IMAGE: &str = "sshell_vm_image";
pub const HOST_LOCATIONS_LIST: &str = "sshell_host_locations_list";
pub const VM_SCREENSHOT: &str = "sshell_vm_screenshot";
pub const VMREST_READ: &str = "sshell_vmrest_read";
pub const VMREST_NETWORK_CREATE: &str = "sshell_vmrest_network_create";

const VM_OPERATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

// -- Inputs -------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigReadAction {
    Config,
    Chipset,
    Disk,
    Ethernet,
    Serial,
    Sata,
    Nvme,
    Hgfs,
    Vprobes,
    Mks,
    Variable,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmConfigReadInput {
    pub session_id: String,
    pub action: ConfigReadAction,
    #[serde(default)]
    pub var_type: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigWriteAction {
    SetCpu,
    SetMemory,
    SetCores,
    DiskCreate,
    DiskExtend,
    EthernetSetType,
    EthernetSetPresent,
    EthernetSetConnected,
    EthernetSetDevice,
    EthernetSetNetwork,
    EthernetPurge,
    SerialSetPresent,
    SerialPurge,
    SataSetPresent,
    SataPurge,
    NvmeSetPresent,
    NvmePurge,
    HgfsSetEnabled,
    HgfsSetGuestName,
    HgfsSetPresent,
    HgfsSetRead,
    HgfsSetWrite,
    VprobesEnable,
    VprobesLoad,
    VprobesReset,
    ConfigSet,
    DeviceConnect,
    DeviceDisconnect,
    VariableWrite,
    ToolsUpgrade,
    SharedEnable,
    SharedDisable,
    SharedRemove,
    HgfsSetHostPath,
    SharedAdd,
    SharedSetState,
}

impl ConfigWriteAction {
    /// The name the policy table and the audit line use.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SetCpu => "set-cpu",
            Self::SetMemory => "set-memory",
            Self::SetCores => "set-cores",
            Self::DiskCreate => "disk-create",
            Self::DiskExtend => "disk-extend",
            Self::EthernetSetType => "ethernet-set-type",
            Self::EthernetSetPresent => "ethernet-set-present",
            Self::EthernetSetConnected => "ethernet-set-connected",
            Self::EthernetSetDevice => "ethernet-set-device",
            Self::EthernetSetNetwork => "ethernet-set-network",
            Self::EthernetPurge => "ethernet-purge",
            Self::SerialSetPresent => "serial-set-present",
            Self::SerialPurge => "serial-purge",
            Self::SataSetPresent => "sata-set-present",
            Self::SataPurge => "sata-purge",
            Self::NvmeSetPresent => "nvme-set-present",
            Self::NvmePurge => "nvme-purge",
            Self::HgfsSetEnabled => "hgfs-set-enabled",
            Self::HgfsSetGuestName => "hgfs-set-guest-name",
            Self::HgfsSetPresent => "hgfs-set-present",
            Self::HgfsSetRead => "hgfs-set-read",
            Self::HgfsSetWrite => "hgfs-set-write",
            Self::VprobesEnable => "vprobes-enable",
            Self::VprobesLoad => "vprobes-load",
            Self::VprobesReset => "vprobes-reset",
            Self::ConfigSet => "config-set",
            Self::DeviceConnect => "device-connect",
            Self::DeviceDisconnect => "device-disconnect",
            Self::VariableWrite => "variable-write",
            Self::ToolsUpgrade => "tools-upgrade",
            Self::SharedEnable => "shared-enable",
            Self::SharedDisable => "shared-disable",
            Self::SharedRemove => "shared-remove",
            Self::HgfsSetHostPath => "hgfs-set-host-path",
            Self::SharedAdd => "shared-add",
            Self::SharedSetState => "shared-set-state",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmConfigWriteInput {
    pub session_id: String,
    pub action: ConfigWriteAction,
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub memory_mb: Option<u32>,
    #[serde(default)]
    pub cores: Option<u32>,
    #[serde(default)]
    pub size_gb: Option<u32>,
    #[serde(default)]
    pub disk_type: Option<String>,
    #[serde(default)]
    pub adapter: Option<u32>,
    #[serde(default)]
    pub device: Option<u32>,
    #[serde(default)]
    pub index: Option<u32>,
    #[serde(default)]
    pub present: Option<bool>,
    #[serde(default)]
    pub connected: Option<bool>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub read: Option<bool>,
    #[serde(default)]
    pub write: Option<bool>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub var_type: Option<String>,
    #[serde(default)]
    pub device_name: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub writable: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HostReadAction {
    Networks,
    PortForwards,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmHostReadInput {
    pub session_id: String,
    pub action: HostReadAction,
    #[serde(default)]
    pub network: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HostWriteAction {
    SetPortForward,
    DeletePortForward,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmHostWriteInput {
    pub session_id: String,
    pub action: HostWriteAction,
    pub network: String,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub host_port: Option<u32>,
    #[serde(default)]
    pub guest_ip: Option<String>,
    #[serde(default)]
    pub guest_port: Option<u32>,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageActionInput {
    Delete,
    Upgrade,
    Clone,
    TemplateCreate,
    TemplateDeploy,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmImageInput {
    pub session_id: String,
    pub action: ImageActionInput,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub clone_type: Option<String>,
    #[serde(default)]
    pub snapshot: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostLocationsInput {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VmrestReadAction {
    Vms,
    Vm,
    Networks,
    MacToIp,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmrestReadInput {
    pub action: VmrestReadAction,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub vmnet: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmrestNetworkCreateInput {
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmScreenshotInput {
    pub session_id: String,
    pub token: String,
}

// -- Descriptors --------------------------------------------------------

fn string(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn string_enum(description: &str, values: &[&str]) -> Value {
    json!({ "type": "string", "enum": values, "description": description })
}

fn boolean(description: &str) -> Value {
    json!({ "type": "boolean", "description": description })
}

fn integer(description: &str) -> Value {
    json!({ "type": "integer", "minimum": 0, "description": description })
}

fn session_id_property() -> Value {
    string("The id of a saved virtual-machine session, from sshell_sessions_list.")
}

pub fn vm_config_read_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: VM_CONFIG_READ.to_owned(),
        title: "Read a virtual machine's configuration".to_owned(),
        description: "Reads one configuration module: the chipset, disks, network adapters, serial \
                      ports, SATA or NVMe adapters, shared folders, VProbes, the MKS state, the \
                      key/value configuration, or a runtime configuration variable."
            .to_owned(),
        input_schema: ToolInputSchema::object(
            json!({
                "sessionId": session_id_property(),
                "action": string_enum(
                    "Which module to read.",
                    &["config", "chipset", "disk", "ethernet", "serial", "sata", "nvme", "hgfs", "vprobes", "mks", "variable"]
                ),
                "varType": string("For variable: `runtimeConfig`, the only kind read without a guest sign-in."),
                "name": string("For variable: the variable's name.")
            }),
            &["sessionId", "action"],
        ),
        access: ToolAccess::Read,
    }
}

pub fn vm_config_write_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: VM_CONFIG_WRITE.to_owned(),
        title: "Change a virtual machine's configuration".to_owned(),
        description: "Changes CPU, memory or cores; creates or extends a disk; changes or removes \
                      network, serial, SATA or NVMe adapters; changes a shared folder's name, \
                      presence or permissions; enables, loads or resets VProbes; sets a configuration key; \
                      connects or disconnects a device; writes a runtime variable; upgrades VMware \
                      Tools; or enables, disables or removes shared folders. Purging an adapter, \
                      extending a disk and disconnecting a device are graded destructive."
            .to_owned(),
        input_schema: ToolInputSchema::object(
            json!({
                "sessionId": session_id_property(),
                "action": string_enum(
                    "Which change to make. The fields it uses are named in each action's name.",
                    &[
                        "setCpu", "setMemory", "setCores", "diskCreate", "diskExtend",
                        "ethernetSetType", "ethernetSetPresent", "ethernetSetConnected",
                        "ethernetSetDevice", "ethernetSetNetwork", "ethernetPurge",
                        "serialSetPresent", "serialPurge", "sataSetPresent", "sataPurge",
                        "nvmeSetPresent", "nvmePurge", "hgfsSetEnabled", "hgfsSetGuestName",
                        "hgfsSetPresent", "hgfsSetRead", "hgfsSetWrite", "vprobesEnable",
                        "vprobesLoad", "vprobesReset", "configSet", "deviceConnect", "deviceDisconnect",
                        "variableWrite", "toolsUpgrade", "sharedEnable", "sharedDisable", "sharedRemove",
                        "hgfsSetHostPath", "sharedAdd", "sharedSetState"
                    ]
                ),
                "count": integer("For setCpu: the number of virtual CPUs."),
                "memoryMb": integer("For setMemory: the memory size in MB."),
                "cores": integer("For setCores: cores per socket."),
                "sizeGb": integer("For diskCreate and diskExtend: the size in GB."),
                "diskType": string("For diskCreate: `scsi`, `sata` or `nvme`. Defaults to `scsi`."),
                "adapter": integer("For disk and SATA/NVMe actions: the adapter number."),
                "device": integer("For disk actions: the device number on the adapter."),
                "index": integer("For ethernet, serial and shared-folder actions: the adapter's index."),
                "present": boolean("For *SetPresent: whether the adapter exists."),
                "connected": boolean("For ethernetSetConnected: connect it at power-on."),
                "kind": string("For ethernetSetType: `bridged`, `nat`, `hostonly` or `custom`."),
                "name": string("For ethernetSetNetwork, hgfsSetGuestName and sharedRemove: the name."),
                "read": boolean("For hgfsSetRead: allow reading the shared folder."),
                "write": boolean("For hgfsSetWrite: allow writing the shared folder."),
                "enabled": boolean("For hgfsSetEnabled and vprobesEnable: the switch."),
                "key": string("For configSet: the configuration key."),
                "value": string("For configSet and variableWrite: the value."),
                "varType": string("For variableWrite: `runtimeConfig`."),
                "deviceName": string("For deviceConnect and deviceDisconnect: e.g. `ethernet0` or `cdrom0`."),
                "token": string("For hgfsSetHostPath, sharedAdd and sharedSetState: a folder token from sshell_host_locations_list. For vprobesLoad: a file token naming the probe script."),
                "writable": boolean("For sharedSetState: whether the shared folder is writable.")
            }),
            &["sessionId", "action"],
        ),
        access: ToolAccess::Write,
    }
}

pub fn vm_host_read_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: VM_HOST_READ.to_owned(),
        title: "Read this machine's virtual network".to_owned(),
        description: "Lists the host's virtual networks, or one network's port forwards.".to_owned(),
        input_schema: ToolInputSchema::object(
            json!({
                "sessionId": session_id_property(),
                "action": string_enum("What to read.", &["networks", "portForwards"]),
                "network": string("For portForwards: the network's name, e.g. `vmnet8`.")
            }),
            &["sessionId", "action"],
        ),
        access: ToolAccess::Read,
    }
}

pub fn vm_host_write_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: VM_HOST_WRITE.to_owned(),
        title: "Change this machine's port forwards".to_owned(),
        description: "Sets or deletes one port forward on a host virtual network. A forward pointing \
                      at a guest address exposes that service on this machine's network."
            .to_owned(),
        input_schema: ToolInputSchema::object(
            json!({
                "sessionId": session_id_property(),
                "action": string_enum("What to do.", &["setPortForward", "deletePortForward"]),
                "network": string("The network's name, e.g. `vmnet8`."),
                "protocol": string_enum("For setPortForward and deletePortForward: `tcp` or `udp`.", &["tcp", "udp"]),
                "hostPort": integer("For both: the port on this machine."),
                "guestIp": string("For setPortForward: the guest address the forward points at."),
                "guestPort": integer("For setPortForward: the port inside the guest."),
                "description": string("For setPortForward: a note shown by Workstation.")
            }),
            &["sessionId", "action", "network", "protocol", "hostPort"],
        ),
        access: ToolAccess::Write,
    }
}

pub fn vm_image_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: VM_IMAGE.to_owned(),
        title: "Delete or upgrade a virtual machine".to_owned(),
        description: "Deletes a machine and its files, upgrades its hardware format, clones it into \
                      a host folder, or creates and deploys a .vmtx template. Delete and upgrade \
                      are irreversible and stop for a person's approval on the desktop."
            .to_owned(),
        input_schema: ToolInputSchema::object(
            json!({
                "sessionId": session_id_property(),
                "action": string_enum(
                    "What to do with the machine.",
                    &["delete", "upgrade", "clone", "templateCreate", "templateDeploy"]
                ),
                "token": string("For clone and templateCreate: a folder token from sshell_host_locations_list. For templateDeploy: a file token naming the .vmtx."),
                "name": string("For clone: the new machine's name. For templateCreate: the template's name, which also names its .vmtx; the machine must be powered off."),
                "cloneType": string("For clone: `full` or `linked`. Defaults to `linked`."),
                "snapshot": string("For clone: clone from this snapshot instead of the current state.")
            }),
            &["sessionId", "action"],
        ),
        access: ToolAccess::Write,
    }
}

// -- Argument helpers ---------------------------------------------------

fn required_text(value: Option<&str>, name: &str) -> Result<String, ToolError> {
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

fn required_number(value: Option<u32>, name: &str) -> Result<u32, ToolError> {
    value.ok_or_else(|| ToolError::invalid_arguments(format!("{name} is required for this action")))
}

fn required_switch(value: Option<bool>, name: &str) -> Result<bool, ToolError> {
    value.ok_or_else(|| ToolError::invalid_arguments(format!("{name} is required for this action")))
}

fn port(value: Option<u32>, name: &str) -> Result<u32, ToolError> {
    let port = required_number(value, name)?;
    if port == 0 || port > 65535 {
        return Err(ToolError::invalid_arguments(format!(
            "{name} must be between 1 and 65535"
        )));
    }
    Ok(port)
}

fn protocol(value: Option<&str>) -> Result<String, ToolError> {
    let value = required_text(value, "protocol")?.to_ascii_lowercase();
    if !matches!(value.as_str(), "tcp" | "udp") {
        return Err(ToolError::invalid_arguments("protocol must be `tcp` or `udp`"));
    }
    Ok(value)
}

fn runtime_variable_type(value: Option<&str>) -> Result<String, ToolError> {
    let value = value.unwrap_or("runtimeConfig").trim().to_owned();
    if value != "runtimeConfig" {
        return Err(ToolError::invalid_arguments(
            "only `runtimeConfig` is read or written here; guest environment variables need the guest tools",
        ));
    }
    Ok(value)
}

fn vm_id(context: &ToolContext, session_id: &str) -> Result<String, ToolError> {
    context.core().vmware_session_vm_id(session_id).map_err(ToolError::from)
}

async fn run_write(
    context: &ToolContext,
    session_id: &str,
    tool: &str,
    action: &str,
    subject: &str,
    step: impl std::future::Future<Output = Result<String, sevenshell_error::ShellError>>,
) -> Result<ToolResult, ToolError> {
    let vm_id = vm_id(context, session_id)?;
    let subject = gate::subject(&format!("{vm_id} {action} {subject}"));
    let verdict = sevenshell_policy::vm_config(action);
    let name = saved_session(context.core(), session_id).map(|session| session.name);
    let approval = gate::approve_risk(context, tool, &verdict, Some((session_id, name)), None, &subject).await?;
    let output = gate::bounded(context, VM_OPERATION_TIMEOUT, async {
        step.await.map_err(ToolError::from)
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

// -- The tools ----------------------------------------------------------

pub struct VmConfigReadTool;

#[async_trait]
impl Tool for VmConfigReadTool {
    fn descriptor(&self) -> ToolDescriptor {
        vm_config_read_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: VmConfigReadInput = parse_arguments(arguments)?;
        let session_id = session_id(&input.session_id)?;
        context.guard_session(session_id, ToolAccess::Read)?;
        let action = match input.action {
            ConfigReadAction::Config => ConfigRead::Config,
            ConfigReadAction::Chipset => ConfigRead::Chipset,
            ConfigReadAction::Disk => ConfigRead::Disk,
            ConfigReadAction::Ethernet => ConfigRead::Ethernet,
            ConfigReadAction::Serial => ConfigRead::Serial,
            ConfigReadAction::Sata => ConfigRead::Sata,
            ConfigReadAction::Nvme => ConfigRead::Nvme,
            ConfigReadAction::Hgfs => ConfigRead::Hgfs,
            ConfigReadAction::Vprobes => ConfigRead::Vprobes,
            ConfigReadAction::Mks => ConfigRead::Mks,
            ConfigReadAction::Variable => ConfigRead::Variable {
                var_type: runtime_variable_type(input.var_type.as_deref())?,
                name: required_text(input.name.as_deref(), "name")?,
            },
        };
        let vm_id = vm_id(context, session_id)?;
        let subject = gate::subject(&format!("{vm_id} {:?}", input.action));
        let output = gate::bounded(context, VM_OPERATION_TIMEOUT, async {
            context
                .core()
                .vmware_config_read(session_id, &action)
                .await
                .map_err(ToolError::from)
        })
        .await?;
        Ok(output_result(session_id, Some(subject), output))
    }
}

pub struct VmConfigWriteTool;

#[async_trait]
impl Tool for VmConfigWriteTool {
    fn descriptor(&self) -> ToolDescriptor {
        vm_config_write_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: VmConfigWriteInput = parse_arguments(arguments)?;
        let session_id = session_id(&input.session_id)?;
        context.guard_session(session_id, ToolAccess::Write)?;
        let token_action = matches!(
            input.action,
            ConfigWriteAction::HgfsSetHostPath
                | ConfigWriteAction::SharedAdd
                | ConfigWriteAction::SharedSetState
                | ConfigWriteAction::VprobesLoad
        );
        if token_action {
            let token = required_text(input.token.as_deref(), "token")?;
            let index = if input.action == ConfigWriteAction::HgfsSetHostPath {
                Some(required_number(input.index, "index")?)
            } else {
                None
            };
            let name = if matches!(
                input.action,
                ConfigWriteAction::SharedAdd | ConfigWriteAction::SharedSetState
            ) {
                Some(required_text(input.name.as_deref(), "name")?)
            } else {
                None
            };
            let writable = if input.action == ConfigWriteAction::SharedSetState {
                Some(required_switch(input.writable, "writable")?)
            } else {
                None
            };
            let action_name = input.action.as_str();
            let detail = match input.action {
                ConfigWriteAction::HgfsSetHostPath => index.unwrap_or_default().to_string(),
                ConfigWriteAction::SharedAdd => name.clone().unwrap_or_default(),
                ConfigWriteAction::VprobesLoad => String::new(),
                _ => format!("{} {}", name.clone().unwrap_or_default(), writable.unwrap_or_default()),
            };
            let vm_id = vm_id(context, session_id)?;
            let subject = gate::subject(&format!("{vm_id} {action_name} {detail}"));
            let verdict = sevenshell_policy::vm_config(action_name);
            let session_name = saved_session(context.core(), session_id).map(|session| session.name);
            let approval = gate::approve_risk(
                context,
                VM_CONFIG_WRITE,
                &verdict,
                Some((session_id, session_name)),
                None,
                &subject,
            )
            .await?;
            let output = gate::bounded(context, VM_OPERATION_TIMEOUT, async {
                let result = match input.action {
                    ConfigWriteAction::HgfsSetHostPath => {
                        context
                            .core()
                            .vmware_hgfs_set_host_path(session_id, index.unwrap_or_default(), &token)
                            .await
                    }
                    ConfigWriteAction::SharedAdd => {
                        context
                            .core()
                            .vmware_shared_add(session_id, name.as_deref().unwrap_or_default(), &token)
                            .await
                    }
                    ConfigWriteAction::VprobesLoad => context.core().vmware_vprobes_load(session_id, &token).await,
                    _ => {
                        context
                            .core()
                            .vmware_shared_set_state(
                                session_id,
                                name.as_deref().unwrap_or_default(),
                                &token,
                                writable.unwrap_or(false),
                            )
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
        let action_name = input.action.as_str();
        let detail;
        let action = match input.action {
            ConfigWriteAction::SetCpu => {
                detail = format!("{}", required_number(input.count, "count")?);
                ConfigWrite::SetCpu(required_number(input.count, "count")?)
            }
            ConfigWriteAction::SetMemory => {
                detail = format!("{}", required_number(input.memory_mb, "memoryMb")?);
                ConfigWrite::SetMemory(required_number(input.memory_mb, "memoryMb")?)
            }
            ConfigWriteAction::SetCores => {
                detail = format!("{}", required_number(input.cores, "cores")?);
                ConfigWrite::SetCores(required_number(input.cores, "cores")?)
            }
            ConfigWriteAction::DiskCreate => {
                let size_gb = required_number(input.size_gb, "sizeGb")?;
                detail = format!("{size_gb}");
                ConfigWrite::DiskCreate {
                    size_gb,
                    disk_type: input.disk_type.unwrap_or_else(|| "scsi".to_owned()),
                    adapter: input.adapter.unwrap_or(0),
                    device: input.device.unwrap_or(0),
                }
            }
            ConfigWriteAction::DiskExtend => {
                let size_gb = required_number(input.size_gb, "sizeGb")?;
                detail = format!("{size_gb}");
                ConfigWrite::DiskExtend {
                    size_gb,
                    adapter: input.adapter.unwrap_or(0),
                    device: input.device.unwrap_or(0),
                }
            }
            ConfigWriteAction::EthernetSetType => {
                let index = required_number(input.index, "index")?;
                let kind = required_text(input.kind.as_deref(), "kind")?;
                detail = format!("{index} {kind}");
                ConfigWrite::EthernetSetType { index, kind }
            }
            ConfigWriteAction::EthernetSetPresent => {
                let index = required_number(input.index, "index")?;
                let present = required_switch(input.present, "present")?;
                detail = format!("{index} {present}");
                ConfigWrite::EthernetSetPresent { index, present }
            }
            ConfigWriteAction::EthernetSetConnected => {
                let index = required_number(input.index, "index")?;
                let connected = required_switch(input.connected, "connected")?;
                detail = format!("{index} {connected}");
                ConfigWrite::EthernetSetConnected { index, connected }
            }
            ConfigWriteAction::EthernetSetDevice => {
                let index = required_number(input.index, "index")?;
                let device = required_text(input.device_name.as_deref(), "deviceName")?;
                detail = format!("{index} {device}");
                ConfigWrite::EthernetSetDevice { index, device }
            }
            ConfigWriteAction::EthernetSetNetwork => {
                let index = required_number(input.index, "index")?;
                let name = required_text(input.name.as_deref(), "name")?;
                detail = format!("{index} {name}");
                ConfigWrite::EthernetSetNetwork { index, name }
            }
            ConfigWriteAction::EthernetPurge => {
                let index = required_number(input.index, "index")?;
                detail = format!("{index}");
                ConfigWrite::EthernetPurge { index }
            }
            ConfigWriteAction::SerialSetPresent => {
                let index = required_number(input.index, "index")?;
                let present = required_switch(input.present, "present")?;
                detail = format!("{index} {present}");
                ConfigWrite::SerialSetPresent { index, present }
            }
            ConfigWriteAction::SerialPurge => {
                let index = required_number(input.index, "index")?;
                detail = format!("{index}");
                ConfigWrite::SerialPurge { index }
            }
            ConfigWriteAction::SataSetPresent => {
                let adapter = required_number(input.adapter, "adapter")?;
                let present = required_switch(input.present, "present")?;
                detail = format!("{adapter} {present}");
                ConfigWrite::SataSetPresent { adapter, present }
            }
            ConfigWriteAction::SataPurge => {
                let adapter = required_number(input.adapter, "adapter")?;
                detail = format!("{adapter}");
                ConfigWrite::SataPurge { adapter }
            }
            ConfigWriteAction::NvmeSetPresent => {
                let adapter = required_number(input.adapter, "adapter")?;
                let present = required_switch(input.present, "present")?;
                detail = format!("{adapter} {present}");
                ConfigWrite::NvmeSetPresent { adapter, present }
            }
            ConfigWriteAction::NvmePurge => {
                let adapter = required_number(input.adapter, "adapter")?;
                detail = format!("{adapter}");
                ConfigWrite::NvmePurge { adapter }
            }
            ConfigWriteAction::HgfsSetEnabled => {
                let index = required_number(input.index, "index")?;
                let enabled = required_switch(input.enabled, "enabled")?;
                detail = format!("{index} {enabled}");
                ConfigWrite::HgfsSetEnabled { index, enabled }
            }
            ConfigWriteAction::HgfsSetGuestName => {
                let index = required_number(input.index, "index")?;
                let name = required_text(input.name.as_deref(), "name")?;
                detail = format!("{index} {name}");
                ConfigWrite::HgfsSetGuestName { index, name }
            }
            ConfigWriteAction::HgfsSetPresent => {
                let index = required_number(input.index, "index")?;
                let present = required_switch(input.present, "present")?;
                detail = format!("{index} {present}");
                ConfigWrite::HgfsSetPresent { index, present }
            }
            ConfigWriteAction::HgfsSetRead => {
                let index = required_number(input.index, "index")?;
                let read = required_switch(input.read, "read")?;
                detail = format!("{index} {read}");
                ConfigWrite::HgfsSetRead { index, read }
            }
            ConfigWriteAction::HgfsSetWrite => {
                let index = required_number(input.index, "index")?;
                let write = required_switch(input.write, "write")?;
                detail = format!("{index} {write}");
                ConfigWrite::HgfsSetWrite { index, write }
            }
            ConfigWriteAction::VprobesEnable => {
                let enabled = required_switch(input.enabled, "enabled")?;
                detail = format!("{enabled}");
                ConfigWrite::VprobesEnable(enabled)
            }
            ConfigWriteAction::VprobesReset => {
                detail = String::new();
                ConfigWrite::VprobesReset
            }
            ConfigWriteAction::ConfigSet => {
                let key = required_text(input.key.as_deref(), "key")?;
                let value = required_text(input.value.as_deref(), "value")?;
                detail = key.clone();
                ConfigWrite::ConfigSet { key, value }
            }
            ConfigWriteAction::DeviceConnect => {
                let device = required_text(input.device_name.as_deref(), "deviceName")?;
                detail = device.clone();
                ConfigWrite::DeviceConnect(device)
            }
            ConfigWriteAction::DeviceDisconnect => {
                let device = required_text(input.device_name.as_deref(), "deviceName")?;
                detail = device.clone();
                ConfigWrite::DeviceDisconnect(device)
            }
            ConfigWriteAction::VariableWrite => {
                let var_type = runtime_variable_type(input.var_type.as_deref())?;
                let name = required_text(input.name.as_deref(), "name")?;
                let value = required_text(input.value.as_deref(), "value")?;
                detail = name.clone();
                ConfigWrite::VariableWrite { var_type, name, value }
            }
            ConfigWriteAction::ToolsUpgrade => {
                detail = String::new();
                ConfigWrite::ToolsUpgrade
            }
            ConfigWriteAction::SharedEnable => {
                detail = String::new();
                ConfigWrite::SharedEnable
            }
            ConfigWriteAction::SharedDisable => {
                detail = String::new();
                ConfigWrite::SharedDisable
            }
            ConfigWriteAction::SharedRemove => {
                let name = required_text(input.name.as_deref(), "name")?;
                detail = name.clone();
                ConfigWrite::SharedRemove(name)
            }
            ConfigWriteAction::HgfsSetHostPath
            | ConfigWriteAction::SharedAdd
            | ConfigWriteAction::SharedSetState
            | ConfigWriteAction::VprobesLoad => {
                unreachable!("the token actions are handled before the change is built")
            }
        };
        run_write(context, session_id, VM_CONFIG_WRITE, action_name, &detail, async {
            context.core().vmware_config_write(session_id, &action).await
        })
        .await
    }
}

pub struct VmHostReadTool;

#[async_trait]
impl Tool for VmHostReadTool {
    fn descriptor(&self) -> ToolDescriptor {
        vm_host_read_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: VmHostReadInput = parse_arguments(arguments)?;
        let session_id = session_id(&input.session_id)?;
        context.guard_session(session_id, ToolAccess::Read)?;
        let action = match input.action {
            HostReadAction::Networks => HostRead::Networks,
            HostReadAction::PortForwards => HostRead::PortForwards(required_text(input.network.as_deref(), "network")?),
        };
        let vm_id = vm_id(context, session_id)?;
        let subject = gate::subject(&format!("{vm_id} {:?}", input.action));
        let output = gate::bounded(context, VM_OPERATION_TIMEOUT, async {
            context
                .core()
                .vmware_host_read(session_id, &action)
                .await
                .map_err(ToolError::from)
        })
        .await?;
        Ok(output_result(session_id, Some(subject), output))
    }
}

pub struct VmHostWriteTool;

#[async_trait]
impl Tool for VmHostWriteTool {
    fn descriptor(&self) -> ToolDescriptor {
        vm_host_write_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: VmHostWriteInput = parse_arguments(arguments)?;
        let session_id = session_id(&input.session_id)?;
        context.guard_session(session_id, ToolAccess::Write)?;
        let network = required_text(Some(input.network.as_str()), "network")?;
        let action_name = match input.action {
            HostWriteAction::SetPortForward => "port-forward-set",
            HostWriteAction::DeletePortForward => "port-forward-delete",
        };
        let action = match input.action {
            HostWriteAction::SetPortForward => HostWrite::SetPortForward {
                network,
                protocol: protocol(input.protocol.as_deref())?,
                host_port: port(input.host_port, "hostPort")?,
                guest_ip: required_text(input.guest_ip.as_deref(), "guestIp")?,
                guest_port: port(input.guest_port, "guestPort")?,
                description: input.description.trim().to_owned(),
            },
            HostWriteAction::DeletePortForward => HostWrite::DeletePortForward {
                network,
                protocol: protocol(input.protocol.as_deref())?,
                host_port: port(input.host_port, "hostPort")?,
            },
        };
        let vm_id = vm_id(context, session_id)?;
        let subject = gate::subject(&format!("{vm_id} {action_name}"));
        let verdict = sevenshell_policy::vm_host(action_name);
        let name = saved_session(context.core(), session_id).map(|session| session.name);
        let approval = gate::approve_risk(
            context,
            VM_HOST_WRITE,
            &verdict,
            Some((session_id, name)),
            None,
            &subject,
        )
        .await?;
        let output = gate::bounded(context, VM_OPERATION_TIMEOUT, async {
            context
                .core()
                .vmware_host_write(session_id, &action)
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

pub struct VmImageTool;

#[async_trait]
impl Tool for VmImageTool {
    fn descriptor(&self) -> ToolDescriptor {
        vm_image_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: VmImageInput = parse_arguments(arguments)?;
        let session_id = session_id(&input.session_id)?;
        context.guard_session(session_id, ToolAccess::Write)?;
        if matches!(
            input.action,
            ImageActionInput::TemplateCreate | ImageActionInput::TemplateDeploy
        ) {
            let token = required_text(input.token.as_deref(), "token")?;
            let create = input.action == ImageActionInput::TemplateCreate;
            let name = if create {
                Some(required_text(input.name.as_deref(), "name")?)
            } else {
                None
            };
            let action_name = if create { "template-create" } else { "template-deploy" };
            let vm_id = vm_id(context, session_id)?;
            let detail = if create {
                format!("{token} {}", name.as_deref().unwrap_or_default())
            } else {
                token.clone()
            };
            let subject = gate::subject(&format!("{vm_id} {action_name} {detail}"));
            let verdict = sevenshell_policy::vm_image(action_name);
            let session_name = saved_session(context.core(), session_id).map(|session| session.name);
            let approval = gate::approve_risk(
                context,
                VM_IMAGE,
                &verdict,
                Some((session_id, session_name)),
                None,
                &subject,
            )
            .await?;
            let output = gate::bounded(context, VM_OPERATION_TIMEOUT, async {
                let result = if create {
                    context
                        .core()
                        .vmware_template_create(session_id, &token, name.as_deref().unwrap_or_default())
                        .await
                } else {
                    context.core().vmware_template_deploy(session_id, &token).await
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
        if input.action == ImageActionInput::Clone {
            let token = required_text(input.token.as_deref(), "token")?;
            let name = required_text(input.name.as_deref(), "name")?;
            let clone_type = input.clone_type.as_deref().unwrap_or("linked");
            let vm_id = vm_id(context, session_id)?;
            let subject = gate::subject(&format!("{vm_id} clone {name}"));
            let verdict = sevenshell_policy::vm_image("clone");
            let session_name = saved_session(context.core(), session_id).map(|session| session.name);
            let approval = gate::approve_risk(
                context,
                VM_IMAGE,
                &verdict,
                Some((session_id, session_name)),
                None,
                &subject,
            )
            .await?;
            let output = gate::bounded(context, VM_OPERATION_TIMEOUT, async {
                context
                    .core()
                    .vmware_clone(session_id, &token, &name, clone_type, input.snapshot.as_deref())
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
        let (action, action_name) = match input.action {
            ImageActionInput::Delete => (ImageAction::Delete, "delete"),
            ImageActionInput::Upgrade => (ImageAction::Upgrade, "upgrade"),
            ImageActionInput::Clone | ImageActionInput::TemplateCreate | ImageActionInput::TemplateDeploy => {
                unreachable!("handled above")
            }
        };
        let vm_id = vm_id(context, session_id)?;
        let subject = gate::subject(&format!("{vm_id} {action_name}"));
        let verdict = sevenshell_policy::vm_image(action_name);
        let name = saved_session(context.core(), session_id).map(|session| session.name);
        let approval =
            gate::approve_risk(context, VM_IMAGE, &verdict, Some((session_id, name)), None, &subject).await?;
        let output = gate::bounded(context, VM_OPERATION_TIMEOUT, async {
            context
                .core()
                .vmware_image(session_id, action)
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

pub fn host_locations_list_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: HOST_LOCATIONS_LIST.to_owned(),
        title: "List host locations".to_owned(),
        description: "Lists the files and folders somebody has handed to agents on this desktop, as \
                      tokens and display names. A tool that reads or writes a host file takes the \
                      token; the path itself never leaves the host."
            .to_owned(),
        input_schema: ToolInputSchema::object(json!({}), &[]),
        access: ToolAccess::Read,
    }
}

pub fn vm_screenshot_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: VM_SCREENSHOT.to_owned(),
        title: "Capture a virtual machine's screen".to_owned(),
        description: "Writes a PNG of the machine's screen into the folder a host location token \
                      names and answers the file name. The token comes from \
                      sshell_host_locations_list."
            .to_owned(),
        input_schema: ToolInputSchema::object(
            json!({
                "sessionId": session_id_property(),
                "token": string("A folder token from sshell_host_locations_list.")
            }),
            &["sessionId", "token"],
        ),
        access: ToolAccess::Write,
    }
}

pub struct HostLocationsListTool;

#[async_trait]
impl Tool for HostLocationsListTool {
    fn descriptor(&self) -> ToolDescriptor {
        host_locations_list_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let _input: HostLocationsInput = parse_arguments(arguments)?;
        Ok(ToolResult::new(json!({ "locations": context.core().host_locations() })).with_audit(ToolAudit::default()))
    }
}

pub struct VmScreenshotTool;

#[async_trait]
impl Tool for VmScreenshotTool {
    fn descriptor(&self) -> ToolDescriptor {
        vm_screenshot_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: VmScreenshotInput = parse_arguments(arguments)?;
        let session_id = session_id(&input.session_id)?;
        context.guard_session(session_id, ToolAccess::Write)?;
        let token = required_text(Some(input.token.as_str()), "token")?;
        let vm_id = vm_id(context, session_id)?;
        let subject = gate::subject(&format!("{vm_id} screenshot {token}"));
        let verdict = sevenshell_types::RiskAssessment::safe();
        let file_name = gate::bounded(context, VM_OPERATION_TIMEOUT, async {
            context
                .core()
                .vmware_screenshot(session_id, &token)
                .await
                .map_err(ToolError::from)
        })
        .await?;
        Ok(ToolResult::new(json!({
            "sessionId": session_id,
            "fileName": file_name,
        }))
        .with_audit(
            ToolAudit::for_session(session_id)
                .with_subject(subject)
                .with_risk(verdict),
        ))
    }
}

pub fn vmrest_read_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: VMREST_READ.to_owned(),
        title: "Read the Workstation REST service".to_owned(),
        description: "Reads the virtual machines vmrest knows, one of them by id, the host virtual                       networks, or one network's MAC-to-IP map. The service must be running and                       its credentials saved."
            .to_owned(),
        input_schema: ToolInputSchema::object(
            json!({
                "action": string_enum("What to read.", &["vms", "vm", "networks", "macToIp"]),
                "id": string("For vm: the id vmrest reports."),
                "vmnet": string("For macToIp: the network, e.g. `vmnet8`.")
            }),
            &["action"],
        ),
        access: ToolAccess::Read,
    }
}

pub fn vmrest_network_create_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: VMREST_NETWORK_CREATE.to_owned(),
        title: "Create a host virtual network".to_owned(),
        description: "Creates one virtual network on this machine. This is the operation no                       command line offers; the service must be running and its credentials saved."
            .to_owned(),
        input_schema: ToolInputSchema::object(
            json!({
                "name": string("The network's name, e.g. `vmnet20`."),
                "kind": string_enum("The network's type.", &["bridged", "nat", "hostonly"])
            }),
            &["name", "kind"],
        ),
        access: ToolAccess::Write,
    }
}

pub struct VmrestReadTool;

#[async_trait]
impl Tool for VmrestReadTool {
    fn descriptor(&self) -> ToolDescriptor {
        vmrest_read_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: VmrestReadInput = parse_arguments(arguments)?;
        let id = if input.action == VmrestReadAction::Vm {
            Some(required_text(input.id.as_deref(), "id")?)
        } else {
            None
        };
        let vmnet = if input.action == VmrestReadAction::MacToIp {
            Some(required_text(input.vmnet.as_deref(), "vmnet")?)
        } else {
            None
        };
        let subject = match input.action {
            VmrestReadAction::Vms => "vmrest vms".to_owned(),
            VmrestReadAction::Vm => format!("vmrest vm {}", id.as_deref().unwrap_or_default()),
            VmrestReadAction::Networks => "vmrest networks".to_owned(),
            VmrestReadAction::MacToIp => {
                format!("vmrest macToIp {}", vmnet.as_deref().unwrap_or_default())
            }
        };
        let output = gate::bounded(context, VM_OPERATION_TIMEOUT, async {
            let result = match input.action {
                VmrestReadAction::Vms => context.core().vmrest_list_vms().await,
                VmrestReadAction::Vm => context.core().vmrest_get_vm(id.as_deref().unwrap_or_default()).await,
                VmrestReadAction::Networks => context.core().vmrest_list_networks().await,
                VmrestReadAction::MacToIp => {
                    context
                        .core()
                        .vmrest_mac_to_ip(vmnet.as_deref().unwrap_or_default())
                        .await
                }
            };
            result.map_err(ToolError::from)
        })
        .await?;
        Ok(ToolResult::new(json!({ "output": output })).with_audit(ToolAudit::default().with_subject(subject)))
    }
}

pub struct VmrestNetworkCreateTool;

#[async_trait]
impl Tool for VmrestNetworkCreateTool {
    fn descriptor(&self) -> ToolDescriptor {
        vmrest_network_create_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: VmrestNetworkCreateInput = parse_arguments(arguments)?;
        let name = required_text(Some(input.name.as_str()), "name")?;
        let kind = required_text(Some(input.kind.as_str()), "kind")?;
        let subject = gate::subject(&format!("{name} {kind}"));
        let verdict = sevenshell_policy::vm_host("create-network");
        let approval = gate::approve_risk(context, VMREST_NETWORK_CREATE, &verdict, None, None, &subject).await?;
        let output = gate::bounded(context, VM_OPERATION_TIMEOUT, async {
            context
                .core()
                .vmrest_create_network(&name, &kind)
                .await
                .map_err(ToolError::from)
        })
        .await?;
        let mut audit = ToolAudit::default().with_subject(subject).with_risk(verdict);
        if let Some(record) = approval {
            audit = audit.with_approval(record);
        }
        Ok(ToolResult::new(json!({ "output": output })).with_audit(audit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_port_is_a_port_and_not_a_zero() {
        assert!(port(Some(0), "hostPort").is_err());
        assert!(port(Some(65536), "hostPort").is_err());
        assert_eq!(port(Some(2222), "hostPort").expect("a port"), 2222);
    }

    #[test]
    fn a_protocol_is_one_of_the_two_vmrun_names() {
        assert_eq!(protocol(Some("TCP")).expect("tcp"), "tcp");
        assert!(protocol(Some("icmp")).is_err());
    }

    #[test]
    fn only_runtime_variables_are_read_here() {
        assert_eq!(runtime_variable_type(None).expect("the default"), "runtimeConfig");
        assert!(runtime_variable_type(Some("guestEnv")).is_err());
    }
}
