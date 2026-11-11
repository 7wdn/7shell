//! The stdio side of the bridge: one line in, one line out, and nothing else on stdout.
//!
//! stdout is the protocol. A log line, a warning or a stray `println!` written there is a client
//! parsing something that is not JSON, so everything this module has to say about its own trouble
//! goes to stderr.
//!
//! Requests are handled concurrently and each answer carries the id of the request it belongs to,
//! which is what lets a slow call (a batch, an approval window) sit behind a fast one without either
//! waiting for the other. Answers are written through one channel, so two of them can never
//! interleave inside a line.
//!
//! Before the handshake nothing is negotiated and nothing is enforced: a client that skips
//! `initialize` still gets answers, because there is nothing this bridge would decide differently.
//! What would be wrong is refusing a request over a step that changes no answer.
//!
//! Cancellation is forwarded rather than swallowed: each call is started under an id this bridge
//! minted, so `notifications/cancelled` can name a request, the mapping turns it into that call id,
//! and the desktop calls off exactly that work. The notification still gets no reply — the protocol
//! has none for it — and the request the agent named still gets its own answer, which is the
//! cancelled tool's outcome. A cancellation naming something that already ended is ignored.
//!
//! What is deliberately absent is an unbounded fan-out: a client may send lines faster than calls
//! finish, and a bridge that spawned a task per line would hold every one of them in memory behind
//! one person's approval window.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;
use uuid::Uuid;

use sevenshell_api::{ToolAccess, ToolDescriptor, ToolOutcome, ToolResult};

use crate::jsonrpc::{Id, LATEST_PROTOCOL_VERSION, Request, Response, RpcError, SUPPORTED_PROTOCOL_VERSIONS};

/// How many lines are handled at once unless the caller says otherwise.
///
/// Eight: far enough above one so a slow call cannot stall everything a client sent after it, small
/// enough that a runaway client cannot fill this process with pending calls — and with pending
/// approvals, which are people's windows rather than this bridge's memory.
pub const DEFAULT_MAX_CONCURRENCY: usize = 8;

/// What the bridge needs from the desktop.
///
/// A trait rather than the HTTP client itself so the protocol can be tested without a listener: the
/// fake supplies descriptors and outcomes, and the server cannot tell the difference. Listing starts
/// from descriptors compiled into this build and is refreshed from the desktop when it answers —
/// which is how an external server's tools reach a client — so `tools/list` always has an answer,
/// and it is a *call* that then says why the desktop cannot be reached.
#[async_trait]
pub trait ToolClient: Send + Sync {
    /// The descriptors to list, as last known. Cached: [`Self::refresh_descriptors`] moves it on.
    fn descriptors(&self) -> Vec<ToolDescriptor>;

    /// Asks the desktop for its whole tool table.
    ///
    /// The default is "nothing to refresh", which is what a client with a fixed list answers. A
    /// failure is information, not a dead end: the compiled list is still a complete answer for this
    /// build, so `tools/list` never turns into an error because the desktop is not running.
    async fn refresh_descriptors(&self) -> Result<(), RpcError> {
        Ok(())
    }

    /// Runs one tool. `call_id` is this bridge's name for the call, and it is what a later
    /// cancellation addresses: the desktop starts the work under that id, so a notification reaches
    /// exactly this call and no other.
    async fn call(&self, name: &str, arguments: Value, call_id: &str) -> Result<ToolOutcome, RpcError>;

    /// Asks the desktop to call off the work it started under `call_id`.
    ///
    /// The default is "nothing to cancel" for a client that starts no work of its own. A failure is
    /// reported on stderr but changes no answer: the original request is answered by [`Self::call`]
    /// whatever a cancellation did.
    async fn cancel(&self, _call_id: &str) -> Result<(), RpcError> {
        Ok(())
    }

    /// The largest result this bridge will put on stdout.
    ///
    /// The desktop's endpoint file carries the configured value; the default is what a bridge uses
    /// before it has read one, and what a test client that does not care inherits.
    fn max_result_bytes(&self) -> usize {
        usize::try_from(sevenshell_api::DEFAULT_MAX_RESULT_BYTES).unwrap_or(usize::MAX)
    }
}

/// One stdio session.
pub struct McpServer {
    client: Arc<dyn ToolClient>,
    max_result_bytes: usize,
    max_concurrency: usize,
    /// The request id of every call in flight, mapped to the id this bridge gave the call.
    ///
    /// A `std::sync::Mutex` because the map is read and written in one statement and never held
    /// across an await: the lock is a few hash lookups, not a gate on the desktop's answer.
    calls: Mutex<HashMap<Id, String>>,
}

impl McpServer {
    pub fn new(client: Arc<dyn ToolClient>, max_result_bytes: usize) -> Self {
        Self {
            client,
            max_result_bytes,
            max_concurrency: DEFAULT_MAX_CONCURRENCY,
            calls: Mutex::new(HashMap::new()),
        }
    }

    /// How many lines may be in flight at once.
    ///
    /// A zero is raised to one rather than obeyed: a session with no slots would stop reading its
    /// own input, and a bridge that cannot read the next line cannot be told to stop either.
    pub fn with_max_concurrency(mut self, max_concurrency: usize) -> Self {
        self.max_concurrency = max_concurrency.max(1);
        self
    }

    pub fn max_result_bytes(&self) -> usize {
        self.max_result_bytes
    }

    /// Answers one line, or answers nothing when the line is a notification.
    pub async fn dispatch(&self, message: &str) -> Option<Value> {
        let value: Value = match serde_json::from_str(message) {
            Ok(value) => value,
            Err(error) => return Some(Response::error(None, RpcError::parse_error(&error.to_string())).to_value()),
        };

        if !value.is_object() {
            return Some(Response::error(None, RpcError::invalid_request("a request is a JSON object")).to_value());
        }

        // Taken before the request is parsed, so an invalid request still has its id echoed when it
        // had one the protocol can carry.
        let id = id_of(&value);
        let request: Request = match serde_json::from_value(value) {
            Ok(request) => request,
            Err(error) => return Some(Response::error(id, RpcError::invalid_request(&error.to_string())).to_value()),
        };

        if request.jsonrpc.as_deref() != Some("2.0") {
            return Some(
                Response::error(request.id, RpcError::invalid_request("`jsonrpc` has to be \"2.0\"")).to_value(),
            );
        }

        // A message without an id is a notification, and the protocol gives it no answer. That is
        // also everything `notifications/initialized` needs: it changes nothing here. Cancellation is
        // the one notification that does something, and it still gets no reply — the answer belongs
        // to the request it names, and that request is already in flight.
        if request.id.is_none() {
            if request.method == "notifications/cancelled" {
                self.cancel_request(request.params).await;
            }
            return None;
        }

        let id = request.id?;

        let response = match request.method.as_str() {
            "initialize" => self.initialize(id, request.params),
            "ping" => Response::result(Some(id), json!({})),
            "tools/list" => self.tools_list(id).await,
            "tools/call" => self.tools_call(id, request.params).await,
            other => Response::error(Some(id), RpcError::method_not_found(other)),
        };
        Some(response.to_value())
    }

    /// Reads lines until the client closes its end, answering each in its own task.
    ///
    /// At most [`Self::with_max_concurrency`] tasks are alive at once; the reader waits for a slot
    /// rather than spawning without a bound.
    pub async fn serve<R, W>(self: Arc<Self>, reader: R, writer: W) -> std::io::Result<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (sender, mut receiver) = mpsc::channel::<Value>(64);
        let mut writer = writer;

        // One writer, so two answers can never share a line. A response that cannot be serialized is
        // dropped with a line on stderr rather than written as a broken line.
        let writer_task = tokio::spawn(async move {
            while let Some(response) = receiver.recv().await {
                let line = match serde_json::to_string(&response) {
                    Ok(line) => line,
                    Err(error) => {
                        eprintln!("sshell-mcp: a response could not be serialized: {error}");
                        continue;
                    }
                };
                if writer.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if writer.write_all(b"\n").await.is_err() {
                    break;
                }
                if writer.flush().await.is_err() {
                    break;
                }
            }
        });

        let mut lines = BufReader::new(reader).lines();
        let mut tasks: JoinSet<()> = JoinSet::new();
        // One slot per line in flight. Taken on the reading side, before a task exists: when every
        // slot is busy the next line waits in the pipe, so the backpressure reaches the client that
        // is writing rather than turning into tasks this process holds on its behalf.
        let permits = Arc::new(Semaphore::new(self.max_concurrency));

        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if line.trim().is_empty() {
                        continue;
                    }
                    let permit = match Arc::clone(&permits).acquire_owned().await {
                        Ok(permit) => permit,
                        // Unreachable while this function owns the only strong reference that can
                        // close the semaphore, and reading on is the safe direction if it ever is.
                        Err(_) => break,
                    };
                    let server = Arc::clone(&self);
                    let sender = sender.clone();
                    tasks.spawn(async move {
                        if let Some(response) = server.dispatch(&line).await {
                            // Best effort: a writer that has gone away means the client closed the
                            // pipe, and a call that finished after that has nowhere to go.
                            let _ = sender.send(response).await;
                        }
                        // The slot is given back here, when this call is done rather than when it
                        // was read, which is what bounds the work in flight.
                        drop(permit);
                    });
                }
                Ok(None) => break,
                Err(error) => return Err(error),
            }
        }

        while tasks.join_next().await.is_some() {}
        drop(sender);
        let _ = writer_task.await;
        Ok(())
    }

    fn initialize(&self, id: Id, params: Option<Value>) -> Response {
        let asked = params
            .as_ref()
            .and_then(|params| params.get("protocolVersion"))
            .and_then(Value::as_str);

        // A version this build does not speak is answered with the one it does, and the client
        // decides whether to go on. Refusing the handshake instead would leave a client with an
        // error it cannot act on and no way to say "then the newest of yours".
        let version = asked
            .and_then(|asked| {
                SUPPORTED_PROTOCOL_VERSIONS
                    .iter()
                    .copied()
                    .find(|supported| *supported == asked)
            })
            .unwrap_or(LATEST_PROTOCOL_VERSION);

        Response::result(
            Some(id),
            json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "sshell-mcp", "version": env!("CARGO_PKG_VERSION") }
            }),
        )
    }

    async fn tools_list(&self, id: Id) -> Response {
        // The desktop is authoritative: it knows the external servers' tools, which no bridge
        // compiles in. A refresh that cannot reach it leaves the compiled list in place, so the
        // answer is still complete for this build — what is missing is only what this build could
        // never have named.
        if let Err(error) = self.client.refresh_descriptors().await {
            eprintln!(
                "sshell-mcp: the desktop's tool list could not be read; answering with the built-in one: {}",
                error.message
            );
        }

        let tools: Vec<Value> = self.client.descriptors().into_iter().map(tool_listing).collect();
        Response::result(Some(id), json!({ "tools": tools }))
    }

    /// Forwards one `notifications/cancelled` to the desktop.
    ///
    /// Every outcome here is quiet, because a notification has nowhere to carry an explanation: no
    /// `requestId`, an id nothing is in flight under (the answer was already written, or the client
    /// named a request this bridge never accepted), or a desktop that cannot be reached — the last
    /// one with a line on stderr, where a person debugging an agent looks.
    async fn cancel_request(&self, params: Option<Value>) {
        let Some(request_id) = params
            .as_ref()
            .and_then(|params| params.get("requestId"))
            .and_then(|value| serde_json::from_value::<Id>(value.clone()).ok())
        else {
            return;
        };

        // The mapping is removed by the call itself once it has an answer, so "not found" means the
        // work is over. There is nothing to call off, and an error would be addressed to nobody.
        let Some(call_id) = self.call_id_for(&request_id) else {
            return;
        };

        if let Err(error) = self.client.cancel(&call_id).await {
            eprintln!(
                "sshell-mcp: cancelling request {request_id} did not reach the desktop: {}",
                error.message
            );
        }
    }

    async fn tools_call(&self, id: Id, params: Option<Value>) -> Response {
        let Some(params) = params.filter(Value::is_object) else {
            return Response::error(
                Some(id),
                RpcError::invalid_params("tools/call takes an object with a tool name and its arguments"),
            );
        };

        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return Response::error(Some(id), RpcError::invalid_params("tools/call needs a `name`"));
        };

        let arguments = match params.get("arguments") {
            None => json!({}),
            Some(arguments) if arguments.is_object() => arguments.clone(),
            Some(_) => {
                return Response::error(Some(id), RpcError::invalid_params("`arguments` has to be an object"));
            }
        };

        // The one argument an agent may never send. The tools refuse it too, but refusing it here
        // says why, and it means a request that could never be honoured never reaches the desktop.
        if arguments.get("acknowledged").is_some() {
            return Response::error(
                Some(id),
                RpcError::invalid_params(
                    "`acknowledged` is not a tool argument: a destructive action is approved in the 7Shell window, by a person",
                ),
            );
        }

        let known = self
            .client
            .descriptors()
            .iter()
            .any(|descriptor| descriptor.name == name);
        if !known {
            return Response::error(Some(id), RpcError::invalid_params(format!("no tool is named `{name}`")));
        }

        // The call's own id, minted here rather than by the desktop: the client's request id is only
        // unique within its session, and a cancellation has to name a call in a way the desktop can
        // match after the bridge is gone. It is remembered before the call starts, so a cancellation
        // that arrives while the desktop is still working finds it; it is forgotten as soon as the
        // answer exists, so a later cancellation is ignored instead of calling off something else.
        let call_id = Uuid::new_v4().to_string();
        self.remember_call(&id, &call_id);

        let outcome = self.client.call(name, arguments, &call_id).await;
        self.forget_call(&id);

        match outcome {
            Ok(ToolOutcome::Succeeded { result }) => Response::result(Some(id), self.tool_content(*result)),
            Ok(ToolOutcome::Failed { error }) => Response::error(Some(id), RpcError::from(error)),
            Err(error) => Response::error(Some(id), error),
        }
    }

    fn remember_call(&self, id: &Id, call_id: &str) {
        self.calls().insert(id.clone(), call_id.to_owned());
    }

    fn forget_call(&self, id: &Id) {
        self.calls().remove(id);
    }

    fn call_id_for(&self, id: &Id) -> Option<String> {
        self.calls().get(id).cloned()
    }

    /// The map, with a poisoned lock recovered rather than propagated.
    ///
    /// No code path here panics while holding it, and a session that answered no more calls because
    /// an unrelated task died is worse than one that goes on with the map it had.
    fn calls(&self) -> std::sync::MutexGuard<'_, HashMap<Id, String>> {
        self.calls.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Turns one tool result into the protocol's content blocks, bounded.
    ///
    /// Over the limit the text is a prefix and `structuredContent` says so instead of carrying the
    /// value: a truncated JSON document would be a document that parses as nothing, which is worse
    /// for an agent than one that knows it was cut. Below the limit the value is carried whole and
    /// only then — a `structuredContent` that is sometimes complete and sometimes not, with no
    /// marker, is the thing an agent cannot tell apart. The marker is data rather than a sentence,
    /// so nothing here has to be translated.
    fn tool_content(&self, result: ToolResult) -> Value {
        let text = match serde_json::to_string(&result.value) {
            Ok(text) => text,
            Err(error) => {
                return json!({
                    "content": [{ "type": "text", "text": format!("the result could not be serialized: {error}") }],
                    "isError": true,
                });
            }
        };

        // The endpoint's configured value is the effective ceiling; the server's own field is the
        // test/one-shot cap and the guard against a malformed zero. Both are respected, so a test
        // that asks for 64 bytes and a desktop that asks for 512 KiB each get what they asked for.
        let limit = self.client.max_result_bytes().clamp(1, self.max_result_bytes.max(1));

        if text.len() <= limit {
            return json!({
                "content": [{ "type": "text", "text": text }],
                "structuredContent": result.value,
            });
        }

        json!({
            "content": [{ "type": "text", "text": truncate(&text, limit) }],
            "structuredContent": { "truncated": true, "originalBytes": text.len() },
        })
    }
}

/// The id of a value that did not parse as a request, when it carries one of the protocol's shapes.
fn id_of(value: &Value) -> Option<Id> {
    value.get("id").and_then(|id| serde_json::from_value(id.clone()).ok())
}

/// One entry of `tools/list`.
fn tool_listing(descriptor: ToolDescriptor) -> Value {
    let ToolDescriptor {
        name,
        title,
        description,
        input_schema,
        access,
    } = descriptor;

    let annotation_title = title.clone();

    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": input_schema,
        "annotations": {
            "title": annotation_title,
            "readOnlyHint": access == ToolAccess::Read,
            // Every tool answers from this machine's own vault, servers and cluster: nothing here
            // reaches a third party the person did not name.
            "openWorldHint": false,
        },
    })
}

/// Cuts at a character boundary no further than `limit` bytes in.
fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}
