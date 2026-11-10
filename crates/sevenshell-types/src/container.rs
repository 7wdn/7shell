//! What the interface knows about containers, container images and the runtime behind them.
//!
//! Every field here is either an identifier the runtime assigned or something the CLI printed.
//! Nothing in this module carries a credential, and nothing carries a local path: an image archive
//! or a kubeconfig is chosen in a native dialog and stays on the Rust side.

use serde::{Deserialize, Serialize};

use crate::session::SecretInput;

/// Which command line speaks to the container runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContainerRuntime {
    #[default]
    Docker,
    Podman,
    Nerdctl,
    Wslc,
}

impl ContainerRuntime {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
            Self::Nerdctl => "nerdctl",
            Self::Wslc => "wslc",
        }
    }

    /// The runtime's own command name. Kept beside `as_str` so a label can never become a program.
    pub const fn program(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
            Self::Nerdctl => "nerdctl",
            Self::Wslc => "wslc",
        }
    }

    pub const fn is_windows_only(self) -> bool {
        matches!(self, Self::Wslc)
    }
}

/// Where the runtime is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContainerTransport {
    /// A process on this machine.
    #[default]
    Local,
    /// Through a saved SSH session, which supplies the host, the credentials and any jump route.
    Ssh,
}

/// The container half of a saved session.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerSettings {
    #[serde(default)]
    pub runtime: ContainerRuntime,
    #[serde(default)]
    pub transport: ContainerTransport,
    /// The saved SSH session used when `transport` is `ssh`. A reference, never a copy.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ssh_session: String,
    /// The nerdctl namespace. Empty and `default` mean the same thing.
    #[serde(default)]
    pub namespace: String,
}

impl ContainerSettings {
    /// Whether this session is not a container one, which keeps the field out of every other
    /// session's document.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// One container as a list row shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerSummary {
    pub id: String,
    pub name: String,
    pub image: String,
    /// The runtime's own state word (`running`, `exited`, `paused`, …), shown as printed.
    pub state: String,
    /// The human sentence the runtime prints next to the state.
    pub status: String,
    pub ports: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerPort {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub host_ip: String,
    pub host_port: String,
    pub container_port: String,
    pub protocol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerMount {
    pub source: String,
    pub destination: String,
    pub read_only: bool,
}

/// One container in full, plus the runtime's own JSON for the inspection tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerDetail {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub started_at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub finished_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(default)]
    pub oom_killed: bool,
    #[serde(default)]
    pub pid: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub restart_policy: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub entrypoint: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub work_dir: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub user: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub network_mode: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ip: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub gateway: String,
    #[serde(default)]
    pub ports: Vec<ContainerPort>,
    #[serde(default)]
    pub mounts: Vec<ContainerMount>,
    #[serde(default)]
    pub env: Vec<String>,
    /// The runtime's inspect output, exactly as it printed it.
    pub raw: String,
}

/// One image as a list row shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerImage {
    pub id: String,
    pub repository: String,
    pub tag: String,
    pub size: String,
    pub created_at: String,
}

/// One layer of an image's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageLayer {
    pub id: String,
    pub created_by: String,
    pub size: String,
    pub created_at: String,
}

/// A point-in-time sample from `stats`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerStats {
    pub id: String,
    pub name: String,
    pub cpu_percent: f64,
    pub memory_usage: u64,
    pub memory_limit: u64,
    pub memory_percent: f64,
    pub network_input: u64,
    pub network_output: u64,
    pub block_input: u64,
    pub block_output: u64,
    pub pids: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerStateCount {
    pub state: String,
    pub count: u32,
}

/// What the runtime says about itself and its host, for the overview page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerRuntimeInfo {
    pub runtime: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub client_version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub client_component: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub server_version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub server_component: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub storage_driver: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cgroup_driver: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cgroup_version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub os: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub arch: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kernel: String,
    #[serde(default)]
    pub cpus: u32,
    #[serde(default)]
    pub memory: u64,
    #[serde(default)]
    pub containers_total: u32,
    #[serde(default)]
    pub containers_by_state: Vec<ContainerStateCount>,
    #[serde(default)]
    pub images_total: u32,
    #[serde(default)]
    pub images_tagged: u32,
    #[serde(default)]
    pub images_dangling: u32,
}

/// One line of a stream the backend is following, tagged with the stream it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerStreamLine {
    pub stream_id: String,
    pub line: String,
}

/// The last event of a stream. `error` is present when it ended because something failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerStreamEnd {
    pub stream_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A port mapping asked for when a container is created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerPortMapping {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub host_ip: String,
    pub host_port: u16,
    pub container_port: u16,
    #[serde(default)]
    pub protocol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerVolume {
    pub source: String,
    pub destination: String,
}

/// Everything the create dialog collects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerCreateRequest {
    pub image: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default)]
    pub ports: Vec<ContainerPortMapping>,
    #[serde(default)]
    pub volumes: Vec<ContainerVolume>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub restart_policy: String,
    /// The command as one line; the backend splits it into argv without a shell.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    #[serde(default)]
    pub interactive: bool,
    #[serde(default)]
    pub tty: bool,
}

/// A pull or a push.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerImageTransfer {
    pub image: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub platform: String,
    #[serde(default)]
    pub all_tags: bool,
    #[serde(default)]
    pub insecure: bool,
}

/// A registry login. The password is the one secret that travels in this module, and only in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerRegistryLogin {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub registry: String,
    pub username: String,
    pub secret: SecretInput,
    #[serde(default)]
    pub insecure: bool,
}

/// A commit of a container's filesystem into an image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerCommitRequest {
    pub container_id: String,
    pub image: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
}
