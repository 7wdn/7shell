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
#[allow(dead_code)]
pub struct K8sClient {
    pub(crate) transport: K8sTransport,
}

/// Everything a request needs that is not the path or the body.
///
/// The REST client and the WebSocket upgrade share this: one TLS story, one authorization header
/// and one base URL, so an exec channel cannot accidentally trust a different server than a list.
#[allow(dead_code)]
#[derive(Clone)]
pub(crate) struct K8sTransport {
    /// The server URL exactly as the kubeconfig spells it, without a trailing slash.
    pub(crate) base: String,
    /// The host name the certificate is checked against and the `Host` header carries.
    pub(crate) server: String,
    pub(crate) tls: native_tls::TlsConnector,
    /// `Bearer …` when the kubeconfig uses a token, otherwise nothing (client certificates).
    pub(crate) authorization: Option<String>,
}

impl K8sClient {
    pub async fn connect(_config: &Kubeconfig, _context: Option<&str>) -> Result<Self, ShellError> {
        Err(ShellError::app_unsupported(
            "the Kubernetes client is not implemented yet",
        ))
    }

    /// The parts a WebSocket upgrade needs, which are the same parts every request uses.
    #[allow(dead_code)]
    pub(crate) fn transport(&self) -> &K8sTransport {
        &self.transport
    }

    /// One API request. A non-2xx answer is returned as a response, not an error: the caller decides
    /// what a 404 or a 409 means, and the status is part of the answer.
    #[allow(dead_code)]
    pub(crate) async fn request(
        &self,
        _method: &str,
        _path: &str,
        _body: Option<&[u8]>,
        _content_type: Option<&str>,
    ) -> Result<sevenshell_types::K8sResponse, ShellError> {
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
