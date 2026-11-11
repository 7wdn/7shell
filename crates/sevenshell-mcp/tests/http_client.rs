//! The client half against a real loopback listener.
//!
//! `HttpToolClient` is the one piece of this crate that speaks to another process, so it is tested
//! against a socket rather than a mock: the assertions are about what actually crossed — the method
//! line, the bearer token, the content type and the body — and about what the bridge says when the
//! answer is a refusal. The fake desktop is in `support`, one route, hand-written.

mod support;

use serde_json::json;

use sevenshell_api::{ENDPOINT_FILE_NAME, MAX_BODY_BYTES, TOOL_CALL_PATH, ToolCall};
use sevenshell_mcp::{DESKTOP_UNAVAILABLE, HttpToolClient, INVALID_PARAMS, ToolClient};

use support::{FakeDesktop, Reply, write_endpoint_file};

#[tokio::test]
async fn a_call_is_posted_as_a_tool_call_with_the_bearer_token() {
    let desktop = FakeDesktop::start(Reply::succeeded(json!({ "sessions": [] }))).await;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let endpoint = desktop.write_endpoint(directory.path(), "a-token");

    let client = HttpToolClient::new(endpoint).expect("a client");
    let outcome = client
        .call("sshell_sessions_list", json!({ "kind": "open" }))
        .await
        .expect("the desktop answered");

    assert!(outcome.is_succeeded(), "{outcome:?}");

    let captured = desktop.captured();
    assert_eq!(captured.len(), 1, "one call is one request");

    let request = &captured[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, TOOL_CALL_PATH);
    assert_eq!(request.header("authorization"), Some("Bearer a-token"));
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
        .call("sshell_sessions_list", json!({}))
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
        .call("sshell_sftp_write_text", json!({ "text": "small enough" }))
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
        .call("sshell_sessions_list", json!({}))
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
        .call("sshell_sessions_list", json!({}))
        .await
        .expect("the first call");

    // The desktop restarted between the two calls: a new token in the same file, same port. A
    // client that cached the token would present the one that just stopped working.
    write_endpoint_file(&endpoint, desktop.port(), "the-second-run");
    client
        .call("sshell_sessions_list", json!({}))
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
