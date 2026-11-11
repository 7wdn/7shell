//! Container connections: the order in which the vault, the SSH route and the runtime are used.
//!
//! A container session names a runtime and where it runs. The first decision is the transport:
//! a process on this machine needs nothing but a `PATH`, while an SSH host needs the saved session
//! it references opened with the same route, probe and host-key verification a terminal gets. Only
//! then is a provider registered under an id the interface keeps.
//!
//! Destructive operations are decided here, before the connection is looked up, so an
//! unacknowledged stop or removal fails the same way whether or not a runtime is reachable.

use std::path::Path;
use std::sync::Arc;

use sevenshell_container::{LineStream, Provider, Runner};
use sevenshell_error::ShellError;
use sevenshell_session::{ForwardedControl, ForwardedSession};
use sevenshell_types::{
    ContainerCommitRequest, ContainerCreateRequest, ContainerDetail, ContainerImage, ContainerImageTransfer,
    ContainerRegistryLogin, ContainerRuntime, ContainerRuntimeInfo, ContainerStats, ContainerStreamEnd,
    ContainerStreamLine, ContainerSummary, ContainerTransport, ImageLayer, Protocol,
};
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::Core;
use crate::sessions::{ConnectionOutcome, HostKeyDecision};

impl Core {
    /// Opens the runtime a saved session names, and answers the id the interface keeps.
    pub async fn container_connect(&self, session_id: &str) -> Result<String, ShellError> {
        let session = self.session(session_id)?;
        if session.protocol != Protocol::Container {
            return Err(ShellError::app_unsupported(
                "a container connection was asked for a session that is not a container session",
            ));
        }

        let settings = &session.container;
        let runner = match settings.transport {
            ContainerTransport::Local => Runner::Local,
            ContainerTransport::Ssh => {
                let referenced = settings.ssh_session.trim();
                if referenced.is_empty() {
                    return Err(ShellError::app_unsupported(
                        "this container session has no SSH session to reach the runtime through",
                    ));
                }
                let transport = self.session(referenced)?;
                if transport.protocol != Protocol::Ssh {
                    return Err(ShellError::app_unsupported(
                        "the session a container connection references is not an SSH session",
                    ));
                }
                match self.open_carrier(&transport).await? {
                    ConnectionOutcome::Ready(session) => Runner::Ssh(session),
                    // A container workspace has no host-key prompt of its own. Refusing with the
                    // host's identity is honest: opening that SSH session once is how the decision
                    // is made, and every later container connection reuses the trust.
                    ConnectionOutcome::Decision(decision) => return Err(decision_error(*decision)),
                }
            }
        };

        let namespace = if settings.runtime == ContainerRuntime::Nerdctl {
            let namespace = settings.namespace.trim();
            if namespace.is_empty() { "default" } else { namespace }
        } else {
            ""
        };

        let id = self.containers.new_id();
        self.containers
            .connect(id.clone(), settings.runtime, namespace, runner)?;
        Ok(id)
    }

    /// Drops a connection and every stream it was feeding.
    pub async fn container_disconnect(&self, connection_id: &str) {
        let stops: Vec<_> = {
            let mut streams = self.container_streams.lock().await;
            streams.drain().map(|(_, stop)| stop).collect()
        };
        for stop in stops {
            stop();
        }
        self.containers.remove(connection_id);
    }

    pub async fn container_list(&self, connection_id: &str) -> Result<Vec<ContainerSummary>, ShellError> {
        self.container_provider(connection_id)?.list().await
    }

    pub async fn container_inspect(
        &self,
        connection_id: &str,
        container_id: &str,
    ) -> Result<ContainerDetail, ShellError> {
        self.container_provider(connection_id)?.inspect(container_id).await
    }

    /// Runs one container action after the policy verdict for it.
    pub async fn container_action(
        &self,
        connection_id: &str,
        container_id: &str,
        action: &str,
        acknowledged: bool,
    ) -> Result<(), ShellError> {
        let verdict = sevenshell_policy::container_action(action);
        if verdict.needs_approval() && !acknowledged {
            let rule = verdict
                .reasons
                .first()
                .map_or("container.unknown", |reason| reason.rule_id.as_str());
            return Err(ShellError::policy_approval_required(rule));
        }
        self.container_provider(connection_id)?
            .action(container_id, action)
            .await
    }

    pub async fn container_rename(
        &self,
        connection_id: &str,
        container_id: &str,
        name: &str,
    ) -> Result<(), ShellError> {
        self.container_provider(connection_id)?.rename(container_id, name).await
    }

    pub async fn container_create(
        &self,
        connection_id: &str,
        request: &ContainerCreateRequest,
    ) -> Result<(), ShellError> {
        self.container_provider(connection_id)?.create(request).await
    }

    pub async fn container_stats(&self, connection_id: &str) -> Result<Vec<ContainerStats>, ShellError> {
        self.container_provider(connection_id)?.stats().await
    }

    pub async fn container_namespaces(&self, connection_id: &str) -> Result<Vec<String>, ShellError> {
        self.container_provider(connection_id)?.namespaces().await
    }

    pub fn container_set_namespace(&self, connection_id: &str, namespace: &str) -> Result<(), ShellError> {
        self.containers.set_namespace(connection_id, namespace)
    }

    pub async fn container_runtime_info(&self, connection_id: &str) -> Result<ContainerRuntimeInfo, ShellError> {
        self.container_provider(connection_id)?.runtime_info().await
    }

    pub async fn container_images(&self, connection_id: &str) -> Result<Vec<ContainerImage>, ShellError> {
        self.container_provider(connection_id)?.images().await
    }

    pub async fn container_image_inspect(&self, connection_id: &str, image: &str) -> Result<String, ShellError> {
        self.container_provider(connection_id)?.image_inspect(image).await
    }

    pub async fn container_image_history(
        &self,
        connection_id: &str,
        image: &str,
    ) -> Result<Vec<ImageLayer>, ShellError> {
        self.container_provider(connection_id)?.image_history(image).await
    }

    pub async fn container_image_remove(
        &self,
        connection_id: &str,
        image: &str,
        acknowledged: bool,
    ) -> Result<(), ShellError> {
        require_ack(
            sevenshell_policy::container_action("image-remove"),
            "container.remove",
            acknowledged,
        )?;
        self.container_provider(connection_id)?.image_remove(image).await
    }

    pub async fn container_image_prune(&self, connection_id: &str, acknowledged: bool) -> Result<(), ShellError> {
        require_ack(
            sevenshell_policy::container_action("prune"),
            "container.remove",
            acknowledged,
        )?;
        self.container_provider(connection_id)?.image_prune().await
    }

    pub async fn container_image_tag(&self, connection_id: &str, image: &str, tag: &str) -> Result<(), ShellError> {
        self.container_provider(connection_id)?.image_tag(image, tag).await
    }

    /// Loads an image archive from a file the person chose in a native dialog.
    ///
    /// The path stays on this side of the window: it is never handed to the interface, and it never
    /// becomes part of a command line — the archive's bytes reach the runtime on standard input.
    pub async fn container_image_load(&self, connection_id: &str, archive: &Path) -> Result<String, ShellError> {
        self.container_provider(connection_id)?.image_load_file(archive).await
    }

    pub async fn container_image_commit(
        &self,
        connection_id: &str,
        request: &ContainerCommitRequest,
        acknowledged: bool,
    ) -> Result<String, ShellError> {
        require_ack(
            sevenshell_policy::container_action("commit"),
            "container.change",
            acknowledged,
        )?;
        self.container_provider(connection_id)?.image_commit(request).await
    }

    pub async fn container_registry_login(
        &self,
        connection_id: &str,
        login: &ContainerRegistryLogin,
    ) -> Result<(), ShellError> {
        self.container_provider(connection_id)?.registry_login(login).await
    }

    /// Follows a container's logs, sending every line to `events` and the end to `end`.
    pub async fn container_logs_start(
        &self,
        connection_id: &str,
        container_id: &str,
        tail: u32,
        timestamps: bool,
        events: UnboundedSender<ContainerStreamLine>,
        end: UnboundedSender<ContainerStreamEnd>,
    ) -> Result<String, ShellError> {
        let stream = self
            .container_provider(connection_id)?
            .logs(container_id, tail, timestamps)
            .await?;
        Ok(self.spawn_container_stream(stream, events, end).await)
    }

    /// Starts a pull or a push, whose output is the CLI's own progress lines.
    pub async fn container_image_transfer_start(
        &self,
        connection_id: &str,
        request: &ContainerImageTransfer,
        push: bool,
        events: UnboundedSender<ContainerStreamLine>,
        end: UnboundedSender<ContainerStreamEnd>,
    ) -> Result<String, ShellError> {
        let provider = self.container_provider(connection_id)?;
        let stream = if push {
            provider.image_push(request).await?
        } else {
            provider.image_pull(request).await?
        };
        Ok(self.spawn_container_stream(stream, events, end).await)
    }

    pub async fn container_stream_stop(&self, stream_id: &str) {
        let stop = self.container_streams.lock().await.remove(stream_id);
        if let Some(stop) = stop {
            stop();
        }
    }

    /// Opens an exec terminal and registers it with the terminal registry.
    pub async fn container_exec_open(
        &self,
        connection_id: &str,
        container_id: &str,
        shell: &str,
        columns: u32,
        rows: u32,
    ) -> Result<String, ShellError> {
        let mut terminal = self
            .container_provider(connection_id)?
            .open_exec(container_id, shell, columns, rows)
            .await?;

        let output = terminal.take_output();
        let (control, mut requests) = mpsc::channel(32);
        tokio::spawn(async move {
            while let Some(request) = requests.recv().await {
                let applied = match request {
                    ForwardedControl::Write(bytes) => terminal.write(&bytes).await,
                    ForwardedControl::Resize { columns, rows } => terminal.resize(columns, rows).await,
                    ForwardedControl::Close => break,
                };
                if applied.is_err() {
                    break;
                }
            }
            let _ = terminal.close().await;
        });

        self.registry
            .open_forwarded(connection_id, ForwardedSession::new(output, control), None)
            .await
    }

    fn container_provider(&self, connection_id: &str) -> Result<Arc<Provider>, ShellError> {
        self.containers.provider(connection_id)
    }

    async fn spawn_container_stream(
        &self,
        mut stream: LineStream,
        events: UnboundedSender<ContainerStreamLine>,
        end: UnboundedSender<ContainerStreamEnd>,
    ) -> String {
        let stream_id = uuid::Uuid::new_v4().to_string();
        let registry = Arc::clone(&self.container_streams);
        registry.lock().await.insert(stream_id.clone(), stream.stopper());

        let task_id = stream_id.clone();
        tokio::spawn(async move {
            while let Some(line) = stream.next().await {
                let event = ContainerStreamLine {
                    stream_id: task_id.clone(),
                    line,
                };
                if events.send(event).is_err() {
                    break;
                }
            }
            registry.lock().await.remove(&task_id);
            let _ = end.send(ContainerStreamEnd {
                stream_id: task_id,
                error: None,
            });
        });

        stream_id
    }
}

/// Turns a host-key decision into the refusal a workspace can show without a prompt.
fn decision_error(decision: HostKeyDecision) -> ShellError {
    match decision {
        HostKeyDecision::Unknown(key) => ShellError::ssh_host_key_unknown(&key.host, key.port, &key.key_type),
        HostKeyDecision::Mismatch { trusted, .. } => {
            ShellError::ssh_host_key_mismatch(&trusted.host, trusted.port, &trusted.key_type)
        }
    }
}

fn require_ack(
    verdict: sevenshell_types::RiskAssessment,
    fallback: &str,
    acknowledged: bool,
) -> Result<(), ShellError> {
    if verdict.needs_approval() && !acknowledged {
        let rule = verdict
            .reasons
            .first()
            .map_or(fallback, |reason| reason.rule_id.as_str());
        return Err(ShellError::policy_approval_required(rule));
    }
    Ok(())
}
