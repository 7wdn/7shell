//! Kubeconfig and the client built from it.

use sevenshell_error::ShellError;

/// One context as the interface sees it.
///
/// Secrets are not in this type: a context names a cluster and a user, and the credential behind
/// either stays on this side of the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KubeContext {
    pub name: String,
    pub cluster: String,
    pub user: String,
    pub namespace: String,
    pub current: bool,
}

/// A parsed kubeconfig.
pub struct Kubeconfig {
    contexts: Vec<KubeContext>,
}

impl Kubeconfig {
    /// Parses the YAML document a person imported or that lives at the default location.
    pub fn parse(_bytes: &[u8]) -> Result<Self, ShellError> {
        Err(ShellError::app_unsupported("kubeconfig parsing is not implemented yet"))
    }

    pub fn contexts(&self) -> &[KubeContext] {
        &self.contexts
    }
}

/// A connection to one cluster.
pub struct K8sClient;

impl K8sClient {
    pub async fn connect(_config: &Kubeconfig, _context: Option<&str>) -> Result<Self, ShellError> {
        Err(ShellError::app_unsupported(
            "the Kubernetes client is not implemented yet",
        ))
    }
}

/// The cluster connections a window holds open.
pub struct K8sManager;

impl K8sManager {
    pub fn new() -> Self {
        Self
    }
}

impl Default for K8sManager {
    fn default() -> Self {
        Self::new()
    }
}
