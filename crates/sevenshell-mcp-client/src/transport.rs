//! How a session talks to one server.
//!
//! Three transports, one trait. The trait is deliberately narrow — a request that waits for one
//! answer, a notification that does not, and closing the session — because everything else about
//! MCP (initialise, list, call, ping) is built from those three and should not have to be written
//! once per transport.
//!
//! Each transport owns JSON-RPC ids and correlation, and nothing above this module ever sees an id.
//! A request that is dropped is a request nobody is waiting for any more: the reply, if it ever
//! arrives, is discarded by the pending map, and a call's cancellation is exactly that drop.
//!
//! Three things are shared by all of them:
//!
//! * **A budget for the connection.** [`CONNECT_TIMEOUT`] covers dialling and, for the two network
//!   transports, the wait for the stream a server answers on. The handshake above it has its own
//!   copy of the same budget, because "connected" and "speaking the protocol" fail differently.
//! * **Bounded reading.** A line or a body is read up to a stated limit and refused past it: a
//!   server that sends a 4 GB answer must not be able to make this build hold it in memory.
//! * **No secret leaves except into the place it belongs.** A vault header becomes one request's
//!   header and is marked sensitive; an environment value becomes one child's environment. Neither
//!   appears in an error: transport details are scrubbed and cut before an error keeps them.
//!
//! The stdio transport's `Debug` is absent on purpose, and the two network transports print only the
//! server id: their state holds header values, which are credentials.

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use bytes::{Buf, Bytes, BytesMut};
use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};
use sevenshell_types::{McpServerConfig, McpServerTransport};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex as AsyncMutex, oneshot};
use tokio::task::JoinHandle;

use crate::McpClientError;
use crate::config::{McpServerSecrets, validate_server};

/// How long opening a session may take.
///
/// Fifteen seconds: a local child process starts in milliseconds, and a remote server that has not
/// answered a handshake by then is one whose row should say `failed` rather than hang on
/// `connecting` for as long as the application runs.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// How long asking for the tool list may take.
///
/// The same fifteen seconds, for the same reason: a list is a request like any other, and a server
/// that cannot answer one is one whose tools are not worth waiting for.
pub const LIST_TIMEOUT: Duration = Duration::from_secs(15);

/// The most one line (stdio) or one body (HTTP) may be before it is refused.
///
/// The settings page's largest result limit is 16 MiB; a serialized answer plus its JSON-RPC
/// envelope fits under twice that with room to spare. A server past this is broken rather than
/// verbose, and holding the line would be this build's memory, not the server's.
const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

/// How much of an HTTP error body is read for the refusal's detail. It is a sentence, not a result.
const MAX_ERROR_BODY_BYTES: usize = 4 * 1024;

/// How much of a child's stderr is kept for a diagnostic.
const STDERR_CAPACITY: usize = 8 * 1024;

/// The longest detail string an error keeps.
///
/// The envelope's own bound is 64 KiB because a detail is usually a system error; a refusal from
/// another server is a sentence, and a short one is what a log line wants.
const DETAIL_CHARS: usize = 512;

/// How long closing a Streamable HTTP session may take to tell the server, and how long waiting
/// for a killed child to be reaped may take.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// The listener a session registers for server-initiated notifications.
///
/// The transport only says which method arrived; what to do about it belongs to the session. A
/// notification has no answer, so a handler is synchronous and must not block the reader — the one
/// caller here forwards it to a channel.
pub type NotificationHandler = Arc<dyn Fn(&str) + Send + Sync>;

/// One live session's message channel.
#[async_trait]
pub trait McpTransport: Send + Sync {
    /// Sends one JSON-RPC request and waits for its answer.
    ///
    /// The transport owns ids and correlation: the caller passes a method and params and gets the
    /// `result` back, or the server's error as [`McpClientError::Failed`]. A cancellation is the
    /// future being dropped.
    async fn request(&self, method: &str, params: Option<Value>) -> Result<Value, McpClientError>;

    /// Sends one JSON-RPC notification. There is no answer to wait for, by definition.
    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), McpClientError>;

    /// Ends the session: closes the child's stdin or the event stream. Idempotent.
    async fn close(&self) -> Result<(), McpClientError>;

    /// Registers the session's listener for server-initiated notifications.
    ///
    /// A transport that has no channel for them leaves the default in place: the session then
    /// simply does not hear about a changed tool list and the person can refresh by hand.
    fn set_notification_handler(&self, _handler: NotificationHandler) {}

    /// Tells the transport which revision the handshake agreed on.
    ///
    /// Only Streamable HTTP has a place to put it: the protocol asks a client to carry
    /// `MCP-Protocol-Version` on the requests that follow `initialize`. A child process agreed once
    /// and an event stream has no per-message headers, so the default does nothing.
    fn set_protocol_version(&self, _version: &str) {}

    /// The tail of what a child process wrote to stderr, when there is a child.
    ///
    /// This exists for a log line and for a test that proves stderr does not block the protocol. It
    /// is deliberately not part of any error: a server's stderr is arbitrary text, and a server that
    /// printed its own configuration would be quoted into an event if this reached one. Whoever logs
    /// it decides what to do with it.
    fn diagnostics(&self) -> Option<String> {
        None
    }
}

/// Opens the transport a server's configuration names.
///
/// The configuration is validated here as well as in the manager: a caller that builds a session
/// directly (a test, a future command line) gets the same refusal rather than a child process
/// started from a missing program.
pub async fn connect(
    server: &McpServerConfig,
    secrets: Option<McpServerSecrets>,
) -> Result<Box<dyn McpTransport>, McpClientError> {
    let server = validate_server(server)?;
    match server.transport {
        McpServerTransport::Stdio => Ok(Box::new(StdioTransport::connect(&server, secrets)?)),
        McpServerTransport::Http => Ok(Box::new(HttpTransport::connect(&server, secrets)?)),
        McpServerTransport::Sse => Ok(Box::new(SseTransport::connect(&server, secrets).await?)),
    }
}

/// Where a session's transport comes from.
///
/// A trait rather than a free function call so a test can hand the manager a scripted transport
/// without a child process or a socket, and so the host can one day install a transport of its own
/// (a tunnel, a proxy) without any of the layers above noticing.
#[async_trait]
pub trait TransportFactory: Send + Sync {
    async fn open(
        &self,
        server: &McpServerConfig,
        secrets: Option<McpServerSecrets>,
    ) -> Result<Box<dyn McpTransport>, McpClientError>;
}

/// The factory every real manager uses: [`connect`].
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTransportFactory;

#[async_trait]
impl TransportFactory for DefaultTransportFactory {
    async fn open(
        &self,
        server: &McpServerConfig,
        secrets: Option<McpServerSecrets>,
    ) -> Result<Box<dyn McpTransport>, McpClientError> {
        connect(server, secrets).await
    }
}

// ---------------------------------------------------------------------------------------------
// The pieces all three transports share
// ---------------------------------------------------------------------------------------------

type PendingSender = oneshot::Sender<Result<Value, McpClientError>>;

/// Requests that are waiting for an answer, by id.
///
/// A reply whose id is not here belongs to a request nobody is waiting for any more — the future
/// was dropped when its caller gave up — and is discarded. That is the whole of cancellation at
/// this layer.
#[derive(Default)]
struct Pending {
    entries: Mutex<HashMap<i64, PendingSender>>,
}

impl Pending {
    fn add(&self, id: i64, sender: PendingSender) {
        self.lock().insert(id, sender);
    }

    fn remove(&self, id: i64) {
        self.lock().remove(&id);
    }

    /// Answers one waiting request. `false` when nobody was waiting.
    fn settle(&self, id: i64, outcome: Result<Value, McpClientError>) -> bool {
        match self.lock().remove(&id) {
            Some(sender) => {
                // The receiver is gone exactly when the caller dropped the future; the reply has
                // nowhere to go, which is not an error here.
                let _ = sender.send(outcome);
                true
            }
            None => false,
        }
    }

    fn fail_all(&self, error: &McpClientError) {
        let waiting: Vec<(i64, PendingSender)> = self.lock().drain().collect();
        for (_, sender) in waiting {
            let _ = sender.send(Err(error.clone()));
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<i64, PendingSender>> {
        match self.entries.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// Removes a request from the pending map when the waiting future is dropped.
///
/// A caller that gives up — a timeout, a person cancelling — drops the future. Without this, the
/// id and its sender would stay in the map until the whole transport failed, which on a long-lived
/// session with a slow server is a growing map nobody reads. Dropping a request that was already
/// answered removes nothing, which is why no case needs to be distinguished here.
struct PendingEntry<'a> {
    pending: &'a Pending,
    id: i64,
}

impl Drop for PendingEntry<'_> {
    fn drop(&mut self) {
        self.pending.remove(self.id);
    }
}

fn request_message(id: i64, method: &str, params: Option<Value>) -> Value {
    let mut message = json!({ "jsonrpc": "2.0", "id": id, "method": method });
    if let Some(params) = params {
        message["params"] = params;
    }
    message
}

fn notification_message(method: &str, params: Option<Value>) -> Value {
    let mut message = json!({ "jsonrpc": "2.0", "method": method });
    if let Some(params) = params {
        message["params"] = params;
    }
    message
}

/// Answers the request a message is a reply to, when it is a reply.
///
/// A message carrying `method` is a request or a notification from the server, not an answer; those
/// are the caller's business (the stdio reader refuses them explicitly, the HTTP streams ignore
/// them because a one-request response has no place to send a reply).
fn settle_reply(server: &str, pending: &Pending, message: &Value) -> bool {
    if message.get("method").is_some() {
        return false;
    }
    let Some(id) = message.get("id").and_then(Value::as_i64) else {
        return false;
    };
    let outcome = match message.get("error") {
        Some(error) => Err(rpc_error(server, error)),
        None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
    };
    pending.settle(id, outcome)
}

/// The server's own error, with its JSON-RPC code kept.
///
/// A JSON-RPC error is data (`code`, `message`, sometimes `data`); only the first two are kept.
/// `data` is left out because a server may put a request's arguments in it, and those can carry
/// anything the tool was given. The code is kept because `-32601` means "this optional method is
/// not implemented", which is not the same as a session that went away.
fn rpc_error(server: &str, error: &Value) -> McpClientError {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("it sent no message");
    match error.get("code").and_then(Value::as_i64) {
        Some(code) => McpClientError::Rpc {
            server: server.to_owned(),
            code,
            message: bounded_detail(message),
        },
        None => failed(server, &format!("the server answered with an error: {message}")),
    }
}

fn bounded_detail(detail: &str) -> String {
    sevenshell_error::scrub_bounded(detail, DETAIL_CHARS)
}

fn connect_failed(server: &str, detail: &str) -> McpClientError {
    McpClientError::ConnectFailed {
        server: server.to_owned(),
        detail: bounded_detail(detail),
    }
}

fn failed(server: &str, message: &str) -> McpClientError {
    McpClientError::Failed {
        server: server.to_owned(),
        message: bounded_detail(message),
    }
}

/// The headers a server's vault entry becomes.
///
/// Every value is marked sensitive: which of them is a credential is not knowable from here, and a
/// header dump that printed one would be the leak this whole arrangement exists to prevent. A name
/// or value an HTTP header cannot carry is refused without quoting it.
fn secret_headers(server: &McpServerConfig, secrets: Option<McpServerSecrets>) -> Result<HeaderMap, McpClientError> {
    let mut headers = HeaderMap::new();
    let Some(secrets) = secrets else {
        return Ok(headers);
    };

    for (name, value) in &secrets.headers {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| McpClientError::InvalidConfig {
            server: server.id.clone(),
            detail: "a configured header name is not a name an HTTP header can carry".to_owned(),
        })?;
        let mut value = HeaderValue::from_str(value).map_err(|_| McpClientError::InvalidConfig {
            server: server.id.clone(),
            detail: "a configured header value is not a value an HTTP header can carry".to_owned(),
        })?;
        value.set_sensitive(true);
        headers.insert(name, value);
    }

    Ok(headers)
}

/// One HTTP client for one server.
///
/// No total request timeout: a tool call may legitimately take the server's whole budget (ten
/// minutes at the ceiling). The connect timeout is the part that must always be quick, and every
/// request above this layer is wrapped in its own budget.
fn build_client(server: &McpServerConfig) -> Result<reqwest::Client, McpClientError> {
    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(concat!("7Shell/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| connect_failed(&server.id, &format!("the HTTP client could not be built: {error}")))?;
    Ok(client)
}

fn parse_url(server: &McpServerConfig) -> Result<reqwest::Url, McpClientError> {
    let raw = server.url.as_deref().ok_or_else(|| McpClientError::InvalidConfig {
        server: server.id.clone(),
        detail: format!("a {} server needs an address", server.transport.as_str()),
    })?;
    reqwest::Url::parse(raw).map_err(|_| McpClientError::InvalidConfig {
        server: server.id.clone(),
        detail: "the address is not a URL".to_owned(),
    })
}

/// Reads one line, up to `limit` bytes, discarding the rest of a line that is longer.
///
/// The overflow is drained rather than left in the buffer: a reader that stopped mid-line would
/// hand the next call the tail of a message it has already refused.
async fn read_line_bounded<R>(reader: &mut R, limit: usize) -> io::Result<Option<Line>>
where
    R: AsyncBufRead + Unpin,
{
    let mut bytes = Vec::new();
    let mut truncated = false;

    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return if bytes.is_empty() && !truncated {
                Ok(None)
            } else {
                Ok(Some(Line { bytes, truncated }))
            };
        }

        let newline = available.iter().position(|byte| *byte == b'\n');
        let upto = newline.unwrap_or(available.len());
        let room = limit.saturating_sub(bytes.len());
        if upto > room {
            truncated = true;
            bytes.extend_from_slice(&available[..room]);
        } else {
            bytes.extend_from_slice(&available[..upto]);
        }
        let consumed = match newline {
            Some(_) => upto + 1,
            None => available.len(),
        };
        reader.consume(consumed);

        if newline.is_some() {
            return Ok(Some(Line { bytes, truncated }));
        }
    }
}

struct Line {
    bytes: Vec<u8>,
    truncated: bool,
}

/// The tail of a child's stderr, capped at [`STDERR_CAPACITY`].
///
/// A ring in the sense that matters: the newest bytes are kept and the oldest are dropped, so a
/// server that logs every request forever cannot grow this.
#[derive(Default)]
struct StderrLog {
    bytes: Mutex<Vec<u8>>,
}

impl StderrLog {
    fn append(&self, chunk: &[u8]) {
        let mut bytes = match self.bytes.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        bytes.extend_from_slice(chunk);
        if bytes.len() > STDERR_CAPACITY {
            let drop = bytes.len() - STDERR_CAPACITY;
            bytes.drain(..drop);
        }
    }

    fn tail(&self) -> String {
        let bytes = match self.bytes.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

// ---------------------------------------------------------------------------------------------
// stdio
// ---------------------------------------------------------------------------------------------

/// A child process spoken to over stdin and stdout, one JSON message per line.
struct StdioTransport {
    server: String,
    stdin: Arc<AsyncMutex<Option<ChildStdin>>>,
    child: Mutex<Option<Child>>,
    pending: Arc<Pending>,
    next_id: AtomicI64,
    closed: Arc<AtomicBool>,
    stderr: Arc<StderrLog>,
    notification: Arc<Mutex<Option<NotificationHandler>>>,
}

impl StdioTransport {
    fn connect(server: &McpServerConfig, secrets: Option<McpServerSecrets>) -> Result<Self, McpClientError> {
        let program = server.command.as_deref().ok_or_else(|| McpClientError::InvalidConfig {
            server: server.id.clone(),
            detail: "an stdio server needs a command to start".to_owned(),
        })?;

        let mut command = Command::new(program);
        command
            .args(&server.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            // A session that is dropped without `close` — a window being torn down, a manager
            // replaced — still takes the child with it. A server left running would hold whatever
            // the person configured it with.
            .kill_on_drop(true);

        // Only the configured variables are added; the child inherits the rest of this process's
        // environment, because an interpreter on `PATH` is what a command like `npx` needs.
        if let Some(secrets) = &secrets {
            for (name, value) in &secrets.env {
                command.env(name, value);
            }
        }

        let mut child = command
            .spawn()
            .map_err(|error| connect_failed(&server.id, &format!("`{program}` could not be started: {error}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| connect_failed(&server.id, "the child's input could not be opened"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| connect_failed(&server.id, "the child's output could not be opened"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| connect_failed(&server.id, "the child's error output could not be opened"))?;

        let stdin = Arc::new(AsyncMutex::new(Some(stdin)));
        let pending = Arc::new(Pending::default());
        let closed = Arc::new(AtomicBool::new(false));
        let stderr_log = Arc::new(StderrLog::default());
        let notification = Arc::new(Mutex::new(None));

        spawn_stdout_reader(
            server.id.clone(),
            stdout,
            Arc::clone(&stdin),
            Arc::clone(&pending),
            Arc::clone(&closed),
            Arc::clone(&notification),
        );
        spawn_stderr_reader(stderr, Arc::clone(&stderr_log));

        Ok(Self {
            server: server.id.clone(),
            stdin,
            child: Mutex::new(Some(child)),
            pending,
            next_id: AtomicI64::new(1),
            closed,
            stderr: stderr_log,
            notification,
        })
    }

    fn not_running(&self) -> McpClientError {
        failed(&self.server, "the server is not running")
    }

    fn mark_closed(&self, error: McpClientError) {
        self.closed.store(true, Ordering::SeqCst);
        self.pending.fail_all(&error);
    }

    async fn write(&self, message: &Value) -> bool {
        write_message(&self.stdin, message).await
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn request(&self, method: &str, params: Option<Value>) -> Result<Value, McpClientError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(self.not_running());
        }

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = oneshot::channel();
        self.pending.add(id, sender);
        let _waiting = PendingEntry {
            pending: &self.pending,
            id,
        };

        if !self.write(&request_message(id, method, params)).await {
            self.mark_closed(self.not_running());
            return Err(self.not_running());
        }

        match receiver.await {
            Ok(outcome) => outcome,
            // The sender is dropped only when the pending map is drained, which is the child
            // going away. Saying so is better than an error that names no cause.
            Err(_) => Err(self.not_running()),
        }
    }

    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), McpClientError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(self.not_running());
        }
        if self.write(&notification_message(method, params)).await {
            Ok(())
        } else {
            self.mark_closed(self.not_running());
            Err(self.not_running())
        }
    }

    async fn close(&self) -> Result<(), McpClientError> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        // The input is dropped first so a child that watches its stdin sees the end of the session
        // before the kill lands; both happen either way.
        let stdin = self.stdin.lock().await.take();
        drop(stdin);

        let child = self.child.lock().ok().and_then(|mut guard| guard.take());
        if let Some(mut child) = child {
            let _ = child.start_kill();
            // A killed child is reaped almost immediately; the timeout is for the platform that
            // makes that untrue, and dropping the handle after it still leaves `kill_on_drop`.
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, child.wait()).await;
        }

        self.pending.fail_all(&failed(&self.server, "the session was closed"));
        Ok(())
    }

    fn set_notification_handler(&self, handler: NotificationHandler) {
        if let Ok(mut guard) = self.notification.lock() {
            *guard = Some(handler);
        }
    }

    fn diagnostics(&self) -> Option<String> {
        Some(self.stderr.tail())
    }
}

async fn write_message(stdin: &AsyncMutex<Option<ChildStdin>>, message: &Value) -> bool {
    let Ok(mut body) = serde_json::to_vec(message) else {
        return false;
    };
    body.push(b'\n');

    let mut guard = stdin.lock().await;
    let Some(stdin) = guard.as_mut() else {
        return false;
    };
    if stdin.write_all(&body).await.is_err() {
        return false;
    }
    stdin.flush().await.is_ok()
}

fn spawn_stdout_reader(
    server: String,
    stdout: tokio::process::ChildStdout,
    stdin: Arc<AsyncMutex<Option<ChildStdin>>>,
    pending: Arc<Pending>,
    closed: Arc<AtomicBool>,
    notification: Arc<Mutex<Option<NotificationHandler>>>,
) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);

        loop {
            match read_line_bounded(&mut reader, MAX_RESPONSE_BYTES).await {
                Ok(Some(line)) if line.truncated => {
                    closed.store(true, Ordering::SeqCst);
                    pending.fail_all(&failed(&server, "the server sent a line larger than this build reads"));
                    break;
                }
                Ok(Some(line)) => {
                    if line.bytes.iter().all(u8::is_ascii_whitespace) {
                        continue;
                    }
                    let Ok(message) = serde_json::from_slice::<Value>(&line.bytes) else {
                        // Not JSON on the protocol channel. Whatever it was, no request can be
                        // answered from it, and the requests that are waiting will fail at their
                        // own budgets rather than be failed here by an id nobody can read.
                        continue;
                    };
                    if settle_reply(&server, &pending, &message) {
                        continue;
                    }
                    if let Some(id) = message.get("id") {
                        // A request from the server (sampling, roots) is a capability this build
                        // does not offer. Answering says so, and leaves the server free to carry
                        // on instead of waiting for a reply that never comes.
                        let refusal = json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": {
                                "code": -32601,
                                "message": "this client does not implement that method",
                            },
                        });
                        if !write_message(&stdin, &refusal).await {
                            break;
                        }
                    } else if let Some(method) = message.get("method").and_then(Value::as_str) {
                        // A notification: no id, no answer. The reader must not wait on one, so the
                        // handler only forwards the name to the session's own task.
                        let handler = notification.lock().ok().and_then(|guard| guard.clone());
                        if let Some(handler) = handler {
                            handler(method);
                        }
                    }
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }

        closed.store(true, Ordering::SeqCst);
        pending.fail_all(&failed(&server, "the server stopped answering (it closed its output)"));
    });
}

fn spawn_stderr_reader(stderr: tokio::process::ChildStderr, log: Arc<StderrLog>) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        while let Ok(Some(line)) = read_line_bounded(&mut reader, STDERR_CAPACITY).await {
            log.append(&line.bytes);
            log.append(b"\n");
        }
    });
}

// ---------------------------------------------------------------------------------------------
// Streamable HTTP
// ---------------------------------------------------------------------------------------------

/// One request per message, with the session header a server hands back.
struct HttpTransport {
    server: String,
    client: reqwest::Client,
    url: reqwest::Url,
    headers: HeaderMap,
    session_id: Mutex<Option<String>>,
    protocol_version: Mutex<Option<String>>,
    next_id: AtomicI64,
    closed: AtomicBool,
    notification: Arc<Mutex<Option<NotificationHandler>>>,
    listener: Mutex<Option<JoinHandle<()>>>,
}

impl HttpTransport {
    fn connect(server: &McpServerConfig, secrets: Option<McpServerSecrets>) -> Result<Self, McpClientError> {
        Ok(Self {
            server: server.id.clone(),
            client: build_client(server)?,
            url: parse_url(server)?,
            headers: secret_headers(server, secrets)?,
            session_id: Mutex::new(None),
            protocol_version: Mutex::new(None),
            next_id: AtomicI64::new(1),
            closed: AtomicBool::new(false),
            notification: Arc::new(Mutex::new(None)),
            listener: Mutex::new(None),
        })
    }

    fn session_id(&self) -> Option<String> {
        self.session_id.lock().ok().and_then(|guard| guard.clone())
    }

    fn protocol_version(&self) -> Option<String> {
        self.protocol_version.lock().ok().and_then(|guard| guard.clone())
    }

    /// Keeps the id a server handed out, so every later request carries it.
    ///
    /// The id is read from every response rather than only from `initialize`: a server is allowed
    /// to rotate it, and a client that only looked once would go on using a header it has been told
    /// to stop using.
    fn capture_session(&self, response: &reqwest::Response) {
        let Some(value) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
        else {
            return;
        };
        let value = value.trim();
        if value.is_empty() {
            return;
        }
        if let Ok(mut guard) = self.session_id.lock() {
            *guard = Some(value.to_owned());
        }
    }

    fn transport_error(&self, error: &reqwest::Error) -> McpClientError {
        failed(&self.server, &error.to_string())
    }

    /// Opens the one GET stream that carries server-initiated messages, when it makes sense to.
    ///
    /// Streamable HTTP has no long-lived channel unless the client asks for one; `notifications`
    /// such as a changed tool list are delivered on it. The stream is started after the handshake
    /// has given us a session id, and a server that refuses the GET (405 is legal) simply has no
    /// push channel — its tools remain usable and the person can refresh by hand.
    fn ensure_listener(&self) {
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
        let Some(handler) = self.notification.lock().ok().and_then(|guard| guard.clone()) else {
            return;
        };
        let Some(session) = self.session_id() else {
            return;
        };
        let Ok(mut listener) = self.listener.lock() else {
            return;
        };
        if listener.is_some() {
            return;
        }

        let mut request = self
            .client
            .get(self.url.clone())
            .header(ACCEPT, "text/event-stream")
            .headers(self.headers.clone())
            .header("Mcp-Session-Id", session);
        if let Some(version) = self.protocol_version() {
            request = request.header("MCP-Protocol-Version", version);
        }

        *listener = Some(tokio::spawn(async move {
            let Ok(response) = request.send().await else {
                return;
            };
            if !response.status().is_success() {
                return;
            }
            read_notification_stream(response, handler).await;
        }));
    }

    fn stop_listener(&self) {
        if let Ok(mut listener) = self.listener.lock()
            && let Some(task) = listener.take()
        {
            task.abort();
        }
    }

    async fn send(&self, message: &Value) -> Result<reqwest::Response, McpClientError> {
        let body = serde_json::to_vec(message)
            .map_err(|error| failed(&self.server, &format!("the request could not be serialized: {error}")))?;

        let mut request = self
            .client
            .post(self.url.clone())
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json, text/event-stream")
            .headers(self.headers.clone())
            .body(body);

        if let Some(session) = self.session_id() {
            request = request.header("Mcp-Session-Id", session);
        }
        if let Some(version) = self.protocol_version() {
            request = request.header("MCP-Protocol-Version", version);
        }

        request.send().await.map_err(|error| self.transport_error(&error))
    }

    async fn read_answer(&self, id: i64, response: reqwest::Response) -> Result<Value, McpClientError> {
        let status = response.status();
        self.capture_session(&response);

        if status.as_u16() == 202 || status.as_u16() == 204 {
            // An acknowledgement with no body. A request answered this way has no result, which is
            // `null`; a server that does it for `tools/call` is answered as a server that said
            // nothing rather than as a success with content.
            return Ok(Value::Null);
        }

        if !status.is_success() {
            let body = self.read_body(response, MAX_ERROR_BODY_BYTES).await.unwrap_or_default();
            let summary = summarize_body(&body);
            return Err(failed(
                &self.server,
                &format!("the server answered status {}{summary}", status.as_u16()),
            ));
        }

        let is_event_stream = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.to_ascii_lowercase().contains("text/event-stream"));

        if is_event_stream {
            return self.read_streamed(id, response).await;
        }

        let body = self.read_body(response, MAX_RESPONSE_BYTES).await?;
        let message: Value = serde_json::from_slice(&body).map_err(|error| {
            failed(
                &self.server,
                &format!("the server answered something that is not JSON: {error}"),
            )
        })?;
        Self::match_answer(&self.server, id, &message)
    }

    /// A response whose body is an event stream: the answer is the event carrying our id.
    async fn read_streamed(&self, id: i64, response: reqwest::Response) -> Result<Value, McpClientError> {
        let mut stream = response.bytes_stream();
        let mut buffer = SseBuffer::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| self.transport_error(&error))?;
            for event in buffer.push(&chunk) {
                let data = event.data.trim();
                if data.is_empty() {
                    continue;
                }
                let Ok(message) = serde_json::from_str::<Value>(data) else {
                    continue;
                };
                if message.get("id").and_then(Value::as_i64) == Some(id) {
                    return Self::match_answer(&self.server, id, &message);
                }
                // Notifications and messages for other ids have no place in a one-request
                // response; ignoring them is the only thing a body that has already been read can
                // do with them.
            }
            if buffer.buffered() > MAX_RESPONSE_BYTES {
                return Err(failed(
                    &self.server,
                    "the server sent an event larger than this build reads",
                ));
            }
        }

        Err(failed(
            &self.server,
            "the server closed the response stream before answering",
        ))
    }

    fn match_answer(server: &str, id: i64, message: &Value) -> Result<Value, McpClientError> {
        if let Some(answered) = message.get("id").and_then(Value::as_i64)
            && answered != id
        {
            return Err(failed(
                server,
                &format!("the server answered request {answered} while request {id} was waiting"),
            ));
        }
        match message.get("error") {
            Some(error) => Err(rpc_error(server, error)),
            None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
        }
    }

    async fn read_body(&self, response: reqwest::Response, limit: usize) -> Result<Bytes, McpClientError> {
        let mut stream = response.bytes_stream();
        let mut body = BytesMut::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| self.transport_error(&error))?;
            if body.len() + chunk.len() > limit {
                return Err(failed(
                    &self.server,
                    &format!("the server answered more than {limit} bytes"),
                ));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body.freeze())
    }
}

#[async_trait]
impl McpTransport for HttpTransport {
    async fn request(&self, method: &str, params: Option<Value>) -> Result<Value, McpClientError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(failed(&self.server, "the session was closed"));
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let response = self.send(&request_message(id, method, params)).await?;
        let answer = self.read_answer(id, response).await;
        if method == "initialize" {
            // The handshake has given us a session id; now the push channel can be opened.
            self.ensure_listener();
        }
        answer
    }

    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), McpClientError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(failed(&self.server, "the session was closed"));
        }
        let response = self.send(&notification_message(method, params)).await?;
        let status = response.status();
        self.capture_session(&response);
        if status.is_success() {
            self.ensure_listener();
            Ok(())
        } else {
            Err(failed(
                &self.server,
                &format!("the server answered status {}", status.as_u16()),
            ))
        }
    }

    async fn close(&self) -> Result<(), McpClientError> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.stop_listener();

        // The protocol asks a client to end a Streamable HTTP session with a DELETE, so that a
        // server holding state for us can drop it. A server that does not implement it answers
        // something; nothing about closing this side should fail because of that.
        if let Some(session) = self.session_id() {
            let request = self
                .client
                .delete(self.url.clone())
                .headers(self.headers.clone())
                .header("Mcp-Session-Id", session);
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, request.send()).await;
        }
        Ok(())
    }

    fn set_protocol_version(&self, version: &str) {
        if let Ok(mut guard) = self.protocol_version.lock() {
            *guard = Some(version.to_owned());
        }
    }

    fn set_notification_handler(&self, handler: NotificationHandler) {
        if let Ok(mut guard) = self.notification.lock() {
            *guard = Some(handler);
        }
        self.ensure_listener();
    }
}

impl Drop for HttpTransport {
    fn drop(&mut self) {
        self.stop_listener();
    }
}

/// Reads the standalone GET stream: server messages that are not answers to a POST.
///
/// Only notifications are passed on. A request from the server (sampling, roots) is a capability
/// this build does not offer, and answering it here would need a second POST channel; ignoring it
/// leaves the server waiting, which is the same thing the older transport already does.
async fn read_notification_stream(response: reqwest::Response, handler: NotificationHandler) {
    let mut stream = response.bytes_stream();
    let mut buffer = SseBuffer::new();

    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            break;
        };
        for event in buffer.push(&chunk) {
            if event.event.as_deref().is_some_and(|name| name != "message") {
                continue;
            }
            let Ok(message) = serde_json::from_str::<Value>(&event.data) else {
                continue;
            };
            if message.get("id").is_none()
                && let Some(method) = message.get("method").and_then(Value::as_str)
            {
                handler(method);
            }
        }
        if buffer.buffered() > MAX_RESPONSE_BYTES {
            break;
        }
    }
}

/// A short, scrubbed hint from an HTTP error body, inside parentheses.
///
/// The body of a refusal is a server's own words and may name a path or an account; it is scrubbed
/// and cut like every other detail. What an operator needs from it is the first sentence.
fn summarize_body(body: &Bytes) -> String {
    let text = String::from_utf8_lossy(body);
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    let cut = text.chars().take(240).collect::<String>();
    format!(" ({})", bounded_detail(&cut))
}

// ---------------------------------------------------------------------------------------------
// HTTP + SSE (the 2024-11-05 style stream)
// ---------------------------------------------------------------------------------------------

/// A long-lived event stream for what the server says, with requests posted back to an endpoint
/// the stream names.
///
/// A stream that ends is an error, not a reason to reconnect. Reconnecting would be the transport
/// deciding that the session is still alive, and every request in flight would have to be replayed
/// onto a server that may have already acted on it. The manager's own refresh is the one place a
/// session is rebuilt, and it does not replay anything.
struct SseTransport {
    server: String,
    client: reqwest::Client,
    headers: HeaderMap,
    session_id: Mutex<Option<String>>,
    post_url: Arc<RwLock<Option<reqwest::Url>>>,
    pending: Arc<Pending>,
    next_id: AtomicI64,
    closed: Arc<AtomicBool>,
    stream: Mutex<Option<JoinHandle<()>>>,
    notification: Arc<Mutex<Option<NotificationHandler>>>,
}

impl SseTransport {
    async fn connect(server: &McpServerConfig, secrets: Option<McpServerSecrets>) -> Result<Self, McpClientError> {
        let url = parse_url(server)?;
        let client = build_client(server)?;
        let headers = secret_headers(server, secrets)?;

        let response = tokio::time::timeout(
            CONNECT_TIMEOUT,
            client
                .get(url.clone())
                .header(ACCEPT, "text/event-stream")
                .headers(headers.clone())
                .send(),
        )
        .await
        .map_err(|_| {
            connect_failed(
                &server.id,
                "the server did not answer the event stream within 15 seconds",
            )
        })?
        .map_err(|error| connect_failed(&server.id, &error.to_string()))?;

        if !response.status().is_success() {
            return Err(connect_failed(
                &server.id,
                &format!("the server answered status {}", response.status().as_u16()),
            ));
        }

        let session_id = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);

        let pending = Arc::new(Pending::default());
        let closed = Arc::new(AtomicBool::new(false));
        let post_url = Arc::new(RwLock::new(None));
        let notification = Arc::new(Mutex::new(None));
        let (endpoint_sender, endpoint_receiver) = oneshot::channel();

        let task = tokio::spawn(read_event_stream(
            server.id.clone(),
            url,
            response,
            Arc::clone(&pending),
            Arc::clone(&closed),
            Arc::clone(&post_url),
            Arc::clone(&notification),
            endpoint_sender,
        ));

        let endpoint = tokio::time::timeout(CONNECT_TIMEOUT, endpoint_receiver)
            .await
            .map_err(|_| {
                task.abort();
                connect_failed(
                    &server.id,
                    "the server did not name its message endpoint within 15 seconds",
                )
            })?
            .map_err(|_| connect_failed(&server.id, "the event stream ended before naming its message endpoint"))??;

        if let Ok(mut guard) = post_url.write() {
            *guard = Some(endpoint);
        }

        Ok(Self {
            server: server.id.clone(),
            client,
            headers,
            session_id: Mutex::new(session_id),
            post_url,
            pending,
            next_id: AtomicI64::new(1),
            closed,
            stream: Mutex::new(Some(task)),
            notification,
        })
    }

    fn session_id(&self) -> Option<String> {
        self.session_id.lock().ok().and_then(|guard| guard.clone())
    }

    fn endpoint(&self) -> Option<reqwest::Url> {
        self.post_url.read().ok().and_then(|guard| guard.clone())
    }

    /// Posts one message to the endpoint the stream named.
    async fn post(&self, message: &Value) -> Result<reqwest::Response, McpClientError> {
        let endpoint = self
            .endpoint()
            .ok_or_else(|| connect_failed(&self.server, "the server has not named its message endpoint"))?;
        let body = serde_json::to_vec(message)
            .map_err(|error| failed(&self.server, &format!("the request could not be serialized: {error}")))?;

        let mut request = self
            .client
            .post(endpoint)
            .header(CONTENT_TYPE, "application/json")
            .headers(self.headers.clone())
            .body(body);
        if let Some(session) = self.session_id() {
            request = request.header("Mcp-Session-Id", session);
        }

        request
            .send()
            .await
            .map_err(|error| failed(&self.server, &error.to_string()))
    }

    fn not_running(&self) -> McpClientError {
        failed(&self.server, "the server's event stream is closed")
    }
}

async fn read_event_stream(
    server: String,
    base: reqwest::Url,
    response: reqwest::Response,
    pending: Arc<Pending>,
    closed: Arc<AtomicBool>,
    post_url: Arc<RwLock<Option<reqwest::Url>>>,
    notification: Arc<Mutex<Option<NotificationHandler>>>,
    endpoint: oneshot::Sender<Result<reqwest::Url, McpClientError>>,
) {
    let mut endpoint = Some(endpoint);
    let mut stream = response.bytes_stream();
    let mut buffer = SseBuffer::new();

    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            break;
        };
        for event in buffer.push(&chunk) {
            let name = event.event.as_deref().unwrap_or("message");
            if name == "endpoint" {
                match base.join(event.data.trim()) {
                    Ok(url) => {
                        if let Ok(mut guard) = post_url.write() {
                            *guard = Some(url.clone());
                        }
                        if let Some(sender) = endpoint.take() {
                            let _ = sender.send(Ok(url));
                        }
                    }
                    Err(_) => {
                        if let Some(sender) = endpoint.take() {
                            let _ = sender.send(Err(connect_failed(
                                &server,
                                "the server named an endpoint that is not a URL",
                            )));
                        }
                    }
                }
                continue;
            }

            if let Ok(message) = serde_json::from_str::<Value>(&event.data) {
                if settle_reply(&server, &pending, &message) {
                    continue;
                }
                if message.get("id").is_none()
                    && let Some(method) = message.get("method").and_then(Value::as_str)
                {
                    let handler = notification.lock().ok().and_then(|guard| guard.clone());
                    if let Some(handler) = handler {
                        handler(method);
                    }
                }
            }
        }
        if buffer.buffered() > MAX_RESPONSE_BYTES {
            break;
        }
    }

    closed.store(true, Ordering::SeqCst);
    if let Some(sender) = endpoint.take() {
        let _ = sender.send(Err(connect_failed(
            &server,
            "the event stream ended before naming its message endpoint",
        )));
    }
    pending.fail_all(&failed(&server, "the server's event stream ended"));
}

#[async_trait]
impl McpTransport for SseTransport {
    async fn request(&self, method: &str, params: Option<Value>) -> Result<Value, McpClientError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(self.not_running());
        }

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = oneshot::channel();
        self.pending.add(id, sender);
        let _waiting = PendingEntry {
            pending: &self.pending,
            id,
        };

        let response = self.post(&request_message(id, method, params)).await;
        match response {
            Ok(response) if response.status().is_success() => {}
            Ok(response) => {
                return Err(failed(
                    &self.server,
                    &format!(
                        "the server refused the message with status {}",
                        response.status().as_u16()
                    ),
                ));
            }
            Err(error) => {
                return Err(error);
            }
        }

        match receiver.await {
            Ok(outcome) => outcome,
            Err(_) => Err(self.not_running()),
        }
    }

    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), McpClientError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(self.not_running());
        }
        let response = self.post(&notification_message(method, params)).await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(failed(
                &self.server,
                &format!(
                    "the server refused the message with status {}",
                    response.status().as_u16()
                ),
            ))
        }
    }

    async fn close(&self) -> Result<(), McpClientError> {
        self.closed.store(true, Ordering::SeqCst);
        if let Some(task) = self.stream.lock().ok().and_then(|mut guard| guard.take()) {
            task.abort();
        }
        self.pending.fail_all(&failed(&self.server, "the session was closed"));
        Ok(())
    }

    fn set_notification_handler(&self, handler: NotificationHandler) {
        if let Ok(mut guard) = self.notification.lock() {
            *guard = Some(handler);
        }
    }
}

impl Drop for SseTransport {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Some(task) = self.stream.lock().ok().and_then(|mut guard| guard.take()) {
            task.abort();
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Server-sent events
// ---------------------------------------------------------------------------------------------

/// One complete event, as it came off the stream.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SseEvent {
    event: Option<String>,
    data: String,
}

/// Turns a byte stream into events, keeping the incomplete tail until the next chunk.
///
/// The rules are the event-stream format's own: a blank line dispatches, several `data:` lines in
/// one event are joined with `\n`, a line starting with `:` is a comment (a keep-alive), and `\r\n`
/// is one line end. The same parser serves both a `text/event-stream` response to one POST and the
/// long-lived stream of the older transport, because the framing is the same and only what an event
/// *means* differs.
#[derive(Debug, Default)]
struct SseBuffer {
    pending: BytesMut,
    event: Option<String>,
    data: Option<String>,
}

impl SseBuffer {
    fn new() -> Self {
        Self::default()
    }

    /// How many bytes of an unfinished event are held.
    ///
    /// A stream whose event never ends would otherwise grow this buffer for as long as the
    /// connection stays open; the readers check this against the same limit a body has.
    fn buffered(&self) -> usize {
        self.pending.len()
    }

    fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.pending.extend_from_slice(chunk);

        let mut events = Vec::new();
        let mut consumed = 0;
        while let Some(offset) = self.pending[consumed..].iter().position(|byte| *byte == b'\n') {
            let end = consumed + offset;
            let content_end = if end > consumed && self.pending[end - 1] == b'\r' {
                end - 1
            } else {
                end
            };
            let line = self.pending[consumed..content_end].to_vec();
            consumed = end + 1;
            self.feed_line(&line, &mut events);
        }
        self.pending.advance(consumed);

        events
    }

    fn feed_line(&mut self, line: &[u8], events: &mut Vec<SseEvent>) {
        if line.is_empty() {
            if self.event.is_some() || self.data.is_some() {
                events.push(SseEvent {
                    event: self.event.take(),
                    data: self.data.take().unwrap_or_default(),
                });
            }
            return;
        }

        if line[0] == b':' {
            return;
        }

        let (field, value) = match line.iter().position(|byte| *byte == b':') {
            Some(colon) => {
                let value = &line[colon + 1..];
                (&line[..colon], value.strip_prefix(b" ").unwrap_or(value))
            }
            None => (line, &b""[..]),
        };
        let value = String::from_utf8_lossy(value).into_owned();

        match field {
            b"data" => match &mut self.data {
                Some(data) => {
                    data.push('\n');
                    data.push_str(&value);
                }
                None => self.data = Some(value),
            },
            b"event" => self.event = Some(value),
            // `id`, `retry` and anything a future stream adds are not this layer's business.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    use serde_json::json;
    use sevenshell_types::McpServerTransport;

    use crate::testing::{FakeReply, FakeRequest, FakeServer};

    fn server(id: &str, transport: McpServerTransport) -> McpServerConfig {
        McpServerConfig::new(id, id, transport)
    }

    #[test]
    fn a_long_stderr_line_is_kept_only_up_to_the_ring_and_the_newest_bytes_survive() {
        let log = StderrLog::default();
        log.append(&vec![b'a'; 9_000]);
        log.append(b"the newest line\n");

        let tail = log.tail();
        assert!(
            tail.len() <= STDERR_CAPACITY,
            "the ring is capped at {STDERR_CAPACITY} bytes, kept {}",
            tail.len()
        );
        assert!(
            tail.ends_with("the newest line\n"),
            "the newest bytes are the ones kept"
        );
    }

    #[tokio::test]
    async fn a_line_longer_than_the_limit_is_cut_and_the_rest_of_it_is_drained() {
        let mut reader = BufReader::new(&b"short\n0123456789\nafter\n"[..]);

        let first = read_line_bounded(&mut reader, 64).await.expect("read").expect("a line");
        assert_eq!(first.bytes, b"short");
        assert!(!first.truncated);

        let second = read_line_bounded(&mut reader, 4).await.expect("read").expect("a line");
        assert_eq!(second.bytes, b"0123");
        assert!(second.truncated, "a cut line says it was cut");

        let third = read_line_bounded(&mut reader, 64).await.expect("read").expect("a line");
        assert_eq!(
            third.bytes, b"after",
            "the tail of an over-long line is not handed to the next read"
        );
        assert!(read_line_bounded(&mut reader, 64).await.expect("read").is_none());
    }

    #[test]
    fn an_sse_event_is_dispatched_only_at_its_blank_line() {
        let mut buffer = SseBuffer::new();
        assert!(
            buffer.push(b"event: message\ndata: {\"id\":1}\n").is_empty(),
            "an event without its blank line is not complete"
        );
        let events = buffer.push(b"\n");
        assert_eq!(
            events,
            vec![SseEvent {
                event: Some("message".to_owned()),
                data: "{\"id\":1}".to_owned(),
            }]
        );

        let events = buffer.push(b": keep-alive\r\ndata: hello\r\n\r\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, None);
        assert_eq!(events[0].data, "hello");
    }

    #[test]
    fn a_header_value_an_http_request_cannot_carry_is_refused_without_quoting_it() {
        let mut secrets = McpServerSecrets::default();
        secrets
            .headers
            .insert("Authorization".to_owned(), "Bearer line\nbreak".to_owned());

        let error = secret_headers(&server("files", McpServerTransport::Http), Some(secrets))
            .expect_err("a newline is not a header value");
        assert_eq!(error.code(), "MCP-INVALID-ARGUMENTS");
        assert!(
            !format!("{error:?}").contains("line\\nbreak") && !format!("{error:?}").contains("line\nbreak"),
            "the value leaked into the refusal: {error:?}"
        );
    }

    #[tokio::test]
    async fn a_request_a_caller_gave_up_on_leaves_nothing_waiting() {
        let pending = Pending::default();
        let (sender, receiver) = oneshot::channel();
        pending.add(7, sender);
        {
            let _waiting = PendingEntry {
                pending: &pending,
                id: 7,
            };
            // What a timeout or a cancelled future does: the receiver goes away.
            drop(receiver);
        }
        assert!(
            !pending.settle(7, Ok(Value::Null)),
            "a request nobody is waiting for is no longer in the map"
        );
    }

    #[tokio::test]
    async fn the_http_transport_posts_json_and_answers_with_the_result() {
        let server = FakeServer::start(|request: FakeRequest| {
            assert_eq!(request.method, "POST");
            assert_eq!(request.path, "/mcp");
            assert!(
                request
                    .header("accept")
                    .is_some_and(|value| value.contains("application/json") && value.contains("text/event-stream")),
                "a Streamable HTTP client accepts both answers: {:?}",
                request.header("accept")
            );
            let body: Value = serde_json::from_slice(&request.body).expect("a JSON-RPC body");
            assert_eq!(body["method"], json!("tools/list"));
            FakeReply::json(json!({
                "jsonrpc": "2.0",
                "id": body["id"],
                "result": { "tools": [] },
            }))
        })
        .await;

        let mut config = config_for(&server, McpServerTransport::Http, "files");
        config.url = Some(format!("{}mcp", server.url));
        let transport = connect(&config, None).await.expect("a transport");

        let result = transport.request("tools/list", None).await.expect("an answer");
        assert_eq!(result, json!({ "tools": [] }));
    }

    #[tokio::test]
    async fn a_session_id_the_server_hands_out_is_echoed_on_the_next_request() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::<Option<String>>::new()));
        let recorder = Arc::clone(&seen);

        let server = FakeServer::start(move |request: FakeRequest| {
            recorder
                .lock()
                .expect("not poisoned")
                .push(request.header("mcp-session-id").map(str::to_owned));
            let body: Value = serde_json::from_slice(&request.body).expect("a JSON-RPC body");
            let reply = FakeReply::json(json!({ "jsonrpc": "2.0", "id": body["id"], "result": {} }));
            if request.header("mcp-session-id").is_none() {
                reply.with_header("Mcp-Session-Id", "session-1")
            } else {
                reply
            }
        })
        .await;

        let mut config = config_for(&server, McpServerTransport::Http, "files");
        config.url = Some(server.url.clone());
        let transport = connect(&config, None).await.expect("a transport");

        transport.request("initialize", None).await.expect("first answer");
        transport.request("tools/list", None).await.expect("second answer");

        let seen = seen.lock().expect("not poisoned").clone();
        assert_eq!(seen, vec![None, Some("session-1".to_owned())]);
    }

    #[tokio::test]
    async fn the_negotiated_revision_travels_on_later_http_requests() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::<Option<String>>::new()));
        let recorder = Arc::clone(&seen);

        let server = FakeServer::start(move |request: FakeRequest| {
            recorder
                .lock()
                .expect("not poisoned")
                .push(request.header("mcp-protocol-version").map(str::to_owned));
            let body: Value = serde_json::from_slice(&request.body).expect("a JSON-RPC body");
            FakeReply::json(json!({ "jsonrpc": "2.0", "id": body["id"], "result": {} }))
        })
        .await;

        let mut config = config_for(&server, McpServerTransport::Http, "files");
        config.url = Some(server.url.clone());
        let transport = connect(&config, None).await.expect("a transport");

        transport.request("initialize", None).await.expect("the handshake");
        transport.set_protocol_version("2025-06-18");
        transport.request("tools/list", None).await.expect("a list");

        assert_eq!(
            seen.lock().expect("not poisoned").clone(),
            vec![None, Some("2025-06-18".to_owned())],
            "the revision the server agreed to is carried on the requests that follow"
        );
    }

    #[tokio::test]
    async fn an_http_server_may_answer_a_request_inside_an_event_stream() {
        let server = FakeServer::start(|_request: FakeRequest| {
            FakeReply::event_stream(vec![
                // A notification first, which is not the answer and must be skipped.
                b": keep-alive\n\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\"}\n\n"
                    .to_vec(),
                b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"tools\":[{\"name\":\"read\"}]}}\n\n"
                    .to_vec(),
            ])
        })
        .await;

        let mut config = config_for(&server, McpServerTransport::Http, "files");
        config.url = Some(server.url.clone());
        let transport = connect(&config, None).await.expect("a transport");

        let result = transport.request("tools/list", None).await.expect("an answer");
        assert_eq!(result, json!({ "tools": [{ "name": "read" }] }));
    }

    #[tokio::test]
    async fn a_refused_status_becomes_a_failure_that_names_it() {
        let server = FakeServer::start(|_request: FakeRequest| FakeReply::status(503)).await;

        let mut config = config_for(&server, McpServerTransport::Http, "files");
        config.url = Some(server.url.clone());
        let transport = connect(&config, None).await.expect("a transport");

        let error = transport.request("tools/list", None).await.expect_err("a refusal");
        assert_eq!(error.code(), "MCP-EXTERNAL-FAILED");
        assert!(
            error.detail().is_some_and(|detail| detail.contains("503")),
            "the status is what a person needs to see: {error:?}"
        );
    }

    #[tokio::test]
    async fn a_vault_header_reaches_the_request_and_never_a_refusal() {
        let server = FakeServer::start(|request: FakeRequest| {
            if request.header("authorization") == Some("Bearer hunter2") {
                FakeReply::json(json!({ "jsonrpc": "2.0", "id": 1, "result": { "ok": true } }))
            } else {
                FakeReply::status(401)
            }
        })
        .await;

        let mut config = config_for(&server, McpServerTransport::Http, "files");
        config.url = Some(server.url.clone());

        let mut secrets = McpServerSecrets::default();
        secrets
            .headers
            .insert("Authorization".to_owned(), "Bearer hunter2".to_owned());

        let transport = connect(&config, Some(secrets)).await.expect("a transport");
        let result = transport.request("ping", None).await.expect("an answer");
        assert_eq!(result, json!({ "ok": true }));

        // The same header with a value no request can carry is refused, and the refusal does not
        // quote it.
        let mut secrets = McpServerSecrets::default();
        secrets
            .headers
            .insert("Authorization".to_owned(), "Bearer hunter2\nx".to_owned());
        let error = match connect(&config, Some(secrets)).await {
            Ok(_) => panic!("a newline cannot travel in a header"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "MCP-INVALID-ARGUMENTS");
        assert!(!format!("{error:?}").contains("hunter2"), "{error:?}");
    }

    #[tokio::test]
    async fn the_older_sse_transport_reads_its_endpoint_and_takes_answers_from_the_stream() {
        let streams = Arc::new(std::sync::Mutex::new(
            None::<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
        ));
        let posts = Arc::new(AtomicUsize::new(0));
        let base = Arc::new(std::sync::Mutex::new(String::new()));

        let server = FakeServer::start({
            let streams = Arc::clone(&streams);
            let posts = Arc::clone(&posts);
            let base = Arc::clone(&base);
            move |request: FakeRequest| match (request.method.as_str(), request.path.as_str()) {
                ("GET", "/sse") => {
                    let (frames, receiver) = tokio::sync::mpsc::unbounded_channel();
                    let endpoint = format!("{}messages", base.lock().expect("not poisoned"));
                    let _ = frames.send(format!("event: endpoint\ndata: {endpoint}\n\n").into_bytes());
                    *streams.lock().expect("not poisoned") = Some(frames);
                    FakeReply::event_stream_live(receiver)
                }
                ("POST", "/messages") => {
                    posts.fetch_add(1, Ordering::SeqCst);
                    let body: Value = serde_json::from_slice(&request.body).expect("a JSON-RPC body");
                    let answer = json!({
                        "jsonrpc": "2.0",
                        "id": body["id"],
                        "result": { "tools": [] },
                    });
                    if let Some(frames) = streams.lock().expect("not poisoned").as_ref() {
                        let _ = frames.send(format!("event: message\ndata: {answer}\n\n").into_bytes());
                    }
                    FakeReply::status(202)
                }
                _ => FakeReply::status(404),
            }
        })
        .await;
        *base.lock().expect("not poisoned") = server.url.clone();

        let mut config = config_for(&server, McpServerTransport::Sse, "files");
        config.url = Some(format!("{}sse", server.url));
        let transport = connect(&config, None).await.expect("a transport");

        let result = transport.request("tools/list", None).await.expect("an answer");
        assert_eq!(result, json!({ "tools": [] }));
        assert_eq!(posts.load(Ordering::SeqCst), 1, "one message was posted");
    }

    #[tokio::test]
    async fn an_event_stream_that_ends_fails_the_request_waiting_on_it() {
        let base = Arc::new(std::sync::Mutex::new(String::new()));
        let server = FakeServer::start({
            let base = Arc::clone(&base);
            move |request: FakeRequest| match request.path.as_str() {
                "/sse" => {
                    let endpoint = format!("{}messages", base.lock().expect("not poisoned"));
                    FakeReply::event_stream(vec![format!("event: endpoint\ndata: {endpoint}\n\n").into_bytes()])
                }
                "/messages" => FakeReply::status(202),
                _ => FakeReply::status(404),
            }
        })
        .await;
        *base.lock().expect("not poisoned") = server.url.clone();

        let mut config = config_for(&server, McpServerTransport::Sse, "files");
        config.url = Some(format!("{}sse", server.url));
        let transport = connect(&config, None).await.expect("a transport");

        let error = transport
            .request("tools/list", None)
            .await
            .expect_err("the stream is gone");
        assert_eq!(error.code(), "MCP-EXTERNAL-FAILED");
        assert!(
            error.detail().is_some_and(|detail| detail.contains("event stream")),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn closing_a_streamable_http_session_tells_the_server() {
        let methods = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let recorded = Arc::clone(&methods);

        let server = FakeServer::start(move |request: FakeRequest| {
            recorded.lock().expect("not poisoned").push(request.method.clone());
            if request.method == "DELETE" {
                FakeReply::status(200)
            } else {
                FakeReply::json(json!({ "jsonrpc": "2.0", "id": 1, "result": {} }))
                    .with_header("Mcp-Session-Id", "session-1")
            }
        })
        .await;

        let mut config = config_for(&server, McpServerTransport::Http, "files");
        config.url = Some(server.url.clone());
        let transport = connect(&config, None).await.expect("a transport");

        transport.request("initialize", None).await.expect("an answer");
        transport.close().await.expect("a close");

        assert_eq!(
            methods.lock().expect("not poisoned").clone(),
            vec!["POST".to_owned(), "DELETE".to_owned()],
            "a session with an id is ended with a DELETE, as the protocol asks"
        );
    }

    #[tokio::test]
    async fn a_closed_transport_refuses_more_requests() {
        let server = FakeServer::start(|_request: FakeRequest| FakeReply::status(202)).await;
        let mut config = config_for(&server, McpServerTransport::Http, "files");
        config.url = Some(server.url.clone());

        let transport = connect(&config, None).await.expect("a transport");
        transport.close().await.expect("closing is idempotent");
        transport.close().await.expect("closing twice is fine");

        let error = transport.request("ping", None).await.expect_err("closed");
        assert_eq!(error.code(), "MCP-EXTERNAL-FAILED");
    }

    #[tokio::test]
    async fn a_json_rpc_error_keeps_its_code_and_never_quotes_its_data() {
        let error = rpc_error(
            "files",
            &json!({
                "code": -32601,
                "message": "no method is named `ping`",
                "data": { "secret": "hunter2" },
            }),
        );
        match &error {
            McpClientError::Rpc { code, message, .. } => {
                assert_eq!(*code, -32601);
                assert_eq!(message, "no method is named `ping`");
            }
            other => panic!("a coded refusal is not a plain failure: {other:?}"),
        }
        assert!(!format!("{error:?}").contains("hunter2"), "{error:?}");
    }

    #[tokio::test]
    async fn the_older_sse_transport_delivers_notifications_to_its_handler() {
        let streams = Arc::new(std::sync::Mutex::new(
            None::<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
        ));
        let base = Arc::new(std::sync::Mutex::new(String::new()));

        let server = FakeServer::start({
            let streams = Arc::clone(&streams);
            let base = Arc::clone(&base);
            move |request: FakeRequest| match (request.method.as_str(), request.path.as_str()) {
                ("GET", "/sse") => {
                    let (frames, receiver) = tokio::sync::mpsc::unbounded_channel();
                    let endpoint = format!("{}messages", base.lock().expect("not poisoned"));
                    let _ = frames.send(format!("event: endpoint\ndata: {endpoint}\n\n").into_bytes());
                    *streams.lock().expect("not poisoned") = Some(frames);
                    FakeReply::event_stream_live(receiver)
                }
                ("POST", "/messages") => FakeReply::status(202),
                _ => FakeReply::status(404),
            }
        })
        .await;
        *base.lock().expect("not poisoned") = server.url.clone();

        let mut config = config_for(&server, McpServerTransport::Sse, "files");
        config.url = Some(format!("{}sse", server.url));
        let transport = connect(&config, None).await.expect("a transport");

        let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let recorded = Arc::clone(&seen);
        transport.set_notification_handler(Arc::new(move |method: &str| {
            recorded.lock().expect("not poisoned").push(method.to_owned());
        }));

        let frames = streams
            .lock()
            .expect("not poisoned")
            .clone()
            .expect("the GET stream was opened");
        frames
            .send(
                b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n\n"
                    .to_vec(),
            )
            .expect("the fake stream is open");

        tokio::time::timeout(Duration::from_secs(2), async {
            while seen.lock().expect("not poisoned").is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the notification reached the handler");
        assert_eq!(
            seen.lock().expect("not poisoned").clone(),
            vec!["notifications/tools/list_changed".to_owned()]
        );

        transport.close().await.expect("a close");
    }

    #[tokio::test]
    async fn the_http_transport_opens_one_get_stream_for_notifications() {
        let frames = Arc::new(std::sync::Mutex::new(
            None::<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
        ));
        let got_get = Arc::new(AtomicBool::new(false));

        let server = FakeServer::start({
            let frames = Arc::clone(&frames);
            let got_get = Arc::clone(&got_get);
            move |request: FakeRequest| match request.method.as_str() {
                "GET" => {
                    got_get.store(true, Ordering::SeqCst);
                    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
                    *frames.lock().expect("not poisoned") = Some(sender);
                    FakeReply::event_stream_live(receiver)
                }
                "POST" => {
                    let body: Value = serde_json::from_slice(&request.body).unwrap_or_default();
                    let id = body.get("id").cloned().unwrap_or(Value::Null);
                    match body.get("method").and_then(Value::as_str) {
                        Some("initialize") => FakeReply::json(json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "protocolVersion": "2025-06-18",
                                "capabilities": {},
                                "serverInfo": { "name": "fake-http", "version": "0" },
                            },
                        }))
                        .with_header("Mcp-Session-Id", "session-1"),
                        Some("tools/list") => {
                            FakeReply::json(json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": [] } }))
                        }
                        _ => FakeReply::status(202),
                    }
                }
                _ => FakeReply::status(404),
            }
        })
        .await;

        let mut config = config_for(&server, McpServerTransport::Http, "files");
        config.url = Some(server.url.clone());
        let transport = connect(&config, None).await.expect("a transport");

        let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let recorded = Arc::clone(&seen);
        transport.set_notification_handler(Arc::new(move |method: &str| {
            recorded.lock().expect("not poisoned").push(method.to_owned());
        }));
        transport
            .request("initialize", None)
            .await
            .expect("the handshake gives us a session id");

        tokio::time::timeout(Duration::from_secs(2), async {
            while !got_get.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the GET stream was opened");

        let sender = frames
            .lock()
            .expect("not poisoned")
            .clone()
            .expect("the fake GET stream is open");
        sender
            .send(
                b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n\n"
                    .to_vec(),
            )
            .expect("the fake stream is open");

        tokio::time::timeout(Duration::from_secs(2), async {
            while seen.lock().expect("not poisoned").is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the notification reached the handler");
        assert_eq!(
            seen.lock().expect("not poisoned").clone(),
            vec!["notifications/tools/list_changed".to_owned()]
        );

        transport.close().await.expect("a close");
    }

    fn config_for(fake: &FakeServer, transport: McpServerTransport, id: &str) -> McpServerConfig {
        let mut config = server(id, transport);
        if transport.uses_url() {
            config.url = Some(fake.url.clone());
        }
        config
    }
}
