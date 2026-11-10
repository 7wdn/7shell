//! Cluster connections: the order in which the vault, the kubeconfig and the client are used.
//!
//! Three rules live here rather than in the command layer, because each of them is an invariant the
//! project states somewhere else:
//!
//! 1. **The kubeconfig is read on this side of the wire.** An imported one comes out of the vault
//!    keyed by the session it belongs to; the default one comes off this disk. What the interface
//!    is ever told is the list of contexts.
//! 2. **A write is decided before it is sent.** The policy verdict is made here, before the
//!    connection is even looked up, so a destructive request without an acknowledgement fails the
//!    same way whether or not there is a cluster to send it to.
//! 3. **Disconnecting stops the streams.** A watch outlives no connection: the socket would keep
//!    reporting on a tab that no longer exists.

use sevenshell_error::ShellError;
use sevenshell_k8s::Kubeconfig;
use sevenshell_session::{ForwardedControl, ForwardedSession};
use sevenshell_types::{
    K8sExecRequest, K8sLogRequest, K8sRequest, K8sResponse, K8sStreamEnd, K8sStreamLine, K8sWatchEvent,
    KubeContextInfo, KubeconfigSource, Protocol, SecretInput,
};
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::Core;

impl Core {
    /// The contexts a kubeconfig offers, without any credential in them.
    ///
    /// `None` reads the file `kubectl` would read on this machine; `Some` parses a document that was
    /// imported — one being chosen in a native dialog, or one stored beside a session — and has no
    /// directory of its own.
    pub async fn k8s_contexts(&self, kubeconfig: Option<SecretInput>) -> Result<Vec<KubeContextInfo>, ShellError> {
        let config = match kubeconfig {
            Some(input) => Kubeconfig::parse(input.value.as_bytes())?,
            None => Kubeconfig::load_default()?,
        };

        Ok(config
            .contexts()
            .iter()
            .map(|context| KubeContextInfo {
                name: context.name.clone(),
                cluster: context.cluster.clone(),
                user: context.user.clone(),
                namespace: context.namespace.clone(),
                current: context.current,
            })
            .collect())
    }

    /// Opens the cluster a saved session names, and answers the id the interface keeps.
    pub async fn k8s_connect(&self, session_id: &str) -> Result<String, ShellError> {
        let session = self.session(session_id)?;
        if session.protocol != Protocol::Kubernetes {
            return Err(ShellError::app_unsupported(
                "a cluster connection was asked for a session that is not a Kubernetes session",
            ));
        }

        // Reaching an API server through an SSH connection is the shape a jump host has for a
        // terminal: it is a later feature, and saying so is better than connecting somewhere else
        // and reporting a timeout.
        if let Some(via) = session.kubernetes.via_ssh.as_deref().filter(|via| !via.is_empty()) {
            return Err(ShellError::app_unsupported(&format!(
                "reaching an API server through the SSH session `{via}`"
            )));
        }

        let config = match session.kubernetes.source {
            KubeconfigSource::DefaultPath => Kubeconfig::load_default()?,
            // The namespace a list opens in belongs to the interface, not to the connection: the
            // context's own namespace is what the API says, and every request names its namespace
            // in its path anyway.
            KubeconfigSource::Imported => self.imported_kubeconfig(session_id)?,
        };

        self.clusters
            .connect(&config, session.kubernetes.context.as_deref())
            .await
    }

    /// Drops a connection and every stream it was feeding.
    pub fn k8s_disconnect(&self, connection_id: &str) {
        self.clusters.disconnect(connection_id);
    }

    /// One request against a cluster, after the policy verdict for it.
    pub async fn k8s_request(&self, connection_id: &str, request: K8sRequest) -> Result<K8sResponse, ShellError> {
        // The gate comes first, and it comes before the connection is looked up: an unacknowledged
        // delete must be refused with the reason it is refused, not with "no such connection".
        let verdict = sevenshell_policy::kubernetes_request(request.method.as_str(), &request.path);
        if verdict.needs_approval() && !request.acknowledged {
            let rule = verdict
                .reasons
                .first()
                .map_or("k8s.write", |reason| reason.rule_id.as_str());
            return Err(ShellError::policy_approval_required(rule));
        }

        self.clusters
            .request(
                connection_id,
                request.method.as_str(),
                &request.path,
                request.body.as_deref().map(str::as_bytes),
                request.content_type.as_deref(),
            )
            .await
    }

    /// Follows one list, sending every event to `events` and the end to `end`.
    ///
    /// The two channels are the two frozen event names, and keeping them apart is what lets the
    /// host emit `k8s:watch` for one and `k8s:stream-end` for the other without inspecting what
    /// came through.
    pub fn k8s_watch_start(
        &self,
        connection_id: &str,
        path: &str,
        events: UnboundedSender<K8sWatchEvent>,
        end: UnboundedSender<K8sStreamEnd>,
    ) -> Result<String, ShellError> {
        self.clusters.watch_start(connection_id, path, events, end)
    }

    /// Stops one stream. A stream that already ended is not an error — the interface may be a
    /// moment late, and being late is not a failure.
    pub fn k8s_stream_stop(&self, stream_id: &str) {
        self.clusters.stop_stream(stream_id);
    }

    /// Follows one pod's log, sending every line to `events` and the end to `end`.
    pub async fn k8s_logs_start(
        &self,
        connection_id: &str,
        request: &K8sLogRequest,
        events: UnboundedSender<K8sStreamLine>,
        end: UnboundedSender<K8sStreamEnd>,
    ) -> Result<String, ShellError> {
        self.clusters.logs_start(connection_id, request, events, end).await
    }

    /// Opens a pod exec channel and registers it as an ordinary terminal.
    pub async fn k8s_exec_open(
        &self,
        connection_id: &str,
        request: &K8sExecRequest,
        columns: u32,
        rows: u32,
    ) -> Result<String, ShellError> {
        let session = self.clusters.exec_open(connection_id, request, columns, rows).await?;
        let output = session
            .take_output()
            .ok_or_else(|| ShellError::app_internal("a pod terminal was registered twice"))?;

        let session = std::sync::Arc::new(session);
        let (control, mut requests) = mpsc::channel(32);
        tokio::spawn(async move {
            while let Some(request) = requests.recv().await {
                let applied = match request {
                    ForwardedControl::Write(bytes) => session.write(&bytes),
                    ForwardedControl::Resize { columns, rows } => session.resize(columns, rows),
                    ForwardedControl::Close => break,
                };
                if applied.is_err() {
                    break;
                }
            }
            session.close();
        });

        self.registry
            .open_forwarded(connection_id, ForwardedSession::new(output, control), None)
            .await
    }

    /// The kubeconfig stored beside a session.
    fn imported_kubeconfig(&self, session_id: &str) -> Result<Kubeconfig, ShellError> {
        let secret = self.vault.secret_for_session(session_id)?.ok_or_else(|| {
            ShellError::k8s_kubeconfig_invalid(format!("the session `{session_id}` has no stored kubeconfig"))
        })?;

        // A Kubernetes session's secret *is* its kubeconfig. `into_value` takes it whichever kind
        // the vault labelled it with — the session's protocol already says what the value is — and
        // the text is dropped as soon as it has been parsed, leaving only what the connection needs.
        let document = secret.into_value();
        Kubeconfig::parse(document.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sevenshell_types::{K8sMethod, RiskLevel};
    use sevenshell_vault::Vault;

    use crate::Core;

    /// A core with a vault that has never been unlocked.
    ///
    /// Enough for the gate: it is decided before anything is read out of the vault or off the
    /// network, which is exactly what these tests are about.
    fn core() -> Arc<Core> {
        let path = std::env::temp_dir().join(format!("7shell-k8s-core-{}.vault", uuid::Uuid::new_v4()));
        let (core, _events) = Core::new(Arc::new(Vault::open_at(path)));
        core
    }

    fn request(method: K8sMethod, path: &str, acknowledged: bool) -> sevenshell_types::K8sRequest {
        sevenshell_types::K8sRequest {
            method,
            path: path.to_owned(),
            body: None,
            content_type: None,
            acknowledged,
        }
    }

    #[tokio::test]
    async fn a_delete_without_an_acknowledgement_is_refused_before_the_connection_is_looked_at() {
        let core = core();
        let error = core
            .k8s_request(
                "no-such-connection",
                request(K8sMethod::Delete, "/api/v1/namespaces/prod/pods/web", false),
            )
            .await
            .expect_err("an unacknowledged delete is refused");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::PolicyApprovalRequired);
        assert!(matches!(
            sevenshell_policy::kubernetes_request("DELETE", "/api/v1/namespaces/prod/pods/web").level,
            RiskLevel::Destructive
        ));
    }

    #[tokio::test]
    async fn an_acknowledged_delete_reaches_the_connection_and_finds_it_missing() {
        let core = core();
        let error = core
            .k8s_request(
                "no-such-connection",
                request(K8sMethod::Delete, "/api/v1/namespaces/prod/pods/web", true),
            )
            .await
            .expect_err("there is no such connection");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::K8sNotConnected);
    }

    #[tokio::test]
    async fn a_read_never_asks_for_approval() {
        let core = core();
        let error = core
            .k8s_request("no-such-connection", request(K8sMethod::Get, "/api/v1/pods", false))
            .await
            .expect_err("there is no such connection");

        // A read gets to the connection and fails there: the gate did not stand in its way.
        assert_eq!(error.code(), sevenshell_error::ErrorCode::K8sNotConnected);
    }
}
