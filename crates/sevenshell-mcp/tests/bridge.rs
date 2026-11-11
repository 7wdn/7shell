//! The bridge as a client sees it: one pipe, lines in, lines out.
//!
//! Three things are checked here that the protocol tests cannot. The first is the whole path — a
//! real `HttpToolClient`, a real loopback listener, the real `serve` loop — because the two halves
//! are only correct together. The second is the bound on work in flight: a client that writes faster
//! than calls finish must be made to wait rather than have every line turned into a task. The third
//! is that an answer is never something a client has to guess about: every line on stdout parses as
//! JSON, and a result that was cut says so instead of pretending to be whole.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};
use tokio::sync::Semaphore;
use tokio::time::timeout;

use sevenshell_api::{
    CALL_ID_HEADER, ENDPOINT_FILE_NAME, TOOL_CALL_PATH, TOOLS_PATH, ToolAccess, ToolDescriptor, ToolInputSchema,
    ToolOutcome, ToolResult, local,
};
use sevenshell_mcp::{DESKTOP_UNAVAILABLE, HttpToolClient, LATEST_PROTOCOL_VERSION, McpServer, RpcError, ToolClient};

use support::{FakeDesktop, Reply};

/// One bridge on one in-memory pipe, driven the way a client drives it.
struct Bridge {
    side: BufReader<DuplexStream>,
    next_id: i64,
}

impl Bridge {
    fn over(server: McpServer) -> Self {
        let server = Arc::new(server);
        let (server_side, client_side) = tokio::io::duplex(64 * 1024);
        let (reader, writer) = tokio::io::split(server_side);
        tokio::spawn(async move {
            let _ = server.serve(reader, writer).await;
        });

        Self {
            side: BufReader::new(client_side),
            next_id: 0,
        }
    }

    async fn write_raw(&mut self, line: &str) {
        let stream = self.side.get_mut();
        stream
            .write_all(line.as_bytes())
            .await
            .expect("the pipe accepts a line");
        stream.write_all(b"\n").await.expect("the pipe accepts a newline");
        stream.flush().await.expect("the pipe flushes");
    }

    async fn send(&mut self, id: i64, method: &str, params: Value) {
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.write_raw(&message.to_string()).await;
    }

    /// Reads one line and requires it to be JSON: a log line on stdout would fail here rather than
    /// somewhere in a client's parser.
    async fn read(&mut self) -> Value {
        let mut line = String::new();
        let read = timeout(Duration::from_secs(5), self.side.read_line(&mut line))
            .await
            .expect("the bridge answered in time")
            .expect("the bridge's output is readable");
        assert!(read > 0, "the bridge closed its output instead of answering");
        serde_json::from_str(line.trim()).unwrap_or_else(|error| panic!("not JSON: {line:?} ({error})"))
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(id, method, params).await;
        self.read().await
    }

    async fn initialize(&mut self) -> Value {
        self.request(
            "initialize",
            json!({
                "protocolVersion": LATEST_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0" }
            }),
        )
        .await
    }
}

/// A tool result of exactly the JSON this client was built with.
struct ValueClient {
    value: Value,
}

#[async_trait]
impl ToolClient for ValueClient {
    fn descriptors(&self) -> Vec<ToolDescriptor> {
        local::default_registry().descriptors()
    }

    async fn call(&self, _name: &str, _arguments: Value, _call_id: &str) -> Result<ToolOutcome, RpcError> {
        Ok(ToolOutcome::succeeded(ToolResult::new(self.value.clone())))
    }
}

/// A client that reports the ceiling its endpoint file would carry.
struct LimitedValueClient {
    value: Value,
    limit: usize,
}

#[async_trait]
impl ToolClient for LimitedValueClient {
    fn descriptors(&self) -> Vec<ToolDescriptor> {
        local::default_registry().descriptors()
    }

    async fn call(&self, _name: &str, _arguments: Value, _call_id: &str) -> Result<ToolOutcome, RpcError> {
        Ok(ToolOutcome::succeeded(ToolResult::new(self.value.clone())))
    }

    fn max_result_bytes(&self) -> usize {
        self.limit
    }
}

/// A desktop whose calls do not finish until the test says so.
///
/// The counters are what the concurrency bound is asserted against: how many calls the bridge
/// started, and the largest number it ever had running at once.
struct GatedClient {
    started: AtomicUsize,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
    gate: Arc<Semaphore>,
}

impl GatedClient {
    fn new() -> Self {
        Self {
            started: AtomicUsize::new(0),
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
            gate: Arc::new(Semaphore::new(0)),
        }
    }

    fn started(&self) -> usize {
        self.started.load(Ordering::SeqCst)
    }

    fn max_in_flight(&self) -> usize {
        self.max_in_flight.load(Ordering::SeqCst)
    }

    /// Lets `calls` waiting calls through.
    fn release(&self, calls: usize) {
        self.gate.add_permits(calls);
    }
}

#[async_trait]
impl ToolClient for GatedClient {
    fn descriptors(&self) -> Vec<ToolDescriptor> {
        local::default_registry().descriptors()
    }

    async fn call(&self, _name: &str, _arguments: Value, _call_id: &str) -> Result<ToolOutcome, RpcError> {
        self.started.fetch_add(1, Ordering::SeqCst);
        let in_flight = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_in_flight.fetch_max(in_flight, Ordering::SeqCst);

        let permit = self.gate.acquire().await.expect("the gate is never closed");
        permit.forget();

        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        Ok(ToolOutcome::succeeded(ToolResult::new(json!({ "ok": true }))))
    }
}

/// A desktop with one tool that takes its time.
struct SlowClient {
    slow: &'static str,
    delay: Duration,
}

#[async_trait]
impl ToolClient for SlowClient {
    fn descriptors(&self) -> Vec<ToolDescriptor> {
        local::default_registry().descriptors()
    }

    async fn call(&self, name: &str, _arguments: Value, _call_id: &str) -> Result<ToolOutcome, RpcError> {
        if name == self.slow {
            tokio::time::sleep(self.delay).await;
        }
        Ok(ToolOutcome::succeeded(ToolResult::new(json!({ "tool": name }))))
    }
}

/// A desktop that starts one slow call and records both the id it was started under and every
/// cancellation it receives. `ToolInputSchema` is unused here but the descriptors are the real ones.
struct CancelRecordingClient {
    started: AtomicUsize,
    call_ids: Mutex<Vec<String>>,
    cancels: Mutex<Vec<String>>,
}

impl Default for CancelRecordingClient {
    fn default() -> Self {
        Self {
            started: AtomicUsize::new(0),
            call_ids: Mutex::new(Vec::new()),
            cancels: Mutex::new(Vec::new()),
        }
    }
}

impl CancelRecordingClient {
    fn started(&self) -> usize {
        self.started.load(Ordering::SeqCst)
    }

    fn call_ids(&self) -> Vec<String> {
        self.call_ids.lock().expect("the call id lock is not poisoned").clone()
    }

    fn cancels(&self) -> Vec<String> {
        self.cancels.lock().expect("the cancel lock is not poisoned").clone()
    }
}

#[async_trait]
impl ToolClient for CancelRecordingClient {
    fn descriptors(&self) -> Vec<ToolDescriptor> {
        local::default_registry().descriptors()
    }

    async fn call(&self, name: &str, _arguments: Value, call_id: &str) -> Result<ToolOutcome, RpcError> {
        self.call_ids
            .lock()
            .expect("the call id lock is not poisoned")
            .push(call_id.to_owned());
        self.started.fetch_add(1, Ordering::SeqCst);
        // Only the tool a cancellation test names is slow: long enough for the notice to be read and
        // forwarded while the call is still in flight, without slowing the other tests down.
        if name == "sshell_exec_run" {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Ok(ToolOutcome::succeeded(ToolResult::new(json!({ "tool": name }))))
    }

    async fn cancel(&self, call_id: &str) -> Result<(), RpcError> {
        self.cancels
            .lock()
            .expect("the cancel lock is not poisoned")
            .push(call_id.to_owned());
        Ok(())
    }
}

/// A desktop that lists one tool this build does not compile in, and one compiled tool, so a call
/// to the compiled tool is still known to the bridge after the refresh.
fn a_table_with_an_external_tool() -> Vec<ToolDescriptor> {
    let mut descriptors = local::default_registry().descriptors();
    descriptors.push(ToolDescriptor {
        name: "mcp__lab__list_things".to_owned(),
        title: "List things".to_owned(),
        description: "Lists things on the lab server.".to_owned(),
        input_schema: ToolInputSchema::object(json!({}), &[]),
        access: ToolAccess::Read,
    });
    descriptors
}

/// Waits for a condition a task sets, or fails with the sentence a reader needs.
async fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    for _ in 0..300 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting: {what}");
}

// -- The whole path -----------------------------------------------------

#[tokio::test]
async fn a_session_over_the_pipe_reaches_the_desktop_and_every_line_is_json() {
    let table = serde_json::to_string(&a_table_with_an_external_tool()).expect("a table is JSON");
    let desktop = FakeDesktop::start_with_tools(
        Reply::with_body(200, table),
        Reply::succeeded(json!({ "sessions": [] })),
    )
    .await;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let endpoint = desktop.write_endpoint(directory.path(), "a-token");

    let client = HttpToolClient::new(endpoint).expect("a client");
    let mut bridge = Bridge::over(McpServer::new(Arc::new(client), 262_144));

    let initialize = bridge.initialize().await;
    assert_eq!(initialize["result"]["protocolVersion"], json!(LATEST_PROTOCOL_VERSION));

    // The table came from the desktop over `GET /tools`, not from the bridge's compiled list.
    let listing = bridge.request("tools/list", json!({})).await;
    let tools = listing["result"]["tools"].as_array().expect("a list of tools");
    assert_eq!(tools.len(), 11, "the desktop's table has one tool more: {listing}");
    assert!(
        tools.iter().any(|tool| tool["name"] == json!("mcp__lab__list_things")),
        "a tool the desktop listed has to reach the agent: {listing}"
    );

    let call = bridge
        .request(
            "tools/call",
            json!({ "name": "sshell_sessions_list", "arguments": { "kind": "saved" } }),
        )
        .await;
    assert_eq!(
        call["result"]["structuredContent"],
        json!({ "sessions": [] }),
        "the answer the desktop gave is the answer the agent reads: {call}"
    );

    let captured = desktop.captured();
    let tool_list = captured
        .iter()
        .find(|request| request.path == TOOLS_PATH)
        .expect("the bridge has to read the desktop's table");
    assert_eq!(tool_list.method, "GET");
    assert_eq!(tool_list.header("authorization"), Some("Bearer a-token"));

    let calls: Vec<_> = captured
        .iter()
        .filter(|request| request.path == TOOL_CALL_PATH)
        .collect();
    assert_eq!(calls.len(), 1, "exactly one call reached the desktop");
    assert_eq!(calls[0].header("authorization"), Some("Bearer a-token"));
    assert!(
        calls[0]
            .header(CALL_ID_HEADER)
            .is_some_and(|call_id| !call_id.is_empty()),
        "every call travels under an id a cancellation can name: {:?}",
        calls[0].header(CALL_ID_HEADER)
    );

    let sent: sevenshell_api::ToolCall = serde_json::from_slice(&calls[0].body).expect("the body is a tool call");
    assert_eq!(sent.name, "sshell_sessions_list");
    assert_eq!(sent.arguments, json!({ "kind": "saved" }));
}

// -- The list -----------------------------------------------------------

#[tokio::test]
async fn the_list_an_agent_sees_is_the_registry_this_build_compiles_in() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    // No endpoint file: the list is compiled in and must not depend on the desktop answering.
    let client = HttpToolClient::new(directory.path().join(ENDPOINT_FILE_NAME)).expect("a client");
    let server = McpServer::new(Arc::new(client), 262_144);

    let response = server
        .dispatch(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} }).to_string())
        .await
        .expect("a request is answered");
    let tools = response["result"]["tools"].as_array().expect("a list of tools");

    let registry = local::default_registry().descriptors();
    let mut expected: Vec<&str> = registry.iter().map(|descriptor| descriptor.name.as_str()).collect();
    expected.sort_unstable();

    let mut listed: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().expect("every tool has a name"))
        .collect();
    listed.sort_unstable();

    assert_eq!(expected.len(), 10, "the first wave is ten tools");
    assert_eq!(
        listed, expected,
        "a list that disagrees with the registry is a tool an agent can see and never call"
    );

    for descriptor in &registry {
        let tool = tools
            .iter()
            .find(|tool| tool["name"] == json!(descriptor.name))
            .unwrap_or_else(|| panic!("{} is missing from the listing", descriptor.name));
        assert_eq!(
            tool["inputSchema"],
            *descriptor.input_schema.as_value(),
            "{} is described to the agent with a schema it is not parsed with",
            descriptor.name
        );
        assert_eq!(tool["description"], json!(descriptor.description));
        assert_eq!(
            tool["annotations"]["readOnlyHint"],
            json!(descriptor.access == ToolAccess::Read),
            "{} is annotated with the wrong access level",
            descriptor.name
        );
    }
}

// -- A desktop that is not there ---------------------------------------

#[tokio::test]
async fn a_bridge_with_no_endpoint_file_still_answers_the_handshake_and_the_list() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let client = HttpToolClient::new(directory.path().join(ENDPOINT_FILE_NAME)).expect("a client");
    let mut bridge = Bridge::over(McpServer::new(Arc::new(client), 262_144));

    let initialize = bridge.initialize().await;
    assert_eq!(initialize["result"]["protocolVersion"], json!(LATEST_PROTOCOL_VERSION));

    let listing = bridge.request("tools/list", json!({})).await;
    assert_eq!(listing["result"]["tools"].as_array().expect("tools").len(), 10);

    let call = bridge
        .request("tools/call", json!({ "name": "sshell_sessions_list", "arguments": {} }))
        .await;
    assert_eq!(
        call["error"]["code"],
        json!(DESKTOP_UNAVAILABLE),
        "a call without a desktop has its own code, not a tool failure: {call}"
    );

    // The session is still a session: the next line is answered normally.
    let ping = bridge.request("ping", json!({})).await;
    assert_eq!(ping["result"], json!({}));
}

// -- Bounds on work in flight ------------------------------------------

#[tokio::test]
async fn lines_wait_for_a_slot_instead_of_becoming_tasks_without_a_bound() {
    let client = Arc::new(GatedClient::new());
    let mut bridge = Bridge::over(McpServer::new(client.clone(), 262_144).with_max_concurrency(2));

    for id in 1..=5 {
        bridge
            .send(
                id,
                "tools/call",
                json!({ "name": "sshell_sessions_list", "arguments": {} }),
            )
            .await;
    }

    wait_until("the first two calls to start", || client.started() == 2).await;
    // Long enough that an unbounded reader would have started the rest.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(client.started(), 2, "a third call started while both slots were busy");

    client.release(5);
    let mut answered = Vec::new();
    for _ in 0..5 {
        answered.push(bridge.read().await["id"].as_i64().expect("an id"));
    }
    answered.sort_unstable();

    assert_eq!(answered, vec![1, 2, 3, 4, 5], "every line was eventually answered");
    assert_eq!(
        client.max_in_flight(),
        2,
        "more calls than the limit ran at the same time"
    );
}

#[tokio::test]
async fn a_fast_call_is_answered_while_a_slow_one_is_still_running() {
    let client = Arc::new(SlowClient {
        slow: "sshell_exec_run",
        delay: Duration::from_millis(1_200),
    });
    let mut bridge = Bridge::over(McpServer::new(client, 262_144));

    let started = Instant::now();
    bridge
        .send(
            10,
            "tools/call",
            json!({ "name": "sshell_exec_run", "arguments": { "sessionId": "lab", "command": "sleep 1" } }),
        )
        .await;
    bridge
        .send(
            11,
            "tools/call",
            json!({ "name": "sshell_sessions_list", "arguments": {} }),
        )
        .await;

    let fast = bridge.read().await;
    assert_eq!(fast["id"], json!(11), "the fast answer has to be the first one out");
    assert!(
        started.elapsed() < Duration::from_millis(700),
        "the fast call was held behind the slow one for {:?}",
        started.elapsed()
    );

    let slow = bridge.read().await;
    assert_eq!(slow["id"], json!(10));
}

// -- Notifications ------------------------------------------------------

#[tokio::test]
async fn a_cancellation_reaches_the_desktop_under_the_call_id_of_its_request() {
    let client = Arc::new(CancelRecordingClient::default());
    let mut bridge = Bridge::over(McpServer::new(client.clone(), 262_144));

    bridge
        .send(
            7,
            "tools/call",
            json!({ "name": "sshell_exec_run", "arguments": { "sessionId": "lab", "command": "sleep 1" } }),
        )
        .await;
    // The mapping exists before the call reaches the desktop, so waiting for the call is enough to
    // know a cancellation naming request 7 has something to find.
    wait_until("the call to start", || client.started() == 1).await;

    bridge
        .write_raw(
            &json!({
                "jsonrpc": "2.0",
                "method": "notifications/cancelled",
                "params": { "requestId": 7, "reason": "the agent changed its mind" }
            })
            .to_string(),
        )
        .await;
    wait_until("the cancellation to reach the desktop", || !client.cancels().is_empty()).await;

    assert_eq!(
        client.cancels(),
        client.call_ids(),
        "the desktop has to be asked to call off the id the call was started under"
    );

    // No line may answer the notification: the next line is the answer to request 7, which the
    // protocol still requires even though the work was called off.
    let answer = bridge.read().await;
    assert_eq!(
        answer["id"],
        json!(7),
        "a reply to the notification would be read as this answer: {answer}"
    );
    assert_eq!(answer["result"]["structuredContent"]["tool"], json!("sshell_exec_run"));
}

#[tokio::test]
async fn a_cancellation_for_a_request_that_already_ended_is_ignored() {
    let client = Arc::new(CancelRecordingClient::default());
    let mut bridge = Bridge::over(McpServer::new(client.clone(), 262_144));

    // The call completes first, so its mapping is gone by the time the notification is read.
    let answer = bridge
        .request("tools/call", json!({ "name": "sshell_sessions_list", "arguments": {} }))
        .await;
    assert_eq!(
        answer["result"]["structuredContent"]["tool"],
        json!("sshell_sessions_list")
    );

    bridge
        .write_raw(
            &json!({
                "jsonrpc": "2.0",
                "method": "notifications/cancelled",
                "params": { "requestId": 1 }
            })
            .to_string(),
        )
        .await;

    bridge.next_id = 40;
    let ping = bridge.request("ping", json!({})).await;
    assert_eq!(
        ping["id"],
        json!(41),
        "the session goes on after an unanswerable notice"
    );
    assert!(
        client.cancels().is_empty(),
        "a request that already has its answer has no work left to cancel: {:?}",
        client.cancels()
    );
}

// -- The result limit ---------------------------------------------------

#[tokio::test]
async fn a_result_exactly_at_the_limit_is_carried_whole() {
    const LIMIT: usize = 512;
    // A JSON string of `LIMIT - 2` characters serializes to exactly `LIMIT` bytes, quotes included.
    let value = json!("x".repeat(LIMIT - 2));
    let serialized = serde_json::to_string(&value).expect("a value is JSON");
    assert_eq!(serialized.len(), LIMIT, "the fixture has to sit on the boundary");
    let mut bridge = Bridge::over(McpServer::new(Arc::new(ValueClient { value: value.clone() }), LIMIT));

    let response = bridge
        .request("tools/call", json!({ "name": "sshell_sessions_list", "arguments": {} }))
        .await;

    assert_eq!(response["result"]["content"][0]["text"], json!(serialized));
    assert_eq!(
        response["result"]["structuredContent"], value,
        "a result that fits has to keep its complete value: {response}"
    );
}

#[tokio::test]
async fn the_ceiling_the_client_reports_is_the_effective_limit() {
    const LIMIT: usize = 64;
    let value = json!("x".repeat(LIMIT * 2));
    // The server's own cap is deliberately out of the way: the desktop's setting, carried through
    // the client, is what decides how much of an answer reaches stdout.
    let mut bridge = Bridge::over(McpServer::new(
        Arc::new(LimitedValueClient { value, limit: LIMIT }),
        usize::MAX,
    ));

    let response = bridge
        .request("tools/call", json!({ "name": "sshell_sessions_list", "arguments": {} }))
        .await;

    let structured = response["result"]["structuredContent"]
        .as_object()
        .expect("the over-limit marker");
    assert_eq!(structured["truncated"], json!(true));
    let text = response["result"]["content"][0]["text"].as_str().expect("a text block");
    assert!(text.len() <= LIMIT, "the client's ceiling was ignored: {text:?}");
}

#[tokio::test]
async fn a_result_over_the_limit_says_it_was_cut_and_carries_no_partial_value() {
    const LIMIT: usize = 512;
    let value = json!("x".repeat(LIMIT - 1));
    let mut bridge = Bridge::over(McpServer::new(Arc::new(ValueClient { value }), LIMIT));

    let response = bridge
        .request("tools/call", json!({ "name": "sshell_sessions_list", "arguments": {} }))
        .await;

    let structured = response["result"]["structuredContent"]
        .as_object()
        .expect("a marker object, not the value");
    assert_eq!(structured["truncated"], json!(true));
    assert_eq!(structured["originalBytes"], json!(LIMIT + 1));
    assert_eq!(
        structured.len(),
        2,
        "the cut value must not be echoed next to the marker: {response}"
    );

    let text = response["result"]["content"][0]["text"].as_str().expect("text");
    assert_eq!(text.len(), LIMIT, "the text block is the prefix that fits");
}
