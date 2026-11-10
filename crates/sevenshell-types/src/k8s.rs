//! What the interface knows about Kubernetes.
//!
//! The API objects themselves are JSON the interface already understands, so a list or a watch
//! event carries the document the server sent, not a flattened copy of it. What is typed here is
//! the envelope: what was asked, what came back, and where a stream line came from.

use serde::{Deserialize, Serialize};

/// Where the kubeconfig comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KubeconfigSource {
    /// The file `kubectl` would read on this machine, `~/.kube/config`.
    #[default]
    DefaultPath,
    /// A document imported into the vault, keyed by the session it belongs to.
    Imported,
}

/// The Kubernetes half of a saved session.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KubernetesSettings {
    #[serde(default)]
    pub source: KubeconfigSource,
    /// Empty asks for the kubeconfig's current context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// The namespace a list opens in. Empty means all namespaces.
    #[serde(default)]
    pub namespace: String,
    /// A saved SSH session the API server is reached through, when it is not reachable directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_ssh: Option<String>,
}

impl KubernetesSettings {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// One context as the interface sees it. No credential and no local path is in this type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KubeContextInfo {
    pub name: String,
    pub cluster: String,
    pub user: String,
    pub namespace: String,
    pub current: bool,
}

/// The method a generic resource request uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum K8sMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl K8sMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }

    /// Whether the method changes something and therefore needs a policy verdict before it runs.
    pub const fn is_write(self) -> bool {
        !matches!(self, Self::Get)
    }
}

/// A request against the API server.
///
/// `acknowledged` is the person's answer to the policy verdict for a write. A read ignores it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct K8sRequest {
    pub method: K8sMethod,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default)]
    pub acknowledged: bool,
}

/// What the API server answered, status and body exactly as they came.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct K8sResponse {
    pub status: u16,
    pub body: String,
}

/// One line of a log stream the backend is following.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct K8sStreamLine {
    pub stream_id: String,
    pub line: String,
}

/// The last event of a log stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct K8sStreamEnd {
    pub stream_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// One watch event. `object` is the API object exactly as the server sent it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct K8sWatchEvent {
    pub stream_id: String,
    /// `ADDED`, `MODIFIED`, `DELETED`, `BOOKMARK` or `ERROR`, as the API server spelled it.
    pub kind: String,
    pub object: String,
}

/// A pod exec request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct K8sExecRequest {
    pub namespace: String,
    pub pod: String,
    pub container: String,
    /// The command line run inside the container. Empty asks for a shell.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
}

/// A pod log request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct K8sLogRequest {
    pub namespace: String,
    pub pod: String,
    pub container: String,
    #[serde(default)]
    pub tail_lines: u32,
    #[serde(default)]
    pub timestamps: bool,
    #[serde(default)]
    pub previous: bool,
}
