//! The loopback control endpoint: the only door an MCP bridge has.
//!
//! One route, one method. `POST /tools/call` carries a [`ToolCall`] and answers a [`ToolOutcome`]; a
//! `GET /health` exists so a person or a script can tell a running endpoint from a port somebody else
//! took, without a token and without touching a tool.
//!
//! The checks happen in an order that keeps work off a request that could never be allowed:
//!
//! 1. **The boundary.** `Host` must name this machine's loopback address and an `Origin` header is
//!    refused outright. A page in a browser sets `Origin`; nothing here is a browser, and a request
//!    that arrives with one is either a mistake or an attempt to reach the endpoint through one.
//! 2. **The token.** Bearer, compared without `==` (see `auth`).
//! 3. **The shape.** JSON content type, a body no larger than two mebibytes.
//! 4. **The call.** Deserialized into the frozen [`ToolCall`], which refuses an unknown field — so an
//!    `acknowledged` an agent made up is a bad request, not a flag that was quietly dropped.
//!
//! There is no total timeout on the server side. A batch over a slow link or an approval window a
//! person has not looked at yet may legitimately take minutes; the deadline belongs to the approval
//! question and to each tool's own timeout, not to the socket.
//!
//! The handler is a trait rather than a direct call into the registry so the HTTP layer can be
//! tested with a fake tool and no window: the real handler below is the only place a [`ToolContext`]
//! is built, and it is built per request because approval is a question about this call and the
//! broker must be able to answer it by id.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, HOST, ORIGIN, WWW_AUTHENTICATE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use sevenshell_api::{
    AuditSink, MAX_BODY_BYTES, TOOL_CALL_PATH, ToolCall, ToolContext, ToolOutcome, ToolRegistry, local::ConnectionCache,
};
use sevenshell_core::Core;
use sevenshell_types::McpScope;

use crate::mcp::approval::{ApprovalPresenter, ApprovalRegistry, WindowBroker};
use crate::mcp::auth::{bearer_value, host_is_loopback, is_json_content_type, tokens_match};

/// What the server keeps between requests: the token every call must present and one way to run it.
pub struct ServerState {
    token: Arc<str>,
    handler: Arc<dyn CallHandler>,
}

impl ServerState {
    pub fn new(token: impl Into<String>, handler: Arc<dyn CallHandler>) -> Self {
        Self {
            token: Arc::from(token.into()),
            handler,
        }
    }
}

/// Runs one validated call.
///
/// The call id is minted per request and is what an approval answer is addressed to. A handler that
/// runs nothing (a test) can ignore both arguments.
#[async_trait]
pub trait CallHandler: Send + Sync {
    async fn call(&self, call_id: &str, call: ToolCall) -> ToolOutcome;
}

/// The limits one run was started with.
#[derive(Debug, Clone, Copy)]
pub struct CallLimits {
    pub scope: McpScope,
    pub approval_timeout_seconds: u64,
    /// The most one answer may serialize to. Tools truncate their own answers; this is the backstop
    /// that makes an over-limit answer visible in the log rather than only in the bridge's refusal.
    pub max_result_bytes: u64,
}

/// The real handler: the registry, with per-request approval and the audit trail behind it.
pub struct RegistryHandler {
    core: Arc<Core>,
    registry: Arc<ToolRegistry>,
    /// The run's one cache. A reopened file channel or container connection is the difference between
    /// a conversation and one authentication per question.
    connections: Arc<ConnectionCache>,
    approvals: Arc<ApprovalRegistry>,
    presenter: Arc<dyn ApprovalPresenter>,
    audit: Arc<dyn AuditSink>,
    limits: CallLimits,
}

impl RegistryHandler {
    pub fn new(
        core: Arc<Core>,
        registry: Arc<ToolRegistry>,
        connections: Arc<ConnectionCache>,
        approvals: Arc<ApprovalRegistry>,
        presenter: Arc<dyn ApprovalPresenter>,
        audit: Arc<dyn AuditSink>,
        limits: CallLimits,
    ) -> Self {
        Self {
            core,
            registry,
            connections,
            approvals,
            presenter,
            audit,
            limits,
        }
    }
}

#[async_trait]
impl CallHandler for RegistryHandler {
    async fn call(&self, call_id: &str, call: ToolCall) -> ToolOutcome {
        let broker = WindowBroker::new(self.approvals.clone(), self.presenter.clone(), call_id.to_owned());
        let context = ToolContext::new(self.core.clone(), self.limits.scope)
            .with_cache(self.connections.clone())
            .with_approvals(Arc::new(broker))
            .with_audit(self.audit.clone())
            .with_approval_timeout_seconds(self.limits.approval_timeout_seconds);

        let outcome = self.registry.call(&context, call).await;

        if let Ok(encoded) = serde_json::to_vec(&outcome)
            && encoded.len() as u64 > self.limits.max_result_bytes
        {
            log::warn!(
                "a tool answer of {} bytes is over the configured {}; a tool is expected to truncate before this point",
                encoded.len(),
                self.limits.max_result_bytes
            );
        }

        outcome
    }
}

/// The service, ready to be handed to `axum::serve`.
pub fn router(state: Arc<ServerState>) -> Router {
    Router::new()
        .route(TOOL_CALL_PATH, post(handle_tool_call))
        .route("/health", get(handle_health))
        .with_state(state)
}

async fn handle_tool_call(State(state): State<Arc<ServerState>>, headers: HeaderMap, body: Body) -> Response {
    if let Some(refusal) = boundary_refusal(&headers) {
        return refusal;
    }
    if !authorized(&headers, &state.token) {
        let mut response = StatusCode::UNAUTHORIZED.into_response();
        response
            .headers_mut()
            .insert(WWW_AUTHENTICATE, "Bearer".parse().expect("a valid header value"));
        return response;
    }
    if !headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(is_json_content_type)
    {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    if content_length(&headers).is_some_and(|length| length > MAX_BODY_BYTES) {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }

    // The limit is passed to the reader as well as checked above: a chunked body states no length,
    // and reading it without a bound is how a two-mebibyte rule becomes advisory.
    let bytes = match to_bytes(body, MAX_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };

    let call: ToolCall = match serde_json::from_slice(&bytes) {
        Ok(call) => call,
        Err(error) => {
            log::debug!("a control endpoint body was not a tool call: {error}");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    let call_id = uuid::Uuid::new_v4().to_string();
    Json(state.handler.call(&call_id, call).await).into_response()
}

async fn handle_health(State(_state): State<Arc<ServerState>>, headers: HeaderMap) -> Response {
    if let Some(refusal) = boundary_refusal(&headers) {
        return refusal;
    }
    Json(serde_json::json!({ "status": "ok" })).into_response()
}

/// Refuses a request whose address or origin is not this machine's own.
fn boundary_refusal(headers: &HeaderMap) -> Option<Response> {
    // Any `Origin`, including an empty one: the header is the browser's, and nothing that reaches
    // this endpoint is a browser. Checking only "an origin I know" would be checking a list that
    // would have to grow, rather than a fact that is always true here.
    if headers.contains_key(ORIGIN) {
        return Some(StatusCode::FORBIDDEN.into_response());
    }

    let host = headers.get(HOST).and_then(|value| value.to_str().ok());
    match host {
        Some(host) if host_is_loopback(host) => None,
        _ => Some(StatusCode::FORBIDDEN.into_response()),
    }
}

fn authorized(headers: &HeaderMap, expected: &str) -> bool {
    let presented = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| bearer_value(Some(value)));
    match presented {
        Some(presented) => tokens_match(expected, presented),
        None => false,
    }
}

fn content_length(headers: &HeaderMap) -> Option<usize> {
    headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use axum::http::{Request, StatusCode};
    use serde_json::{Value, json};
    use sevenshell_api::{
        AuditRecord, NoAuditSink, Tool, ToolAccess, ToolDescriptor, ToolError, ToolInputSchema, ToolResult, local,
    };
    use sevenshell_vault::Vault;
    use tower::ServiceExt as _;

    const TOKEN: &str = "the-token-of-this-run";

    #[derive(Default)]
    struct FakeHandler {
        calls: Mutex<Vec<(String, ToolCall)>>,
    }

    #[async_trait]
    impl CallHandler for FakeHandler {
        async fn call(&self, call_id: &str, call: ToolCall) -> ToolOutcome {
            self.calls
                .lock()
                .expect("not poisoned")
                .push((call_id.to_owned(), call));
            ToolOutcome::succeeded(ToolResult::new(json!({ "ok": true })))
        }
    }

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor {
                name: "test_echo".to_owned(),
                title: "Echo".to_owned(),
                description: "Answers with its arguments.".to_owned(),
                input_schema: ToolInputSchema::object(json!({}), &[]),
                access: ToolAccess::Read,
            }
        }

        async fn call(&self, _context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::new(json!({ "echo": arguments })))
        }
    }

    fn state_with(handler: Arc<dyn CallHandler>) -> Arc<ServerState> {
        Arc::new(ServerState::new(TOKEN, handler))
    }

    fn a_request(method: &str, path: &str, headers: &[(&str, &str)], body: impl Into<Body>) -> Request<Body> {
        let mut builder = Request::builder().method(method).uri(path);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder.body(body.into()).expect("a request")
    }

    fn authenticated(body: impl Into<Body>) -> Request<Body> {
        a_request(
            "POST",
            TOOL_CALL_PATH,
            &[
                (HOST.as_str(), "127.0.0.1:4711"),
                (AUTHORIZATION.as_str(), "Bearer the-token-of-this-run"),
                (CONTENT_TYPE.as_str(), "application/json"),
            ],
            body,
        )
    }

    async fn send(state: Arc<ServerState>, request: Request<Body>) -> (StatusCode, Vec<u8>) {
        let response = router(state).oneshot(request).await.expect("the service answers");
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("a body")
            .to_vec();
        (status, body)
    }

    #[tokio::test]
    async fn a_call_without_a_token_is_refused_before_anything_runs() {
        let handler = Arc::new(FakeHandler::default());
        let state = state_with(handler.clone());

        let request = a_request(
            "POST",
            TOOL_CALL_PATH,
            &[
                (HOST.as_str(), "127.0.0.1:4711"),
                (CONTENT_TYPE.as_str(), "application/json"),
            ],
            r#"{"name":"test_echo"}"#,
        );
        let (status, _) = send(state, request).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(handler.calls.lock().expect("not poisoned").is_empty());
    }

    #[tokio::test]
    async fn a_wrong_token_is_refused() {
        let state = state_with(Arc::new(FakeHandler::default()));
        let request = a_request(
            "POST",
            TOOL_CALL_PATH,
            &[
                (HOST.as_str(), "127.0.0.1:4711"),
                (AUTHORIZATION.as_str(), "Bearer some-other-run"),
                (CONTENT_TYPE.as_str(), "application/json"),
            ],
            r#"{"name":"test_echo"}"#,
        );

        let (status, _) = send(state, request).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn an_origin_header_is_refused_even_with_the_right_token() {
        let handler = Arc::new(FakeHandler::default());
        let state = state_with(handler.clone());
        let request = a_request(
            "POST",
            TOOL_CALL_PATH,
            &[
                (HOST.as_str(), "127.0.0.1:4711"),
                (ORIGIN.as_str(), "http://evil.example"),
                (AUTHORIZATION.as_str(), "Bearer the-token-of-this-run"),
                (CONTENT_TYPE.as_str(), "application/json"),
            ],
            r#"{"name":"test_echo"}"#,
        );

        let (status, _) = send(state, request).await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(handler.calls.lock().expect("not poisoned").is_empty());
    }

    #[tokio::test]
    async fn a_host_that_is_not_the_loopback_literal_is_refused() {
        let state = state_with(Arc::new(FakeHandler::default()));
        let request = a_request(
            "POST",
            TOOL_CALL_PATH,
            &[
                (HOST.as_str(), "localhost:4711"),
                (AUTHORIZATION.as_str(), "Bearer the-token-of-this-run"),
                (CONTENT_TYPE.as_str(), "application/json"),
            ],
            r#"{"name":"test_echo"}"#,
        );

        let (status, _) = send(state, request).await;

        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn a_body_that_is_not_json_is_refused() {
        let state = state_with(Arc::new(FakeHandler::default()));
        let request = a_request(
            "POST",
            TOOL_CALL_PATH,
            &[
                (HOST.as_str(), "127.0.0.1:4711"),
                (AUTHORIZATION.as_str(), "Bearer the-token-of-this-run"),
                (CONTENT_TYPE.as_str(), "text/plain"),
            ],
            r#"{"name":"test_echo"}"#,
        );

        let (status, _) = send(state, request).await;

        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn an_oversized_body_is_refused_by_the_length_header() {
        let handler = Arc::new(FakeHandler::default());
        let state = state_with(handler.clone());
        let request = a_request(
            "POST",
            TOOL_CALL_PATH,
            &[
                (HOST.as_str(), "127.0.0.1:4711"),
                (AUTHORIZATION.as_str(), "Bearer the-token-of-this-run"),
                (CONTENT_TYPE.as_str(), "application/json"),
                ("content-length", &format!("{}", MAX_BODY_BYTES + 1)),
            ],
            Body::empty(),
        );

        let (status, _) = send(state, request).await;

        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(handler.calls.lock().expect("not poisoned").is_empty());
    }

    #[tokio::test]
    async fn an_oversized_body_is_refused_even_when_its_length_is_not_stated() {
        let state = state_with(Arc::new(FakeHandler::default()));
        let request = authenticated(" ".repeat(MAX_BODY_BYTES + 1));

        let (status, _) = send(state, request).await;

        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn an_authenticated_call_reaches_the_handler() {
        let handler = Arc::new(FakeHandler::default());
        let state = state_with(handler.clone());

        let (status, body) = send(state, authenticated(r#"{"name":"test_echo","arguments":{"x":1}}"#)).await;

        assert_eq!(status, StatusCode::OK);
        let outcome: ToolOutcome = serde_json::from_slice(&body).expect("a tool outcome");
        assert!(outcome.is_succeeded());

        let calls = handler.calls.lock().expect("not poisoned");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1.name, "test_echo");
        assert_eq!(calls[0].1.arguments, json!({ "x": 1 }));
        assert!(!calls[0].0.is_empty(), "every request gets an approval id");
    }

    #[tokio::test]
    async fn a_malformed_call_is_a_bad_request() {
        let state = state_with(Arc::new(FakeHandler::default()));

        let (status, _) = send(state, authenticated(r#"{"tool":"test_echo"}"#)).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    fn registry_handler(registry: ToolRegistry) -> Arc<dyn CallHandler> {
        let directory = tempfile::tempdir().expect("a temp directory");
        let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
        let (core, events) = Core::new(vault);
        // The receiver lives as long as the handler would; no terminal event is produced here.
        drop(events);
        let connections = Arc::new(ConnectionCache::new(Arc::clone(&core)));
        Arc::new(RegistryHandler::new(
            core,
            Arc::new(registry),
            connections,
            Arc::new(ApprovalRegistry::default()),
            Arc::new(silent_presenter()),
            Arc::new(NoAuditSink),
            CallLimits {
                scope: McpScope::ReadOnly,
                approval_timeout_seconds: 120,
                max_result_bytes: sevenshell_api::DEFAULT_MAX_RESULT_BYTES,
            },
        ))
    }

    struct SilentPresenter;

    impl ApprovalPresenter for SilentPresenter {
        fn present(&self, _request: &crate::mcp::approval::McpApprovalRequest) -> bool {
            false
        }
    }

    fn silent_presenter() -> SilentPresenter {
        SilentPresenter
    }

    #[tokio::test]
    async fn a_known_tool_runs_through_the_registry() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool)).expect("a unique name");
        let state = state_with(registry_handler(registry));

        let (status, body) = send(state, authenticated(r#"{"name":"test_echo","arguments":{"x":1}}"#)).await;

        assert_eq!(status, StatusCode::OK);
        let outcome: ToolOutcome = serde_json::from_slice(&body).expect("a tool outcome");
        match outcome {
            ToolOutcome::Succeeded { result } => assert_eq!(result.value, json!({ "echo": { "x": 1 } })),
            ToolOutcome::Failed { error } => panic!("the tool ran: {}", error.code()),
        }
    }

    #[tokio::test]
    async fn an_unknown_tool_is_an_outcome_not_an_http_error() {
        let state = state_with(registry_handler(ToolRegistry::new()));

        let (status, body) = send(state, authenticated(r#"{"name":"sshell_no_such_tool"}"#)).await;

        // A working desktop answered; the desktop's answer is that the tool does not exist. The
        // bridge turns that into an MCP error, but HTTP is not the place for the distinction.
        assert_eq!(status, StatusCode::OK);
        let outcome: ToolOutcome = serde_json::from_slice(&body).expect("a tool outcome");
        assert_eq!(outcome.error().expect("a refusal").code(), "MCP-UNKNOWN-TOOL");
    }

    #[tokio::test]
    async fn the_default_registry_is_reachable_without_a_window() {
        // The list tool is the one implemented tool in this wave; calling it with a locked vault must
        // still answer rather than crash, and it must be an outcome the bridge can render.
        let state = state_with(registry_handler(local::default_registry()));

        let (status, body) = send(
            state,
            authenticated(r#"{"name":"sshell_sessions_list","arguments":{"kind":"saved"}}"#),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let outcome: ToolOutcome = serde_json::from_slice(&body).expect("a tool outcome");
        assert!(
            outcome.error().is_some(),
            "the vault is locked, so the listing is refused"
        );
    }

    #[tokio::test]
    async fn health_answers_without_a_token_but_still_checks_the_boundary() {
        let state = state_with(Arc::new(FakeHandler::default()));

        let (status, _) = send(
            state.clone(),
            a_request("GET", "/health", &[(HOST.as_str(), "127.0.0.1:4711")], Body::empty()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, _) = send(
            state,
            a_request("GET", "/health", &[(HOST.as_str(), "evil.example")], Body::empty()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn a_call_that_is_not_a_call_is_not_routed() {
        let state = state_with(Arc::new(FakeHandler::default()));

        let (status, _) = send(
            state,
            a_request(
                "GET",
                TOOL_CALL_PATH,
                &[(HOST.as_str(), "127.0.0.1:4711")],
                Body::empty(),
            ),
        )
        .await;

        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    }

    /// A sink that proves the registry's audit line reached the handler's trail.
    struct CountingSink(std::sync::atomic::AtomicUsize);

    impl AuditSink for CountingSink {
        fn record(&self, _record: AuditRecord) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn every_call_writes_one_audit_line_through_the_server() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool)).expect("a unique name");
        let directory = tempfile::tempdir().expect("a temp directory");
        let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
        let (core, events) = Core::new(vault);
        drop(events);
        let sink = Arc::new(CountingSink(std::sync::atomic::AtomicUsize::new(0)));
        let connections = Arc::new(ConnectionCache::new(Arc::clone(&core)));
        let handler = Arc::new(RegistryHandler::new(
            core,
            Arc::new(registry),
            connections,
            Arc::new(ApprovalRegistry::default()),
            Arc::new(SilentPresenter),
            sink.clone(),
            CallLimits {
                scope: McpScope::ReadOnly,
                approval_timeout_seconds: 120,
                max_result_bytes: sevenshell_api::DEFAULT_MAX_RESULT_BYTES,
            },
        ));

        let (status, _) = send(state_with(handler), authenticated(r#"{"name":"test_echo"}"#)).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(sink.0.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
