//! One server's session: the handshake, the cached tool list, and the calls that go through it.
//!
//! A session is what the manager holds per server. It is created once per successful handshake and
//! its tool list is replaced when the server says it changed (or when a person refreshes), never
//! per call, because a list that changed between the moment an agent was shown a tool and the
//! moment it called one is a race no caller can see. A server whose session failed is rebuilt from
//! scratch; unfinished calls are never replayed, because a tool may have had a side effect the
//! first time.
//!
//! Alongside the tools sit two read-only lists: the server's resources and its prompt templates.
//! Both are fetched at the handshake when the server advertises them and refreshed on the same
//! `list_changed` signal as the tools. They are deliberately weaker than the tool list: a server
//! that cannot list its content is still a server whose tools work, so a failed content list is
//! logged and left empty rather than failing the session. A tool list that fails still fails the
//! session, because tools are what a session is opened for.
//!
//! Two version numbers are spoken. The handshake asks for [`LATEST_PROTOCOL_VERSION`], and a server
//! that only knows the revision before it is accepted rather than refused: the tools surface this
//! build uses is the same in both. A third revision is a refusal, because this build cannot know
//! what changed.
//!
//! The budgets are the protocol's own numbers: fifteen seconds for the handshake and for the list,
//! and the server's configured budget (five seconds to ten minutes) for one call.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde_json::{Value, json};
use sevenshell_api::ToolDescriptor;
use sevenshell_types::{
    McpPrompt, McpPromptArgument, McpPromptMessage, McpPromptRole, McpPromptText, McpResource, McpResourceContent,
    McpResourceRead, McpServerConfig, McpServerContent, McpServerState, McpServerStatus,
};
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

/// How many pages one `resources/list` or `prompts/list` read takes.
///
/// The same bound as the tool list, per list: a cursor that never ends must not keep a refresh
/// running until its own timeout either.
const MAX_CONTENT_PAGES: usize = 32;

/// The most text one `resources/read` carries, in characters.
///
/// A resource is a whole document and a person asked for it, so it gets four times what one tool
/// answer does — and still a bound, because the answer is drawn in a window and travels to a model.
pub const MAX_RESOURCE_TEXT_CHARS: usize = 256 * 1024;

/// The most text one `prompts/get` carries, in characters.
///
/// A rendered prompt goes straight into an editor a person still has to read and send, so it is a
/// message rather than a document: a quarter of the resource budget.
pub const MAX_PROMPT_TEXT_CHARS: usize = 64 * 1024;

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
    resources: Arc<RwLock<Vec<McpResource>>>,
    prompts: Arc<RwLock<Vec<McpPrompt>>>,
    /// Whether the handshake was told this server has resources at all.
    ///
    /// Kept rather than inferred from the list, because "this server offers none" and "this server
    /// offers some and its list could not be read" are answered differently: the first is a
    /// refusal a caller is told about, the second is an empty picker.
    offers_resources: bool,
    offers_prompts: bool,
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
        // Content is the weaker half of a session: a person asked to attach a file, not to reach
        // this server, so a list that cannot be read leaves an empty picker rather than a failed
        // row that also takes the tools away.
        let resources = if handshake.offers_resources {
            match fetch_resources(&server, transport.as_ref()).await {
                Ok(resources) => resources,
                Err(error) => {
                    log::warn!("the resource list of `{}` could not be read: {error}", server.id);
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        let prompts = if handshake.offers_prompts {
            match fetch_prompts(&server, transport.as_ref()).await {
                Ok(prompts) => prompts,
                Err(error) => {
                    log::warn!("the prompt list of `{}` could not be read: {error}", server.id);
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        let tools = Arc::new(RwLock::new(tools));
        let resources = Arc::new(RwLock::new(resources));
        let prompts = Arc::new(RwLock::new(prompts));
        start_notification_refresh(
            &server,
            Arc::clone(&transport),
            Caches {
                tools: Arc::clone(&tools),
                resources: Arc::clone(&resources),
                prompts: Arc::clone(&prompts),
            },
            handshake.offers_resources,
            handshake.offers_prompts,
        );

        Ok(Self {
            call_budget: tool_timeout(&server),
            server,
            transport,
            protocol_version: handshake.version,
            tools,
            resources,
            prompts,
            offers_resources: handshake.offers_resources,
            offers_prompts: handshake.offers_prompts,
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
        read_lock(&self.tools).clone()
    }

    /// This server's resources and prompts as they were cached at the handshake or the last refresh.
    ///
    /// The picker asks for the catalogue when it opens rather than having each list pushed into the
    /// status event: a server's content is not what its row is about, and a list of a hundred
    /// resources travelling on every status change would be a copy nobody reads.
    pub fn content(&self) -> McpServerContent {
        McpServerContent {
            server_id: self.server.id.clone(),
            name: self.server.display_name().to_owned(),
            resources: read_lock(&self.resources).clone(),
            prompts: read_lock(&self.prompts).clone(),
        }
    }

    /// How this server is drawn right now.
    pub fn status(&self) -> McpServerStatus {
        if self.server.enabled {
            McpServerStatus::new(&self.server.id, McpServerState::Ready).with_tools(read_lock(&self.tools).len())
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
        *write_lock(&self.tools) = tools;
        Ok(())
    }

    /// Reads one resource the server offered.
    ///
    /// This is a content read rather than a call: it changes nothing on the server, so it takes no
    /// approval, and its answer is bounded by [`MAX_RESOURCE_TEXT_CHARS`] instead of by the result
    /// limit, because a document and a tool result are read at different sizes.
    pub async fn read_resource(&self, uri: &str) -> Result<McpResourceRead, McpClientError> {
        if !self.offers_resources {
            return Err(McpClientError::Failed {
                server: self.server.id.clone(),
                message: "the server does not offer resources".to_owned(),
            });
        }

        let params = json!({ "uri": uri });
        let seconds = self.call_budget.as_secs().max(1);
        let result = timeout(self.call_budget, self.transport.request("resources/read", Some(params)))
            .await
            .map_err(|_| McpClientError::Timeout {
                server: self.server.id.clone(),
                seconds,
            })??;

        read_resource_contents(&self.server.id, uri, &result)
    }

    /// Renders one of the server's prompt templates with the arguments a person filled in.
    ///
    /// The server does the rendering; this build only turns the answer into the messages an editor
    /// can hold, and cuts the joined text at [`MAX_PROMPT_TEXT_CHARS`].
    pub async fn get_prompt(
        &self,
        name: &str,
        arguments: BTreeMap<String, String>,
    ) -> Result<McpPromptText, McpClientError> {
        if !self.offers_prompts {
            return Err(McpClientError::Failed {
                server: self.server.id.clone(),
                message: "the server does not offer prompts".to_owned(),
            });
        }

        let params = json!({ "name": name, "arguments": arguments });
        let seconds = self.call_budget.as_secs().max(1);
        let result = timeout(self.call_budget, self.transport.request("prompts/get", Some(params)))
            .await
            .map_err(|_| McpClientError::Timeout {
                server: self.server.id.clone(),
                seconds,
            })??;

        read_prompt(&self.server.id, name, &result)
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
}

/// A read guard that survives a panic in a task that held the write side.
///
/// The caches are replaced whole by one writer at a time and are read by anything that asks; a
/// poisoned lock would otherwise make a changed list take the session's tools away for good.
fn read_lock<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    match lock.read() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn write_lock<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    match lock.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Prints what a log line may carry: the server, the revision, and how much content was learned.
/// The transport is not reachable from here, so no header or environment value can be printed.
impl fmt::Debug for McpSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tools = read_lock(&self.tools).len();
        let resources = read_lock(&self.resources).len();
        let prompts = read_lock(&self.prompts).len();
        formatter
            .debug_struct("McpSession")
            .field("server", &self.server.id)
            .field("protocol_version", &self.protocol_version)
            .field("tools", &tools)
            .field("resources", &resources)
            .field("prompts", &prompts)
            .field("call_budget", &self.call_budget)
            .finish()
    }
}

struct Handshake {
    version: String,
    offers_tools: bool,
    offers_resources: bool,
    offers_prompts: bool,
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

    // A capability the server did not advertise is one whose list would be a request it never
    // promised to answer. All three are read here so that the rest of the session asks nothing it
    // was not offered.
    let offers = |capability: &str| {
        result
            .get("capabilities")
            .and_then(|capabilities| capabilities.get(capability))
            .is_some()
    };

    timeout(CONNECT_TIMEOUT, transport.notify("notifications/initialized", None))
        .await
        .map_err(|_| connect_failed_detail(server, "the server did not accept the handshake's last step"))?
        .map_err(|error| connect_failed(server, error))?;

    Ok(Handshake {
        version: version.to_owned(),
        offers_tools: offers("tools"),
        offers_resources: offers("resources"),
        offers_prompts: offers("prompts"),
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

/// Reads every page of `resources/list`.
///
/// A listing this build cannot show is skipped rather than refused: one resource without a URI is a
/// bad entry in somebody else's server, and dropping the whole list over it would hide the rest.
/// Two entries with one URI are the other way round — the second is dropped and said to the log,
/// because a picker that showed one URI twice would leave a person guessing which one they read.
async fn fetch_resources(
    server: &McpServerConfig,
    transport: &dyn McpTransport,
) -> Result<Vec<McpResource>, McpClientError> {
    let mut resources: Vec<McpResource> = Vec::new();
    let mut uris: BTreeSet<String> = BTreeSet::new();
    let mut cursor: Option<String> = None;

    for _ in 0..MAX_CONTENT_PAGES {
        let params = cursor.as_ref().map(|cursor| json!({ "cursor": cursor }));
        let result = timeout(LIST_TIMEOUT, transport.request("resources/list", params))
            .await
            .map_err(|_| McpClientError::Timeout {
                server: server.id.clone(),
                seconds: LIST_TIMEOUT.as_secs(),
            })??;

        let listings = result
            .get("resources")
            .and_then(Value::as_array)
            .ok_or_else(|| McpClientError::Failed {
                server: server.id.clone(),
                message: "the server answered resources/list without a list of resources".to_owned(),
            })?;

        for listing in listings {
            let Some(uri) = listing.get("uri").and_then(Value::as_str).filter(|uri| !uri.is_empty()) else {
                log::warn!("the server `{}` listed a resource with no URI", server.id);
                continue;
            };
            if !uris.insert(uri.to_owned()) {
                log::warn!("the server `{}` listed the resource `{uri}` twice", server.id);
                continue;
            }
            resources.push(McpResource {
                uri: uri.to_owned(),
                // A missing name is drawn as the URI: the only thing a person can act on anyway.
                name: listing
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.is_empty())
                    .unwrap_or(uri)
                    .to_owned(),
                title: text_field(listing, "title"),
                description: text_field(listing, "description"),
                mime_type: text_field(listing, "mimeType"),
                size: listing.get("size").and_then(Value::as_u64),
            });
        }

        match result
            .get("nextCursor")
            .and_then(Value::as_str)
            .filter(|cursor| !cursor.is_empty())
        {
            Some(next) => cursor = Some(next.to_owned()),
            None => return Ok(resources),
        }
    }

    Err(McpClientError::Failed {
        server: server.id.clone(),
        message: format!("the server's resource list did not end within {MAX_CONTENT_PAGES} pages"),
    })
}

/// Reads every page of `prompts/list`, keeping the first entry of any name offered twice.
async fn fetch_prompts(
    server: &McpServerConfig,
    transport: &dyn McpTransport,
) -> Result<Vec<McpPrompt>, McpClientError> {
    let mut prompts: Vec<McpPrompt> = Vec::new();
    let mut names: BTreeSet<String> = BTreeSet::new();
    let mut cursor: Option<String> = None;

    for _ in 0..MAX_CONTENT_PAGES {
        let params = cursor.as_ref().map(|cursor| json!({ "cursor": cursor }));
        let result = timeout(LIST_TIMEOUT, transport.request("prompts/list", params))
            .await
            .map_err(|_| McpClientError::Timeout {
                server: server.id.clone(),
                seconds: LIST_TIMEOUT.as_secs(),
            })??;

        let listings = result
            .get("prompts")
            .and_then(Value::as_array)
            .ok_or_else(|| McpClientError::Failed {
                server: server.id.clone(),
                message: "the server answered prompts/list without a list of prompts".to_owned(),
            })?;

        for listing in listings {
            let Some(name) = listing
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
            else {
                log::warn!("the server `{}` listed a prompt with no name", server.id);
                continue;
            };
            if !names.insert(name.to_owned()) {
                log::warn!("the server `{}` listed the prompt `{name}` twice", server.id);
                continue;
            }
            prompts.push(McpPrompt {
                name: name.to_owned(),
                title: text_field(listing, "title"),
                description: text_field(listing, "description"),
                arguments: listing
                    .get("arguments")
                    .and_then(Value::as_array)
                    .map(|arguments| arguments.iter().filter_map(prompt_argument).collect())
                    .unwrap_or_default(),
            });
        }

        match result
            .get("nextCursor")
            .and_then(Value::as_str)
            .filter(|cursor| !cursor.is_empty())
        {
            Some(next) => cursor = Some(next.to_owned()),
            None => return Ok(prompts),
        }
    }

    Err(McpClientError::Failed {
        server: server.id.clone(),
        message: format!("the server's prompt list did not end within {MAX_CONTENT_PAGES} pages"),
    })
}

fn text_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// One prompt argument, or nothing for one without a name — a form cannot draw a nameless box.
fn prompt_argument(argument: &Value) -> Option<McpPromptArgument> {
    Some(McpPromptArgument {
        name: argument
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())?
            .to_owned(),
        title: text_field(argument, "title"),
        description: text_field(argument, "description"),
        required: argument.get("required").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// The three caches a notification can invalidate.
#[derive(Clone)]
struct Caches {
    tools: Arc<RwLock<Vec<ToolDescriptor>>>,
    resources: Arc<RwLock<Vec<McpResource>>>,
    prompts: Arc<RwLock<Vec<McpPrompt>>>,
}

/// Keeps the cached lists in step with a server that says they changed.
///
/// The transport only forwards the notification's name; the fetch happens on this task because a
/// reader must never wait on a request. A failed fetch leaves the previous list in place: the
/// server's tools did not become unknown because one refresh failed. The weak handle to the
/// transport is what lets the task end when the session is gone, rather than keeping it alive from
/// inside its own notification slot.
///
/// A changed content list is refreshed only for a server that advertised that capability: a
/// notification for a list this session never asked for is a server telling us about something it
/// did not say it had.
fn start_notification_refresh(
    server: &McpServerConfig,
    transport: Arc<dyn McpTransport>,
    caches: Caches,
    offers_resources: bool,
    offers_prompts: bool,
) {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<String>();
    transport.set_notification_handler(Arc::new(move |method: &str| {
        let _ = sender.send(method.to_owned());
    }));

    let server = server.clone();
    let transport = Arc::downgrade(&transport);
    tokio::spawn(async move {
        while let Some(method) = receiver.recv().await {
            let Some(transport) = transport.upgrade() else {
                break;
            };
            match method.as_str() {
                "notifications/tools/list_changed" => {
                    if let Ok(refreshed) = fetch_tools(&server, transport.as_ref()).await
                        && let Ok(mut guard) = caches.tools.write()
                    {
                        *guard = refreshed;
                    }
                }
                "notifications/resources/list_changed" if offers_resources => {
                    match fetch_resources(&server, transport.as_ref()).await {
                        Ok(refreshed) => *write_lock(&caches.resources) = refreshed,
                        Err(error) => log::warn!("the resource list of `{}` could not be read: {error}", server.id),
                    }
                }
                "notifications/prompts/list_changed" if offers_prompts => {
                    match fetch_prompts(&server, transport.as_ref()).await {
                        Ok(refreshed) => *write_lock(&caches.prompts) = refreshed,
                        Err(error) => log::warn!("the prompt list of `{}` could not be read: {error}", server.id),
                    }
                }
                _ => {}
            }
        }
    });
}

/// Turns one `resources/read` result into what a picker shows.
///
/// One budget covers the whole answer rather than each piece, because the answer is what a person
/// reads and attaches. Text is cut at what is left of it; a binary piece is left out whole when it
/// does not fit, since half a PNG is not half a picture; anything that is neither is dropped
/// silently, because there is no way to show it and saying so for every piece would fill the pane
/// with sentences about what is not there.
fn read_resource_contents(server: &str, uri: &str, result: &Value) -> Result<McpResourceRead, McpClientError> {
    let contents = result
        .get("contents")
        .and_then(Value::as_array)
        .ok_or_else(|| McpClientError::Failed {
            server: server.to_owned(),
            message: "the server answered resources/read without a list of contents".to_owned(),
        })?;

    let mut remaining = MAX_RESOURCE_TEXT_CHARS;
    let mut pieces = Vec::with_capacity(contents.len());
    let mut truncated = false;

    for content in contents {
        let piece_uri = content.get("uri").and_then(Value::as_str).unwrap_or(uri).to_owned();
        let mime_type = text_field(content, "mimeType");

        if let Some(text) = content.get("text").and_then(Value::as_str) {
            let (text, cut) = cut_to_budget(text, remaining);
            remaining -= text.chars().count();
            truncated |= cut;
            pieces.push(McpResourceContent {
                uri: piece_uri,
                mime_type,
                text: Some(text),
                blob: None,
                truncated: cut,
            });
            continue;
        }

        if let Some(blob) = content.get("blob").and_then(Value::as_str) {
            let length = blob.chars().count();
            if length > remaining {
                truncated = true;
                continue;
            }
            remaining -= length;
            pieces.push(McpResourceContent {
                uri: piece_uri,
                mime_type,
                text: None,
                blob: Some(blob.to_owned()),
                truncated: false,
            });
        }
    }

    Ok(McpResourceRead {
        server_id: server.to_owned(),
        uri: uri.to_owned(),
        contents: pieces,
        truncated,
    })
}

/// Turns one `prompts/get` result into the messages an editor can hold.
///
/// Only text is carried. A piece of any other type becomes one line naming its type, and a role
/// this build does not know is drawn as `user`: a template that said `system` is still something a
/// person has to read before sending, and refusing the whole render over one role would hide it.
fn read_prompt(server: &str, name: &str, result: &Value) -> Result<McpPromptText, McpClientError> {
    let messages = result
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| McpClientError::Failed {
            server: server.to_owned(),
            message: "the server answered prompts/get without a list of messages".to_owned(),
        })?;

    let mut remaining = MAX_PROMPT_TEXT_CHARS;
    let mut rendered = Vec::with_capacity(messages.len());
    let mut truncated = false;

    for message in messages {
        if remaining == 0 {
            // Nothing left to say and no way to say it: the rest is dropped rather than appended as
            // empty messages that would look like part of the prompt.
            truncated = true;
            break;
        }

        let role = message
            .get("role")
            .and_then(Value::as_str)
            .and_then(McpPromptRole::from_id)
            .unwrap_or(McpPromptRole::User);
        let (text, cut) = cut_to_budget(&prompt_message_text(message.get("content")), remaining);
        remaining -= text.chars().count();
        rendered.push(McpPromptMessage { role, text });
        if cut {
            truncated = true;
            break;
        }
    }

    Ok(McpPromptText {
        server_id: server.to_owned(),
        name: name.to_owned(),
        description: text_field(result, "description"),
        messages: rendered,
        truncated,
    })
}

fn prompt_message_text(content: Option<&Value>) -> String {
    match content.and_then(|content| content.get("type")).and_then(Value::as_str) {
        Some("text") => content
            .and_then(|content| content.get("text"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        Some(kind) => format!("[non-text content omitted: {kind}]"),
        None => "[non-text content omitted: unknown]".to_owned(),
    }
}

/// Cuts text to what is left of a budget, on a character boundary, and says whether it was cut.
fn cut_to_budget(text: &str, budget: usize) -> (String, bool) {
    // The byte offset of the first character that does not fit is the place to cut: a character
    // count is never a byte count, and cutting at a byte count would split one in half.
    match text.char_indices().nth(budget) {
        Some((index, _)) => (text[..index].to_owned(), true),
        None => (text.to_owned(), false),
    }
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
        FakeTransport, initialize_reply, initialize_reply_offering, initialize_reply_without_tools, prompt_listing,
        prompt_message, prompt_reply, prompts_reply, resource_listing, resources_reply, text_resource_read,
        text_result, tool_listing, tools_reply,
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

    // -----------------------------------------------------------------------------------------
    // Resources and prompts
    // -----------------------------------------------------------------------------------------

    #[tokio::test]
    async fn a_server_without_the_content_capabilities_opens_with_no_content_list_request() {
        let fake = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[tool_listing("read", "")])),
        );
        let session = McpSession::open(server(), Box::new(Arc::clone(&fake)))
            .await
            .expect("a session");

        assert!(
            fake.calls("resources/list").is_empty(),
            "a capability the server did not advertise is not requested"
        );
        assert!(fake.calls("prompts/list").is_empty());
        let content = session.content();
        assert_eq!(content.server_id, "files");
        assert_eq!(content.name, "Files", "the picker draws the name the person typed");
        assert!(content.resources.is_empty() && content.prompts.is_empty());
    }

    #[tokio::test]
    async fn resources_and_prompts_are_listed_at_open_when_the_server_advertises_them() {
        let fake = Arc::new(
            FakeTransport::new()
                .on(
                    "initialize",
                    initialize_reply_offering(LATEST_PROTOCOL_VERSION, &["resources", "prompts"]),
                )
                .on(
                    "resources/list",
                    resources_reply(&[resource_listing("file:///welcome.txt", "welcome")]),
                )
                .on(
                    "prompts/list",
                    prompts_reply(&[prompt_listing("summarize", &["topic"])]),
                ),
        );
        let session = McpSession::open(server(), Box::new(Arc::clone(&fake)))
            .await
            .expect("a session");

        let content = session.content();
        assert_eq!(content.resources.len(), 1);
        assert_eq!(content.resources[0].uri, "file:///welcome.txt");
        assert_eq!(content.resources[0].mime_type.as_deref(), Some("text/plain"));
        assert_eq!(content.prompts.len(), 1);
        assert_eq!(content.prompts[0].arguments[0].name, "topic");
        assert!(content.prompts[0].arguments[0].required);
        assert!(
            session.tools().is_empty(),
            "a server that advertises no tools has none to list"
        );
        assert!(
            fake.calls("tools/list").is_empty(),
            "a capability the server did not advertise is not requested"
        );
    }

    #[tokio::test]
    async fn a_failed_content_list_leaves_the_session_and_its_tools_online() {
        let fake = Arc::new(
            FakeTransport::new()
                .on(
                    "initialize",
                    initialize_reply_offering(LATEST_PROTOCOL_VERSION, &["tools", "resources", "prompts"]),
                )
                .on("tools/list", tools_reply(&[tool_listing("read", "")]))
                .on_error(
                    "resources/list",
                    McpClientError::Failed {
                        server: "files".to_owned(),
                        message: "the server cannot list its resources".to_owned(),
                    },
                )
                .on_error(
                    "prompts/list",
                    McpClientError::Failed {
                        server: "files".to_owned(),
                        message: "the server cannot list its prompts".to_owned(),
                    },
                )
                .on(
                    "resources/read",
                    text_resource_read("file:///welcome.txt", "still readable"),
                ),
        );
        let session = McpSession::open(server(), Box::new(Arc::clone(&fake)))
            .await
            .expect("a session, because the tool list is what it is opened for");

        assert_eq!(
            session.tools().len(),
            1,
            "a content list that failed does not take the tools away"
        );
        assert!(
            session.content().resources.is_empty(),
            "the picker is empty, not an error"
        );
        assert!(session.content().prompts.is_empty());

        let read = session
            .read_resource("file:///welcome.txt")
            .await
            .expect("the capability is still there even though its list was not");
        assert_eq!(read.contents[0].text.as_deref(), Some("still readable"));
    }

    #[tokio::test]
    async fn every_page_of_a_resource_list_is_read() {
        let fake = Arc::new(
            FakeTransport::new()
                .on(
                    "initialize",
                    initialize_reply_offering(LATEST_PROTOCOL_VERSION, &["resources"]),
                )
                .on(
                    "resources/list",
                    json!({ "resources": [resource_listing("file:///a", "a")], "nextCursor": "page-2" }),
                )
                .on("resources/list", resources_reply(&[resource_listing("file:///b", "b")])),
        );
        let session = McpSession::open(server(), Box::new(Arc::clone(&fake)))
            .await
            .expect("a session");

        let uris: Vec<String> = session
            .content()
            .resources
            .into_iter()
            .map(|resource| resource.uri)
            .collect();
        assert_eq!(uris, vec!["file:///a", "file:///b"]);

        let calls = fake.calls("resources/list");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1], Some(json!({ "cursor": "page-2" })));
    }

    #[tokio::test]
    async fn every_page_of_a_prompt_list_is_read() {
        let fake = Arc::new(
            FakeTransport::new()
                .on(
                    "initialize",
                    initialize_reply_offering(LATEST_PROTOCOL_VERSION, &["prompts"]),
                )
                .on(
                    "prompts/list",
                    json!({ "prompts": [prompt_listing("first", &[])], "nextCursor": "page-2" }),
                )
                .on("prompts/list", prompts_reply(&[prompt_listing("second", &[])])),
        );
        let session = McpSession::open(server(), Box::new(Arc::clone(&fake)))
            .await
            .expect("a session");

        let names: Vec<String> = session
            .content()
            .prompts
            .into_iter()
            .map(|prompt| prompt.name)
            .collect();
        assert_eq!(names, vec!["first", "second"]);
        assert_eq!(
            fake.calls("prompts/list")[1],
            Some(json!({ "cursor": "page-2" })),
            "the cursor the server answered with is the one the next page is asked for"
        );
    }

    #[tokio::test]
    async fn a_duplicate_resource_uri_or_prompt_name_keeps_the_first_one() {
        let fake = Arc::new(
            FakeTransport::new()
                .on(
                    "initialize",
                    initialize_reply_offering(LATEST_PROTOCOL_VERSION, &["resources", "prompts"]),
                )
                .on(
                    "resources/list",
                    resources_reply(&[
                        resource_listing("file:///a", "first"),
                        resource_listing("file:///a", "second"),
                    ]),
                )
                .on(
                    "prompts/list",
                    prompts_reply(&[
                        prompt_listing("summarize", &["topic"]),
                        prompt_listing("summarize", &["other"]),
                    ]),
                ),
        );
        let session = McpSession::open(server(), Box::new(fake))
            .await
            .expect("a duplicate is a bad listing, not a broken session");

        let content = session.content();
        assert_eq!(content.resources.len(), 1, "a URI offered twice is shown once");
        assert_eq!(content.resources[0].name, "first", "the first entry is the one kept");
        assert_eq!(content.prompts.len(), 1);
        assert_eq!(content.prompts[0].arguments[0].name, "topic");
    }

    #[tokio::test]
    async fn a_text_resource_is_read_with_its_type_and_no_truncation() {
        let fake = Arc::new(
            FakeTransport::new()
                .on(
                    "initialize",
                    initialize_reply_offering(LATEST_PROTOCOL_VERSION, &["resources"]),
                )
                .on(
                    "resources/list",
                    resources_reply(&[resource_listing("file:///welcome.txt", "welcome")]),
                )
                .on(
                    "resources/read",
                    text_resource_read("file:///welcome.txt", "Hello from the fixture."),
                ),
        );
        let session = McpSession::open(server(), Box::new(Arc::clone(&fake)))
            .await
            .expect("a session");

        let read = session.read_resource("file:///welcome.txt").await.expect("a read");
        assert_eq!(read.server_id, "files");
        assert_eq!(read.uri, "file:///welcome.txt");
        assert_eq!(read.contents.len(), 1);
        assert_eq!(read.contents[0].uri, "file:///welcome.txt");
        assert_eq!(read.contents[0].text.as_deref(), Some("Hello from the fixture."));
        assert_eq!(read.contents[0].mime_type.as_deref(), Some("text/plain"));
        assert!(read.contents[0].blob.is_none());
        assert!(!read.truncated && !read.contents[0].truncated);
        assert_eq!(
            fake.calls("resources/read"),
            vec![Some(json!({ "uri": "file:///welcome.txt" }))]
        );
    }

    #[tokio::test]
    async fn a_text_resource_past_the_read_budget_is_cut_and_says_so() {
        let long = "x".repeat(MAX_RESOURCE_TEXT_CHARS + 10);
        let fake = Arc::new(
            FakeTransport::new()
                .on(
                    "initialize",
                    initialize_reply_offering(LATEST_PROTOCOL_VERSION, &["resources"]),
                )
                .on("resources/read", text_resource_read("file:///long.txt", &long)),
        );
        let session = McpSession::open(server(), Box::new(fake)).await.expect("a session");

        let read = session.read_resource("file:///long.txt").await.expect("a read");
        assert!(read.truncated, "the answer is not whole and says so");
        assert!(read.contents[0].truncated);
        assert_eq!(
            read.contents[0].text.as_deref().map(str::len),
            Some(MAX_RESOURCE_TEXT_CHARS),
            "the prefix is the budget, not a guess at it"
        );
    }

    #[tokio::test]
    async fn a_blob_over_the_budget_is_left_out_whole_and_a_smaller_one_is_kept() {
        let big = "A".repeat(MAX_RESOURCE_TEXT_CHARS + 1);
        let fake = Arc::new(
            FakeTransport::new()
                .on(
                    "initialize",
                    initialize_reply_offering(LATEST_PROTOCOL_VERSION, &["resources"]),
                )
                .on(
                    "resources/read",
                    json!({
                        "contents": [
                            { "uri": "file:///big.png", "mimeType": "image/png", "blob": big },
                            { "uri": "file:///small.png", "mimeType": "image/png", "blob": "AAAA" }
                        ]
                    }),
                ),
        );
        let session = McpSession::open(server(), Box::new(fake)).await.expect("a session");

        let read = session.read_resource("file:///big.png").await.expect("a read");
        assert_eq!(
            read.contents.len(),
            1,
            "a binary piece that does not fit is left out rather than cut in half"
        );
        assert_eq!(read.contents[0].uri, "file:///small.png");
        assert_eq!(
            read.contents[0].blob.as_deref(),
            Some("AAAA"),
            "base64 travels as it was sent"
        );
        assert!(read.contents[0].text.is_none());
        assert!(read.truncated, "a piece was left out, so the answer is not whole");
    }

    #[tokio::test]
    async fn a_piece_that_is_neither_text_nor_blob_is_ignored() {
        let fake = Arc::new(
            FakeTransport::new()
                .on(
                    "initialize",
                    initialize_reply_offering(LATEST_PROTOCOL_VERSION, &["resources"]),
                )
                .on(
                    "resources/read",
                    json!({ "contents": [{ "uri": "file:///a", "type": "image", "data": "AAAA" }] }),
                ),
        );
        let session = McpSession::open(server(), Box::new(fake)).await.expect("a session");

        let read = session.read_resource("file:///a").await.expect("a read");
        assert!(read.contents.is_empty());
        assert!(
            !read.truncated,
            "dropping a piece there is no way to show is not truncation"
        );
    }

    #[tokio::test]
    async fn reading_a_resource_from_a_server_without_the_capability_names_what_is_missing() {
        let fake =
            Arc::new(FakeTransport::new().on("initialize", initialize_reply_without_tools(LATEST_PROTOCOL_VERSION)));
        let session = McpSession::open(server(), Box::new(Arc::clone(&fake)))
            .await
            .expect("a session");

        let error = session
            .read_resource("file:///a")
            .await
            .expect_err("this server said it has no resources");
        assert_eq!(error.code(), "MCP-EXTERNAL-FAILED");
        assert_eq!(error.detail(), Some("the server does not offer resources"));
        assert!(
            fake.calls("resources/read").is_empty(),
            "nothing is asked of a server that said it has none"
        );
    }

    #[tokio::test]
    async fn getting_a_prompt_from_a_server_without_the_capability_names_what_is_missing() {
        let fake =
            Arc::new(FakeTransport::new().on("initialize", initialize_reply_without_tools(LATEST_PROTOCOL_VERSION)));
        let session = McpSession::open(server(), Box::new(Arc::clone(&fake)))
            .await
            .expect("a session");

        let error = session
            .get_prompt("summarize", BTreeMap::new())
            .await
            .expect_err("this server said it has no prompts");
        assert_eq!(error.code(), "MCP-EXTERNAL-FAILED");
        assert_eq!(error.detail(), Some("the server does not offer prompts"));
        assert!(fake.calls("prompts/get").is_empty());
    }

    #[tokio::test]
    async fn a_prompt_keeps_its_roles_and_names_what_it_leaves_out() {
        let fake = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply_offering(LATEST_PROTOCOL_VERSION, &["prompts"]))
                .on("prompts/list", prompts_reply(&[prompt_listing("summarize", &["topic"])]))
                .on(
                    "prompts/get",
                    prompt_reply(&[
                        prompt_message("user", "Summarize networks"),
                        prompt_message("assistant", "Sure"),
                        json!({
                            "role": "system",
                            "content": { "type": "text", "text": "a role this build does not have" }
                        }),
                        json!({ "role": "user", "content": { "type": "image", "mimeType": "image/png", "data": "AAAA" } }),
                    ]),
                ),
        );
        let session = McpSession::open(server(), Box::new(Arc::clone(&fake)))
            .await
            .expect("a session");

        let prompt = session
            .get_prompt(
                "summarize",
                BTreeMap::from([("topic".to_owned(), "networks".to_owned())]),
            )
            .await
            .expect("a rendered prompt");

        assert_eq!(prompt.server_id, "files");
        assert_eq!(prompt.name, "summarize");
        assert_eq!(prompt.description.as_deref(), Some("rendered"));
        let roles: Vec<McpPromptRole> = prompt.messages.iter().map(|message| message.role).collect();
        assert_eq!(
            roles,
            vec![
                McpPromptRole::User,
                McpPromptRole::Assistant,
                McpPromptRole::User,
                McpPromptRole::User
            ],
            "an unknown role is drawn as user rather than refused"
        );
        assert_eq!(prompt.messages[0].text, "Summarize networks");
        assert_eq!(prompt.messages[1].text, "Sure");
        assert_eq!(prompt.messages[3].text, "[non-text content omitted: image]");
        assert!(!prompt.truncated);

        let params = fake.calls("prompts/get");
        assert_eq!(params.len(), 1);
        assert_eq!(
            params[0].as_ref().map(|params| params["name"].clone()),
            Some(json!("summarize"))
        );
        assert_eq!(
            params[0].as_ref().map(|params| params["arguments"].clone()),
            Some(json!({ "topic": "networks" })),
            "the answers a person typed are what the server is asked to render with"
        );
    }

    #[tokio::test]
    async fn a_prompt_past_its_budget_is_cut_and_the_messages_after_it_are_dropped() {
        let long = "y".repeat(MAX_PROMPT_TEXT_CHARS + 5);
        let exact = "y".repeat(MAX_PROMPT_TEXT_CHARS);
        let fake = Arc::new(
            FakeTransport::new()
                .on(
                    "initialize",
                    initialize_reply_offering(LATEST_PROTOCOL_VERSION, &["prompts"]),
                )
                .on("prompts/get", prompt_reply(&[prompt_message("user", &long)]))
                .on(
                    "prompts/get",
                    prompt_reply(&[prompt_message("user", &exact), prompt_message("assistant", "second")]),
                ),
        );
        let session = McpSession::open(server(), Box::new(fake)).await.expect("a session");

        let cut = session
            .get_prompt("long", BTreeMap::new())
            .await
            .expect("a rendered prompt");
        assert!(cut.truncated);
        assert_eq!(cut.messages.len(), 1);
        assert_eq!(
            cut.messages[0].text.len(),
            MAX_PROMPT_TEXT_CHARS,
            "the prompt is cut at the budget, not past it"
        );

        let rest = session
            .get_prompt("exact", BTreeMap::new())
            .await
            .expect("a rendered prompt");
        assert!(
            rest.truncated,
            "a message with no budget left is left out and said to be"
        );
        assert_eq!(
            rest.messages.len(),
            1,
            "the message after the budget is not appended as an empty one"
        );
    }

    #[tokio::test]
    async fn a_changed_resource_or_prompt_list_is_fetched_when_the_server_says_so() {
        let fake = Arc::new(
            FakeTransport::new()
                .on(
                    "initialize",
                    initialize_reply_offering(LATEST_PROTOCOL_VERSION, &["tools", "resources", "prompts"]),
                )
                .on("tools/list", tools_reply(&[tool_listing("read", "")]))
                .on("resources/list", resources_reply(&[resource_listing("file:///a", "a")]))
                .on(
                    "resources/list",
                    resources_reply(&[resource_listing("file:///a", "a"), resource_listing("file:///b", "b")]),
                )
                .on(
                    "prompts/list",
                    prompts_reply(&[prompt_listing("summarize", &["topic"])]),
                )
                .on(
                    "prompts/list",
                    prompts_reply(&[prompt_listing("summarize", &["topic"]), prompt_listing("explain", &[])]),
                ),
        );
        let session = McpSession::open(server(), Box::new(Arc::clone(&fake)))
            .await
            .expect("a session");
        assert_eq!(session.content().resources.len(), 1);

        fake.emit_notification("notifications/message");
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            fake.calls("resources/list").len(),
            1,
            "an unrelated notification refreshes nothing"
        );
        assert_eq!(fake.calls("prompts/list").len(), 1);

        fake.emit_notification("notifications/resources/list_changed");
        tokio::time::timeout(Duration::from_secs(2), async {
            while session.content().resources.len() != 2 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the changed resource list was fetched");
        assert_eq!(
            session.content().prompts.len(),
            1,
            "a changed resource list does not refetch the prompts"
        );

        fake.emit_notification("notifications/prompts/list_changed");
        tokio::time::timeout(Duration::from_secs(2), async {
            while session.content().prompts.len() != 2 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the changed prompt list was fetched");

        assert_eq!(
            session.tools().len(),
            1,
            "the tools are still the ones the session learned"
        );
        assert_eq!(
            fake.calls("tools/list").len(),
            1,
            "a list change is a refresh, not a reconnect"
        );
        assert!(!fake.is_closed(), "the session is the one it was, still open");
    }
}
