//! The listener's lifetime: one token, one socket, one file, one shutdown.
//!
//! The runtime owns the pieces a command cannot: the bound port, the token for this run, the sender
//! that stops `axum::serve`, the task serving it, and the registry of questions waiting to be
//! answered. Starting and stopping are the only two things that touch them, and both are serialized
//! by one mutex — a person clicking the switch twice must not leave two listeners with two tokens
//! and one file that names whichever won the race.
//!
//! The vault is deliberately *not* consulted here. A locked vault is a tool error (`VAULT-LOCKED`),
//! not a stopped listener: the bridge can still reach the desktop, ask what tools exist and learn why
//! a call did not run. Stopping the listener on lockdown would turn a diagnosable answer into "the
//! desktop is not running", which is a different and untrue thing to tell an agent.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sevenshell_api::local::ConnectionCache;
use sevenshell_api::{ApprovalDecision, AuditSink, EndpointFile, ToolRegistry};
use sevenshell_core::Core;
use sevenshell_error::ShellError;
use sevenshell_types::{AgentOrigin, AppSettings};
use tauri::AppHandle;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, oneshot};
use tokio::task::JoinHandle;

use crate::mcp::McpStatus;
use crate::mcp::approval::{ApprovalRegistry, TauriPresenter};
use crate::mcp::auth::{generate_token, tokens_match};
use crate::mcp::endpoint;
use crate::mcp::server::{CallLimits, RegistryHandler, ServerState, router};
use crate::state::{SharedAgent, SharedAudit};

/// How long a stop waits for in-flight requests before cutting them off.
///
/// The endpoint is turned off to stop an agent, so a request that is still running must not delay
/// that indefinitely; two seconds is enough for a request between steps, and a long batch is exactly
/// the thing somebody flipping the switch is trying to stop.
const STOP_GRACE: Duration = Duration::from_secs(2);

/// What one run holds.
struct Running {
    endpoint: EndpointFile,
    endpoint_path: PathBuf,
    limits: CallLimits,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

/// The agent interface as the shell keeps it.
///
/// The tool list, the approval questions, the audit trail and the cached connections belong to the
/// [`SharedAgent`] the assistant panel is given too: one window answers both callers' questions, one
/// JSONL file records both callers' calls, and one cache means a session an assistant opened is the
/// one an MCP call reads through. What is the listener's alone is the socket, the token and the file
/// that names them.
pub struct McpRuntime {
    core: Arc<Core>,
    registry: Arc<ToolRegistry>,
    /// One cache for every call of a run: a conversation is one authenticated connection per
    /// session, not one per question. It is also what the vault lock empties.
    connections: Arc<ConnectionCache>,
    approvals: Arc<ApprovalRegistry>,
    /// The trail both callers write to. Not owned by a run: the assistant writes to the same file
    /// while no listener is up, and two writers appending to one file is what the shared sink is for.
    audit: Arc<SharedAudit>,
    running: Mutex<Option<Running>>,
    /// Bumped by every stop. A start that was waiting for the lock while a stop happened is a start
    /// whose premise is gone — the common case is startup still binding when somebody turns the
    /// interface off — and it must not put the listener back on behind them.
    stopped: AtomicU64,
}

impl McpRuntime {
    pub fn new(shared: Arc<SharedAgent>) -> Self {
        Self {
            core: Arc::clone(shared.connections().core()),
            // The tool list is this build's own and does not change with settings; building it once
            // means a restart cannot accidentally publish a different set.
            registry: Arc::clone(shared.registry()),
            connections: Arc::clone(shared.connections()),
            approvals: Arc::clone(shared.approvals()),
            audit: Arc::clone(shared.audit()),
            running: Mutex::new(None),
            stopped: AtomicU64::new(0),
        }
    }

    /// Starts listening, or answers the status of the listener already running.
    ///
    /// The endpoint file is written last: a bridge that saw it before the socket existed would be
    /// pointed at a port that refuses connections.
    pub async fn start(&self, app: &AppHandle, settings: &AppSettings) -> Result<McpStatus, ShellError> {
        let generation = self.stopped.load(Ordering::SeqCst);
        let mut guard = self.running.lock().await;
        if guard.is_some() {
            drop(guard);
            return Ok(self.status(settings).await);
        }
        if self.stopped.load(Ordering::SeqCst) != generation {
            drop(guard);
            log::debug!("an agent interface start was abandoned because it was turned off while waiting");
            return Ok(self.status(settings).await);
        }

        // Resolved before anything is opened: a machine without a configuration directory cannot
        // publish an endpoint, and a socket opened for nothing is a port somebody else could see.
        let endpoint_path = endpoint::path()?;
        let audit_path = crate::mcp::audit::default_path()?;
        log::debug!("the agent interface's audit trail is {}", audit_path.display());

        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, settings.mcp_port));
        let listener = TcpListener::bind(address).await.map_err(|error| {
            ShellError::store_io(format!(
                "the agent interface could not listen on 127.0.0.1:{}: {error}",
                settings.mcp_port
            ))
        })?;
        let port = listener
            .local_addr()
            .map_err(|error| ShellError::store_io(format!("the bound address could not be read: {error}")))?
            .port();

        let token = generate_token()?;
        let published = EndpointFile::new(port, token.clone(), std::process::id(), now_seconds())
            .with_max_result_bytes(settings.mcp_max_result_bytes);
        // If this fails the listener is dropped with the error, and no file names a port that is not
        // listening.
        endpoint::write_to(&endpoint_path, &published)?;

        let limits = CallLimits {
            scope: settings.mcp_scope,
            approval_timeout_seconds: settings.mcp_approval_timeout_seconds,
            max_result_bytes: settings.mcp_max_result_bytes,
        };

        // The retention this installation chose, applied to the one trail both callers write to. It
        // is set here rather than when the file is first written because a person who changed the
        // setting should not have to wait for a restart — and a start is exactly that moment.
        self.audit.configure(settings.mcp_audit_retention_days);

        let handler = Arc::new(RegistryHandler::new(
            self.core.clone(),
            self.registry.clone(),
            self.connections.clone(),
            self.approvals.clone(),
            Arc::new(TauriPresenter::new(app.clone())),
            self.audit.clone() as Arc<dyn AuditSink>,
            limits,
        ));
        let service = router(Arc::new(ServerState::new(token, handler)));

        let (shutdown, shutdown_receiver) = oneshot::channel();
        let task = tokio::spawn(async move {
            let served = axum::serve(listener, service)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_receiver.await;
                })
                .await;
            if let Err(error) = served {
                log::warn!("the agent interface stopped answering: {error}");
            }
        });

        *guard = Some(Running {
            endpoint: published,
            endpoint_path,
            limits,
            shutdown: Some(shutdown),
            task,
        });
        drop(guard);

        log::info!("the agent interface is listening on 127.0.0.1:{port}");
        Ok(self.status(settings).await)
    }

    /// Stops listening, removes this run's endpoint file, and resolves every waiting question as a
    /// refusal.
    pub async fn stop(&self) {
        let running = {
            let mut guard = self.running.lock().await;
            self.stopped.fetch_add(1, Ordering::SeqCst);
            guard.take()
        };
        let Some(Running {
            endpoint: published,
            endpoint_path,
            shutdown,
            mut task,
            ..
        }) = running
        else {
            return;
        };

        // First, so the tools waiting on an answer finish instead of holding the connection until
        // their timeout — a person turning the interface off should not watch it stay on for two
        // minutes. Only this caller's questions: the assistant panel shares the registry, and a
        // dialog it is waiting on is not the listener's to drop.
        if !self.approvals.is_empty() {
            log::info!(
                "the agent interface is stopping with {} question(s) on screen",
                self.approvals.len()
            );
        }
        let dropped = self.approvals.clear_origin(AgentOrigin::Mcp);
        if dropped > 0 {
            log::info!("{dropped} of them belonged to the agent interface and are refused");
        }

        // Stop accepting before the connections go: a call that arrived between the two would find
        // half a cache and report a failure that has nothing to do with what it asked.
        if let Some(shutdown) = shutdown {
            let _ = shutdown.send(());
        }

        // The cached file, container and cluster connections belong to the tools this listener was
        // serving. Once it is off, nothing is going to ask for them again.
        self.connections.close_all().await;

        // Only the file this run wrote. Comparing tokens rather than removing by name keeps a stop
        // that raced a restart from deleting the newer run's file.
        let ours = endpoint::read(&endpoint_path)
            .map(|on_disk| tokens_match(&on_disk.token, &published.token))
            .unwrap_or(true);
        if ours {
            endpoint::remove(&endpoint_path);
        }

        if tokio::time::timeout(STOP_GRACE, &mut task).await.is_err() {
            task.abort();
            log::warn!("the agent interface did not stop within its grace period and was cut off");
        }
    }

    pub async fn is_running(&self) -> bool {
        self.running.lock().await.is_some()
    }

    /// The interface's status, from the listener when there is one and from the settings otherwise.
    pub async fn status(&self, settings: &AppSettings) -> McpStatus {
        let guard = self.running.lock().await;
        let (enabled, port, scope) = match guard.as_ref() {
            Some(running) => (true, Some(running.endpoint.port), running.limits.scope),
            None => (false, None, settings.mcp_scope),
        };
        drop(guard);

        McpStatus {
            enabled,
            port,
            scope,
            // Best effort: a bridge that is not installed has no command to name, and the dedicated
            // `mcp_client_config` command is where that is said in words. An empty string here keeps
            // the status shape fixed.
            client_config: crate::mcp::client_config().unwrap_or_default(),
            audit_path: crate::mcp::audit::default_path()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            pending_approvals: self.approvals.len(),
        }
    }

    /// Answers one pending approval. An id that is not waiting is ignored.
    pub fn answer(&self, call_id: &str, decision: ApprovalDecision) -> bool {
        self.approvals.answer(call_id, decision)
    }

    /// Where the audit trail lives, for the command that reads its tail.
    pub fn audit_path(&self) -> Result<PathBuf, ShellError> {
        match self.audit.path() {
            Some(path) => Ok(path.to_path_buf()),
            // No trail was ever configured because this machine has no configuration directory.
            // Asking for the default path is what turns that into the sentence the command returns.
            None => crate::mcp::audit::default_path(),
        }
    }

    /// Closes every cached connection, because the vault is locking.
    ///
    /// Spawned rather than awaited: the lock has to happen now and its callers include a synchronous
    /// tray handler. The handles were opened with credentials that are about to stop existing, and
    /// the listener stays up so an agent gets `VAULT-LOCKED` rather than "the desktop is not running".
    pub fn close_connections(&self) {
        let connections = self.connections.clone();
        tauri::async_runtime::spawn(async move { connections.close_all().await });
    }
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}
