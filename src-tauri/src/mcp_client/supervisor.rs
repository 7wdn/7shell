//! When an external server is opened, what the window is told, and when it is let go.
//!
//! The lower crate answers "refresh this one, now"; the timers and the application's lifetime are
//! this module's, because they are the same lifetime the vault and the window have. Four moments
//! drive it:
//!
//! * **Launch.** The settings file's list is applied with the vault's secrets; nothing is opened on
//!   the setup thread — every enabled server is opened on a task of its own, so a server that does
//!   not answer cannot delay the window.
//! * **A save.** One server's configuration or secrets changed, so that server is reopened; the
//!   others keep the sessions they had.
//! * **The vault opening.** Secrets that were unreadable become readable, so every enabled server
//!   is reopened with them.
//! * **The vault locking (or the window closing).** The manager forgets every secret and ends every
//!   session. A lock is the moment the person said these credentials may no longer be used, and a
//!   refresh that reopened a server a millisecond later would be the one way around that.
//!
//! The window hears about a change through `mcp:server_status`, one row at a time, and only when the
//! row actually changed: a save that alters one server must not make five rows flicker. A row whose
//! server was *removed* produces no event — there is nothing left to draw — and the page learns about
//! it from the list it asked for.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sevenshell_api::ToolRegistry;
use sevenshell_mcp_client::{McpClientError, McpClientManager, McpServerSecrets, validate_server};
use sevenshell_types::{AppSettings, McpServerConfig, McpServerState, McpServerStatus};
use sevenshell_vault::Vault;
use tauri::{AppHandle, Emitter as _};

use crate::mcp_client::McpServerView;

/// The event one server's row changes on.
pub const SERVER_STATUS_EVENT: &str = "mcp:server_status";

/// How often a server that answered is asked whether it is still there.
const PING_INTERVAL: Duration = Duration::from_secs(60);

/// How often the host looks at its health book. Small enough that a one-second backoff is honoured,
/// cheap enough that a book of a few servers costs nothing when nothing is due.
const TICK_INTERVAL: Duration = Duration::from_secs(1);

/// The first failed attempt is retried after a second; each further failure doubles the wait, and
/// thirty seconds is the ceiling. A server that is down is not hammered, and one that comes back is
/// picked up without anybody pressing refresh.
fn retry_delay(failures: u32) -> Duration {
    let exponent = failures.saturating_sub(1).min(5);
    Duration::from_secs((1u64 << exponent).min(30))
}

/// What the host remembers about one server between ticks: how many times in a row it failed, when
/// its next attempt is due, and when it was last asked whether it is alive.
#[derive(Default)]
struct Health {
    failures: u32,
    retry_at: Option<Instant>,
    last_ping: Option<Instant>,
}

#[derive(Default)]
struct HealthBook {
    entries: BTreeMap<String, Health>,
}

/// Where a status change goes. A trait rather than an `AppHandle` so a test can read what the window
/// would have been told, the same seam the approval presenter has.
pub trait ServerStatusEvents: Send + Sync {
    fn server_status(&self, status: &McpServerStatus);
}

/// The real one: one `mcp:server_status` event per change.
pub struct TauriServerStatusEvents {
    app: AppHandle,
}

impl TauriServerStatusEvents {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl ServerStatusEvents for TauriServerStatusEvents {
    fn server_status(&self, status: &McpServerStatus) {
        if let Err(error) = self.app.emit(SERVER_STATUS_EVENT, status) {
            log::warn!("an external server's state could not be sent to the window: {error}");
        }
    }
}

/// The host's half of the external-server client.
///
/// Holds the one manager the window supervises and the one vault its credentials come from. The
/// manager is also registered as the tool registry's dynamic source here, exactly once — that is
/// what makes an external server's tools appear in the same list the bridge and the assistant read,
/// without either of them knowing an external server exists.
pub struct McpClientSupervisor {
    manager: Arc<McpClientManager>,
    vault: Arc<Vault>,
    events: Arc<dyn ServerStatusEvents>,
    health: Mutex<HealthBook>,
    /// The last row the window was told about, so a changed tool count on an otherwise ready server
    /// is still one event — and a row that did not change is not sent again.
    reported: Mutex<BTreeMap<String, McpServerStatus>>,
    running: AtomicBool,
}

impl McpClientSupervisor {
    pub fn new(
        manager: Arc<McpClientManager>,
        registry: Arc<ToolRegistry>,
        vault: Arc<Vault>,
        events: Arc<dyn ServerStatusEvents>,
    ) -> Arc<Self> {
        registry.add_source(Arc::clone(&manager).source());
        Arc::new(Self {
            manager,
            vault,
            events,
            health: Mutex::new(HealthBook::default()),
            reported: Mutex::new(BTreeMap::new()),
            running: AtomicBool::new(false),
        })
    }

    /// Starts the one health loop this window has.
    ///
    /// A failed server is retried on its own schedule and a ready one is pinged; both timers belong
    /// to the window's lifetime, not to any view of it. The loop is a single task: the manager's own
    /// handshake timeout bounds each attempt, so a second ticker could only make two attempts race
    /// for the same session. Calling this twice is a no-op.
    pub fn start(self: &Arc<Self>) {
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let supervisor = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let mut ticker = tokio::time::interval(TICK_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                supervisor.tick(Instant::now()).await;
            }
        });
    }

    /// Whether the vault holds anything for this server. `None` is a vault that cannot be read —
    /// locked, or absent — which is a different answer from "none stored".
    pub fn has_secrets(&self, server_id: &str) -> Option<bool> {
        match self.vault.mcp_server_secrets(server_id) {
            Ok(Some(stored)) => Some(!McpServerSecrets::from_vault_value(stored.expose()).is_empty()),
            Ok(None) => Some(false),
            Err(_) => None,
        }
    }

    /// Every configured server as a row, in the order the settings file holds them.
    pub fn list(&self, settings: &AppSettings) -> Vec<McpServerView> {
        settings
            .mcp_client_servers
            .iter()
            .cloned()
            .map(|config| self.view_of(config))
            .collect()
    }

    /// One configured server as a row, or nothing when no server has that id.
    pub fn view(&self, settings: &AppSettings, server_id: &str) -> Option<McpServerView> {
        self.server(settings, server_id).map(|config| self.view_of(config))
    }

    /// One entry of the settings file, which is what a command checks before it writes anything.
    pub fn server(&self, settings: &AppSettings, server_id: &str) -> Option<McpServerConfig> {
        settings
            .mcp_client_servers
            .iter()
            .find(|server| server.id == server_id)
            .cloned()
    }

    /// Writes the configured list and the vault's secrets into the manager.
    ///
    /// This is the one function that hands the manager its configuration, so a command that saves a
    /// server cannot put one list in the file and another in the manager. The vault is read for
    /// every configured server; a vault that is locked yields no secrets rather than a failure,
    /// because a locked vault is the normal state of a window that was just opened.
    pub fn apply(&self, settings: &AppSettings) -> Result<(), McpClientError> {
        // The bound is a property of every call, not of a session: a settings save moves it for the
        // next call on the sessions that are already open, without reconnecting anything.
        self.manager
            .set_max_result_bytes(usize::try_from(settings.mcp_max_result_bytes).unwrap_or(usize::MAX));
        self.manager
            .set_secrets(self.secrets_from_vault(&settings.mcp_client_servers));
        self.manager.replace(settings.mcp_client_servers.clone())?;
        self.announce(&self.manager.statuses());
        Ok(())
    }

    /// Applies, then opens every server that has not been opened since.
    ///
    /// Spawned rather than awaited: a start that waited for a handshake would make the settings page
    /// (and the window showing it) wait on somebody else's server. Only the rows that are
    /// `connecting` are opened — `replace` kept the sessions of the entries that did not change, and
    /// reopening those would undo the very thing it compares for.
    pub fn apply_and_refresh(self: &Arc<Self>, settings: &AppSettings) -> Result<(), McpClientError> {
        self.apply(settings)?;
        let supervisor = Arc::clone(self);
        tauri::async_runtime::spawn(async move { supervisor.refresh_pending().await });
        Ok(())
    }

    /// Saves one server's configuration into `settings` and applies the new list.
    ///
    /// `settings` is only changed when the manager accepted the list — a refused id or an
    /// incomplete transport is the page's answer, not a half-applied file. The returned row is the
    /// state the server is in now: `connecting` for a server that was just added or changed, and
    /// whatever it had for one whose settings did not move.
    pub fn save_server(
        &self,
        settings: &mut AppSettings,
        config: McpServerConfig,
    ) -> Result<McpServerView, McpClientError> {
        let config = validate_server(&config)?;

        let mut next = settings.clone();
        match next.mcp_client_servers.iter_mut().find(|server| server.id == config.id) {
            Some(existing) => *existing = config.clone(),
            None => next.mcp_client_servers.push(config.clone()),
        }

        self.apply(&next)?;
        *settings = next;
        Ok(self
            .view(settings, &config.id)
            .expect("the server was just applied to the settings file's copy"))
    }

    /// Removes one server, with the secrets the vault files under it.
    ///
    /// Answers whether anything was removed: removing a server twice is not a failure, because two
    /// windows (or a retried click) can both ask. A locked vault cannot delete the vault entry, so
    /// that failure is logged rather than returned — the secret of a server nobody is configured for
    /// is unreachable anyway, and refusing the removal would leave the page with a row it cannot
    /// get rid of.
    pub fn remove_server(&self, settings: &mut AppSettings, server_id: &str) -> Result<bool, McpClientError> {
        let mut next = settings.clone();
        let before = next.mcp_client_servers.len();
        next.mcp_client_servers.retain(|server| server.id != server_id);
        let removed = next.mcp_client_servers.len() != before;

        if !removed {
            return Ok(false);
        }

        if let Err(error) = self.vault.set_mcp_server_secrets(server_id, None) {
            log::info!("the secrets of the removed server `{server_id}` stayed in the vault: {error}");
        }

        self.apply(&next)?;
        *settings = next;
        Ok(true)
    }

    /// Reconnects one server now and answers the state that attempt reached.
    ///
    /// A failed attempt is still an answer about the row — the manager records `failed` with the
    /// catalogue code — so the row is emitted before the error travels to the caller. That way a
    /// page that shows the error and a page that only listens to the event agree about what
    /// happened.
    pub async fn refresh(&self, server_id: &str) -> Result<McpServerStatus, McpClientError> {
        let result = self.manager.refresh(server_id).await;
        self.announce(&self.manager.statuses());
        result
    }

    /// Reconnects one server on a task of its own, for a command that should not wait on it.
    pub fn refresh_in_background(self: &Arc<Self>, server_id: String) {
        let supervisor = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            if let Err(error) = supervisor.refresh(&server_id).await {
                log::info!("the external server `{server_id}` was reopened and answered {error}");
            }
        });
    }

    /// Reconnects one server unless it is already connected with the settings it has.
    ///
    /// A save of unchanged settings keeps the session — `replace` compares the entries — so a
    /// reconnect there would be a connection nobody asked for. Anything else (a new entry, a
    /// changed one, a failure) is opened: that is what the person saving the form meant.
    pub fn refresh_if_needed(self: &Arc<Self>, server_id: &str) {
        if self
            .manager
            .status_of(server_id)
            .is_some_and(|status| status.state == McpServerState::Ready)
        {
            return;
        }
        self.refresh_in_background(server_id.to_owned());
    }

    /// Opens what the last apply left `connecting`: a new server, a changed one, or one whose
    /// session the vault lock ended.
    async fn refresh_pending(&self) {
        let pending: Vec<String> = self
            .manager
            .statuses()
            .into_iter()
            .filter(|status| status.state == McpServerState::Connecting)
            .map(|status| status.server_id)
            .collect();

        for server_id in pending {
            if let Err(error) = self.refresh(&server_id).await {
                log::info!("the external server `{server_id}` could not be opened: {error}");
            }
        }
    }

    /// One pass of the health loop, at a moment the caller supplies.
    ///
    /// `now` is a parameter rather than read here so a test can walk a minute of retries without
    /// sleeping; the loop passes `Instant::now()`. A failed server is retried once its backoff is
    /// due; a ready one is pinged every [`PING_INTERVAL`]. A server that does not answer a ping is
    /// marked failed by the manager and then follows the same retry schedule as any other failure.
    async fn tick(&self, now: Instant) {
        let statuses = self.manager.statuses();
        let mut retries = Vec::new();
        let mut pings = Vec::new();

        {
            let mut book = self.health.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let configured: BTreeSet<&str> = statuses.iter().map(|status| status.server_id.as_str()).collect();
            book.entries.retain(|id, _| configured.contains(id.as_str()));

            for status in &statuses {
                let health = book.entries.entry(status.server_id.clone()).or_default();
                match status.state {
                    McpServerState::Failed => {
                        health.last_ping = None;
                        match health.retry_at {
                            Some(at) if at <= now => retries.push(status.server_id.clone()),
                            Some(_) => {}
                            None => {
                                // A failure first seen here — a save's attempt, or a manual refresh —
                                // has not been counted yet. Counting it starts the backoff correctly.
                                health.failures = health.failures.max(1);
                                health.retry_at = Some(now + retry_delay(health.failures));
                            }
                        }
                    }
                    McpServerState::Ready => {
                        health.failures = 0;
                        health.retry_at = None;
                        match health.last_ping {
                            None => health.last_ping = Some(now),
                            Some(at) if now.saturating_duration_since(at) >= PING_INTERVAL => {
                                pings.push(status.server_id.clone());
                            }
                            _ => {}
                        }
                    }
                    McpServerState::Connecting | McpServerState::Disabled => {
                        health.failures = 0;
                        health.retry_at = None;
                        health.last_ping = None;
                    }
                }
            }
        }

        for server_id in retries {
            // A deadline that has been acted on is cleared before the attempt. The attempt is
            // bounded by the client's handshake timeout; its result decides the next deadline.
            self.clear_deadline(&server_id);
            match self.refresh(&server_id).await {
                Ok(status) if status.state == McpServerState::Ready => self.record_alive(&server_id, now),
                Ok(_) => self.record_failure(&server_id, now),
                Err(_) => self.record_failure(&server_id, now),
            }
        }

        for server_id in pings {
            self.check_alive(&server_id, now).await;
        }

        // A notification may have replaced a ready session's tool list during the ticks above; the
        // row is the same state with a different count, and the settings page is where it is drawn.
        self.announce(&self.manager.statuses());
    }

    /// Pings one ready server and turns a silent session into a failed row.
    async fn check_alive(&self, server_id: &str, now: Instant) {
        match self.manager.ping(server_id).await {
            Ok(()) => self.record_alive(server_id, now),
            Err(error) => {
                log::info!("the external server `{server_id}` did not answer a ping and will be retried: {error}");
                self.record_failure(server_id, now);
            }
        }
        // A ping is also the moment the row is published if a `list_changed` refresh moved the tool
        // count while the state stayed `ready`.
        self.announce(&self.manager.statuses());
    }

    fn record_alive(&self, server_id: &str, now: Instant) {
        let mut book = self.health.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let health = book.entries.entry(server_id.to_owned()).or_default();
        health.failures = 0;
        health.retry_at = None;
        health.last_ping = Some(now);
    }

    fn record_failure(&self, server_id: &str, now: Instant) {
        let mut book = self.health.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let health = book.entries.entry(server_id.to_owned()).or_default();
        health.failures = health.failures.saturating_add(1);
        health.retry_at = Some(now + retry_delay(health.failures));
        health.last_ping = None;
    }

    fn clear_deadline(&self, server_id: &str) {
        let mut book = self.health.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let health = book.entries.entry(server_id.to_owned()).or_default();
        health.retry_at = None;
    }

    /// Ends every session and forgets every secret, without forgetting the configuration.
    ///
    /// The vault is locking or the window is closing: a session keeps the environment it was started
    /// with and the headers it was dialled with, so neither may outlive the vault. Locking the vault
    /// clears the manager's copy before spawning this, which is what closes the window between the
    /// lock and the task running.
    pub async fn disconnect(&self) {
        self.manager.set_secrets(BTreeMap::new());
        self.manager.disconnect_all().await;
        self.announce(&self.manager.statuses());
    }

    /// The vault is locking: nothing here may keep a value that came from it.
    pub fn on_vault_locked(self: &Arc<Self>) {
        // Synchronous, so a refresh racing the lock cannot read the secrets again.
        self.manager.set_secrets(BTreeMap::new());
        let supervisor = Arc::clone(self);
        tauri::async_runtime::spawn(async move { supervisor.disconnect().await });
    }

    /// The vault is open again: the secrets are readable, so every enabled server is retried.
    pub fn on_vault_unlocked(self: &Arc<Self>, settings: &AppSettings) {
        if let Err(error) = self.apply_and_refresh(settings) {
            log::warn!("the external MCP servers could not be applied after the vault was opened: {error}");
        }
    }

    /// The window is going away: the sessions have no reader left.
    pub fn shutdown(self: &Arc<Self>) {
        let supervisor = Arc::clone(self);
        tauri::async_runtime::spawn(async move { supervisor.disconnect().await });
    }

    /// One row: the configuration it was asked about, the manager's state, and the vault's answer.
    fn view_of(&self, config: McpServerConfig) -> McpServerView {
        let status = self.manager.status_of(&config.id).unwrap_or_else(|| {
            // The manager only hears about the list through `apply`; a settings file that arrived
            // from somewhere else (another window, a restored backup) is drawn honestly as "not
            // attempted yet" rather than as a state the manager never reported.
            McpServerStatus::new(
                &config.id,
                if config.enabled {
                    McpServerState::Connecting
                } else {
                    McpServerState::Disabled
                },
            )
        });
        let has_secrets = self.has_secrets(&config.id);
        McpServerView {
            config,
            status,
            has_secrets,
        }
    }

    fn secrets_from_vault(&self, servers: &[McpServerConfig]) -> BTreeMap<String, McpServerSecrets> {
        let mut secrets = BTreeMap::new();
        for server in servers {
            // No entry for this server, or a vault that cannot be read (locked), contributes
            // nothing: a half-set of secrets from a partly readable vault would be worse than none.
            let Ok(Some(stored)) = self.vault.mcp_server_secrets(&server.id) else {
                continue;
            };
            let decoded = McpServerSecrets::from_vault_value(stored.expose());
            if !decoded.is_empty() {
                secrets.insert(server.id.clone(), decoded);
            }
        }
        secrets
    }

    /// Publishes every row that differs from what the window was last told.
    ///
    /// The map is the memory that makes a tool count alone count as a change: the health tick
    /// compares against it, so a `list_changed` refresh that leaves the state `ready` still reaches
    /// the settings page. Rows of servers that are no longer configured are dropped without an
    /// event: there is nothing left to draw.
    fn announce(&self, statuses: &[McpServerStatus]) {
        let changed = {
            let mut reported = self.reported.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let configured: BTreeSet<&str> = statuses.iter().map(|status| status.server_id.as_str()).collect();
            reported.retain(|id, _| configured.contains(id.as_str()));
            let mut changed = Vec::new();
            for status in statuses {
                if reported.get(&status.server_id) != Some(status) {
                    reported.insert(status.server_id.clone(), status.clone());
                    changed.push(status.clone());
                }
            }
            changed
        };
        for status in &changed {
            self.events.server_status(status);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use async_trait::async_trait;
    use serde_json::{Value, json};
    use sevenshell_mcp_client::{McpTransport, TransportFactory};
    use sevenshell_types::McpServerTransport;
    use sevenshell_vault::Vault;
    use tempfile::TempDir;

    // -----------------------------------------------------------------------------------------
    // A scripted transport, a factory that hands it out, and a window that remembers
    // -----------------------------------------------------------------------------------------

    /// What one scripted server answers, and what it was asked.
    #[derive(Default)]
    struct Script {
        tools: Vec<Value>,
        changed_tools: Mutex<Option<Vec<Value>>>,
        tool_lists: AtomicUsize,
        methods: Mutex<Vec<String>>,
        secrets: Mutex<Vec<Option<McpServerSecrets>>>,
        closed: AtomicBool,
        ping_fails: AtomicBool,
        notification: Mutex<Option<sevenshell_mcp_client::NotificationHandler>>,
    }

    impl Script {
        fn with_tools(tools: Vec<Value>) -> Arc<Self> {
            Arc::new(Self {
                tools,
                ..Self::default()
            })
        }

        /// What `tools/list` answers after the first call, when a test needs a list to change.
        fn with_changed_tools(&self, tools: Vec<Value>) {
            *self.changed_tools.lock().expect("not poisoned") = Some(tools);
        }

        /// Delivers a server notification the way the transport reader would.
        fn emit_notification(&self, method: &str) {
            let handler = self.notification.lock().expect("not poisoned").clone();
            if let Some(handler) = handler {
                handler(method);
            }
        }

        fn methods(&self) -> Vec<String> {
            self.methods.lock().expect("not poisoned").clone()
        }

        /// The secrets handed to the most recent open, or `None` when nothing was opened.
        fn last_secrets(&self) -> Option<Option<McpServerSecrets>> {
            self.secrets.lock().expect("not poisoned").last().cloned()
        }

        /// How many times a session was opened.
        fn opens(&self) -> usize {
            self.secrets.lock().expect("not poisoned").len()
        }
    }

    struct ScriptedTransport {
        script: Arc<Script>,
    }

    #[async_trait]
    impl McpTransport for ScriptedTransport {
        async fn request(&self, method: &str, _params: Option<Value>) -> Result<Value, McpClientError> {
            self.script
                .methods
                .lock()
                .expect("not poisoned")
                .push(method.to_owned());
            match method {
                "initialize" => Ok(json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "scripted", "version": "0" },
                })),
                "tools/list" => {
                    let call = self.script.tool_lists.fetch_add(1, Ordering::SeqCst);
                    let changed = self.script.changed_tools.lock().expect("not poisoned").clone();
                    let tools = match (call, changed) {
                        (0, _) | (_, None) => self.script.tools.clone(),
                        (_, Some(changed)) => changed,
                    };
                    Ok(json!({ "tools": tools }))
                }
                "tools/call" => Ok(json!({
                    "content": [{ "type": "text", "text": "ok" }],
                    "isError": false,
                })),
                "ping" if self.script.ping_fails.load(Ordering::SeqCst) => Err(McpClientError::Failed {
                    server: "scripted".to_owned(),
                    message: "the server stopped answering".to_owned(),
                }),
                "ping" => Ok(json!({})),
                other => Err(McpClientError::Failed {
                    server: "scripted".to_owned(),
                    message: format!("`{other}` was not scripted"),
                }),
            }
        }

        async fn notify(&self, _method: &str, _params: Option<Value>) -> Result<(), McpClientError> {
            Ok(())
        }

        async fn close(&self) -> Result<(), McpClientError> {
            self.script.closed.store(true, Ordering::SeqCst);
            Ok(())
        }

        fn set_notification_handler(&self, handler: sevenshell_mcp_client::NotificationHandler) {
            *self.script.notification.lock().expect("not poisoned") = Some(handler);
        }
    }

    struct ScriptedFactory {
        script: Arc<Script>,
        failure: Option<McpClientError>,
    }

    #[async_trait]
    impl TransportFactory for ScriptedFactory {
        async fn open(
            &self,
            _server: &McpServerConfig,
            secrets: Option<McpServerSecrets>,
        ) -> Result<Box<dyn McpTransport>, McpClientError> {
            self.script.secrets.lock().expect("not poisoned").push(secrets);
            if let Some(failure) = &self.failure {
                return Err(failure.clone());
            }
            Ok(Box::new(ScriptedTransport {
                script: Arc::clone(&self.script),
            }))
        }
    }

    /// The first `failures` opens are refused, and every open after that hands out the script.
    struct FailOnceFactory {
        script: Arc<Script>,
        failures: Mutex<u32>,
    }

    #[async_trait]
    impl TransportFactory for FailOnceFactory {
        async fn open(
            &self,
            _server: &McpServerConfig,
            secrets: Option<McpServerSecrets>,
        ) -> Result<Box<dyn McpTransport>, McpClientError> {
            self.script.secrets.lock().expect("not poisoned").push(secrets);
            let mut failures = self.failures.lock().expect("not poisoned");
            if *failures > 0 {
                *failures -= 1;
                return Err(McpClientError::ConnectFailed {
                    server: "files".to_owned(),
                    detail: "connection refused".to_owned(),
                });
            }
            drop(failures);
            Ok(Box::new(ScriptedTransport {
                script: Arc::clone(&self.script),
            }))
        }
    }

    #[derive(Default)]
    struct RecordingEvents {
        statuses: Mutex<Vec<McpServerStatus>>,
    }

    impl RecordingEvents {
        fn statuses(&self) -> Vec<McpServerStatus> {
            self.statuses.lock().expect("not poisoned").clone()
        }

        fn states_for(&self, server_id: &str) -> Vec<McpServerState> {
            self.statuses()
                .into_iter()
                .filter(|status| status.server_id == server_id)
                .map(|status| status.state)
                .collect()
        }
    }

    impl ServerStatusEvents for RecordingEvents {
        fn server_status(&self, status: &McpServerStatus) {
            self.statuses.lock().expect("not poisoned").push(status.clone());
        }
    }

    /// A supervisor over a vault that exists but was never opened, which is how the shell starts.
    struct Host {
        _directory: TempDir,
        vault: Arc<Vault>,
        registry: Arc<ToolRegistry>,
        events: Arc<RecordingEvents>,
        script: Arc<Script>,
        manager: Arc<McpClientManager>,
        supervisor: Arc<McpClientSupervisor>,
    }

    fn host_with_factory(script: Arc<Script>, factory: Arc<dyn TransportFactory>, registry: Arc<ToolRegistry>) -> Host {
        let directory = tempfile::tempdir().expect("a temp directory");
        let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
        let events = Arc::new(RecordingEvents::default());
        let manager = Arc::new(McpClientManager::new().with_transport_factory(factory));
        let supervisor = McpClientSupervisor::new(
            Arc::clone(&manager),
            Arc::clone(&registry),
            Arc::clone(&vault),
            events.clone() as Arc<dyn ServerStatusEvents>,
        );
        Host {
            _directory: directory,
            vault,
            registry,
            events,
            script,
            manager,
            supervisor,
        }
    }

    fn host_with(script: Arc<Script>, failure: Option<McpClientError>, registry: Arc<ToolRegistry>) -> Host {
        host_with_factory(
            Arc::clone(&script),
            Arc::new(ScriptedFactory { script, failure }),
            registry,
        )
    }

    fn host(script: Arc<Script>, failure: Option<McpClientError>) -> Host {
        host_with(script, failure, Arc::new(ToolRegistry::new()))
    }

    fn server(id: &str, enabled: bool) -> McpServerConfig {
        let mut server = McpServerConfig::new(id, id, McpServerTransport::Stdio);
        server.command = Some("unused".to_owned());
        server.enabled = enabled;
        server
    }

    fn settings_of(servers: Vec<McpServerConfig>) -> AppSettings {
        AppSettings {
            mcp_client_servers: servers,
            ..AppSettings::default()
        }
    }

    fn read_listing(name: &str) -> Value {
        json!({
            "name": name,
            "description": "",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true },
        })
    }

    fn names(descriptors: &[sevenshell_api::ToolDescriptor]) -> Vec<String> {
        descriptors.iter().map(|descriptor| descriptor.name.clone()).collect()
    }

    // -----------------------------------------------------------------------------------------
    // The tool table
    // -----------------------------------------------------------------------------------------

    #[tokio::test]
    async fn the_registry_lists_external_tools_beside_the_built_in_ones() {
        // One built-in tool, so "the external ones join the same table" is what is being asserted
        // rather than "the table was empty".
        let mut local = ToolRegistry::new();
        local.register(Arc::new(EchoTool)).expect("a unique name");
        let host = host_with(Script::with_tools(vec![read_listing("read")]), None, Arc::new(local));
        let settings = settings_of(vec![server("files", true)]);

        host.supervisor.apply(&settings).expect("a valid list");
        assert!(
            !names(&host.registry.descriptors()).contains(&"mcp__files__read".to_owned()),
            "nothing has answered a handshake yet"
        );

        host.supervisor.refresh_pending().await;

        let listed = names(&host.registry.descriptors());
        assert!(listed.contains(&"test_echo".to_owned()), "{listed:?}");
        assert!(listed.contains(&"mcp__files__read".to_owned()), "{listed:?}");
        assert_eq!(
            host.script.methods(),
            vec!["initialize".to_owned(), "tools/list".to_owned()],
            "the session was opened the way the protocol asks"
        );
        assert_eq!(
            host.events.states_for("files"),
            vec![McpServerState::Connecting, McpServerState::Ready],
            "the window was told about the attempt and the answer"
        );
    }

    #[tokio::test]
    async fn applying_a_list_twice_joins_the_source_to_the_table_once() {
        let host = host(Script::with_tools(vec![read_listing("read")]), None);
        let settings = settings_of(vec![server("files", true)]);

        host.supervisor.apply(&settings).expect("a valid list");
        host.supervisor.apply(&settings).expect("the same list again");
        host.supervisor.refresh_pending().await;

        assert_eq!(
            host.registry.sources().len(),
            1,
            "a second apply is not a second source"
        );
        assert_eq!(names(&host.registry.descriptors()), vec!["mcp__files__read".to_owned()]);
    }

    #[tokio::test]
    async fn removing_a_server_takes_its_tools_out_of_the_table() {
        let host = host(Script::with_tools(vec![read_listing("read")]), None);
        let mut settings = settings_of(vec![server("files", true), server("tickets", true)]);
        host.supervisor.apply(&settings).expect("a valid list");
        host.supervisor.refresh_pending().await;
        assert_eq!(host.registry.descriptors().len(), 2);

        let removed = host
            .supervisor
            .remove_server(&mut settings, "tickets")
            .expect("the list stays valid");

        assert!(removed);
        assert_eq!(settings.mcp_client_servers.len(), 1);
        assert_eq!(
            names(&host.registry.descriptors()),
            vec!["mcp__files__read".to_owned()],
            "a tool of a server nobody configured is not callable"
        );
        assert!(
            !host
                .supervisor
                .remove_server(&mut settings, "tickets")
                .expect("removing twice is not a failure"),
            "the second removal has nothing to do"
        );
    }

    #[test]
    fn saving_a_server_normalizes_it_and_keeps_the_rest_of_the_list() {
        let host = host(Script::with_tools(vec![]), None);
        let mut settings = settings_of(vec![server("tickets", true)]);

        let mut typed = McpServerConfig::new(" files ", " Files ", McpServerTransport::Stdio);
        typed.command = Some("  npx  ".to_owned());
        let view = host
            .supervisor
            .save_server(&mut settings, typed)
            .expect("a command is all stdio needs");

        assert_eq!(view.config.id, "files");
        assert_eq!(view.config.command.as_deref(), Some("npx"));
        assert_eq!(view.status.state, McpServerState::Connecting);
        assert_eq!(settings.mcp_client_servers.len(), 2, "the other server is still there");

        let invalid = McpServerConfig::new("broken", "Broken", McpServerTransport::Http);
        let error = host
            .supervisor
            .save_server(&mut settings, invalid)
            .expect_err("an address is all http needs");
        assert_eq!(error.code(), "MCP-INVALID-ARGUMENTS");
        assert_eq!(
            settings.mcp_client_servers.len(),
            2,
            "a refused save leaves the settings file's copy alone"
        );
    }

    #[test]
    fn a_list_two_servers_of_one_id_is_refused_rather_than_half_applied() {
        let host = host(Script::with_tools(vec![]), None);
        let settings = settings_of(vec![server("files", true), server("files", false)]);

        let error = host.supervisor.apply(&settings).expect_err("one id, two servers");

        assert_eq!(error.code(), "MCP-SERVER-DUPLICATE");
        assert!(host.manager.configured().is_empty());
    }

    // -----------------------------------------------------------------------------------------
    // The rows
    // -----------------------------------------------------------------------------------------

    #[test]
    fn a_vault_that_cannot_be_read_says_it_cannot_tell_whether_secrets_are_stored() {
        let host = host(Script::with_tools(vec![]), None);
        let settings = settings_of(vec![server("files", true)]);

        let view = host
            .supervisor
            .view(&settings, "files")
            .expect("the server is configured");

        assert_eq!(host.supervisor.has_secrets("files"), None, "the vault was never opened");
        assert_eq!(view.has_secrets, None);
        assert_eq!(
            serde_json::to_value(&view).expect("a view is JSON")["hasSecrets"],
            Value::Null,
            "the page matches on null, not on a missing field"
        );
    }

    #[test]
    fn an_open_vault_says_which_servers_have_secrets_stored() {
        let host = host(Script::with_tools(vec![]), None);
        host.vault.create("a password").expect("a new vault");
        host.vault
            .set_mcp_server_secrets("files", Some(r#"{"env":{"TOKEN":"hunter2"}}"#.to_owned()))
            .expect("an open vault");

        let settings = settings_of(vec![server("files", true), server("tickets", true)]);
        let rows = host.supervisor.list(&settings);

        assert_eq!(rows[0].has_secrets, Some(true));
        assert_eq!(rows[1].has_secrets, Some(false));
    }

    #[tokio::test]
    async fn the_vaults_secrets_reach_the_transport_that_opens_the_session() {
        let host = host(Script::with_tools(vec![read_listing("read")]), None);
        host.vault.create("a password").expect("a new vault");
        host.vault
            .set_mcp_server_secrets("files", Some(r#"{"env":{"TOKEN":"hunter2"}}"#.to_owned()))
            .expect("an open vault");

        host.supervisor
            .apply(&settings_of(vec![server("files", true)]))
            .expect("a valid list");
        host.supervisor.refresh_pending().await;

        let handed = host.script.last_secrets().expect("the server was opened");
        assert_eq!(
            handed.and_then(|secrets| secrets.env.get("TOKEN").cloned()).as_deref(),
            Some("hunter2")
        );
        assert!(
            !format!("{:?}", host.manager).contains("hunter2"),
            "the manager's own Debug never prints a value"
        );
    }

    #[tokio::test]
    async fn locking_the_vault_ends_the_sessions_and_forgets_what_opened_them() {
        let host = host(Script::with_tools(vec![read_listing("read")]), None);
        host.vault.create("a password").expect("a new vault");
        host.vault
            .set_mcp_server_secrets("files", Some(r#"{"env":{"TOKEN":"hunter2"}}"#.to_owned()))
            .expect("an open vault");

        host.supervisor
            .apply(&settings_of(vec![server("files", true)]))
            .expect("a valid list");
        host.supervisor.refresh_pending().await;
        assert_eq!(host.registry.descriptors().len(), 1);

        host.supervisor.disconnect().await;

        assert!(
            host.script.closed.load(Ordering::SeqCst),
            "the session's transport ended"
        );
        assert!(
            host.registry.descriptors().is_empty(),
            "a session that ended offers no tools"
        );
        assert!(
            host.events
                .states_for("files")
                .last()
                .is_some_and(|state| *state == McpServerState::Connecting),
            "the row says it is not connected rather than keeping the old state"
        );

        // The configuration survives, and the next open carries no secrets: the vault is the only
        // place they may come from, and it has just been locked.
        host.supervisor.refresh_pending().await;
        assert_eq!(host.script.last_secrets(), Some(None));
        assert_eq!(host.registry.descriptors().len(), 1);
    }

    #[tokio::test]
    async fn a_refresh_that_cannot_connect_says_so_on_the_row_and_to_the_window() {
        let host = host(
            Script::with_tools(vec![]),
            Some(McpClientError::ConnectFailed {
                server: "files".to_owned(),
                detail: "connection refused".to_owned(),
            }),
        );
        let settings = settings_of(vec![server("files", true)]);
        host.supervisor.apply(&settings).expect("a valid list");

        let error = host
            .supervisor
            .refresh("files")
            .await
            .expect_err("the server did not answer");
        assert_eq!(error.code(), "MCP-SERVER-CONNECT-FAILED");

        let view = host.supervisor.view(&settings, "files").expect("configured");
        assert_eq!(view.status.state, McpServerState::Failed);
        assert_eq!(view.status.code.as_deref(), Some("MCP-SERVER-CONNECT-FAILED"));
        assert_eq!(view.status.message.as_deref(), Some("connection refused"));
        assert!(
            host.events
                .statuses()
                .iter()
                .any(|status| status.state == McpServerState::Failed),
            "the window heard about the failure even though the command answers with an error"
        );
    }

    #[tokio::test]
    async fn a_row_that_did_not_change_is_not_sent_again() {
        let host = host(Script::with_tools(vec![read_listing("read")]), None);
        host.supervisor
            .apply(&settings_of(vec![server("files", true)]))
            .expect("a valid list");
        host.supervisor.refresh_pending().await;
        let after_first = host.events.statuses().len();

        // An explicit reconnect that reaches the same state with the same tools changes nothing a
        // person could see, so nothing is emitted.
        host.supervisor.refresh("files").await.expect("the server answers");

        assert_eq!(host.events.statuses().len(), after_first);
    }

    #[tokio::test]
    async fn a_changed_tool_count_is_announced_without_a_state_change() {
        let script = Script::with_tools(vec![read_listing("read")]);
        script.with_changed_tools(vec![read_listing("read"), read_listing("second")]);
        let host = host(Arc::clone(&script), None);
        host.supervisor
            .apply(&settings_of(vec![server("files", true)]))
            .expect("a valid list");
        host.supervisor.refresh_pending().await;

        let before = host.events.statuses().len();
        script.emit_notification("notifications/tools/list_changed");
        tokio::time::timeout(Duration::from_secs(2), async {
            while host.manager.tools().len() != 2 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the changed list reached the manager");

        host.supervisor.tick(Instant::now()).await;

        let statuses = host.events.statuses();
        assert_eq!(statuses.len(), before + 1, "one row changed, so one event");
        let last = statuses.last().expect("the changed row");
        assert_eq!(last.state, McpServerState::Ready, "the connection did not move");
        assert_eq!(last.tools, 2, "the count is what changed");
    }

    #[tokio::test]
    async fn saving_unchanged_settings_does_not_reopen_a_ready_server() {
        let host = host(Script::with_tools(vec![read_listing("read")]), None);
        let mut settings = settings_of(vec![server("files", true)]);
        host.supervisor.apply(&settings).expect("a valid list");
        host.supervisor.refresh_pending().await;
        let opens = host.script.opens();

        let view = host
            .supervisor
            .save_server(&mut settings, server("files", true))
            .expect("the same server again");

        assert_eq!(view.status.state, McpServerState::Ready, "the session was kept");
        host.supervisor.refresh_if_needed("files");
        assert_eq!(
            host.script.opens(),
            opens,
            "a save that changed nothing does not cost a connection"
        );
    }

    #[tokio::test]
    async fn applying_a_changed_list_reopens_only_the_server_that_changed() {
        let host = host(Script::with_tools(vec![]), None);
        let mut settings = settings_of(vec![server("files", true), server("tickets", true)]);
        host.supervisor.apply(&settings).expect("a valid list");
        host.supervisor.refresh_pending().await;
        assert_eq!(host.script.opens(), 2, "both were opened by the first apply");

        settings.mcp_client_servers[1].tool_timeout_seconds = 120;
        host.supervisor.apply(&settings).expect("a changed list");
        host.supervisor.refresh_pending().await;

        assert_eq!(
            host.script.opens(),
            3,
            "only the entry that changed is opened again; the other kept its session"
        );
    }

    #[test]
    fn the_backoff_starts_at_a_second_doubles_and_stops_at_thirty() {
        assert_eq!(retry_delay(1), Duration::from_secs(1));
        assert_eq!(retry_delay(2), Duration::from_secs(2));
        assert_eq!(retry_delay(3), Duration::from_secs(4));
        assert_eq!(retry_delay(4), Duration::from_secs(8));
        assert_eq!(retry_delay(5), Duration::from_secs(16));
        assert_eq!(retry_delay(6), Duration::from_secs(30));
        assert_eq!(retry_delay(60), Duration::from_secs(30), "the ceiling holds");
    }

    #[tokio::test]
    async fn a_failed_server_is_retried_on_its_own_after_the_backoff() {
        // No command and no settings save after the first failure: only the health loop runs.
        let script = Script::with_tools(vec![read_listing("read")]);
        let factory = Arc::new(FailOnceFactory {
            script: Arc::clone(&script),
            failures: Mutex::new(1),
        });
        let host = host_with_factory(script.clone(), factory, Arc::new(ToolRegistry::new()));
        host.supervisor
            .apply(&settings_of(vec![server("files", true)]))
            .expect("a valid list");
        host.supervisor.refresh_pending().await;
        assert_eq!(host.manager.statuses()[0].state, McpServerState::Failed);
        assert_eq!(host.script.opens(), 1);

        let at = Instant::now();
        host.supervisor.tick(at).await;
        assert_eq!(host.script.opens(), 1, "the failure only starts the clock");
        host.supervisor.tick(at + Duration::from_millis(999)).await;
        assert_eq!(host.script.opens(), 1, "still inside the first second");

        host.supervisor.tick(at + Duration::from_secs(1)).await;
        assert_eq!(host.script.opens(), 2, "the retry ran without a command");
        assert_eq!(host.manager.statuses()[0].state, McpServerState::Ready);
        assert_eq!(
            host.events.states_for("files").last(),
            Some(&McpServerState::Ready),
            "the window was told the server came back"
        );
    }

    #[tokio::test]
    async fn a_ready_server_is_pinged_a_minute_after_it_answered() {
        let host = host(Script::with_tools(vec![read_listing("read")]), None);
        host.supervisor
            .apply(&settings_of(vec![server("files", true)]))
            .expect("a valid list");
        host.supervisor.refresh_pending().await;

        let at = Instant::now();
        host.supervisor.tick(at).await;
        host.supervisor.tick(at + Duration::from_secs(59)).await;
        assert!(
            !host.script.methods().contains(&"ping".to_owned()),
            "a server opened a moment ago is not asked yet"
        );

        host.supervisor.tick(at + PING_INTERVAL).await;
        assert!(
            host.script.methods().contains(&"ping".to_owned()),
            "the first ping happens one interval after it answered"
        );
        assert_eq!(host.manager.statuses()[0].state, McpServerState::Ready);
    }

    #[tokio::test]
    async fn a_ping_that_goes_unanswered_fails_the_row_and_the_next_tick_reopens_it() {
        let host = host(Script::with_tools(vec![read_listing("read")]), None);
        host.supervisor
            .apply(&settings_of(vec![server("files", true)]))
            .expect("a valid list");
        host.supervisor.refresh_pending().await;
        assert_eq!(host.manager.statuses()[0].state, McpServerState::Ready);

        let at = Instant::now();
        host.supervisor.tick(at).await;
        host.script.ping_fails.store(true, Ordering::SeqCst);
        host.supervisor.tick(at + PING_INTERVAL).await;

        assert_eq!(
            host.manager.statuses()[0].state,
            McpServerState::Failed,
            "a session that stopped answering is drawn as failed, not ready"
        );
        assert_eq!(
            host.events.states_for("files").last(),
            Some(&McpServerState::Failed),
            "the window heard about it"
        );

        // The failed ping is what made it due one second later; the loop retries with no command.
        host.script.ping_fails.store(false, Ordering::SeqCst);
        host.supervisor.tick(at + PING_INTERVAL + Duration::from_secs(1)).await;
        assert_eq!(host.manager.statuses()[0].state, McpServerState::Ready);
        assert_eq!(host.script.opens(), 2, "the dead session was replaced");
    }

    // A built-in tool, so a table with local and external tools in it can be built here.
    struct EchoTool;

    #[async_trait]
    impl sevenshell_api::Tool for EchoTool {
        fn descriptor(&self) -> sevenshell_api::ToolDescriptor {
            sevenshell_api::ToolDescriptor {
                name: "test_echo".to_owned(),
                title: "Echo".to_owned(),
                description: "Answers with its arguments.".to_owned(),
                input_schema: sevenshell_api::ToolInputSchema::object(json!({}), &[]),
                access: sevenshell_api::ToolAccess::Read,
            }
        }

        async fn call(
            &self,
            _context: &sevenshell_api::ToolContext,
            arguments: Value,
        ) -> Result<sevenshell_api::ToolResult, sevenshell_api::ToolError> {
            Ok(sevenshell_api::ToolResult::new(json!({ "echo": arguments })))
        }
    }
}
