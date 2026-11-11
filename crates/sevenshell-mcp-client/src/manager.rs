//! The set of servers this installation is configured for, and what the host asks it to do.
//!
//! Each configured server owns one entry: its settings, its state (`connecting`, `ready`,
//! `failed`, `disabled`), its session when it has one, and the tool list that session last
//! announced. One server failing changes nothing about the others — that is the whole reason the
//! entries are separate rather than one shared "MCP is up" flag.
//!
//! What is deliberately *not* here: the retry schedule. The host's supervisor owns the timers, the
//! backoff and the `mcp:server_status` event, because they are driven by the application's lifetime
//! (it starts when the window starts, stops when the vault locks). This crate answers "refresh this
//! one, now" and reports what happened.
//!
//! Two rules that shape the code:
//!
//! * **An external call is never retried.** A tool may have had a side effect the first time, and a
//!   second attempt would do it twice. A failed call fails once; a server that has gone away is
//!   rebuilt by the next refresh, and the calls that were in flight are not replayed onto it.
//! * **Every change goes through one lock.** The manager is read from several calls at once (the
//!   registry asks for descriptors while a call is running) and written by the supervisor, so the
//!   list and each entry's session sit behind one lock that is never held across an await.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde_json::Value;
use sevenshell_api::{Tool, ToolContext, ToolDescriptor, ToolError, ToolResult, ToolSource};
use sevenshell_types::{McpServerConfig, McpServerState, McpServerStatus};

use crate::McpClientError;
use crate::adapter::ExternalTool;
use crate::config::{McpServerSecrets, is_valid_server_id, split_tool_name, validate_server};
use crate::session::McpSession;
use crate::transport::{DefaultTransportFactory, TransportFactory};

/// Every external server this installation may call tools on.
///
/// Held behind an [`Arc`] by the host, because two things read it for as long as the window lives:
/// the supervisor (which refreshes) and the tool registry (which lists and calls). The same `Arc`
/// is both, which is what makes the registry's view of the tools the manager's own state rather
/// than a copy that can drift.
pub struct McpClientManager {
    state: RwLock<State>,
    secrets: RwLock<BTreeMap<String, McpServerSecrets>>,
    factory: Arc<dyn TransportFactory>,
    max_result_bytes: AtomicUsize,
}

#[derive(Default)]
struct State {
    /// Bumped by every `replace`. A refresh that was opening a session for an older list discards
    /// it rather than installing a session for settings that no longer exist.
    generation: u64,
    servers: Vec<ServerEntry>,
}

struct ServerEntry {
    config: McpServerConfig,
    state: McpServerState,
    session: Option<Arc<McpSession>>,
    tools: Vec<ToolDescriptor>,
    code: Option<String>,
    message: Option<String>,
}

impl ServerEntry {
    fn fresh(config: McpServerConfig) -> Self {
        let state = if config.enabled {
            McpServerState::Connecting
        } else {
            McpServerState::Disabled
        };
        Self {
            config,
            state,
            session: None,
            tools: Vec::new(),
            code: None,
            message: None,
        }
    }

    fn status(&self) -> McpServerStatus {
        let mut status = McpServerStatus::new(&self.config.id, self.state).with_tools(self.tools.len());
        if self.state == McpServerState::Failed
            && let (Some(code), Some(message)) = (&self.code, &self.message)
        {
            status = status.with_failure(code.clone(), message.clone());
        }
        status
    }
}

impl McpClientManager {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(State::default()),
            secrets: RwLock::new(BTreeMap::new()),
            factory: Arc::new(DefaultTransportFactory),
            max_result_bytes: AtomicUsize::new(
                usize::try_from(sevenshell_api::DEFAULT_MAX_RESULT_BYTES).unwrap_or(usize::MAX),
            ),
        }
    }

    /// A manager whose answers are bounded at the settings page's value.
    pub fn with_max_result_bytes(self, max_result_bytes: usize) -> Self {
        self.set_max_result_bytes(max_result_bytes);
        self
    }

    /// Moves the bound for every call made after this moment.
    ///
    /// Atomic rather than one field behind the state lock: a call reads it once, without taking a
    /// write lock that a refresh may be holding, and a settings save changes it while calls run.
    pub fn set_max_result_bytes(&self, max_result_bytes: usize) {
        self.max_result_bytes.store(max_result_bytes.max(1), Ordering::Relaxed);
    }

    /// A manager whose transports come from somewhere other than [`crate::connect`].
    ///
    /// The seam a test uses to script every server without a child process, and the one a host
    /// would use if it ever wanted to route a transport through a tunnel of its own.
    pub fn with_transport_factory(mut self, factory: Arc<dyn TransportFactory>) -> Self {
        self.factory = factory;
        self
    }

    /// A manager for a configured list, refusing a list the settings page could not have produced.
    pub fn from_servers(servers: Vec<McpServerConfig>) -> Result<Self, McpClientError> {
        let manager = Self::new();
        manager.replace(servers)?;
        Ok(manager)
    }

    /// Replaces the configured list with the one the settings file now holds.
    ///
    /// Two servers may not share an id: the id names the vault entry their secrets are filed under
    /// and the half of every tool name that says which server answers, so a duplicate would make a
    /// call's secrets depend on list order. An id that cannot be read back out of a tool name is
    /// refused for the same reason. Both are configuration errors, which is why they are errors
    /// rather than entries quietly dropped: the person who saved the form is still looking at it.
    ///
    /// An entry whose settings did not change keeps its session and its tool list, so saving a page
    /// that changed one server does not reconnect the others. Everything else starts again at
    /// `connecting` (or `disabled`) and is reopened by the next refresh.
    pub fn replace(&self, servers: Vec<McpServerConfig>) -> Result<(), McpClientError> {
        let mut validated = Vec::with_capacity(servers.len());
        let mut seen = BTreeSet::new();

        for server in servers {
            let server = validate_server(&server)?;
            if !is_valid_server_id(&server.id) {
                return Err(McpClientError::InvalidServerId(server.id));
            }
            if !seen.insert(server.id.clone()) {
                return Err(McpClientError::DuplicateServer { id: server.id });
            }
            validated.push(server);
        }

        let mut state = self.write_state();
        let mut previous = std::mem::take(&mut state.servers);
        state.generation += 1;
        state.servers = validated
            .into_iter()
            .map(
                |config| match previous.iter().position(|entry| entry.config == config) {
                    Some(index) => previous.remove(index),
                    None => ServerEntry::fresh(config),
                },
            )
            .collect();
        // Whatever is left in `previous` was removed or changed. Letting an entry go ends a session
        // no call is using — the child is killed or the stream stops — while a call still holding
        // the session finishes on it and lets the transport end afterwards.
        Ok(())
    }

    /// Hands the manager the secrets the vault holds, by server id.
    ///
    /// They take effect at the next open: the host saves them and refreshes the server it saved
    /// them for. Nothing here reads or prints a value.
    pub fn set_secrets(&self, secrets: BTreeMap<String, McpServerSecrets>) {
        *self.write_secrets() = secrets;
    }

    pub fn configured(&self) -> Vec<McpServerConfig> {
        self.read_state()
            .servers
            .iter()
            .map(|entry| entry.config.clone())
            .collect()
    }

    /// One row per configured server.
    ///
    /// A server that is switched on is `connecting` until a session exists for it; a server that
    /// failed keeps its code and message until the next successful refresh. Both are honest answers
    /// before the first handshake, which is why there is no "unknown" state here.
    pub fn statuses(&self) -> Vec<McpServerStatus> {
        self.read_state().servers.iter().map(ServerEntry::status).collect()
    }

    /// Every tool of every server that has a session right now.
    pub fn tools(&self) -> Vec<ToolDescriptor> {
        let state = self.read_state();
        state
            .servers
            .iter()
            .filter(|entry| entry.session.is_some())
            .flat_map(|entry| entry.tools.iter().cloned())
            .collect()
    }

    /// Opens (or reopens) one server's session and answers with its new state.
    ///
    /// A call that was in flight on the previous session is never replayed: a refresh exists to
    /// rebuild the connection, not to guess whether a failed call had already done something. The
    /// old session is let go rather than closed, so such a call can finish and report its own
    /// result; the child process or event stream behind it ends when the last caller lets go.
    pub async fn refresh(&self, id: &str) -> Result<McpServerStatus, McpClientError> {
        let (config, secrets, generation) = {
            let state = self.read_state();
            let entry = state
                .servers
                .iter()
                .find(|entry| entry.config.id == id)
                .ok_or_else(|| McpClientError::UnknownServer { id: id.to_owned() })?;
            if !entry.config.enabled {
                return Ok(entry.status());
            }
            (entry.config.clone(), self.secrets_for(id), state.generation)
        };

        let mut opened = Some(self.open(config, secrets).await);

        let mut old_session = None;
        let mut failure = None;
        let mut installed = None;

        {
            let mut state = self.write_state();
            // A configuration change (or a removal) while this was opening means the session
            // belongs to settings that are gone. It is closed below rather than installed.
            if state.generation == generation
                && let Some(entry) = state.servers.iter_mut().find(|entry| entry.config.id == id)
            {
                match opened.take() {
                    Some(Ok(session)) => {
                        let tools = session.tools();
                        let session = Arc::new(session);
                        old_session = entry.session.replace(session);
                        entry.tools = tools;
                        entry.state = McpServerState::Ready;
                        entry.code = None;
                        entry.message = None;
                        installed = Some(entry.status());
                    }
                    Some(Err(error)) => {
                        old_session = entry.session.take();
                        entry.tools.clear();
                        entry.state = McpServerState::Failed;
                        entry.code = Some(error.code().to_owned());
                        entry.message = Some(failure_message(&error));
                        installed = Some(entry.status());
                        failure = Some(error);
                    }
                    None => unreachable!("the opened session was taken at most once"),
                }
            }
        }

        // An old session is let go, not closed. A call that is still in flight holds its own
        // reference to it and will report its own result — closing it here would kill a child
        // process in the middle of a tool that may already have done something, and the caller
        // would have no way to learn what. The transport ends when the last caller lets go.
        drop(old_session);
        if let Some(Ok(session)) = opened.take() {
            let _ = session.close().await;
        }

        match failure {
            Some(error) => Err(error),
            None => match installed {
                Some(status) => Ok(status),
                None => self
                    .status_of(id)
                    .ok_or_else(|| McpClientError::UnknownServer { id: id.to_owned() }),
            },
        }
    }

    /// Refreshes every enabled server, one after another, and answers with the resulting rows.
    ///
    /// Sequential rather than concurrent on purpose: each step is already bounded by the handshake
    /// timeout, and the host's supervisor is the place that decides how many servers to try at
    /// once. A failure is recorded in the row and does not stop the next server.
    pub async fn refresh_all(&self) -> Vec<McpServerStatus> {
        let ids: Vec<String> = {
            let state = self.read_state();
            state
                .servers
                .iter()
                .filter(|entry| entry.config.enabled)
                .map(|entry| entry.config.id.clone())
                .collect()
        };

        for id in ids {
            let _ = self.refresh(&id).await;
        }

        self.statuses()
    }

    /// Asks one server whether its session is still there.
    ///
    /// A session that does not answer is marked `failed` and let go, exactly as a failed refresh
    /// would be; when to try again is the host's supervisor's decision. A server with no session is
    /// not an error — there is nothing connected to ask. The session is dropped after the state
    /// lock is released, because a call still holding it may need to finish on its own.
    pub async fn ping(&self, id: &str) -> Result<(), McpClientError> {
        let (session, generation) = {
            let state = self.read_state();
            let entry = state
                .servers
                .iter()
                .find(|entry| entry.config.id == id)
                .ok_or_else(|| McpClientError::UnknownServer { id: id.to_owned() })?;
            (entry.session.as_ref().map(Arc::clone), state.generation)
        };
        let Some(session) = session else {
            return Ok(());
        };

        let result = session.ping().await;
        if let Err(error) = &result {
            let stale = {
                let mut state = self.write_state();
                if state.generation == generation
                    && let Some(entry) = state.servers.iter_mut().find(|entry| entry.config.id == id)
                    && entry
                        .session
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, &session))
                {
                    let stale = entry.session.take();
                    entry.tools.clear();
                    entry.state = McpServerState::Failed;
                    entry.code = Some(error.code().to_owned());
                    entry.message = Some(failure_message(error));
                    stale
                } else {
                    None
                }
            };
            drop(stale);
        }
        result
    }

    /// Ends every session and keeps every configured server.
    ///
    /// The host calls this when the vault locks: a session's environment or headers came from the
    /// vault, and the window has just said it may no longer use them. Rows go back to `connecting`
    /// (or `disabled`), and the next `refresh_all` opens them again once the vault is open. A call
    /// that is in flight is ended — this is precisely the moment the host has decided it must be.
    pub async fn disconnect_all(&self) {
        let sessions: Vec<Arc<McpSession>> = {
            let mut state = self.write_state();
            // A refresh that read the secrets a moment before the lock must not install its session
            // after this point. Bumping the generation is what makes that refresh discard and close
            // what it opened, the same way a changed list does.
            state.generation += 1;
            state
                .servers
                .iter_mut()
                .filter_map(|entry| {
                    entry.tools.clear();
                    entry.code = None;
                    entry.message = None;
                    entry.state = if entry.config.enabled {
                        McpServerState::Connecting
                    } else {
                        McpServerState::Disabled
                    };
                    entry.session.take()
                })
                .collect()
        };

        for session in sessions {
            let _ = session.close().await;
        }
    }

    /// The manager as the registry's one dynamic source.
    ///
    /// The same `Arc` the host supervises is the one the registry reads, so a refresh is visible to
    /// the next `tools/list` without anything being copied into a second table.
    pub fn source(self: Arc<Self>) -> Arc<dyn ToolSource> {
        self
    }

    pub fn status_of(&self, id: &str) -> Option<McpServerStatus> {
        self.read_state()
            .servers
            .iter()
            .find(|entry| entry.config.id == id)
            .map(ServerEntry::status)
    }

    async fn open(
        &self,
        config: McpServerConfig,
        secrets: Option<McpServerSecrets>,
    ) -> Result<McpSession, McpClientError> {
        let transport = self.factory.open(&config, secrets).await?;
        McpSession::open(config, transport).await
    }

    fn secrets_for(&self, id: &str) -> Option<McpServerSecrets> {
        self.read_secrets().get(id).cloned()
    }

    fn read_state(&self) -> std::sync::RwLockReadGuard<'_, State> {
        match self.state.read() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn write_state(&self) -> std::sync::RwLockWriteGuard<'_, State> {
        match self.state.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn read_secrets(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<String, McpServerSecrets>> {
        match self.secrets.read() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn write_secrets(&self) -> std::sync::RwLockWriteGuard<'_, BTreeMap<String, McpServerSecrets>> {
        match self.secrets.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Default for McpClientManager {
    fn default() -> Self {
        Self::new()
    }
}

/// What a row says about a failure: the code is the interface's to localize, the message is this
/// layer's own sentence. The message is the error's detail when it has one — already scrubbed and
/// cut by the transport — and the error's own name otherwise.
fn failure_message(error: &McpClientError) -> String {
    error.detail().map(str::to_owned).unwrap_or_else(|| error.to_string())
}

#[async_trait]
impl ToolSource for McpClientManager {
    fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.tools()
    }

    async fn call(&self, name: &str, context: &ToolContext, arguments: Value) -> Option<Result<ToolResult, ToolError>> {
        // A name that is not `mcp__server__tool` is not this source's. `None` is how a name is
        // looked for across sources without any of them knowing about the others.
        let (server, tool) = split_tool_name(name)?;

        let (session, descriptor) = {
            let state = self.read_state();
            let entry = state
                .servers
                .iter()
                .find(|entry| entry.config.id == server && entry.session.is_some())?;
            let descriptor = entry.tools.iter().find(|descriptor| descriptor.name == name)?.clone();
            (Arc::clone(entry.session.as_ref()?), descriptor)
        };

        let external = ExternalTool::new(session, server, tool, descriptor)
            .with_result_limit(self.max_result_bytes.load(Ordering::Relaxed));
        Some(external.call(context, arguments).await)
    }
}

/// The configured servers and their states, never their secrets.
impl fmt::Debug for McpClientManager {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.read_state();
        let rows: Vec<(&str, McpServerState, usize)> = state
            .servers
            .iter()
            .map(|entry| (entry.config.id.as_str(), entry.state, entry.tools.len()))
            .collect();
        formatter
            .debug_struct("McpClientManager")
            .field("servers", &rows)
            .field("max_result_bytes", &self.max_result_bytes.load(Ordering::Relaxed))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use serde_json::json;
    use sevenshell_api::{ToolAccess, ToolRegistry, scope_allows};
    use sevenshell_types::{McpScope, McpServerTransport};

    use crate::testing::{
        FakeFactory, FakeTransport, context_denying, initialize_reply, read_tool_listing, text_result, tool_listing,
        tools_reply,
    };

    fn server(id: &str, enabled: bool) -> McpServerConfig {
        let mut server = McpServerConfig::new(id, id, McpServerTransport::Stdio);
        server.command = Some("unused".to_owned());
        server.enabled = enabled;
        server
    }

    fn healthy(id: &str, tools: &[&str]) -> Arc<FakeTransport> {
        let listings: Vec<Value> = tools.iter().map(|name| read_tool_listing(name, "")).collect();
        let mut transport = FakeTransport::new()
            .on("initialize", initialize_reply(crate::session::LATEST_PROTOCOL_VERSION))
            .on("tools/list", tools_reply(&listings));
        for name in tools {
            transport = transport.on("tools/call", text_result(&format!("{id}:{name}")));
        }
        Arc::new(transport)
    }

    fn manager(factory: Arc<FakeFactory>) -> Arc<McpClientManager> {
        Arc::new(McpClientManager::new().with_transport_factory(factory))
    }

    #[test]
    fn a_list_with_one_id_twice_is_refused_rather_than_resolved_by_order() {
        let manager = McpClientManager::new();
        let error = manager
            .replace(vec![server("files", true), server("files", false)])
            .expect_err("two servers cannot answer to one id");

        assert_eq!(error.code(), "MCP-SERVER-DUPLICATE");
        assert!(manager.configured().is_empty(), "a refused list is not half applied");
    }

    #[test]
    fn an_id_that_could_not_be_read_back_out_of_a_tool_name_is_refused() {
        let manager = McpClientManager::new();
        let error = manager
            .replace(vec![server("two__parts", true)])
            .expect_err("`__` is the tool name's separator");
        assert_eq!(error.code(), "MCP-INVALID-ARGUMENTS");
    }

    #[test]
    fn an_entry_with_no_transport_address_is_refused_rather_than_left_connecting() {
        let manager = McpClientManager::new();
        let http = McpServerConfig::new("tickets", "Tickets", McpServerTransport::Http);
        let error = manager.replace(vec![http]).expect_err("no address");
        assert_eq!(error.code(), "MCP-INVALID-ARGUMENTS");
        assert!(manager.configured().is_empty());
    }

    #[test]
    fn each_configured_server_has_a_row_and_a_switched_off_one_is_disabled() {
        let manager = McpClientManager::new();
        manager
            .replace(vec![server("files", true), server("tickets", false)])
            .expect("two distinct ids");

        let statuses = manager.statuses();
        assert_eq!(statuses.len(), 2);
        assert_eq!(statuses[0].state, McpServerState::Connecting);
        assert_eq!(statuses[1].state, McpServerState::Disabled);
        assert_eq!(manager.configured()[0].id, "files");
    }

    #[tokio::test]
    async fn refreshing_something_not_configured_says_which_id_was_asked_for() {
        let manager = McpClientManager::new();
        let error = manager.refresh("absent").await.expect_err("not configured");
        assert_eq!(error.code(), "MCP-SERVER-NOT-FOUND");
        assert!(error.to_string().contains("absent"));
    }

    #[tokio::test]
    async fn a_disabled_server_is_not_opened_at_all() {
        let factory = FakeFactory::new();
        let manager = manager(Arc::clone(&factory));
        manager.replace(vec![server("files", false)]).expect("a valid list");

        let status = manager.refresh("files").await.expect("disabled, not failed");
        assert_eq!(status.state, McpServerState::Disabled);
        assert!(factory.opens().is_empty(), "nothing was attempted");
    }

    #[tokio::test]
    async fn one_server_failing_leaves_the_other_ready_and_usable() {
        let factory = FakeFactory::new()
            .answering("files", healthy("files", &["read"]))
            .failing(
                "tickets",
                McpClientError::ConnectFailed {
                    server: "tickets".to_owned(),
                    detail: "connection refused".to_owned(),
                },
            );
        let manager = manager(Arc::clone(&factory));
        manager
            .replace(vec![server("files", true), server("tickets", true)])
            .expect("a valid list");

        let statuses = manager.refresh_all().await;
        assert_eq!(statuses.len(), 2);

        let files = statuses
            .iter()
            .find(|status| status.server_id == "files")
            .expect("a row");
        assert_eq!(files.state, McpServerState::Ready);
        assert_eq!(files.tools, 1);

        let tickets = statuses
            .iter()
            .find(|status| status.server_id == "tickets")
            .expect("a row");
        assert_eq!(tickets.state, McpServerState::Failed);
        assert_eq!(tickets.code.as_deref(), Some("MCP-SERVER-CONNECT-FAILED"));
        assert_eq!(tickets.message.as_deref(), Some("connection refused"));

        // The healthy server's tool is still callable through the one table.
        let source = Arc::clone(&manager).source();
        let (_directory, context) = context_denying(McpScope::ReadOnly);
        let result = source
            .call("mcp__files__read", &context, json!({}))
            .await
            .expect("this source knows the name")
            .expect("the call ran");
        assert_eq!(result.value, json!("files:read"));
    }

    #[tokio::test]
    async fn a_failed_refresh_is_reported_as_an_error_and_keeps_the_row_saying_so() {
        let factory = FakeFactory::new().failing(
            "files",
            McpClientError::ConnectFailed {
                server: "files".to_owned(),
                detail: "the server refused the handshake".to_owned(),
            },
        );
        let manager = manager(factory);
        manager.replace(vec![server("files", true)]).expect("a valid list");

        let error = manager.refresh("files").await.expect_err("the open failed");
        assert_eq!(error.code(), "MCP-SERVER-CONNECT-FAILED");

        let status = manager.status_of("files").expect("a row");
        assert_eq!(status.state, McpServerState::Failed);
        assert_eq!(status.code.as_deref(), Some("MCP-SERVER-CONNECT-FAILED"));
    }

    #[tokio::test]
    async fn a_refresh_reopens_the_session_and_replaces_the_tool_list() {
        let factory = FakeFactory::new().answering(
            "files",
            Arc::new(
                FakeTransport::new()
                    .on("initialize", initialize_reply(crate::session::LATEST_PROTOCOL_VERSION))
                    .on("tools/list", tools_reply(&[tool_listing("read", "")]))
                    .on("initialize", initialize_reply(crate::session::LATEST_PROTOCOL_VERSION))
                    .on(
                        "tools/list",
                        tools_reply(&[tool_listing("read", ""), tool_listing("write", "")]),
                    ),
            ),
        );
        let manager = manager(Arc::clone(&factory));
        manager.replace(vec![server("files", true)]).expect("a valid list");

        manager.refresh("files").await.expect("a session");
        assert_eq!(manager.tools().len(), 1);

        manager.refresh("files").await.expect("a second session");
        assert_eq!(manager.tools().len(), 2);
        assert_eq!(factory.opened("files"), 2, "a refresh opens, it does not reuse");
    }

    #[tokio::test]
    async fn saving_the_same_list_keeps_the_session_and_saving_a_changed_one_reconnects() {
        let factory = FakeFactory::new().answering(
            "files",
            Arc::new(
                FakeTransport::new()
                    .on("initialize", initialize_reply(crate::session::LATEST_PROTOCOL_VERSION))
                    .on("tools/list", tools_reply(&[tool_listing("read", "")])),
            ),
        );
        let manager = manager(Arc::clone(&factory));
        let config = server("files", true);
        manager.replace(vec![config.clone()]).expect("a valid list");
        manager.refresh("files").await.expect("a session");
        assert_eq!(manager.statuses()[0].state, McpServerState::Ready);

        manager.replace(vec![config.clone()]).expect("the same list again");
        assert_eq!(
            manager.statuses()[0].state,
            McpServerState::Ready,
            "an unchanged entry keeps its session"
        );
        assert_eq!(factory.opened("files"), 1);

        let mut changed = config;
        changed.tool_timeout_seconds = 120;
        manager.replace(vec![changed]).expect("a changed list");
        assert_eq!(
            manager.statuses()[0].state,
            McpServerState::Connecting,
            "a changed entry is opened again"
        );
    }

    #[tokio::test]
    async fn the_secrets_for_a_server_reach_its_session_and_nowhere_else() {
        let factory = FakeFactory::new().answering(
            "files",
            Arc::new(
                FakeTransport::new()
                    .on("initialize", initialize_reply(crate::session::LATEST_PROTOCOL_VERSION))
                    .on("tools/list", tools_reply(&[])),
            ),
        );
        let manager = manager(Arc::clone(&factory));
        manager.replace(vec![server("files", true)]).expect("a valid list");

        let mut secrets = McpServerSecrets::default();
        secrets.env.insert("TOKEN".to_owned(), "hunter2".to_owned());
        manager.set_secrets(BTreeMap::from([("files".to_owned(), secrets)]));

        manager.refresh("files").await.expect("a session");

        let opens = factory.opens();
        assert_eq!(opens.len(), 1);
        assert_eq!(
            opens[0]
                .1
                .as_ref()
                .and_then(|secrets| secrets.env.get("TOKEN"))
                .map(String::as_str),
            Some("hunter2"),
            "the vault's value is handed to the transport that starts the child"
        );
        assert!(
            !format!("{manager:?}").contains("hunter2"),
            "a debug line never prints it"
        );
    }

    #[tokio::test]
    async fn a_call_is_routed_by_its_name_and_an_unknown_name_is_not_this_source_s() {
        let factory = FakeFactory::new().answering("files", healthy("files", &["read"]));
        let manager = manager(factory);
        manager.replace(vec![server("files", true)]).expect("a valid list");
        manager.refresh("files").await.expect("a session");

        let source = Arc::clone(&manager).source();
        let (_directory, context) = context_denying(McpScope::ReadOnly);

        assert!(
            source.call("sshell_sessions_list", &context, json!({})).await.is_none(),
            "a built-in name is not this source's"
        );
        assert!(
            source.call("mcp__absent__read", &context, json!({})).await.is_none(),
            "a server that is not configured has no tools"
        );
        let answered = source
            .call("mcp__files__read", &context, json!({}))
            .await
            .expect("this source knows the name")
            .expect("the call ran");
        assert_eq!(answered.value, json!("files:read"));
    }

    #[tokio::test]
    async fn the_registry_sees_external_tools_through_the_same_source_and_scope_gate() {
        let transport = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(crate::session::LATEST_PROTOCOL_VERSION))
                .on(
                    "tools/list",
                    tools_reply(&[read_tool_listing("read", ""), tool_listing("write", "")]),
                ),
        );
        let factory = FakeFactory::new().answering("files", transport);
        let manager = manager(factory);
        manager.replace(vec![server("files", true)]).expect("a valid list");
        manager.refresh("files").await.expect("a session");

        let registry = ToolRegistry::new();
        registry.add_source(Arc::clone(&manager).source());
        assert_eq!(registry.len(), 2);
        assert_eq!(manager.tools().len(), 2);

        let write = manager
            .tools()
            .into_iter()
            .find(|descriptor| descriptor.name == "mcp__files__write")
            .expect("the write tool");
        assert_eq!(
            write.access,
            ToolAccess::Write,
            "a tool with no readOnlyHint is a write"
        );
        assert!(!scope_allows(McpScope::ReadOnly, write.access));
    }

    #[tokio::test]
    async fn a_failed_call_is_attempted_once_and_never_replayed() {
        let transport = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(crate::session::LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[read_tool_listing("read", "")]))
                .on(
                    "tools/call",
                    json!({ "content": [{ "type": "text", "text": "the server failed" }], "isError": true }),
                ),
        );
        let factory = FakeFactory::new().answering("files", Arc::clone(&transport));
        let manager = manager(Arc::clone(&factory));
        manager.replace(vec![server("files", true)]).expect("a valid list");
        manager.refresh("files").await.expect("a session");

        let source = Arc::clone(&manager).source();
        let (_directory, context) = context_denying(McpScope::ReadOnly);
        let error = source
            .call("mcp__files__read", &context, json!({}))
            .await
            .expect("this source knows the name")
            .expect_err("the tool failed");

        assert_eq!(error.code(), "MCP-EXTERNAL-FAILED");
        assert_eq!(transport.calls("tools/call").len(), 1, "one attempt, no retry");
        assert_eq!(factory.opened("files"), 1, "a failed call does not reopen the session");
    }

    #[tokio::test]
    async fn a_call_after_the_server_went_away_fails_without_a_reconnect() {
        let transport = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(crate::session::LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[read_tool_listing("read", "")]))
                .on_error(
                    "tools/call",
                    McpClientError::Failed {
                        server: "files".to_owned(),
                        message: "the server is not running".to_owned(),
                    },
                ),
        );
        let factory = FakeFactory::new().answering("files", transport);
        let manager = manager(Arc::clone(&factory));
        manager.replace(vec![server("files", true)]).expect("a valid list");
        manager.refresh("files").await.expect("a session");

        let source = Arc::clone(&manager).source();
        let (_directory, context) = context_denying(McpScope::ReadOnly);
        source
            .call("mcp__files__read", &context, json!({}))
            .await
            .expect("the name is known")
            .expect_err("the call failed");
        assert_eq!(
            factory.opened("files"),
            1,
            "the manager is not retrying behind the caller"
        );
    }

    #[tokio::test]
    async fn a_tool_bounded_by_the_manager_uses_the_configured_limit() {
        let transport = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(crate::session::LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[read_tool_listing("read", "")]))
                .on("tools/call", text_result(&"x".repeat(200)))
                .on("tools/call", text_result(&"x".repeat(200))),
        );
        let factory = FakeFactory::new().answering("files", transport);
        let manager = Arc::new(
            McpClientManager::new()
                .with_transport_factory(factory)
                .with_max_result_bytes(32),
        );
        manager.replace(vec![server("files", true)]).expect("a valid list");
        manager.refresh("files").await.expect("a session");

        let source = Arc::clone(&manager).source();
        let (_directory, context) = context_denying(McpScope::ReadOnly);
        let result = source
            .call("mcp__files__read", &context, json!({}))
            .await
            .expect("the name is known")
            .expect("the call ran");

        assert_eq!(result.value["truncated"], json!(true));
        assert_eq!(result.value["originalBytes"], json!(200));

        // A settings save moves the bound for the next call without reopening the session.
        manager.set_max_result_bytes(1024);
        let full = source
            .call("mcp__files__read", &context, json!({}))
            .await
            .expect("the name is known")
            .expect("the call ran");
        assert_eq!(full.value.as_str().map(str::len), Some(200), "the whole answer");
    }

    #[tokio::test]
    async fn a_session_that_finishes_opening_after_the_vault_locked_is_closed_not_installed() {
        let transport = Arc::new(
            FakeTransport::new()
                .on_delay(
                    "initialize",
                    Duration::from_millis(50),
                    initialize_reply(crate::session::LATEST_PROTOCOL_VERSION),
                )
                .on("tools/list", tools_reply(&[read_tool_listing("read", "")])),
        );
        let factory = FakeFactory::new().answering("files", Arc::clone(&transport));
        let manager = manager(factory);
        manager.replace(vec![server("files", true)]).expect("a valid list");

        let opening = {
            let manager = Arc::clone(&manager);
            tokio::spawn(async move { manager.refresh("files").await })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        manager.disconnect_all().await;

        let status = opening
            .await
            .expect("the task ran")
            .expect("the open finished; it was only too late to install");
        assert_eq!(
            status.state,
            McpServerState::Connecting,
            "the lock kept the row from being drawn as connected"
        );
        assert!(manager.tools().is_empty(), "no tool of a locked vault is offered");
        assert!(
            transport.is_closed(),
            "the session that was opened across the lock was closed"
        );
    }

    #[tokio::test]
    async fn refreshing_a_server_does_not_replay_a_call_that_was_in_flight() {
        let transport = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(crate::session::LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[read_tool_listing("read", "")]))
                .on_delay("tools/call", Duration::from_secs(5), text_result("late"))
                .on("initialize", initialize_reply(crate::session::LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[read_tool_listing("read", "")])),
        );
        let factory = FakeFactory::new().answering("files", Arc::clone(&transport));
        let manager = manager(factory);
        manager.replace(vec![server("files", true)]).expect("a valid list");
        manager.refresh("files").await.expect("a session");

        let source = Arc::clone(&manager).source();
        let (_directory, context) = context_denying(McpScope::ReadOnly);
        let mut call = Box::pin(source.call("mcp__files__read", &context, json!({})));

        // Leave the call waiting, rebuild the session, then let the caller give up on the old one.
        let _ = tokio::time::timeout(Duration::from_millis(20), &mut call).await;
        manager.refresh("files").await.expect("a rebuilt session");
        drop(call);

        assert_eq!(
            transport.calls("tools/call").len(),
            1,
            "a rebuilt session does not replay what was in flight"
        );
    }

    #[tokio::test]
    async fn disconnecting_ends_the_sessions_and_keeps_the_configuration() {
        let transport = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(crate::session::LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[read_tool_listing("read", "")]))
                .on("initialize", initialize_reply(crate::session::LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[read_tool_listing("read", "")])),
        );
        let factory = FakeFactory::new().answering("files", Arc::clone(&transport));
        let manager = manager(Arc::clone(&factory));
        manager.replace(vec![server("files", true)]).expect("a valid list");
        manager.refresh("files").await.expect("a session");
        assert_eq!(manager.tools().len(), 1);

        manager.disconnect_all().await;

        assert!(transport.is_closed(), "the session's transport was ended");
        assert!(manager.tools().is_empty(), "a closed session offers no tools");
        assert_eq!(manager.statuses()[0].state, McpServerState::Connecting);
        assert_eq!(manager.configured().len(), 1, "the server is still configured");

        manager.refresh("files").await.expect("a session again");
        assert_eq!(manager.tools().len(), 1);
        assert_eq!(factory.opened("files"), 2);
    }

    #[tokio::test]
    async fn debug_prints_server_ids_and_states_rather_than_any_settings() {
        let manager = McpClientManager::new();
        manager.replace(vec![server("files", true)]).expect("a valid list");
        let printed = format!("{manager:?}");
        assert!(printed.contains("files"));
        assert!(printed.contains("Connecting"));
    }
}
