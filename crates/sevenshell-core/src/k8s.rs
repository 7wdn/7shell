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

use std::sync::Arc;

use sevenshell_error::ShellError;
use sevenshell_k8s::{ApiDialer, BoxedApiStream, DialFuture, Kubeconfig, TunneledCluster};
use sevenshell_policy::PolicyTarget;
use sevenshell_session::{ForwardedControl, ForwardedSession};
use sevenshell_ssh::SshSession;
use sevenshell_types::{
    K8sExecRequest, K8sLogRequest, K8sRequest, K8sResponse, K8sStreamEnd, K8sStreamLine, K8sWatchEvent,
    KubeContextInfo, KubeconfigSource, PolicyAction, Protocol, RiskLevel, SecretInput,
};
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::Core;
use crate::sessions::{ConnectionOutcome, HostKeyDecision};

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

        let config = match session.kubernetes.source {
            KubeconfigSource::DefaultPath => Kubeconfig::load_default()?,
            // The namespace a list opens in belongs to the interface, not to the connection: the
            // context's own namespace is what the API says, and every request names its namespace
            // in its path anyway.
            KubeconfigSource::Imported => self.imported_kubeconfig(session_id)?,
        };
        let context = session.kubernetes.context.as_deref();

        let via = session
            .kubernetes
            .via_ssh
            .as_deref()
            .map(str::trim)
            .filter(|via| !via.is_empty());
        let Some(via) = via else {
            return self.clusters.connect(&config, context).await;
        };

        // Read and checked before the SSH session is opened: a cluster this build cannot carry
        // through a tunnel must not cost a connection to a server, or a host-key question.
        let prepared = TunneledCluster::prepare(&config, context)?;

        let transport = self.session(via)?;
        if transport.protocol != Protocol::Ssh {
            return Err(ShellError::app_unsupported(
                "the session a cluster is reached through is not an SSH session",
            ));
        }
        let connection = match self.open_carrier(&transport).await? {
            ConnectionOutcome::Ready(connection) => connection,
            // The cluster workspace has no host-key prompt of its own, so the decision is refused
            // with the host's identity — the same answer the container workspace gives. Opening
            // that SSH session once is how the decision is made, and every later cluster
            // connection reuses the trust.
            ConnectionOutcome::Decision(decision) => return Err(host_key_error(*decision)),
        };

        self.clusters
            .connect_via(prepared, Arc::new(SshDialer { session: connection }))
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
        // A read is not something the matrix has an opinion about — listings and reads stay out of
        // it — so a `GET` reaches the cluster the way it always did. Everything that changes
        // something is decided here, before the connection is looked up.
        if verdict.level != RiskLevel::Safe {
            // The global rule only: a cluster connection carries no saved session id here, and
            // naming a group or host layer an audit line never consulted would be worse than
            // saying "global".
            let operation = format!("k8s.{}", request.method.as_str().to_ascii_lowercase());
            let decision =
                self.policy_decision(&operation, &verdict, PolicyTarget::default(), None, Some(&request.path));
            match decision.action {
                PolicyAction::Deny => return Err(ShellError::policy_denied(&operation)),
                PolicyAction::Ask if !request.acknowledged => {
                    let rule = verdict
                        .reasons
                        .first()
                        .map_or("k8s.write", |reason| reason.rule_id.as_str());
                    return Err(ShellError::policy_approval_required(rule));
                }
                _ => {}
            }
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

/// Carries the client's connections over one SSH session: one direct-tcpip channel each.
///
/// The session is an `Arc` because the client holds the dialer for as long as the connection: a
/// cluster tab with nothing else on that server is the reason the connection is open at all.
struct SshDialer {
    session: Arc<SshSession>,
}

impl ApiDialer for SshDialer {
    fn dial(&self, host: String, port: u16) -> DialFuture {
        let session = Arc::clone(&self.session);
        Box::pin(async move {
            // The primitive a jump host and a local forward are both built from: a channel to
            // `host:port` as the far side sees it, handed over as a byte stream.
            let stream = session.open_tunnel_stream(&host, port).await?;
            let stream: BoxedApiStream = Box::new(stream);
            Ok(stream)
        })
    }
}

/// The decision a tunnel cannot make on its own, shaped like the container workspace shapes it.
fn host_key_error(decision: HostKeyDecision) -> ShellError {
    match decision {
        HostKeyDecision::Unknown(key) => ShellError::ssh_host_key_unknown(&key.host, key.port, &key.key_type),
        HostKeyDecision::Mismatch { trusted, .. } => {
            ShellError::ssh_host_key_mismatch(&trusted.host, trusted.port, &trusted.key_type)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use sevenshell_policy::{PolicyAuditEvent, PolicyJournal};
    use sevenshell_types::{
        AuthMethod, K8sMethod, KubeconfigSource, KubernetesSettings, PolicyAction, PolicyMatrix, PolicyRule, Protocol,
        RiskLevel, Session, SessionAuth, TerminalSettings,
    };
    use sevenshell_vault::{SessionSecret, Vault};
    use tempfile::{TempDir, tempdir};

    use crate::Core;

    #[derive(Default)]
    struct RecordingJournal {
        events: Mutex<Vec<PolicyAuditEvent>>,
    }

    impl RecordingJournal {
        fn events(&self) -> Vec<PolicyAuditEvent> {
            self.events.lock().expect("not poisoned").clone()
        }
    }

    impl PolicyJournal for RecordingJournal {
        fn record(&self, event: PolicyAuditEvent) {
            self.events.lock().expect("not poisoned").push(event);
        }
    }

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

    /// A vault that has been created and can be filled with sessions.
    fn vault() -> (TempDir, Arc<Vault>) {
        let directory = tempdir().expect("temp dir");
        let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
        vault.create("correct horse battery staple").expect("create");
        (directory, vault)
    }

    fn session(id: &str, protocol: Protocol) -> Session {
        Session {
            id: id.to_owned(),
            name: id.to_owned(),
            group_id: None,
            protocol,
            host: "127.0.0.1".to_owned(),
            // Port 1 is where a connect to this machine fails at once: a test that gets past the
            // local checks would report an unreachable server instead of the refusal it is about.
            port: 1,
            user: "someone".to_owned(),
            auth: SessionAuth::with_password(AuthMethod::Password),
            terminal: TerminalSettings::default(),
            desktop: sevenshell_types::DesktopSettings::default(),
            telnet: sevenshell_types::TelnetSettings::default(),
            serial: sevenshell_types::SerialSettings::default(),
            local: sevenshell_types::LocalSettings::default(),
            wsl: sevenshell_types::WslSettings::default(),
            container: sevenshell_types::ContainerSettings::default(),
            kubernetes: KubernetesSettings::default(),
            webdav: sevenshell_types::WebDavSettings::default(),
            ftp: sevenshell_types::FtpSettings::default(),
            jump: Vec::new(),
            forwards: Vec::new(),
            updated_at: 0,
        }
    }

    /// An SSH session, saved with a password when one is given.
    fn save_ssh(vault: &Vault, id: &str, password: Option<&str>) {
        vault
            .save_session(
                session(id, Protocol::Ssh),
                password.map(|password| SessionSecret::Password(password.to_owned())),
            )
            .expect("save the ssh session");
    }

    /// A cluster session whose kubeconfig points at `server`, reached through `via` when set.
    fn save_cluster(vault: &Vault, server: &str, via: Option<&str>) {
        let mut cluster = session("cluster", Protocol::Kubernetes);
        cluster.kubernetes = KubernetesSettings {
            source: KubeconfigSource::Imported,
            context: None,
            namespace: String::new(),
            via_ssh: via.map(str::to_owned),
        };
        vault
            .save_session(cluster, Some(SessionSecret::Kubeconfig(cluster_document(server))))
            .expect("save the cluster session");
    }

    /// A one-context kubeconfig. The certificate story is deliberately the insecure one: what
    /// these tests ask is where a connection goes, not what it trusts, and a real authority would
    /// be a fixture with no bearing on the answer.
    fn cluster_document(server: &str) -> String {
        format!(
            r#"apiVersion: v1
kind: Config
current-context: test
clusters:
  - name: test
    cluster:
      server: {server}
      insecure-skip-tls-verify: true
contexts:
  - name: test
    context:
      cluster: test
      user: test
users:
  - name: test
    user:
      token: not-a-real-token
"#
        )
    }

    #[tokio::test]
    async fn a_denied_request_never_reaches_the_cluster_even_when_acknowledged() {
        let core = core();
        let journal = Arc::new(RecordingJournal::default());
        core.set_policy(
            PolicyMatrix {
                global: PolicyRule {
                    safe: PolicyAction::Deny,
                    caution: PolicyAction::Deny,
                    destructive: PolicyAction::Deny,
                    catastrophic: PolicyAction::Deny,
                },
                ..PolicyMatrix::default()
            },
            Some(Arc::clone(&journal) as Arc<dyn PolicyJournal>),
        );

        // Acknowledged on purpose: the acknowledgement answers a question, and a deny is not one.
        // The connection does not exist either, so a request that got past the gate would say
        // "not connected" instead of naming the policy.
        let error = core
            .k8s_request(
                "no-such-connection",
                request(K8sMethod::Delete, "/api/v1/namespaces/prod/pods/web", true),
            )
            .await
            .expect_err("a deny refuses the request");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::PolicyDenied);
        let events = journal.events();
        assert_eq!(events.len(), 1, "one request, one line");
        assert_eq!(events[0].action, "k8s.delete");
        assert_eq!(events[0].decision, PolicyAction::Deny);
        assert_eq!(
            events[0].subject.as_deref(),
            Some("/api/v1/namespaces/prod/pods/web"),
            "the line names the path the decision was about"
        );
    }

    #[tokio::test]
    async fn a_read_is_not_something_the_matrix_decides() {
        let core = core();
        let journal = Arc::new(RecordingJournal::default());
        core.set_policy(
            PolicyMatrix {
                global: PolicyRule {
                    safe: PolicyAction::Deny,
                    caution: PolicyAction::Deny,
                    destructive: PolicyAction::Deny,
                    catastrophic: PolicyAction::Deny,
                },
                ..PolicyMatrix::default()
            },
            Some(Arc::clone(&journal) as Arc<dyn PolicyJournal>),
        );

        // Reads and listings are deliberately outside the matrix, so a rule about safe actions does
        // not close cluster browsing. The read gets to the connection lookup and fails there.
        let error = core
            .k8s_request(
                "no-such-connection",
                request(K8sMethod::Get, "/api/v1/namespaces/default/pods", false),
            )
            .await
            .expect_err("there is no such connection");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::K8sNotConnected);
        assert!(journal.events().is_empty(), "a read leaves no policy line");
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

    #[tokio::test]
    async fn a_cluster_address_that_is_not_https_is_refused_before_the_tunnel_is_opened() {
        let (_directory, vault) = vault();
        save_ssh(&vault, "gateway", Some("hunter2"));
        save_cluster(&vault, "http://127.0.0.1:1", Some("gateway"));
        let (core, _events) = Core::new(vault);

        // The SSH session is one that would fail with the network's own error if it were opened;
        // the address refusal proves the kubeconfig was read and judged before any socket was.
        let error = core
            .k8s_connect("cluster")
            .await
            .expect_err("a plain-HTTP server cannot be carried through the tunnel");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::AppUnsupported);
        assert_eq!(
            error.message_params().get("what").map(String::as_str),
            Some("a cluster reached through an SSH session has to be an https address")
        );
    }

    #[tokio::test]
    async fn a_tunnel_session_that_is_not_saved_is_named_in_the_failure() {
        let (_directory, vault) = vault();
        save_cluster(&vault, "https://127.0.0.1:1", Some("gateway-that-never-existed"));
        let (core, _events) = Core::new(vault);

        let error = core.k8s_connect("cluster").await.expect_err("the tunnel is gone");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SessionNotFound);
        assert_eq!(
            error.diagnostics().get("sessionId").map(String::as_str),
            Some("gateway-that-never-existed")
        );
    }

    #[tokio::test]
    async fn a_tunnel_session_that_is_not_an_ssh_session_is_refused_by_name() {
        let (_directory, vault) = vault();
        // A second cluster: it exists, so the failure is about what it is, not about it being gone.
        vault
            .save_session(session("another-cluster", Protocol::Kubernetes), None)
            .expect("save the other cluster");
        save_cluster(&vault, "https://127.0.0.1:1", Some("another-cluster"));
        let (core, _events) = Core::new(vault);

        let error = core
            .k8s_connect("cluster")
            .await
            .expect_err("a cluster is not a carrier");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::AppUnsupported);
        assert_eq!(
            error.message_params().get("what").map(String::as_str),
            Some("the session a cluster is reached through is not an SSH session")
        );
    }

    #[tokio::test]
    async fn a_tunnel_session_without_a_stored_password_fails_before_any_network_use() {
        let (_directory, vault) = vault();
        save_ssh(&vault, "gateway", None);
        save_cluster(&vault, "https://127.0.0.1:1", Some("gateway"));
        let (core, _events) = Core::new(vault);

        // The credential is resolved before the socket is dialled, which is what turns "the tunnel
        // was rejected as unsupported" into "the tunnel ran and asked for a password".
        let error = core.k8s_connect("cluster").await.expect_err("no password stored");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SshAuthFailed);
        assert_eq!(error.detail(), Some("no password is stored for this session"));
    }

    #[tokio::test]
    async fn an_empty_tunnel_setting_connects_directly() {
        let (_directory, vault) = vault();
        save_cluster(&vault, "https://127.0.0.1:1", Some(""));
        let (core, _events) = Core::new(vault);

        // A cleared picker is not a tunnel: the connection is built the way it always was — no
        // session resolved, no listener, and the address is only dialled by the first request.
        let connection = core.k8s_connect("cluster").await.expect("a direct connection");
        let error = core
            .k8s_request(&connection, request(K8sMethod::Get, "/api/v1/pods", false))
            .await
            .expect_err("nothing listens on port 1");
        assert_eq!(error.code(), sevenshell_error::ErrorCode::K8sUnavailable);
    }
}
