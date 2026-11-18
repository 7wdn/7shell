//! Configuration, host network and whole-machine operations.
//!
//! These are the fixed `vmcli` modules and the few `vmrun` verbs that have no equivalent there:
//! chipset, disks, adapters, shared folders, VProbes, the configuration key/value pairs, the host's
//! virtual networks and port forwards, and deleting or upgrading a machine.
//!
//! Nothing here takes a host path a caller chose. Shared folders are changed by name, disks are
//! created inside the machine, and a clone or a template — the operations whose destination is a
//! folder on this machine — wait for a token that can name one without the interface ever holding
//! it. The VProbes script is the one document here; the core resolves its token first.

use sevenshell_error::ShellError;

use crate::ops::GuestAuth;
use crate::vmcli::Vmcli;
use crate::vmrun::Vmrun;
use crate::{checked_id, resolve};

/// Configuration changes and image operations wait on a hypervisor, not on a network round trip.
const OPERATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// One read of a machine's configuration modules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigRead {
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
    Variable { var_type: String, name: String },
}

/// One change to a machine's configuration. Each variant is a fixed command; the VProbes script is
/// the only path, and the core builds it from a file token rather than from anything a caller sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigWrite {
    SetCpu(u32),
    SetMemory(u32),
    SetCores(u32),
    DiskCreate {
        size_gb: u32,
        disk_type: String,
        adapter: u32,
        device: u32,
    },
    DiskExtend {
        size_gb: u32,
        adapter: u32,
        device: u32,
    },
    EthernetSetType {
        index: u32,
        kind: String,
    },
    EthernetSetPresent {
        index: u32,
        present: bool,
    },
    EthernetSetConnected {
        index: u32,
        connected: bool,
    },
    EthernetSetDevice {
        index: u32,
        device: String,
    },
    EthernetSetNetwork {
        index: u32,
        name: String,
    },
    EthernetPurge {
        index: u32,
    },
    SerialSetPresent {
        index: u32,
        present: bool,
    },
    SerialPurge {
        index: u32,
    },
    SataSetPresent {
        adapter: u32,
        present: bool,
    },
    SataPurge {
        adapter: u32,
    },
    NvmeSetPresent {
        adapter: u32,
        present: bool,
    },
    NvmePurge {
        adapter: u32,
    },
    HgfsSetEnabled {
        index: u32,
        enabled: bool,
    },
    HgfsSetGuestName {
        index: u32,
        name: String,
    },
    HgfsSetPresent {
        index: u32,
        present: bool,
    },
    HgfsSetRead {
        index: u32,
        read: bool,
    },
    HgfsSetWrite {
        index: u32,
        write: bool,
    },
    VprobesEnable(bool),
    VprobesLoad(String),
    VprobesReset,
    ConfigSet {
        key: String,
        value: String,
    },
    DeviceConnect(String),
    DeviceDisconnect(String),
    VariableWrite {
        var_type: String,
        name: String,
        value: String,
    },
    ToolsUpgrade,
    SharedEnable,
    SharedDisable,
    SharedRemove(String),
}

/// One read of the host's own virtual network or its port forwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostRead {
    Networks,
    PortForwards(String),
}

/// One change to the host's port forwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostWrite {
    SetPortForward {
        network: String,
        protocol: String,
        host_port: u32,
        guest_ip: String,
        guest_port: u32,
        description: String,
    },
    DeletePortForward {
        network: String,
        protocol: String,
        host_port: u32,
    },
}

/// One whole-machine change. Both are irreversible: a deleted machine's files are gone, and an
/// upgraded machine no longer opens in an older Workstation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageAction {
    Delete,
    Upgrade,
}

impl ImageAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Delete => "delete",
            Self::Upgrade => "upgrade",
        }
    }
}

impl ConfigWrite {
    /// The name the policy table and the audit line use.
    pub const fn action(&self) -> &'static str {
        match self {
            Self::SetCpu(_) => "set-cpu",
            Self::SetMemory(_) => "set-memory",
            Self::SetCores(_) => "set-cores",
            Self::DiskCreate { .. } => "disk-create",
            Self::DiskExtend { .. } => "disk-extend",
            Self::EthernetSetType { .. } => "ethernet-set-type",
            Self::EthernetSetPresent { .. } => "ethernet-set-present",
            Self::EthernetSetConnected { .. } => "ethernet-set-connected",
            Self::EthernetSetDevice { .. } => "ethernet-set-device",
            Self::EthernetSetNetwork { .. } => "ethernet-set-network",
            Self::EthernetPurge { .. } => "ethernet-purge",
            Self::SerialSetPresent { .. } => "serial-set-present",
            Self::SerialPurge { .. } => "serial-purge",
            Self::SataSetPresent { .. } => "sata-set-present",
            Self::SataPurge { .. } => "sata-purge",
            Self::NvmeSetPresent { .. } => "nvme-set-present",
            Self::NvmePurge { .. } => "nvme-purge",
            Self::HgfsSetEnabled { .. } => "hgfs-set-enabled",
            Self::HgfsSetGuestName { .. } => "hgfs-set-guest-name",
            Self::HgfsSetPresent { .. } => "hgfs-set-present",
            Self::HgfsSetRead { .. } => "hgfs-set-read",
            Self::HgfsSetWrite { .. } => "hgfs-set-write",
            Self::VprobesEnable(_) => "vprobes-enable",
            Self::VprobesLoad(_) => "vprobes-load",
            Self::VprobesReset => "vprobes-reset",
            Self::ConfigSet { .. } => "config-set",
            Self::DeviceConnect(_) => "device-connect",
            Self::DeviceDisconnect(_) => "device-disconnect",
            Self::VariableWrite { .. } => "variable-write",
            Self::ToolsUpgrade => "tools-upgrade",
            Self::SharedEnable => "shared-enable",
            Self::SharedDisable => "shared-disable",
            Self::SharedRemove(_) => "shared-remove",
        }
    }
}

impl HostWrite {
    pub const fn action(&self) -> &'static str {
        match self {
            Self::SetPortForward { .. } => "port-forward-set",
            Self::DeletePortForward { .. } => "port-forward-delete",
        }
    }
}

fn vmcli_beside(vmrun: &Vmrun) -> Result<Vmcli, ShellError> {
    Vmcli::beside(vmrun.program())
        .or_else(Vmcli::locate)
        .ok_or_else(crate::vmcli::unavailable)
}

fn bool_word(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn args(values: &[String]) -> Vec<&str> {
    values.iter().map(String::as_str).collect()
}

/// One configuration module, as `vmcli` prints it.
pub async fn config_read(vm_id: &str, action: &ConfigRead) -> Result<String, ShellError> {
    let vm_id = checked_id(vm_id, "config")?;
    let vmrun = Vmrun::locate()?;
    let (path, _) = resolve(&vmrun, &vm_id, "config").await?;
    if let ConfigRead::Variable { var_type, name } = action {
        return vmrun
            .run_within(
                OPERATION_TIMEOUT,
                "config",
                &["-T", "ws", "readVariable", &path, var_type.as_str(), name.as_str()],
            )
            .await;
    }
    let module: Vec<&str> = match action {
        ConfigRead::Config => vec!["ConfigParams", "query"],
        ConfigRead::Chipset => vec!["Chipset", "query"],
        ConfigRead::Disk => vec!["Disk", "query"],
        ConfigRead::Ethernet => vec!["Ethernet", "query"],
        ConfigRead::Serial => vec!["Serial", "Query"],
        ConfigRead::Sata => vec!["Sata", "query"],
        ConfigRead::Nvme => vec!["Nvme", "query"],
        ConfigRead::Hgfs => vec!["HGFS", "query"],
        ConfigRead::Vprobes => vec!["VProbes", "Query"],
        ConfigRead::Mks => vec!["MKS", "query"],
        ConfigRead::Variable { .. } => unreachable!("handled above"),
    };
    let mut values = vec![path];
    values.extend(module.into_iter().map(str::to_owned));
    let refs = args(&values);
    vmcli_beside(&vmrun)?.run("config", &refs, OPERATION_TIMEOUT).await
}

/// One configuration change, as `vmcli` applies it — or `vmrun`, for the verbs it alone has.
pub async fn config_write(vm_id: &str, action: &ConfigWrite) -> Result<String, ShellError> {
    let vm_id = checked_id(vm_id, "config")?;
    let vmrun = Vmrun::locate()?;
    let (path, _) = resolve(&vmrun, &vm_id, "config").await?;

    match action {
        ConfigWrite::DeviceConnect(device) => {
            return vmrun
                .run_within(
                    OPERATION_TIMEOUT,
                    "config",
                    &["-T", "ws", "connectNamedDevice", &path, device],
                )
                .await;
        }
        ConfigWrite::DeviceDisconnect(device) => {
            return vmrun
                .run_within(
                    OPERATION_TIMEOUT,
                    "config",
                    &["-T", "ws", "disconnectNamedDevice", &path, device],
                )
                .await;
        }
        ConfigWrite::VariableWrite { var_type, name, value } => {
            return vmrun
                .run_within(
                    OPERATION_TIMEOUT,
                    "config",
                    &["-T", "ws", "writeVariable", &path, var_type, name, value],
                )
                .await;
        }
        ConfigWrite::SharedEnable => {
            return vmrun
                .run_within(OPERATION_TIMEOUT, "config", &["-T", "ws", "enableSharedFolders", &path])
                .await;
        }
        ConfigWrite::SharedDisable => {
            return vmrun
                .run_within(
                    OPERATION_TIMEOUT,
                    "config",
                    &["-T", "ws", "disableSharedFolders", &path],
                )
                .await;
        }
        ConfigWrite::SharedRemove(name) => {
            return vmrun
                .run_within(
                    OPERATION_TIMEOUT,
                    "config",
                    &["-T", "ws", "removeSharedFolder", &path, name],
                )
                .await;
        }
        _ => {}
    }

    let mut values: Vec<String> = vec![path];
    values.extend(config_write_args(action));
    let refs = args(&values);
    vmcli_beside(&vmrun)?.run("config", &refs, OPERATION_TIMEOUT).await
}

/// The `vmcli` argument vector for one configuration change, without the document's path.
///
/// Public to the crate so a test can pin the flags without a Workstation install.
pub(crate) fn config_write_args(action: &ConfigWrite) -> Vec<String> {
    let mut values: Vec<String> = Vec::new();
    let numeric = |value: u32| value.to_string();
    match action {
        ConfigWrite::SetCpu(count) => {
            values.extend(["Chipset".into(), "SetVCpuCount".into(), "-c".into(), numeric(*count)])
        }
        ConfigWrite::SetMemory(mb) => values.extend(["Chipset".into(), "SetMemSize".into(), "-s".into(), numeric(*mb)]),
        ConfigWrite::SetCores(cores) => values.extend([
            "Chipset".into(),
            "SetCoresPerSocket".into(),
            "-c".into(),
            numeric(*cores),
        ]),
        ConfigWrite::DiskCreate {
            size_gb,
            disk_type,
            adapter,
            device,
        } => values.extend([
            "Disk".into(),
            "Create".into(),
            "-s".into(),
            numeric(*size_gb),
            "-t".into(),
            disk_type.clone(),
            "-a".into(),
            numeric(*adapter),
            "-d".into(),
            numeric(*device),
        ]),
        ConfigWrite::DiskExtend {
            size_gb,
            adapter,
            device,
        } => values.extend([
            "Disk".into(),
            "Extend".into(),
            "-s".into(),
            numeric(*size_gb),
            "-a".into(),
            numeric(*adapter),
            "-d".into(),
            numeric(*device),
        ]),
        ConfigWrite::EthernetSetType { index, kind } => values.extend([
            "Ethernet".into(),
            "SetConnectionType".into(),
            "-i".into(),
            numeric(*index),
            "-t".into(),
            kind.clone(),
        ]),
        ConfigWrite::EthernetSetPresent { index, present } => values.extend([
            "Ethernet".into(),
            "SetPresent".into(),
            "-i".into(),
            numeric(*index),
            "-e".into(),
            bool_word(*present).into(),
        ]),
        ConfigWrite::EthernetSetConnected { index, connected } => values.extend([
            "Ethernet".into(),
            "SetStartConnected".into(),
            "-i".into(),
            numeric(*index),
            "-e".into(),
            bool_word(*connected).into(),
        ]),
        ConfigWrite::EthernetSetDevice { index, device } => values.extend([
            "Ethernet".into(),
            "SetVirtualDevice".into(),
            "-i".into(),
            numeric(*index),
            "-d".into(),
            device.clone(),
        ]),
        ConfigWrite::EthernetSetNetwork { index, name } => values.extend([
            "Ethernet".into(),
            "SetNetworkName".into(),
            "-i".into(),
            numeric(*index),
            "-n".into(),
            name.clone(),
        ]),
        ConfigWrite::EthernetPurge { index } => {
            values.extend(["Ethernet".into(), "Purge".into(), "-i".into(), numeric(*index)])
        }
        ConfigWrite::SerialSetPresent { index, present } => values.extend([
            "Serial".into(),
            "SetPresent".into(),
            "-i".into(),
            numeric(*index),
            "-e".into(),
            bool_word(*present).into(),
        ]),
        ConfigWrite::SerialPurge { index } => {
            values.extend(["Serial".into(), "Purge".into(), "-i".into(), numeric(*index)])
        }
        ConfigWrite::SataSetPresent { adapter, present } => values.extend([
            "Sata".into(),
            "SetPresent".into(),
            "-a".into(),
            numeric(*adapter),
            "-e".into(),
            bool_word(*present).into(),
        ]),
        ConfigWrite::SataPurge { adapter } => {
            values.extend(["Sata".into(), "Purge".into(), "-a".into(), numeric(*adapter)])
        }
        ConfigWrite::NvmeSetPresent { adapter, present } => values.extend([
            "Nvme".into(),
            "SetPresent".into(),
            "-a".into(),
            numeric(*adapter),
            "-e".into(),
            bool_word(*present).into(),
        ]),
        ConfigWrite::NvmePurge { adapter } => {
            values.extend(["Nvme".into(), "Purge".into(), "-a".into(), numeric(*adapter)])
        }
        ConfigWrite::HgfsSetEnabled { index, enabled } => values.extend([
            "HGFS".into(),
            "SetEnabled".into(),
            "-i".into(),
            numeric(*index),
            "-e".into(),
            bool_word(*enabled).into(),
        ]),
        ConfigWrite::HgfsSetGuestName { index, name } => values.extend([
            "HGFS".into(),
            "SetGuestName".into(),
            "-i".into(),
            numeric(*index),
            "-n".into(),
            name.clone(),
        ]),
        ConfigWrite::HgfsSetPresent { index, present } => values.extend([
            "HGFS".into(),
            "SetPresent".into(),
            "-i".into(),
            numeric(*index),
            "-e".into(),
            bool_word(*present).into(),
        ]),
        ConfigWrite::HgfsSetRead { index, read } => values.extend([
            "HGFS".into(),
            "SetReadAccess".into(),
            "-i".into(),
            numeric(*index),
            "-e".into(),
            bool_word(*read).into(),
        ]),
        ConfigWrite::HgfsSetWrite { index, write } => values.extend([
            "HGFS".into(),
            "SetWriteAccess".into(),
            "-i".into(),
            numeric(*index),
            "-e".into(),
            bool_word(*write).into(),
        ]),
        ConfigWrite::VprobesEnable(enabled) => values.extend([
            "VProbes".into(),
            "SetEnabled".into(),
            "-e".into(),
            bool_word(*enabled).into(),
        ]),
        ConfigWrite::VprobesLoad(script) => {
            values.extend(["VProbes".into(), "Load".into(), "-s".into(), script.clone()])
        }
        ConfigWrite::VprobesReset => values.extend(["VProbes".into(), "Reset".into()]),
        ConfigWrite::ConfigSet { key, value } => values.extend([
            "ConfigParams".into(),
            "SetEntry".into(),
            "-k".into(),
            key.clone(),
            "-v".into(),
            value.clone(),
        ]),
        ConfigWrite::ToolsUpgrade => values.extend(["Tools".into(), "Upgrade".into()]),
        ConfigWrite::DeviceConnect(_)
        | ConfigWrite::DeviceDisconnect(_)
        | ConfigWrite::VariableWrite { .. }
        | ConfigWrite::SharedEnable
        | ConfigWrite::SharedDisable
        | ConfigWrite::SharedRemove(_) => unreachable!("handled above"),
    }
    values
}

/// The host's virtual networks, or one network's port forwards.
pub async fn host_read(vm_id: &str, action: &HostRead) -> Result<String, ShellError> {
    let vm_id = checked_id(vm_id, "host")?;
    let vmrun = Vmrun::locate()?;
    resolve(&vmrun, &vm_id, "host").await?;
    match action {
        HostRead::Networks => {
            vmrun
                .run_within(OPERATION_TIMEOUT, "host", &["-T", "ws", "listHostNetworks"])
                .await
        }
        HostRead::PortForwards(network) => {
            vmrun
                .run_within(OPERATION_TIMEOUT, "host", &["-T", "ws", "listPortForwardings", network])
                .await
        }
    }
}

/// One port-forward change on the host.
pub async fn host_write(vm_id: &str, action: &HostWrite) -> Result<String, ShellError> {
    let vm_id = checked_id(vm_id, "host")?;
    let vmrun = Vmrun::locate()?;
    resolve(&vmrun, &vm_id, "host").await?;
    match action {
        HostWrite::SetPortForward {
            network,
            protocol,
            host_port,
            guest_ip,
            guest_port,
            description,
        } => {
            let mut values = vec![
                "-T".to_owned(),
                "ws".to_owned(),
                "setPortForwarding".to_owned(),
                network.clone(),
                protocol.clone(),
                host_port.to_string(),
                guest_ip.clone(),
                guest_port.to_string(),
            ];
            if !description.is_empty() {
                values.push(description.clone());
            }
            let refs = args(&values);
            vmrun.run_within(OPERATION_TIMEOUT, "host", &refs).await
        }
        HostWrite::DeletePortForward {
            network,
            protocol,
            host_port,
        } => {
            vmrun
                .run_within(
                    OPERATION_TIMEOUT,
                    "host",
                    &[
                        "-T",
                        "ws",
                        "deletePortForwarding",
                        network,
                        protocol,
                        &host_port.to_string(),
                    ],
                )
                .await
        }
    }
}

/// Deletes or upgrades one machine.
pub async fn image(vm_id: &str, action: ImageAction) -> Result<String, ShellError> {
    let vm_id = checked_id(vm_id, action.as_str())?;
    let vmrun = Vmrun::locate()?;
    let (path, _) = resolve(&vmrun, &vm_id, action.as_str()).await?;
    let command = match action {
        ImageAction::Delete => "deleteVM",
        ImageAction::Upgrade => "upgradevm",
    };
    vmrun
        .run_within(OPERATION_TIMEOUT, action.as_str(), &["-T", "ws", command, &path])
        .await
}

/// The guest's environment, through `vmcli`, which is the only command that prints it.
pub async fn guest_env(vm_id: &str, auth: &GuestAuth) -> Result<String, ShellError> {
    let vm_id = checked_id(vm_id, "env")?;
    let vmrun = Vmrun::locate()?;
    let (path, _) = resolve(&vmrun, &vm_id, "env").await?;
    let vmcli = vmcli_beside(&vmrun)?;
    let args = [
        path.as_str(),
        "Guest",
        "env",
        "-u",
        auth.user.as_str(),
        "-P",
        auth.password.as_str(),
    ];
    vmcli.run("env", &args, OPERATION_TIMEOUT).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ethernet_change_carries_the_flags_vmcli_documents() {
        let values = config_write_args(&ConfigWrite::EthernetSetConnected {
            index: 1,
            connected: false,
        });
        assert_eq!(values, ["Ethernet", "SetStartConnected", "-i", "1", "-e", "false"]);
    }

    #[test]
    fn a_disk_create_names_its_size_type_and_slot() {
        let values = config_write_args(&ConfigWrite::DiskCreate {
            size_gb: 40,
            disk_type: "scsi".to_owned(),
            adapter: 0,
            device: 1,
        });
        assert_eq!(
            values,
            ["Disk", "Create", "-s", "40", "-t", "scsi", "-a", "0", "-d", "1"]
        );
    }

    #[test]
    fn a_vprobes_load_names_the_script_it_was_handed() {
        let values = config_write_args(&ConfigWrite::VprobesLoad(r"c:\probes\io.ems".to_owned()));
        assert_eq!(values, ["VProbes", "Load", "-s", r"c:\probes\io.ems"]);
    }

    #[test]
    fn the_image_verbs_are_the_two_vmrun_documents() {
        assert_eq!(ImageAction::Delete.as_str(), "delete");
        assert_eq!(ImageAction::Upgrade.as_str(), "upgrade");
    }

    #[test]
    fn the_debug_form_of_a_guest_auth_is_not_printed_here_either() {
        let auth = GuestAuth {
            user: "z".to_owned(),
            password: "hunter2".to_owned(),
        };
        assert!(!format!("{auth:?}").contains("hunter2"));
    }
}
