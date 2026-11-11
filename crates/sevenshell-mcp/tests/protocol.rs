//! The protocol, driven over a pipe rather than over stdin.
//!
//! The server half is exercised through its real `serve` loop on an in-memory duplex stream, so what
//! is tested is what a client gets: lines out, each under the id of the request that caused it, and
//! nothing on the wire that is not JSON. The desktop is a fake `ToolClient`, which is the point of
//! the trait: the protocol can be tested without a running application, a vault or a network.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};
use tokio::time::timeout;

use sevenshell_api::{ToolDescriptor, ToolError, ToolOutcome, ToolResult, local};
use sevenshell_error::ShellError;
use sevenshell_mcp::{
    DESKTOP_UNAVAILABLE, INVALID_PARAMS, INVALID_REQUEST, LATEST_PROTOCOL_VERSION, METHOD_NOT_FOUND, McpServer,
    PARSE_ERROR, RpcError, TOOL_FAILED, ToolClient,
};

/// A desktop that answers with whatever a test told it to.
struct FakeClient {
    descriptors: Vec<ToolDescriptor>,
    /// The tool's own refusal, as a result the desktop returns normally.
    tool_error: Option<ToolError>,
    /// The desktop not answering at all.
    client_error: Option<RpcError>,
    /// One tool that takes its time, so a fast answer sent after it can overtake it.
    slow: Option<(&'static str, Duration)>,
    /// A result larger than any sane context window.
    huge: bool,
    calls: AtomicUsize,
}

impl FakeClient {
    fn new() -> Self {
        Self {
            descriptors: local::default_registry().descriptors(),
            tool_error: None,
            client_error: None,
            slow: None,
            huge: false,
            calls: AtomicUsize::new(0),
        }
    }

    fn huge(self) -> Self {
        Self { huge: true, ..self }
    }

    fn slow_on(self, name: &'static str, delay: Duration) -> Self {
        Self {
            slow: Some((name, delay)),
            ..self
        }
    }

    fn failing_with(self, error: ToolError) -> Self {
        Self {
            tool_error: Some(error),
            ..self
        }
    }

    fn unreachable(self, error: RpcError) -> Self {
        Self {
            client_error: Some(error),
            ..self
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ToolClient for FakeClient {
    fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.descriptors.clone()
    }

    async fn call(&self, name: &str, _arguments: Value) -> Result<ToolOutcome, RpcError> {
        self.calls.fetch_add(1, Ordering::SeqCst);

        if let Some(error) = &self.client_error {
            return Err(error.clone());
        }
        if let Some(error) = &self.tool_error {
            return Ok(ToolOutcome::failed(error.clone()));
        }
        if let Some((slow_name, delay)) = self.slow
            && slow_name == name
        {
            tokio::time::sleep(delay).await;
        }

        let value = if self.huge {
            json!({ "text": "x".repeat(400_000) })
        } else {
            json!({ "tool": name })
        };
        Ok(ToolOutcome::succeeded(ToolResult::new(value)))
    }
}

/// One bridge on one pipe, with the fake desktop behind it.
struct Harness {
    client: Arc<FakeClient>,
    side: BufReader<DuplexStream>,
    next_id: i64,
}

impl Harness {
    fn new() -> Self {
        Self::with(FakeClient::new(), 262_144)
    }

    fn with(client: FakeClient, max_result_bytes: usize) -> Self {
        let client = Arc::new(client);
        let server = Arc::new(McpServer::new(client.clone(), max_result_bytes));
        let (server_side, client_side) = tokio::io::duplex(64 * 1024);
        let (reader, writer) = tokio::io::split(server_side);
        tokio::spawn(async move {
            let _ = server.serve(reader, writer).await;
        });

        Self {
            client,
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

    async fn notify(&mut self, method: &str) {
        let message = json!({ "jsonrpc": "2.0", "method": method });
        self.write_raw(&message.to_string()).await;
    }

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

fn error_code(response: &Value) -> i64 {
    response["error"]["code"]
        .as_i64()
        .unwrap_or_else(|| panic!("no error code in {response}"))
}

// -- The handshake ------------------------------------------------------

#[tokio::test]
async fn the_handshake_answers_with_the_version_the_client_asked_for() {
    let mut harness = Harness::new();

    let response = harness
        .request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0" }
            }),
        )
        .await;

    assert_eq!(response["jsonrpc"], json!("2.0"));
    assert_eq!(response["id"], json!(1));
    assert_eq!(response["result"]["protocolVersion"], json!("2024-11-05"));
    assert_eq!(response["result"]["capabilities"]["tools"]["listChanged"], json!(false));
    assert_eq!(response["result"]["serverInfo"]["name"], json!("sshell-mcp"));
}

#[tokio::test]
async fn a_version_this_build_does_not_speak_falls_back_to_the_one_it_does() {
    let mut harness = Harness::new();

    let response = harness
        .request(
            "initialize",
            json!({ "protocolVersion": "2099-01-01", "capabilities": {}, "clientInfo": { "name": "t", "version": "0" } }),
        )
        .await;

    assert_eq!(response["result"]["protocolVersion"], json!(LATEST_PROTOCOL_VERSION));
}

#[tokio::test]
async fn a_ping_is_answered_with_an_empty_result() {
    let mut harness = Harness::new();
    harness.initialize().await;

    let response = harness.request("ping", json!({})).await;

    assert_eq!(response["result"], json!({}));
}

#[tokio::test]
async fn a_notification_is_never_answered() {
    let mut harness = Harness::new();
    harness.initialize().await;

    harness.notify("notifications/initialized").await;

    // The next line the client reads must be the answer to its own request. A notification that had
    // produced something would be in front of it.
    harness.next_id = 76;
    let response = harness.request("ping", json!({})).await;
    assert_eq!(response["id"], json!(77));
}

// -- The list -----------------------------------------------------------

#[tokio::test]
async fn the_tool_list_is_the_ten_prefixed_tools_with_closed_schemas() {
    let mut harness = Harness::new();
    harness.initialize().await;

    let response = harness.request("tools/list", json!({})).await;
    let tools = response["result"]["tools"].as_array().expect("a list of tools");

    assert_eq!(tools.len(), 10, "the first wave is ten tools");
    for tool in tools {
        let name = tool["name"].as_str().expect("a name");
        assert!(name.starts_with("sshell_"), "{name} is outside the shared prefix");
        assert_eq!(tool["inputSchema"]["type"], json!("object"));
        assert_eq!(tool["inputSchema"]["additionalProperties"], json!(false));
        assert!(tool["description"].as_str().is_some_and(|text| !text.trim().is_empty()));
    }

    let read_only = tools
        .iter()
        .find(|tool| tool["name"] == json!("sshell_k8s_get"))
        .expect("a read tool");
    assert_eq!(read_only["annotations"]["readOnlyHint"], json!(true));

    let write = tools
        .iter()
        .find(|tool| tool["name"] == json!("sshell_exec_run"))
        .expect("a write tool");
    assert_eq!(write["annotations"]["readOnlyHint"], json!(false));
}

// -- Refusals -----------------------------------------------------------

#[tokio::test]
async fn a_line_that_is_not_json_is_a_parse_error() {
    let mut harness = Harness::new();

    harness.write_raw("{ not json").await;
    let response = harness.read().await;

    assert_eq!(error_code(&response), PARSE_ERROR);
    assert_eq!(response["id"], Value::Null);
}

#[tokio::test]
async fn a_request_without_a_method_is_an_invalid_request_that_still_echoes_its_id() {
    let mut harness = Harness::new();

    harness.write_raw(r#"{"jsonrpc":"2.0","id":9}"#).await;
    let response = harness.read().await;

    assert_eq!(error_code(&response), INVALID_REQUEST);
    assert_eq!(response["id"], json!(9));
}

#[tokio::test]
async fn an_unknown_method_is_refused_by_name() {
    let mut harness = Harness::new();
    harness.initialize().await;

    let response = harness.request("resources/list", json!({})).await;

    assert_eq!(error_code(&response), METHOD_NOT_FOUND);
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("resources/list")),
        "the refusal has to name the method: {response}"
    );
}

#[tokio::test]
async fn an_unknown_tool_is_refused_before_the_desktop_is_asked() {
    let mut harness = Harness::new();
    harness.initialize().await;

    let response = harness
        .request("tools/call", json!({ "name": "sshell_never_existed", "arguments": {} }))
        .await;

    assert_eq!(error_code(&response), INVALID_PARAMS);
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("sshell_never_existed")),
        "the refusal has to name the tool: {response}"
    );
    assert_eq!(harness.client.calls(), 0, "an unknown tool never reaches the desktop");
}

#[tokio::test]
async fn an_acknowledgement_is_refused_before_the_desktop_is_asked() {
    let mut harness = Harness::new();
    harness.initialize().await;

    let response = harness
        .request(
            "tools/call",
            json!({
                "name": "sshell_exec_run",
                "arguments": { "sessionId": "lab", "command": "rm -rf /srv/app", "acknowledged": true }
            }),
        )
        .await;

    assert_eq!(error_code(&response), INVALID_PARAMS);
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("acknowledged")),
        "the refusal has to explain itself: {response}"
    );
    assert_eq!(harness.client.calls(), 0, "an approval is never an agent's to send");
}

#[tokio::test]
async fn a_tool_failure_crosses_with_its_own_code_and_key() {
    let mut harness = Harness::with(
        FakeClient::new().failing_with(ToolError::from_shell(ShellError::vault_locked())),
        262_144,
    );
    harness.initialize().await;

    let response = harness
        .request("tools/call", json!({ "name": "sshell_sessions_list", "arguments": {} }))
        .await;

    assert_eq!(error_code(&response), TOOL_FAILED);
    assert_eq!(response["error"]["data"]["code"], json!("VAULT-LOCKED"));
    assert_eq!(response["error"]["data"]["messageKey"], json!("errors.vault.locked"));
}

#[tokio::test]
async fn a_desktop_that_is_not_running_crosses_under_its_own_code() {
    let mut harness = Harness::with(
        FakeClient::new().unreachable(RpcError::desktop_unavailable("its control endpoint is not answering")),
        262_144,
    );
    harness.initialize().await;

    let response = harness
        .request("tools/call", json!({ "name": "sshell_sessions_list", "arguments": {} }))
        .await;

    assert_eq!(error_code(&response), DESKTOP_UNAVAILABLE);
}

// -- Answers out of order ----------------------------------------------

#[tokio::test]
async fn answers_carry_the_id_of_the_request_they_answer_even_out_of_order() {
    let mut harness = Harness::with(
        FakeClient::new().slow_on("sshell_exec_run", Duration::from_millis(250)),
        262_144,
    );
    harness.initialize().await;

    // The slow call is sent first; the fast one is sent immediately after. The protocol allows both
    // to be in flight, and the fast answer must not be held behind the slow one.
    harness
        .send(
            20,
            "tools/call",
            json!({ "name": "sshell_exec_run", "arguments": { "sessionId": "lab", "command": "sleep 1" } }),
        )
        .await;
    harness
        .send(
            21,
            "tools/call",
            json!({ "name": "sshell_sessions_list", "arguments": {} }),
        )
        .await;

    let first = harness.read().await;
    let second = harness.read().await;

    assert_eq!(
        first["id"],
        json!(21),
        "the fast answer should not wait for the slow one"
    );
    assert_eq!(second["id"], json!(20));

    let first_text = first["result"]["content"][0]["text"].as_str().expect("text");
    assert!(first_text.contains("sshell_sessions_list"));
    let second_text = second["result"]["content"][0]["text"].as_str().expect("text");
    assert!(second_text.contains("sshell_exec_run"));
}

#[tokio::test]
async fn a_string_id_is_echoed_as_the_string_it_was() {
    let mut harness = Harness::new();
    harness.initialize().await;

    harness
        .write_raw(r#"{"jsonrpc":"2.0","id":"call-1","method":"ping","params":{}}"#)
        .await;
    let response = harness.read().await;

    assert_eq!(response["id"], json!("call-1"));
}

// -- The result limit ---------------------------------------------------

#[tokio::test]
async fn a_result_over_the_limit_is_cut_at_a_character_boundary_and_says_so() {
    let mut harness = Harness::with(FakeClient::new().huge(), 4096);
    harness.initialize().await;

    let response = harness
        .request("tools/call", json!({ "name": "sshell_sessions_list", "arguments": {} }))
        .await;

    let text = response["result"]["content"][0]["text"].as_str().expect("text");
    assert!(text.len() <= 4096, "the limit was not the one the bridge was given");
    assert!(
        text.is_char_boundary(text.len()),
        "a cut in the middle of a character produces a text block that is not UTF-8"
    );
    assert_eq!(response["result"]["structuredContent"]["truncated"], json!(true));
    assert!(
        response["result"]["structuredContent"]["originalBytes"]
            .as_u64()
            .is_some_and(|bytes| bytes > 4096),
        "the answer has to say how much was cut: {response}"
    );
}

#[tokio::test]
async fn a_result_under_the_limit_is_carried_whole() {
    let mut harness = Harness::new();
    harness.initialize().await;

    let response = harness
        .request("tools/call", json!({ "name": "sshell_sessions_list", "arguments": {} }))
        .await;

    assert_eq!(
        response["result"]["structuredContent"]["tool"],
        json!("sshell_sessions_list")
    );
    assert_eq!(response["result"]["structuredContent"].get("truncated"), None);
}
