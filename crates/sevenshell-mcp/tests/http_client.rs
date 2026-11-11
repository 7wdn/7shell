//! The client half against a real loopback listener.
//!
//! `HttpToolClient` is the one piece of this crate that speaks to another process, so it is tested
//! against a socket rather than a mock: the assertions are about what actually crossed — the method
//! line, the bearer token, the content type and the body — and about what the bridge says when the
//! answer is a refusal. The fake desktop is in `support`, one route, hand-written.

mod support;

use serde_json::{Value, json};

use sevenshell_api::{
    CALL_ID_HEADER, ENDPOINT_FILE_NAME, MAX_BODY_BYTES, TOOL_CALL_PATH, TOOL_CANCEL_PATH, TOOLS_PATH, ToolAccess,
    ToolCall,
};
use sevenshell_mcp::{DESKTOP_UNAVAILABLE, HttpToolClient, INVALID_PARAMS, TOOL_FAILED, ToolClient};

use support::{FakeDesktop, Reply, write_endpoint_file};

/// A tool only a desktop with external servers could list.
fn external_tool() -> Value {
    json!({
        "name": "mcp__lab__list_things",
        "title": "List things",
        "description": "Lists things on the lab server.",
        "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
        "access": "read"
    })
}

#[tokio::test]
async fn a_call_is_posted_as_a_tool_call_with_the_bearer_token() {
    let desktop = FakeDesktop::start(Reply::succeeded(json!({ "sessions": [] }))).await;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let endpoint = desktop.write_endpoint(directory.path(), "a-token");

    let client = HttpToolClient::new(endpoint).expect("a client");
    let outcome = client
        .call("sshell_sessions_list", json!({ "kind": "open" }), "a-call-id")
        .await
        .expect("the desktop answered");

    assert!(outcome.is_succeeded(), "{outcome:?}");

    let captured = desktop.captured();
    assert_eq!(captured.len(), 1, "one call is one request");

    let request = &captured[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, TOOL_CALL_PATH);
    assert_eq!(request.header("authorization"), Some("Bearer a-token"));
    assert_eq!(
        request.header(CALL_ID_HEADER),
        Some("a-call-id"),
        "the call id is what a cancellation will name, so it has to travel with the call"
    );
    assert!(
        request
            .header("content-type")
            .is_some_and(|value| value.starts_with("application/json")),
        "the endpoint only reads a JSON body: {:?}",
        request.header("content-type")
    );

    let sent: ToolCall = serde_json::from_slice(&request.body).expect("the body is a ToolCall");
    assert_eq!(sent.name, "sshell_sessions_list");
    assert_eq!(sent.arguments, json!({ "kind": "open" }));

    let body: serde_json::Value = serde_json::from_slice(&request.body).expect("a JSON object");
    let mut keys: Vec<&str> = body
        .as_object()
        .expect("the body is an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["arguments", "name"],
        "the body must carry no field a tool did not declare"
    );
}

#[tokio::test]
async fn a_token_the_desktop_refuses_is_an_unavailable_desktop_and_never_an_echo() {
    let desktop = FakeDesktop::start(Reply::status(401)).await;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let endpoint = desktop.write_endpoint(directory.path(), "a-secret-token");

    let client = HttpToolClient::new(endpoint).expect("a client");
    let error = client
        .call("sshell_sessions_list", json!({}), "a-call-id")
        .await
        .expect_err("401 is a refusal");

    assert_eq!(error.code, DESKTOP_UNAVAILABLE);
    assert!(
        error.message.contains("restart"),
        "the person's next step has to be in the sentence: {}",
        error.message
    );
    assert!(
        !error.message.contains("a-secret-token"),
        "a token must never travel back through an answer: {}",
        error.message
    );
}

#[tokio::test]
async fn a_body_the_endpoint_rejects_for_its_size_is_an_invalid_call() {
    let desktop = FakeDesktop::start(Reply::status(413)).await;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let endpoint = desktop.write_endpoint(directory.path(), "a-token");

    let client = HttpToolClient::new(endpoint).expect("a client");
    let error = client
        .call("sshell_sftp_write_text", json!({ "text": "small enough" }), "a-call-id")
        .await
        .expect_err("413 is a refusal");

    assert_eq!(error.code, INVALID_PARAMS);
    assert!(
        error.message.contains("larger"),
        "the refusal has to say what was wrong: {}",
        error.message
    );
}

#[tokio::test]
async fn a_body_over_the_endpoints_limit_is_refused_before_anything_is_sent() {
    let desktop = FakeDesktop::start(Reply::succeeded(json!({}))).await;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let endpoint = desktop.write_endpoint(directory.path(), "a-token");

    let client = HttpToolClient::new(endpoint).expect("a client");
    let error = client
        .call(
            "sshell_sftp_write_text",
            // One byte over the endpoint's own limit, so the bridge's pre-flight check is the only
            // thing that can refuse it: the fake desktop would answer 200.
            json!({ "text": "x".repeat(MAX_BODY_BYTES) }),
            "a-call-id",
        )
        .await
        .expect_err("the body is over the limit");

    assert_eq!(error.code, INVALID_PARAMS);
    assert_eq!(
        desktop.captured().len(),
        0,
        "a body the endpoint would reject must not be sent"
    );
}

#[tokio::test]
async fn a_missing_endpoint_file_is_reported_without_reading_the_network() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let missing = directory.path().join(ENDPOINT_FILE_NAME);

    let client = HttpToolClient::new(missing).expect("a client");
    let error = client
        .call("sshell_sessions_list", json!({}), "a-call-id")
        .await
        .expect_err("there is no endpoint file");

    assert_eq!(error.code, DESKTOP_UNAVAILABLE);
    assert!(
        error.message.contains("start 7Shell"),
        "the answer has to say what a person can do: {}",
        error.message
    );
    assert!(
        !error.message.contains(directory.path().to_string_lossy().as_ref()),
        "the path is a stderr fact, not part of an answer: {}",
        error.message
    );
}

#[tokio::test]
async fn every_call_rereads_the_endpoint_file_so_a_rotated_token_is_used() {
    let desktop = FakeDesktop::start(Reply::succeeded(json!({ "sessions": [] }))).await;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let endpoint = directory.path().join(ENDPOINT_FILE_NAME);
    write_endpoint_file(&endpoint, desktop.port(), "the-first-run");

    let client = HttpToolClient::new(endpoint.clone()).expect("a client");
    client
        .call("sshell_sessions_list", json!({}), "a-call-id")
        .await
        .expect("the first call");

    // The desktop restarted between the two calls: a new token in the same file, same port. A
    // client that cached the token would present the one that just stopped working.
    write_endpoint_file(&endpoint, desktop.port(), "the-second-run");
    client
        .call("sshell_sessions_list", json!({}), "a-call-id")
        .await
        .expect("the second call");

    let captured = desktop.captured();
    let tokens: Vec<Option<&str>> = captured.iter().map(|request| request.header("authorization")).collect();
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[0], Some("Bearer the-first-run"));
    assert_eq!(
        tokens[1],
        Some("Bearer the-second-run"),
        "the second call was sent with a token that was no longer in the file"
    );
}

// -- The desktop's tool table ------------------------------------------

#[tokio::test]
async fn the_tool_table_is_read_from_the_desktop_with_the_bearer_token_and_cached() {
    let table = json!({ "tools": [external_tool()] }).to_string();
    let desktop = FakeDesktop::start(Reply::with_body(200, table)).await;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let endpoint = desktop.write_endpoint(directory.path(), "a-token");

    let client = HttpToolClient::new(endpoint).expect("a client");
    assert_eq!(
        client.descriptors().len(),
        10,
        "before a refresh the compiled list is what a list answers"
    );

    client
        .refresh_descriptors()
        .await
        .expect("the desktop listed its tools");

    let listed = client.descriptors();
    assert_eq!(listed.len(), 1, "the desktop's table replaces the compiled one");
    assert_eq!(listed[0].name, "mcp__lab__list_things");
    assert_eq!(listed[0].access, ToolAccess::Read);

    let captured = desktop.captured();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].method, "GET");
    assert_eq!(captured[0].path, TOOLS_PATH);
    assert_eq!(
        captured[0].header("authorization"),
        Some("Bearer a-token"),
        "the table is as much a secret as a call"
    );
}

#[tokio::test]
async fn a_bare_array_is_also_read_as_the_tool_table() {
    let desktop = FakeDesktop::start(Reply::with_body(200, json!([external_tool()]).to_string())).await;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let endpoint = desktop.write_endpoint(directory.path(), "a-token");

    let client = HttpToolClient::new(endpoint).expect("a client");
    client.refresh_descriptors().await.expect("the array is a table");

    assert_eq!(client.descriptors().len(), 1);
    assert_eq!(client.descriptors()[0].name, "mcp__lab__list_things");
}

#[tokio::test]
async fn a_table_that_cannot_be_read_leaves_the_compiled_list_in_place() {
    let table = Reply::with_body(200, json!({ "tools": [external_tool()] }).to_string());
    let desktop = FakeDesktop::start(table.clone()).await;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let endpoint = desktop.write_endpoint(directory.path(), "a-token");

    let client = HttpToolClient::new(endpoint).expect("a client");

    // Every way the table can fail to arrive: a body that is not JSON, an empty table, and two
    // refusals. Each is preceded by a successful fetch so the fallback has something to undo.
    for failure in [
        Reply::with_body(200, "not json"),
        Reply::with_body(200, "[]"),
        Reply::status(401),
        Reply::status(500),
    ] {
        desktop.reply_with(table.clone());
        client.refresh_descriptors().await.expect("a good table");
        assert_eq!(client.descriptors().len(), 1, "the fetched table");

        desktop.reply_with(failure);
        client
            .refresh_descriptors()
            .await
            .expect("a refresh that cannot be read is not a session failure");
        assert_eq!(
            client.descriptors().len(),
            10,
            "the compiled list is the floor, not the table of a desktop that is gone"
        );
    }
}

#[tokio::test]
async fn a_refresh_without_an_endpoint_file_answers_from_the_compiled_list() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let client = HttpToolClient::new(directory.path().join(ENDPOINT_FILE_NAME)).expect("a client");

    client
        .refresh_descriptors()
        .await
        .expect("a missing desktop is not a failed session");

    assert_eq!(client.descriptors().len(), 10);
}

// -- Cancelling ---------------------------------------------------------

#[tokio::test]
async fn a_cancellation_names_the_call_on_the_cancel_route() {
    let desktop = FakeDesktop::start(Reply::with_body(200, json!({ "cancelled": true }).to_string())).await;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let endpoint = desktop.write_endpoint(directory.path(), "a-token");

    let client = HttpToolClient::new(endpoint).expect("a client");
    client.cancel("the-call").await.expect("200 is a cancellation");

    let captured = desktop.captured();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].method, "POST");
    assert_eq!(captured[0].path, TOOL_CANCEL_PATH);
    assert_eq!(captured[0].header("authorization"), Some("Bearer a-token"));
    let body: Value = serde_json::from_slice(&captured[0].body).expect("a JSON object");
    assert_eq!(
        body,
        json!({ "callId": "the-call" }),
        "the body names the call and nothing else"
    );
}

#[tokio::test]
async fn a_cancel_the_desktop_refuses_carries_the_right_code() {
    let desktop = FakeDesktop::start(Reply::status(401)).await;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let endpoint = desktop.write_endpoint(directory.path(), "a-token");

    let client = HttpToolClient::new(endpoint).expect("a client");
    assert_eq!(
        client.cancel("the-call").await.expect_err("401 is a refusal").code,
        DESKTOP_UNAVAILABLE
    );

    // A desktop that answered and has no cancel route is not unreachable: it is a call-level
    // request that went wrong, which is what the tool failure code says.
    desktop.reply_with(Reply::status(404));
    assert_eq!(
        client.cancel("the-call").await.expect_err("no cancel route").code,
        TOOL_FAILED
    );
}

#[tokio::test]
async fn a_cancel_without_an_endpoint_file_is_an_unavailable_desktop() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let client = HttpToolClient::new(directory.path().join(ENDPOINT_FILE_NAME)).expect("a client");

    assert_eq!(
        client.cancel("the-call").await.expect_err("nothing to talk to").code,
        DESKTOP_UNAVAILABLE
    );
}
