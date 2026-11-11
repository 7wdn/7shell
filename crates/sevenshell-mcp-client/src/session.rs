//! One server's session: the handshake, the cached tool list, and the calls that go through it.
//!
//! A session is what the manager holds per server. It is created once per successful handshake and
//! its tool list is replaced when the server says it changed (or when a person refreshes), never
//! per call, because a list that changed between the moment an agent was shown a tool and the
//! moment it called one is a race no caller can see. A server whose session failed is rebuilt from
//! scratch; unfinished calls are never replayed, because a tool may have had a side effect the
//! first time.
//!
//! Two version numbers are spoken. The handshake asks for [`LATEST_PROTOCOL_VERSION`], and a server
//! that only knows the revision before it is accepted rather than refused: the tools surface this
//! build uses is the same in both. A third revision is a refusal, because this build cannot know
//! what changed.
//!
//! The budgets are the protocol's own numbers: fifteen seconds for the handshake and for the list,
//! and the server's configured budget (five seconds to ten minutes) for one call.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde_json::{Value, json};
use sevenshell_api::ToolDescriptor;
use sevenshell_types::{McpServerConfig, McpServerState, McpServerStatus};
use tokio::time::timeout;

use crate::McpClientError;
use crate::adapter::descriptor_for;
use crate::config::{McpServerSecrets, tool_timeout, validate_server};
use crate::transport::{self, CONNECT_TIMEOUT, LIST_TIMEOUT, McpTransport};

/// The protocol revision the handshake asks for.
pub const LATEST_PROTOCOL_VERSION: &str = "2025-06-18";

/// Every revision this build can go on speaking, newest first.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[LATEST_PROTOCOL_VERSION, "2024-11-05"];

/// How many `tools/list` pages one refresh reads.
///
/// A cursor that never ends would otherwise keep a refresh running until its own timeout, one page
/// at a time. Thirty-two pages is far beyond any real server and still a bound.
const MAX_TOOL_PAGES: usize = 32;

/// The result of one `tools/call`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalCall {
    /// What the agent reads: the text blocks joined, or a JSON object when the answer was not text.
    pub value: Value,
    /// The server said the tool ran and failed. The call itself succeeded, which is why this is a
    /// field rather than an error from here: the adapter turns it into the failure an agent sees.
    pub is_error: bool,
    /// Whether the value is the bounded prefix rather than the whole answer.
    pub truncated: bool,
}

/// One open (or cached) session with an external server.
pub struct McpSession {
    server: McpServerConfig,
    transport: Arc<dyn McpTransport>,
    protocol_version: String,
    tools: Arc<RwLock<Vec<ToolDescriptor>>>,
    call_budget: Duration,
}

impl McpSession {
    /// Opens a session: transport, `initialize`, `initialized`, then the first `tools/list`.
    pub async fn connect(server: McpServerConfig, secrets: Option<McpServerSecrets>) -> Result<Self, McpClientError> {
        let server = validate_server(&server)?;
        let transport = transport::connect(&server, secrets).await?;
        Self::open(server, transport).await
    }

    /// Opens a session over a transport the caller already has.
    ///
    /// The seam a test uses, and the one a host uses when its transport came from somewhere other
    /// than [`transport::connect`]. Everything after the transport is the same either way, which is
    /// what makes a fake transport a real test of the handshake rather than of a stub.
    pub async fn open(server: McpServerConfig, transport: Box<dyn McpTransport>) -> Result<Self, McpClientError> {
        let server = validate_server(&server)?;
        let transport: Arc<dyn McpTransport> = Arc::from(transport);

        let handshake = handshake(&server, transport.as_ref()).await?;
        // The revision is the server's answer, not the request: a server that accepted 2024-11-05
        // must be spoken to as one, which for Streamable HTTP is a header on every later request.
        transport.set_protocol_version(&handshake.version);
        let tools = if handshake.offers_tools {
            fetch_tools(&server, transport.as_ref()).await?
        } else {
            // A server that advertised no tools capability has none to list. Asking anyway would
            // fail the whole session over a request the server never promised to answer.
            Vec::new()
        };

        let tools = Arc::new(RwLock::new(tools));
        start_tool_notification_refresh(&server, Arc::clone(&transport), Arc::clone(&tools));

        Ok(Self {
            call_budget: tool_timeout(&server),
            server,
            transport,
            protocol_version: handshake.version,
            tools,
        })
    }

    pub fn server(&self) -> &McpServerConfig {
        &self.server
    }

    /// The revision the server agreed to, which is the one it answered with.
    pub fn protocol_version(&self) -> &str {
        &self.protocol_version
    }

    /// The tool list as it was cached at the handshake or the last refresh.
    pub fn tools(&self) -> Vec<ToolDescriptor> {
        self.read_tools().clone()
    }

    /// How this server is drawn right now.
    pub fn status(&self) -> McpServerStatus {
        if self.server.enabled {
            McpServerStatus::new(&self.server.id, McpServerState::Ready).with_tools(self.read_tools().len())
        } else {
            McpServerStatus::new(&self.server.id, McpServerState::Disabled)
        }
    }

    /// Asks the server for its tools again and replaces the cached list.
    ///
    /// A refresh that fails leaves the previous list in place: the server's tools did not become
    /// unknown because one request failed, and a caller that was shown them can still see them.
    pub async fn refresh_tools(&self) -> Result<(), McpClientError> {
        let tools = fetch_tools(&self.server, self.transport.as_ref()).await?;
        *self.write_tools() = tools;
        Ok(())
    }

    /// Asks the server whether it is still there. Optional in the protocol; the host's supervisor
    /// uses it between refreshes rather than opening a new session.
    ///
    /// A server that answers `-32601` has told us it is alive and does not implement the optional
    /// method. Treating that as a dead session would reconnect a healthy server every minute.
    pub async fn ping(&self) -> Result<(), McpClientError> {
        let outcome = timeout(LIST_TIMEOUT, self.transport.request("ping", None))
            .await
            .map_err(|_| McpClientError::Timeout {
                server: self.server.id.clone(),
                seconds: LIST_TIMEOUT.as_secs(),
            })?;
        match outcome {
            Ok(_) => Ok(()),
            Err(McpClientError::Rpc { code: -32601, .. }) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Calls one of this server's tools.
    ///
    /// The result is the server's content, assembled and bounded. Turning it into a
    /// [`sevenshell_api::ToolResult`], the approval and the audit facts are the adapter's job,
    /// because those are the parts that belong to this build rather than to MCP.
    pub async fn call_tool(
        &self,
        tool: &str,
        arguments: Value,
        max_result_bytes: usize,
    ) -> Result<ExternalCall, McpClientError> {
        let arguments = match arguments {
            Value::Null => json!({}),
            arguments if arguments.is_object() => arguments,
            _ => {
                return Err(McpClientError::Failed {
                    server: self.server.id.clone(),
                    message: "a tool's arguments have to be a JSON object".to_owned(),
                });
            }
        };

        let params = json!({ "name": tool, "arguments": arguments });
        let seconds = self.call_budget.as_secs().max(1);
        let result = timeout(self.call_budget, self.transport.request("tools/call", Some(params)))
            .await
            .map_err(|_| McpClientError::Timeout {
                server: self.server.id.clone(),
                seconds,
            })??;

        read_call(&self.server.id, &result, max_result_bytes)
    }

    /// Ends the session now: the child's input is closed, the event stream is stopped, or the HTTP
    /// session is deleted.
    ///
    /// Dropping a session has the same effect when its last reference goes; this is for a host
    /// that wants the child process gone before it reports the server as stopped.
    pub async fn close(&self) -> Result<(), McpClientError> {
        self.transport.close().await
    }

    /// A shorter or longer budget than the configuration's, for a test.
    #[cfg(test)]
    pub(crate) fn with_call_budget(mut self, budget: Duration) -> Self {
        self.call_budget = budget;
        self
    }

    fn read_tools(&self) -> std::sync::RwLockReadGuard<'_, Vec<ToolDescriptor>> {
        match self.tools.read() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn write_tools(&self) -> std::sync::RwLockWriteGuard<'_, Vec<ToolDescriptor>> {
        match self.tools.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// Prints what a log line may carry: the server, the revision and how many tools were learned.
/// The transport is not reachable from here, so no header or environment value can be printed.
impl fmt::Debug for McpSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tools = self.read_tools().len();
        formatter
            .debug_struct("McpSession")
            .field("server", &self.server.id)
            .field("protocol_version", &self.protocol_version)
            .field("tools", &tools)
            .field("call_budget", &self.call_budget)
            .finish()
    }
}

struct Handshake {
    version: String,
    offers_tools: bool,
}

async fn handshake(server: &McpServerConfig, transport: &dyn McpTransport) -> Result<Handshake, McpClientError> {
    let params = json!({
        "protocolVersion": LATEST_PROTOCOL_VERSION,
        "capabilities": {},
        "clientInfo": { "name": "7shell", "version": env!("CARGO_PKG_VERSION") },
    });

    let result = timeout(CONNECT_TIMEOUT, transport.request("initialize", Some(params)))
        .await
        .map_err(|_| connect_failed_detail(server, "the server did not complete the handshake within 15 seconds"))?
        .map_err(|error| connect_failed(server, error))?;

    let version = result
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !SUPPORTED_PROTOCOL_VERSIONS.contains(&version) {
        return Err(connect_failed_detail(
            server,
            &format!("the server answered protocol version `{version}`, which this build does not speak"),
        ));
    }

    // A server that advertises no tools capability is one whose tool list would be a request it
    // never promised to answer.
    let offers_tools = result
        .get("capabilities")
        .and_then(|capabilities| capabilities.get("tools"))
        .is_some();

    timeout(CONNECT_TIMEOUT, transport.notify("notifications/initialized", None))
        .await
        .map_err(|_| connect_failed_detail(server, "the server did not accept the handshake's last step"))?
        .map_err(|error| connect_failed(server, error))?;

    Ok(Handshake {
        version: version.to_owned(),
        offers_tools,
    })
}

/// Reads every page of `tools/list`.
///
/// A duplicate name is refused rather than kept twice: two descriptors with one name would make a
/// call's target depend on which one the caller happened to hold, and the registry's one-name-one-
/// tool rule cannot be enforced on a list that already broke it.
async fn fetch_tools(
    server: &McpServerConfig,
    transport: &dyn McpTransport,
) -> Result<Vec<ToolDescriptor>, McpClientError> {
    let mut tools: Vec<ToolDescriptor> = Vec::new();
    let mut names: BTreeSet<String> = BTreeSet::new();
    let mut cursor: Option<String> = None;

    for _ in 0..MAX_TOOL_PAGES {
        let params = cursor.as_ref().map(|cursor| json!({ "cursor": cursor }));
        let result = timeout(LIST_TIMEOUT, transport.request("tools/list", params))
            .await
            .map_err(|_| McpClientError::Timeout {
                server: server.id.clone(),
                seconds: LIST_TIMEOUT.as_secs(),
            })??;

        let listings = result
            .get("tools")
            .and_then(Value::as_array)
            .ok_or_else(|| McpClientError::Failed {
                server: server.id.clone(),
                message: "the server answered tools/list without a list of tools".to_owned(),
            })?;

        for listing in listings {
            let descriptor = descriptor_for(&server.id, listing)?;
            let tool = crate::split_tool_name(&descriptor.name)
                .map(|(_, tool)| tool.to_owned())
                .unwrap_or_else(|| descriptor.name.clone());
            if !names.insert(descriptor.name.clone()) {
                return Err(McpClientError::DuplicateTool {
                    server: server.id.clone(),
                    tool,
                });
            }
            tools.push(descriptor);
        }

        match result
            .get("nextCursor")
            .and_then(Value::as_str)
            .filter(|cursor| !cursor.is_empty())
        {
            Some(next) => cursor = Some(next.to_owned()),
            None => return Ok(tools),
        }
    }

    Err(McpClientError::Failed {
        server: server.id.clone(),
        message: format!("the server's tool list did not end within {MAX_TOOL_PAGES} pages"),
    })
}

/// Keeps the cached list in step with a server that says its tools changed.
///
/// The transport only forwards the notification's name; the fetch happens on this task because a
/// reader must never wait on a request. A failed fetch leaves the previous list in place: the
/// server's tools did not become unknown because one refresh failed. The weak handle to the
/// transport is what lets the task end when the session is gone, rather than keeping it alive from
/// inside its own notification slot.
fn start_tool_notification_refresh(
    server: &McpServerConfig,
    transport: Arc<dyn McpTransport>,
    tools: Arc<RwLock<Vec<ToolDescriptor>>>,
) {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<String>();
    transport.set_notification_handler(Arc::new(move |method: &str| {
        let _ = sender.send(method.to_owned());
    }));

    let server = server.clone();
    let transport = Arc::downgrade(&transport);
    tokio::spawn(async move {
        while let Some(method) = receiver.recv().await {
            if method != "notifications/tools/list_changed" {
                continue;
            }
            let Some(transport) = transport.upgrade() else {
                break;
            };
            if let Ok(refreshed) = fetch_tools(&server, transport.as_ref()).await
                && let Ok(mut guard) = tools.write()
            {
                *guard = refreshed;
            }
        }
    });
}

/// Turns one `tools/call` result into what the adapter hands the agent.
///
/// The protocol's content is a list of typed blocks. This build's readers want either text or
/// structure: an answer that is all text becomes one string (the blocks joined with a newline), and
/// anything else keeps its shape with the text blocks reduced to strings. `structuredContent`, when
/// the server sent it, travels as its own member rather than being flattened into the text — a
/// schema-aware reader is the reason the server sent it.
fn read_call(server: &str, result: &Value, max_result_bytes: usize) -> Result<ExternalCall, McpClientError> {
    let Some(object) = result.as_object() else {
        return Err(McpClientError::Failed {
            server: server.to_owned(),
            message: "tools/call answered something that is not an object".to_owned(),
        });
    };

    let is_error = object.get("isError").and_then(Value::as_bool).unwrap_or(false);
    let structured = object
        .get("structuredContent")
        .filter(|value| !value.is_null())
        .cloned();
    let content = object.get("content").and_then(Value::as_array);

    let value = assemble_content(content, structured);
    let (value, truncated) = bound_result(value, max_result_bytes);

    Ok(ExternalCall {
        value,
        is_error,
        truncated,
    })
}

fn assemble_content(content: Option<&Vec<Value>>, structured: Option<Value>) -> Value {
    let blocks = content.map(Vec::as_slice).unwrap_or(&[]);

    let all_text = !blocks.is_empty()
        && blocks.iter().all(|block| {
            block.get("type").and_then(Value::as_str) == Some("text")
                && block.get("text").and_then(Value::as_str).is_some()
        });

    if all_text && structured.is_none() {
        let joined = blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        return Value::String(joined);
    }

    let mut object = serde_json::Map::new();
    if !blocks.is_empty() {
        let items: Vec<Value> = blocks
            .iter()
            .map(|block| match block.get("text").and_then(Value::as_str) {
                Some(text) if block.get("type").and_then(Value::as_str) == Some("text") => {
                    Value::String(text.to_owned())
                }
                _ => block.clone(),
            })
            .collect();
        object.insert("content".to_owned(), Value::Array(items));
    }
    if let Some(structured) = structured {
        object.insert("structuredContent".to_owned(), structured);
    }
    Value::Object(object)
}

/// Cuts a result to the configured limit, on a character boundary, and says so inside the value.
///
/// The marker is data rather than a sentence, for the reason the bridge's is: an agent that sees
/// `truncated: true` knows what it is holding without a locale. A result that fits is returned
/// unchanged, so the common case has no wrapper around it.
fn bound_result(value: Value, limit: usize) -> (Value, bool) {
    let limit = limit.max(1);

    match &value {
        Value::String(text) if text.len() > limit => {
            let prefix = text_prefix(text, limit);
            (
                json!({ "text": prefix, "truncated": true, "originalBytes": text.len() }),
                true,
            )
        }
        _ => {
            let serialized = serde_json::to_string(&value).unwrap_or_default();
            if serialized.len() <= limit {
                (value, false)
            } else {
                let prefix = text_prefix(&serialized, limit);
                (
                    json!({ "text": prefix, "truncated": true, "originalBytes": serialized.len() }),
                    true,
                )
            }
        }
    }
}

fn text_prefix(text: &str, limit: usize) -> String {
    let mut end = limit.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

fn connect_failed(server: &McpServerConfig, cause: McpClientError) -> McpClientError {
    let detail = cause.detail().map(str::to_owned).unwrap_or_else(|| cause.to_string());
    McpClientError::ConnectFailed {
        server: server.id.clone(),
        detail,
    }
}

fn connect_failed_detail(server: &McpServerConfig, detail: &str) -> McpClientError {
    McpClientError::ConnectFailed {
        server: server.id.clone(),
        detail: detail.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use sevenshell_types::McpServerTransport;

    use crate::testing::{
        FakeTransport, initialize_reply, initialize_reply_without_tools, text_result, tool_listing, tools_reply,
    };

    fn server() -> McpServerConfig {
        let mut server = McpServerConfig::new("files", "Files", McpServerTransport::Stdio);
        server.command = Some("unused".to_owned());
        server
    }

    fn ready() -> Arc<FakeTransport> {
        Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[tool_listing("read", "read a file")])),
        )
    }

    #[tokio::test]
    async fn the_handshake_asks_for_the_newest_revision_and_sends_initialized() {
        let fake = ready();
        let session = McpSession::open(server(), Box::new(Arc::clone(&fake)))
            .await
            .expect("a session");

        let initialize = fake.calls("initialize");
        assert_eq!(initialize.len(), 1);
        assert_eq!(
            initialize[0].as_ref().and_then(|params| params.get("protocolVersion")),
            Some(&json!(LATEST_PROTOCOL_VERSION)),
            "the newest revision this build speaks is what is asked for"
        );
        assert_eq!(fake.calls("notifications/initialized").len(), 1);
        assert_eq!(session.protocol_version(), LATEST_PROTOCOL_VERSION);
        assert_eq!(
            fake.versions(),
            vec![LATEST_PROTOCOL_VERSION.to_owned()],
            "the transport is told which revision the server agreed to"
        );
        assert_eq!(session.tools().len(), 1);
    }

    #[tokio::test]
    async fn a_server_that_only_knows_the_previous_revision_is_accepted_at_its_word() {
        let fake = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply("2024-11-05"))
                .on("tools/list", tools_reply(&[])),
        );
        let session = McpSession::open(server(), Box::new(Arc::clone(&fake)))
            .await
            .expect("the older revision is one this build speaks");
        assert_eq!(session.protocol_version(), "2024-11-05");
        assert_eq!(
            fake.versions(),
            vec!["2024-11-05".to_owned()],
            "a server's own version is the one that travels back to it"
        );
    }

    #[tokio::test]
    async fn a_revision_this_build_does_not_speak_fails_the_session_rather_than_guessing() {
        let fake = Arc::new(FakeTransport::new().on("initialize", initialize_reply("2099-01-01")));
        let error = McpSession::open(server(), Box::new(fake))
            .await
            .expect_err("a third revision is not something this build can assume");

        assert_eq!(error.code(), "MCP-SERVER-CONNECT-FAILED");
        assert!(
            error.detail().is_some_and(|detail| detail.contains("2099-01-01")),
            "the refusal names the version it was offered: {error:?}"
        );
    }

    #[tokio::test]
    async fn a_server_without_the_tools_capability_opens_with_no_list_request() {
        let fake =
            Arc::new(FakeTransport::new().on("initialize", initialize_reply_without_tools(LATEST_PROTOCOL_VERSION)));
        let session = McpSession::open(server(), Box::new(Arc::clone(&fake)))
            .await
            .expect("a session");

        assert!(session.tools().is_empty());
        assert!(
            fake.calls("tools/list").is_empty(),
            "a capability the server did not advertise is not requested"
        );
    }

    #[tokio::test]
    async fn every_page_of_a_tool_list_is_read() {
        let fake = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(LATEST_PROTOCOL_VERSION))
                .on(
                    "tools/list",
                    json!({ "tools": [tool_listing("first", "")], "nextCursor": "page-2" }),
                )
                .on("tools/list", tools_reply(&[tool_listing("second", "")])),
        );
        let session = McpSession::open(server(), Box::new(Arc::clone(&fake)))
            .await
            .expect("a session");

        let names: Vec<String> = session.tools().into_iter().map(|descriptor| descriptor.name).collect();
        assert_eq!(names, vec!["mcp__files__first", "mcp__files__second"]);

        let calls = fake.calls("tools/list");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1], Some(json!({ "cursor": "page-2" })));
    }

    #[tokio::test]
    async fn a_refresh_replaces_the_cached_list_and_a_failure_keeps_the_old_one() {
        let fake = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[tool_listing("read", "")]))
                .on(
                    "tools/list",
                    tools_reply(&[tool_listing("read", ""), tool_listing("write", "")]),
                )
                .on_error(
                    "tools/list",
                    McpClientError::Failed {
                        server: "files".to_owned(),
                        message: "the server refused the list".to_owned(),
                    },
                ),
        );
        let session = McpSession::open(server(), Box::new(Arc::clone(&fake)))
            .await
            .expect("a session");
        assert_eq!(session.tools().len(), 1);

        session.refresh_tools().await.expect("a second list");
        assert_eq!(session.tools().len(), 2);

        session
            .refresh_tools()
            .await
            .expect_err("the third answer is a refusal");
        assert_eq!(
            session.tools().len(),
            2,
            "a failed refresh leaves the list a caller was shown"
        );
    }

    #[tokio::test]
    async fn a_server_without_ping_is_alive_and_a_real_refusal_is_not() {
        let without_ping = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[]))
                .on_error(
                    "ping",
                    McpClientError::Rpc {
                        server: "files".to_owned(),
                        code: -32601,
                        message: "no method is named `ping`".to_owned(),
                    },
                ),
        );
        let session = McpSession::open(server(), Box::new(Arc::clone(&without_ping)))
            .await
            .expect("a session");
        session
            .ping()
            .await
            .expect("a method the server does not implement is still an answer");

        let broken = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[]))
                .on_error(
                    "ping",
                    McpClientError::Rpc {
                        server: "files".to_owned(),
                        code: -32000,
                        message: "the server is shutting down".to_owned(),
                    },
                ),
        );
        let session = McpSession::open(server(), Box::new(Arc::clone(&broken)))
            .await
            .expect("a session");
        let error = session.ping().await.expect_err("a real refusal");
        assert_eq!(error.code(), "MCP-EXTERNAL-FAILED");
        assert_eq!(error.detail(), Some("the server is shutting down"));
    }

    #[tokio::test]
    async fn a_changed_tool_list_is_fetched_when_the_server_says_so() {
        let fake = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[tool_listing("read", "")]))
                .on(
                    "tools/list",
                    tools_reply(&[tool_listing("read", ""), tool_listing("write", "")]),
                ),
        );
        let session = McpSession::open(server(), Box::new(Arc::clone(&fake)))
            .await
            .expect("a session");
        assert_eq!(session.tools().len(), 1);

        fake.emit_notification("notifications/message");
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            fake.calls("tools/list").len(),
            1,
            "an unrelated notification refreshes nothing"
        );

        fake.emit_notification("notifications/tools/list_changed");
        tokio::time::timeout(Duration::from_secs(2), async {
            while session.tools().len() != 2 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the changed list was fetched");

        let names: Vec<String> = session.tools().into_iter().map(|descriptor| descriptor.name).collect();
        assert_eq!(names, vec!["mcp__files__read", "mcp__files__write"]);
        assert_eq!(fake.calls("tools/list").len(), 2);
    }

    #[tokio::test]
    async fn two_tools_with_one_name_are_refused_rather_than_kept_twice() {
        let fake = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(LATEST_PROTOCOL_VERSION))
                .on(
                    "tools/list",
                    tools_reply(&[tool_listing("read", ""), tool_listing("read", "")]),
                ),
        );
        let error = McpSession::open(server(), Box::new(fake))
            .await
            .expect_err("one name, two tools");
        assert_eq!(error.code(), "MCP-DUPLICATE-TOOL");
        assert!(error.to_string().contains("read"));
    }

    #[tokio::test]
    async fn a_call_sends_the_name_and_arguments_and_joins_text_blocks() {
        let fake = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[tool_listing("read", "")]))
                .on(
                    "tools/call",
                    json!({
                        "content": [
                            { "type": "text", "text": "first line" },
                            { "type": "text", "text": "second line" }
                        ]
                    }),
                ),
        );
        let session = McpSession::open(server(), Box::new(Arc::clone(&fake)))
            .await
            .expect("a session");

        let call = session
            .call_tool("read", json!({ "path": "/etc/hosts" }), 256 * 1024)
            .await
            .expect("an answer");

        assert_eq!(call.value, json!("first line\nsecond line"));
        assert!(!call.is_error);
        assert!(!call.truncated);

        let calls = fake.calls("tools/call");
        assert_eq!(calls.len(), 1);
        let params = calls[0].as_ref().expect("the call had params");
        assert_eq!(params["name"], json!("read"));
        assert_eq!(params["arguments"], json!({ "path": "/etc/hosts" }));
    }

    #[tokio::test]
    async fn a_missing_arguments_object_is_read_as_an_empty_one_and_anything_else_is_refused() {
        let fake = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[tool_listing("read", "")]))
                .on("tools/call", text_result("ok")),
        );
        let session = McpSession::open(server(), Box::new(Arc::clone(&fake)))
            .await
            .expect("a session");

        session
            .call_tool("read", Value::Null, 1024)
            .await
            .expect("no arguments is an empty object");
        assert_eq!(
            fake.calls("tools/call")[0]
                .as_ref()
                .map(|params| params["arguments"].clone()),
            Some(json!({}))
        );

        let error = session
            .call_tool("read", json!("not an object"), 1024)
            .await
            .expect_err("arguments are an object");
        assert_eq!(error.code(), "MCP-EXTERNAL-FAILED");
    }

    #[tokio::test]
    async fn non_text_content_and_structured_content_keep_their_shape() {
        let fake = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[tool_listing("read", "")]))
                .on(
                    "tools/call",
                    json!({
                        "content": [
                            { "type": "text", "text": "a summary" },
                            { "type": "image", "mimeType": "image/png", "data": "AAAA" }
                        ],
                        "structuredContent": { "rows": 2 }
                    }),
                ),
        );
        let session = McpSession::open(server(), Box::new(fake)).await.expect("a session");

        let call = session
            .call_tool("read", json!({}), 256 * 1024)
            .await
            .expect("an answer");
        assert_eq!(
            call.value,
            json!({
                "content": [
                    "a summary",
                    { "type": "image", "mimeType": "image/png", "data": "AAAA" }
                ],
                "structuredContent": { "rows": 2 }
            })
        );
    }

    #[tokio::test]
    async fn an_is_error_answer_stays_a_result_with_the_flag_set() {
        let fake = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[tool_listing("read", "")]))
                .on(
                    "tools/call",
                    json!({
                        "content": [{ "type": "text", "text": "the path does not exist" }],
                        "isError": true
                    }),
                ),
        );
        let session = McpSession::open(server(), Box::new(fake)).await.expect("a session");

        let call = session.call_tool("read", json!({}), 1024).await.expect("an answer");
        assert!(call.is_error);
        assert_eq!(call.value, json!("the path does not exist"));
    }

    #[tokio::test]
    async fn a_result_past_the_limit_becomes_a_bounded_prefix_that_says_so() {
        let text = "x".repeat(200);
        let fake = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[tool_listing("read", "")]))
                .on("tools/call", text_result(&text)),
        );
        let session = McpSession::open(server(), Box::new(fake)).await.expect("a session");

        let call = session.call_tool("read", json!({}), 64).await.expect("an answer");
        assert!(call.truncated);
        assert_eq!(call.value["truncated"], json!(true));
        assert_eq!(call.value["originalBytes"], json!(200));
        assert_eq!(
            call.value["text"].as_str().map(str::len),
            Some(64),
            "the prefix is the limit, not a guess at it"
        );
    }

    #[tokio::test]
    async fn a_call_that_runs_past_its_budget_is_a_timeout_with_the_configured_seconds() {
        let fake = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[tool_listing("read", "")]))
                .on_delay("tools/call", Duration::from_secs(30), text_result("too late")),
        );
        let session = McpSession::open(server(), Box::new(fake))
            .await
            .expect("a session")
            .with_call_budget(Duration::from_millis(20));

        let error = session
            .call_tool("read", json!({}), 1024)
            .await
            .expect_err("twenty milliseconds cannot wait thirty seconds");
        assert_eq!(error.code(), "MCP-EXTERNAL-TIMEOUT");
    }

    #[tokio::test]
    async fn a_server_error_keeps_its_code_and_message_as_a_failed_call() {
        let fake = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[tool_listing("read", "")]))
                .on_error(
                    "tools/call",
                    McpClientError::Failed {
                        server: "files".to_owned(),
                        message: "the server answered with error -32000: the tool is gone".to_owned(),
                    },
                ),
        );
        let session = McpSession::open(server(), Box::new(fake)).await.expect("a session");

        let error = session.call_tool("read", json!({}), 1024).await.expect_err("a failure");
        assert_eq!(error.code(), "MCP-EXTERNAL-FAILED");
        assert!(error.detail().is_some_and(|detail| detail.contains("the tool is gone")));
    }

    #[tokio::test]
    async fn closing_the_session_reaches_the_transport() {
        let fake = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[])),
        );
        let session = McpSession::open(server(), Box::new(Arc::clone(&fake)))
            .await
            .expect("a session");
        session.close().await.expect("close");
        assert!(fake.is_closed());
    }

    #[test]
    fn the_supported_revisions_are_the_one_asked_for_and_the_one_before_it() {
        assert_eq!(SUPPORTED_PROTOCOL_VERSIONS, &[LATEST_PROTOCOL_VERSION, "2024-11-05"]);
    }
}
