//! The fakes the tests are built from: a scripted transport, a factory that hands one out per
//! server, a core-and-vault context to call an adapter through, and a local HTTP server for the two
//! network transports.
//!
//! Nothing here listens on an interface anybody else can reach, and nothing here is compiled into
//! a release: it exists so that "the stdio transport speaks newline-delimited JSON-RPC" can be a
//! test without a real external server, and so a fake can be scripted to fail in exactly the way a
//! server does.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use sevenshell_api::{ApprovalBroker, ApprovalDecision, ApprovalRequest, DenyingBroker, ToolContext, ToolError};
use sevenshell_core::Core;
use sevenshell_types::{McpScope, McpServerConfig};
use sevenshell_vault::Vault;
use tempfile::TempDir;

use crate::McpClientError;
use crate::config::McpServerSecrets;
use crate::transport::{McpTransport, NotificationHandler, TransportFactory};

// ---------------------------------------------------------------------------------------------
// A scripted transport
// ---------------------------------------------------------------------------------------------

/// One answer a scripted transport gives.
#[derive(Clone)]
enum Step {
    Respond(Value),
    Fail(McpClientError),
    Delay(Duration, Value),
    /// Never answers, like a server that accepted the request and forgot it.
    Hang,
}

/// A transport whose answers are written in the test.
///
/// Scripted per method and consumed in order, so "initialize answers 2024-11-05, then tools/list
/// answers two tools" is readable at the test. An unscripted method answers with the default, which
/// is a plain empty result.
#[derive(Default)]
pub struct FakeTransport {
    steps: Mutex<HashMap<String, VecDeque<Step>>>,
    seen: Mutex<Vec<(String, Option<Value>)>>,
    closed: Mutex<bool>,
    versions: Mutex<Vec<String>>,
    notification: Mutex<Option<NotificationHandler>>,
}

impl FakeTransport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on(self, method: &str, value: Value) -> Self {
        self.push(method, Step::Respond(value))
    }

    pub fn on_error(self, method: &str, error: McpClientError) -> Self {
        self.push(method, Step::Fail(error))
    }

    pub fn on_delay(self, method: &str, delay: Duration, value: Value) -> Self {
        self.push(method, Step::Delay(delay, value))
    }

    pub fn on_hang(self, method: &str) -> Self {
        self.push(method, Step::Hang)
    }

    /// Delivers a server notification the way a real transport's reader would.
    pub fn emit_notification(&self, method: &str) {
        let handler = self.notification.lock().expect("not poisoned").clone();
        if let Some(handler) = handler {
            handler(method);
        }
    }

    fn push(self, method: &str, step: Step) -> Self {
        self.steps
            .lock()
            .expect("not poisoned")
            .entry(method.to_owned())
            .or_default()
            .push_back(step);
        self
    }

    /// Every request this transport was asked to make, in order.
    pub fn seen(&self) -> Vec<(String, Option<Value>)> {
        self.seen.lock().expect("not poisoned").clone()
    }

    /// The params of every request for one method.
    pub fn calls(&self, method: &str) -> Vec<Option<Value>> {
        self.seen()
            .into_iter()
            .filter(|(seen, _)| seen == method)
            .map(|(_, params)| params)
            .collect()
    }

    pub fn is_closed(&self) -> bool {
        *self.closed.lock().expect("not poisoned")
    }

    /// Every revision the session told this transport it should carry.
    pub fn versions(&self) -> Vec<String> {
        self.versions.lock().expect("not poisoned").clone()
    }

    fn take(&self, method: &str) -> Step {
        self.steps
            .lock()
            .expect("not poisoned")
            .get_mut(method)
            .and_then(VecDeque::pop_front)
            .unwrap_or_else(|| Step::Respond(json!({})))
    }
}

#[async_trait]
impl McpTransport for Arc<FakeTransport> {
    async fn request(&self, method: &str, params: Option<Value>) -> Result<Value, McpClientError> {
        self.seen
            .lock()
            .expect("not poisoned")
            .push((method.to_owned(), params));
        match self.take(method) {
            Step::Respond(value) => Ok(value),
            Step::Fail(error) => Err(error),
            Step::Delay(delay, value) => {
                tokio::time::sleep(delay).await;
                Ok(value)
            }
            Step::Hang => {
                std::future::pending::<()>().await;
                unreachable!("a hanging request never resolves")
            }
        }
    }

    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), McpClientError> {
        self.seen
            .lock()
            .expect("not poisoned")
            .push((method.to_owned(), params));
        Ok(())
    }

    async fn close(&self) -> Result<(), McpClientError> {
        *self.closed.lock().expect("not poisoned") = true;
        Ok(())
    }

    fn set_protocol_version(&self, version: &str) {
        self.versions.lock().expect("not poisoned").push(version.to_owned());
    }

    fn set_notification_handler(&self, handler: NotificationHandler) {
        *self.notification.lock().expect("not poisoned") = Some(handler);
    }
}

/// A factory that hands a scripted transport to whichever server asks for it.
#[derive(Default)]
pub struct FakeFactory {
    scripts: Mutex<HashMap<String, Arc<FakeTransport>>>,
    failures: Mutex<HashMap<String, McpClientError>>,
    opens: Mutex<Vec<(String, Option<McpServerSecrets>)>>,
}

impl FakeFactory {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn answering(self: &Arc<Self>, id: &str, transport: Arc<FakeTransport>) -> Arc<Self> {
        self.scripts
            .lock()
            .expect("not poisoned")
            .insert(id.to_owned(), transport);
        Arc::clone(self)
    }

    pub fn failing(self: &Arc<Self>, id: &str, error: McpClientError) -> Arc<Self> {
        self.failures.lock().expect("not poisoned").insert(id.to_owned(), error);
        Arc::clone(self)
    }

    /// Every open this factory was asked for, in order, with the secrets it was handed.
    pub fn opens(&self) -> Vec<(String, Option<McpServerSecrets>)> {
        self.opens.lock().expect("not poisoned").clone()
    }

    pub fn opened(&self, id: &str) -> usize {
        self.opens().into_iter().filter(|(server, _)| server == id).count()
    }
}

#[async_trait]
impl TransportFactory for FakeFactory {
    async fn open(
        &self,
        server: &McpServerConfig,
        secrets: Option<McpServerSecrets>,
    ) -> Result<Box<dyn McpTransport>, McpClientError> {
        self.opens
            .lock()
            .expect("not poisoned")
            .push((server.id.clone(), secrets));

        if let Some(error) = self.failures.lock().expect("not poisoned").get(&server.id) {
            return Err(error.clone());
        }

        match self.scripts.lock().expect("not poisoned").get(&server.id).cloned() {
            Some(transport) => Ok(Box::new(transport)),
            None => Err(McpClientError::ConnectFailed {
                server: server.id.clone(),
                detail: "this test scripted no transport for that server".to_owned(),
            }),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Answers a test writes
// ---------------------------------------------------------------------------------------------

/// An `initialize` result that advertises the tools capability.
pub fn initialize_reply(version: &str) -> Value {
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "fake", "version": "0" },
    })
}

/// An `initialize` result from a server that offers no tools at all.
pub fn initialize_reply_without_tools(version: &str) -> Value {
    json!({
        "protocolVersion": version,
        "capabilities": {},
        "serverInfo": { "name": "fake", "version": "0" },
    })
}

/// One `tools/list` entry.
pub fn tool_listing(name: &str, description: &str) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"],
        },
    })
}

/// The same entry with the one annotation that makes a tool read-only.
///
/// A test that calls through the adapter needs this: a tool without the hint is a write, and a
/// write stops for an approval that a test context would have to script.
pub fn read_tool_listing(name: &str, description: &str) -> Value {
    let mut listing = tool_listing(name, description);
    listing["annotations"] = json!({ "readOnlyHint": true });
    listing
}

pub fn tools_reply(listings: &[Value]) -> Value {
    json!({ "tools": listings })
}

/// A `tools/call` result carrying one text block.
pub fn text_result(text: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": false,
    })
}

// ---------------------------------------------------------------------------------------------
// A context to call an adapter through
// ---------------------------------------------------------------------------------------------

/// A context around a vault that exists but was never unlocked, so nothing here can reach a host.
pub fn context(scope: McpScope, approvals: Arc<dyn ApprovalBroker>) -> (TempDir, ToolContext) {
    let directory = tempfile::tempdir().expect("temp dir");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    let core = Core::new(Arc::clone(&vault)).0;
    let context = ToolContext::new(core, scope).with_approvals(approvals);
    (directory, context)
}

pub fn context_denying(scope: McpScope) -> (TempDir, ToolContext) {
    context(scope, Arc::new(DenyingBroker))
}

/// A broker that answers from a script and remembers every question it was asked.
#[derive(Default)]
pub struct RecordingBroker {
    decisions: Mutex<VecDeque<ApprovalDecision>>,
    seen: Mutex<Vec<ApprovalRequest>>,
}

impl RecordingBroker {
    pub fn asking(decisions: &[ApprovalDecision]) -> Arc<Self> {
        let broker = Self::default();
        broker
            .decisions
            .lock()
            .expect("not poisoned")
            .extend(decisions.iter().copied());
        Arc::new(broker)
    }

    pub fn seen(&self) -> Vec<ApprovalRequest> {
        self.seen.lock().expect("not poisoned").clone()
    }
}

#[async_trait]
impl ApprovalBroker for RecordingBroker {
    async fn request(&self, request: ApprovalRequest) -> Result<ApprovalDecision, ToolError> {
        self.seen.lock().expect("not poisoned").push(request);
        Ok(self
            .decisions
            .lock()
            .expect("not poisoned")
            .pop_front()
            .unwrap_or(ApprovalDecision::Denied))
    }
}

// ---------------------------------------------------------------------------------------------
// A local HTTP server
// ---------------------------------------------------------------------------------------------

/// One request, as the fake server read it.
pub struct FakeRequest {
    pub method: String,
    pub path: String,
    headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl FakeRequest {
    /// A header by its lower-case name, which is how HTTP/1.1 spellings are compared here.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_ascii_lowercase()).map(String::as_str)
    }
}

enum FakeBody {
    Complete(Vec<u8>),
    Stream(mpsc::UnboundedReceiver<Vec<u8>>),
}

/// What the fake answers with.
pub struct FakeReply {
    status: u16,
    headers: Vec<(String, String)>,
    body: FakeBody,
}

impl FakeReply {
    pub fn json(value: Value) -> Self {
        Self {
            status: 200,
            headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
            body: FakeBody::Complete(serde_json::to_vec(&value).expect("a value is JSON")),
        }
    }

    pub fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: FakeBody::Complete(Vec::new()),
        }
    }

    /// A complete event stream: every frame is sent, then the body ends.
    pub fn event_stream(frames: Vec<Vec<u8>>) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        for frame in frames {
            let _ = sender.send(frame);
        }
        drop(sender);
        Self {
            status: 200,
            headers: vec![("Content-Type".to_owned(), "text/event-stream".to_owned())],
            body: FakeBody::Stream(receiver),
        }
    }

    /// An event stream another request can feed: what the older transport's GET and POST sides
    /// look like when they are not the same connection.
    pub fn event_stream_live(receiver: mpsc::UnboundedReceiver<Vec<u8>>) -> Self {
        Self {
            status: 200,
            headers: vec![("Content-Type".to_owned(), "text/event-stream".to_owned())],
            body: FakeBody::Stream(receiver),
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

/// A server on `127.0.0.1`, on a port the system picked.
pub struct FakeServer {
    /// The base address, with a trailing slash so `Url::join` works on it.
    pub url: String,
}

impl FakeServer {
    pub async fn start<H>(handler: H) -> Self
    where
        H: Fn(FakeRequest) -> FakeReply + Send + Sync + 'static,
    {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("a loopback listener");
        let address = listener.local_addr().expect("a bound address");
        let handler: Arc<dyn Fn(FakeRequest) -> FakeReply + Send + Sync> = Arc::new(handler);

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let handler = Arc::clone(&handler);
                tokio::spawn(async move {
                    let Some(request) = read_request(&mut stream).await else {
                        return;
                    };
                    write_reply(&mut stream, handler(request)).await;
                    let _ = stream.shutdown().await;
                });
            }
        });

        Self {
            url: format!("http://{address}/"),
        }
    }
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> Option<FakeRequest> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8 * 1024];

    let head_end = loop {
        if let Some(position) = find(&buffer, b"\r\n\r\n") {
            break position + 4;
        }
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > 64 * 1024 {
            return None;
        }
    };

    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split(' ');
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();

    let mut headers = BTreeMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }

    let length: usize = headers
        .get("content-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut body = buffer[head_end..].to_vec();
    while body.len() < length {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
        if body.len() > 8 * 1024 * 1024 {
            return None;
        }
    }
    body.truncate(length);

    Some(FakeRequest {
        method,
        path,
        headers,
        body,
    })
}

async fn write_reply(stream: &mut tokio::net::TcpStream, reply: FakeReply) {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nConnection: close\r\n",
        reply.status,
        reason(reply.status)
    );
    for (name, value) in &reply.headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }

    match reply.body {
        FakeBody::Complete(body) => {
            head.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
            if stream.write_all(head.as_bytes()).await.is_err() {
                return;
            }
            let _ = stream.write_all(&body).await;
        }
        FakeBody::Stream(mut frames) => {
            head.push_str("Transfer-Encoding: chunked\r\n\r\n");
            if stream.write_all(head.as_bytes()).await.is_err() {
                return;
            }
            while let Some(frame) = frames.recv().await {
                let size = format!("{:x}\r\n", frame.len());
                if stream.write_all(size.as_bytes()).await.is_err()
                    || stream.write_all(&frame).await.is_err()
                    || stream.write_all(b"\r\n").await.is_err()
                {
                    return;
                }
                let _ = stream.flush().await;
            }
            let _ = stream.write_all(b"0\r\n\r\n").await;
        }
    }

    let _ = stream.flush().await;
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    }
}
