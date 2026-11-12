//! The loopback control endpoint: the only door an MCP bridge has.
//!
//! Three routes, all on loopback and all behind the same boundary:
//!
//! * `POST /tools/call` carries a [`ToolCall`] and answers a [`ToolOutcome`]. The call's own id
//!   travels in [`CALL_ID_HEADER`] when the bridge minted one, so a later cancel can name it.
//! * `GET /tools` answers the one table an agent sees — the built-in tools and every external
//!   server's, merged by the registry. The desktop is authoritative: a bridge that cannot reach it
//!   falls back to its compiled list rather than to a partial one.
//! * `POST /tools/cancel` stops one call by the id the caller gave it. The caller's connection is
//!   not the thing being stopped: a tool call may outlive the request that started it.
//!
//! A `GET /health` exists so a person or a script can tell a running endpoint from a port somebody
//! else took, without a token and without touching a tool.
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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, HOST, ORIGIN, WWW_AUTHENTICATE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use sevenshell_api::{
    AuditSink, CALL_ID_HEADER, Cancel, MAX_BODY_BYTES, TOOL_CALL_PATH, TOOL_CANCEL_PATH, TOOLS_PATH, ToolCall,
    ToolContext, ToolOutcome, ToolRegistry, local::ConnectionCache,
};
use sevenshell_core::Core;
use sevenshell_types::{AgentOrigin, McpScope};

use crate::mcp::approval::{ApprovalPresenter, ApprovalRegistry, WindowBroker};
use crate::mcp::auth::{bearer_value, host_is_loopback, is_json_content_type, tokens_match};
use crate::state::AgentPolicy;

/// The longest call id this build will carry.
///
/// The id becomes a map key and an approval id. A bridge sends a UUID; anything far past that is a
/// caller mistake, and the fallback — a fresh id — keeps the call runnable instead of refusing it.
const MAX_CALL_ID_CHARS: usize = 128;

/// The calls in flight, by the id their caller gave them.
///
/// The bridge mints one id per `tools/call` and sends it in [`CALL_ID_HEADER`], so a later
/// `POST /tools/cancel` can name the same call without the desktop knowing anything about JSON-RPC
/// ids. An entry exists only for as long as the call runs: the handler inserts it before the tool is
/// entered and removes it however the call ends, which is what makes `{"cancelled": false}` mean
/// "that call is not running any more" rather than merely "an id I do not know".
///
/// A repeated id replaces the earlier entry. Two calls with one id is a bridge bug, and the later
/// call's handle is the one a cancel reaches; the earlier one is unreachable rather than wrongly
/// stopped.
#[derive(Debug, Default)]
pub struct CallRegistry {
    calls: Mutex<HashMap<String, Cancel>>,
}

impl CallRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one call and answers the handle it can be stopped through.
    fn begin(&self, call_id: &str) -> Cancel {
        let cancel = Cancel::new();
        self.lock().insert(call_id.to_owned(), cancel.clone());
        cancel
    }

    /// The call is over: nothing may stop it, and a later cancel answers `false`.
    fn finish(&self, call_id: &str) {
        self.lock().remove(call_id);
    }

    /// Stops one call. `false` when it is not running any more — already answered, or never began.
    ///
    /// The entry stays until the handler removes it, so a second cancel for a call that is still
    /// unwinding is honestly answered "yes" rather than pretending the id is unknown.
    pub fn cancel(&self, call_id: &str) -> bool {
        match self.lock().get(call_id) {
            Some(cancel) => {
                cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// Stops every call that is running, and answers how many there were.
    pub fn cancel_all(&self) -> usize {
        let guard = self.lock();
        for cancel in guard.values() {
            cancel.cancel();
        }
        guard.len()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Cancel>> {
        match self.calls.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// What the server keeps between requests: the token every call must present, one way to run it,
/// the table `GET /tools` reads, and the calls a cancel can reach.
pub struct ServerState {
    token: Arc<str>,
    handler: Arc<dyn CallHandler>,
    registry: Arc<ToolRegistry>,
    calls: Arc<CallRegistry>,
}

impl ServerState {
    pub fn new(token: impl Into<String>, handler: Arc<dyn CallHandler>) -> Self {
        Self {
            token: Arc::from(token.into()),
            handler,
            // A state without a table still answers `GET /tools` with an empty list rather than
            // with nothing: the route exists for every listener, and a test that does not care
            // about the list should not have to build one.
            registry: Arc::new(ToolRegistry::new()),
            calls: Arc::new(CallRegistry::new()),
        }
    }

    /// The one table an agent sees, which the listener's registry already holds.
    pub fn with_registry(mut self, registry: Arc<ToolRegistry>) -> Self {
        self.registry = registry;
        self
    }

    /// The registry the handler inserts its calls into. It must be the same one for
    /// `POST /tools/cancel` to reach a running call.
    pub fn with_calls(mut self, calls: Arc<CallRegistry>) -> Self {
        self.calls = calls;
        self
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
    /// The calls this handler is running, by the id their caller gave them. Shared with the server,
    /// which is the side `POST /tools/cancel` arrives on.
    calls: Arc<CallRegistry>,
    presenter: Arc<dyn ApprovalPresenter>,
    audit: Arc<dyn AuditSink>,
    limits: CallLimits,
    /// What an agent may see and do right now.
    ///
    /// A handle rather than a value: the settings page can hide a session or narrow one session's
    /// scope, and the next call must see it without the listener being restarted. Read on every call
    /// so a change reaches a listener that has been up for hours.
    policy: AgentPolicy,
}

impl RegistryHandler {
    /// Assembled once, from the pieces the shell already holds. They stay arguments rather than
    /// becoming a bundle: this list is the handler's whole world, and a wrapper struct would only
    /// rename it.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        core: Arc<Core>,
        registry: Arc<ToolRegistry>,
        connections: Arc<ConnectionCache>,
        approvals: Arc<ApprovalRegistry>,
        calls: Arc<CallRegistry>,
        presenter: Arc<dyn ApprovalPresenter>,
        audit: Arc<dyn AuditSink>,
        limits: CallLimits,
        policy: AgentPolicy,
    ) -> Self {
        Self {
            core,
            registry,
            connections,
            approvals,
            calls,
            presenter,
            audit,
            limits,
            policy,
        }
    }
}

#[async_trait]
impl CallHandler for RegistryHandler {
    async fn call(&self, call_id: &str, call: ToolCall) -> ToolOutcome {
        // Registered before the broker and the context exist: a cancel that arrives while the call
        // is waiting for a person has to find something to cancel, and the id is the only thing the
        // caller and this handler share.
        let cancel = self.calls.begin(call_id);

        let broker = WindowBroker::new(self.approvals.clone(), self.presenter.clone(), call_id.to_owned());
        let context = ToolContext::new(self.core.clone(), self.limits.scope)
            .with_cache(self.connections.clone())
            .with_approvals(Arc::new(broker))
            .with_audit(self.audit.clone())
            .with_approval_timeout_seconds(self.limits.approval_timeout_seconds)
            .with_cancel(cancel)
            // Who asked: a call through this endpoint is an MCP client's, and its approvals and its
            // audit line have to say so — the assistant uses the same registry and the same truth.
            .with_origin(AgentOrigin::Mcp)
            // The resolved snapshot, read now rather than captured when the listener started. It
            // replaces the scope handed to `new` above, since it already knows the global default.
            .with_access(self.policy.get());

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

        // However it ended — answered, refused, failed, cancelled — the id is free again and a
        // cancel for it answers `false`.
        self.calls.finish(call_id);

        outcome
    }
}

/// The service, ready to be handed to `axum::serve`.
pub fn router(state: Arc<ServerState>) -> Router {
    Router::new()
        .route(TOOL_CALL_PATH, post(handle_tool_call))
        .route(TOOLS_PATH, get(handle_tools))
        .route(TOOL_CANCEL_PATH, post(handle_tool_cancel))
        .route("/health", get(handle_health))
        .with_state(state)
}

async fn handle_tool_call(State(state): State<Arc<ServerState>>, headers: HeaderMap, body: Body) -> Response {
    if let Some(refusal) = refusal(&state, &headers) {
        return refusal;
    }

    let call: ToolCall = match read_json(&headers, body).await {
        Ok(call) => call,
        Err(refusal) => return *refusal,
    };

    let call_id = call_id_from(&headers);
    Json(state.handler.call(&call_id, call).await).into_response()
}

async fn handle_tools(State(state): State<Arc<ServerState>>, headers: HeaderMap) -> Response {
    if let Some(refusal) = refusal(&state, &headers) {
        return refusal;
    }
    // The desktop's own table: the built-in tools and every external server that has answered a
    // handshake. A bridge that asks twice gets the second answer, which is how a server that came
    // up while the client was connected becomes visible without a restart.
    Json(state.registry.descriptors()).into_response()
}

async fn handle_tool_cancel(State(state): State<Arc<ServerState>>, headers: HeaderMap, body: Body) -> Response {
    if let Some(refusal) = refusal(&state, &headers) {
        return refusal;
    }

    let request: CancelRequest = match read_json(&headers, body).await {
        Ok(request) => request,
        Err(refusal) => return *refusal,
    };

    let cancelled = state.calls.cancel(&request.call_id);
    Json(serde_json::json!({ "cancelled": cancelled })).into_response()
}

/// What a cancel names. Camel case, like every other body this endpoint takes.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelRequest {
    call_id: String,
}

async fn handle_health(State(_state): State<Arc<ServerState>>, headers: HeaderMap) -> Response {
    if let Some(refusal) = boundary_refusal(&headers) {
        return refusal;
    }
    Json(serde_json::json!({ "status": "ok" })).into_response()
}

/// The two checks every route but `GET /health` makes, in the order the module doc describes.
///
/// `GET /health` answers without a token because it is how somebody tells a live edge from a port
/// somebody else took; nothing it says comes from a tool or from the vault.
fn refusal(state: &ServerState, headers: &HeaderMap) -> Option<Response> {
    if let Some(refusal) = boundary_refusal(headers) {
        return Some(refusal);
    }
    if !authorized(headers, &state.token) {
        let mut response = StatusCode::UNAUTHORIZED.into_response();
        response
            .headers_mut()
            .insert(WWW_AUTHENTICATE, "Bearer".parse().expect("a valid header value"));
        return Some(response);
    }
    None
}

/// The content type, the size, and the body — read once, with the same bound the header check used.
///
/// The limit is passed to the reader as well as checked beforehand: a chunked body states no length,
/// and reading it without a bound is how a two-mebibyte rule becomes advisory.
///
/// The refusal is boxed because a response is much larger than the body being parsed: every
/// successful read would otherwise carry the room for an error it did not have.
async fn read_json<T: DeserializeOwned>(headers: &HeaderMap, body: Body) -> Result<T, Box<Response>> {
    if !headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(is_json_content_type)
    {
        return Err(Box::new(StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response()));
    }
    if content_length(headers).is_some_and(|length| length > MAX_BODY_BYTES) {
        return Err(Box::new(StatusCode::PAYLOAD_TOO_LARGE.into_response()));
    }

    let bytes = to_bytes(body, MAX_BODY_BYTES)
        .await
        .map_err(|_| Box::new(StatusCode::PAYLOAD_TOO_LARGE.into_response()))?;

    serde_json::from_slice(&bytes).map_err(|error| {
        log::debug!("a control endpoint body was not what its route takes: {error}");
        Box::new(StatusCode::BAD_REQUEST.into_response())
    })
}

/// The id the bridge minted for this call, or a fresh one when it did not send one.
///
/// A bridge from before the header existed sends none, and it still gets a working call: the id only
/// matters to a cancel, and a call nobody can name is one nobody can stop — which is what every call
/// was before this route existed.
fn call_id_from(headers: &HeaderMap) -> String {
    headers
        .get(CALL_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|id| !id.is_empty() && id.chars().count() <= MAX_CALL_ID_CHARS)
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
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
        AuditRecord, McpAccess, NoAuditSink, Tool, ToolAccess, ToolDescriptor, ToolError, ToolInputSchema, ToolResult,
        local,
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
        registry_handler_with(registry, Arc::new(CallRegistry::new()))
    }

    fn registry_handler_with(registry: ToolRegistry, calls: Arc<CallRegistry>) -> Arc<dyn CallHandler> {
        registry_handler_with_policy(registry, calls, AgentPolicy::default())
    }

    fn registry_handler_with_policy(
        registry: ToolRegistry,
        calls: Arc<CallRegistry>,
        policy: AgentPolicy,
    ) -> Arc<dyn CallHandler> {
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
            calls,
            Arc::new(silent_presenter()),
            Arc::new(NoAuditSink),
            CallLimits {
                scope: McpScope::ReadOnly,
                approval_timeout_seconds: 120,
                max_result_bytes: sevenshell_api::DEFAULT_MAX_RESULT_BYTES,
            },
            policy,
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

    /// A tool that changes something, so the scope gate is what decides whether it runs.
    struct WriteTool;

    #[async_trait]
    impl Tool for WriteTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor {
                name: "test_write".to_owned(),
                title: "Write".to_owned(),
                description: "Changes one thing.".to_owned(),
                input_schema: ToolInputSchema::object(json!({}), &[]),
                access: ToolAccess::Write,
            }
        }

        async fn call(&self, _context: &ToolContext, _arguments: Value) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::new(json!({ "wrote": true })))
        }
    }

    #[tokio::test]
    async fn a_narrowed_policy_stops_the_next_call_without_restarting_the_listener() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(WriteTool)).expect("a unique name");
        // The listener started while the installation allowed writes.
        let policy = AgentPolicy::new(McpAccess::open(McpScope::SafeWrite));
        let state = state_with(registry_handler_with_policy(
            registry,
            Arc::new(CallRegistry::new()),
            policy.clone(),
        ));

        let (status, body) = send(Arc::clone(&state), authenticated(r#"{"name":"test_write"}"#)).await;
        assert_eq!(status, StatusCode::OK);
        let outcome: ToolOutcome = serde_json::from_slice(&body).expect("a tool outcome");
        assert!(
            outcome.is_succeeded(),
            "the scope the listener started with allowed the write"
        );

        // Somebody narrowed the global scope; the listener was not restarted.
        policy.set(McpAccess::open(McpScope::ReadOnly));

        let (status, body) = send(state, authenticated(r#"{"name":"test_write"}"#)).await;
        assert_eq!(status, StatusCode::OK);
        let outcome: ToolOutcome = serde_json::from_slice(&body).expect("a tool outcome");
        assert_eq!(
            outcome.error().expect("the write is refused now").code(),
            "POLICY-DENIED",
            "the handler reads the snapshot on every call rather than at start"
        );
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
            Arc::new(CallRegistry::new()),
            Arc::new(SilentPresenter),
            sink.clone(),
            CallLimits {
                scope: McpScope::ReadOnly,
                approval_timeout_seconds: 120,
                max_result_bytes: sevenshell_api::DEFAULT_MAX_RESULT_BYTES,
            },
            AgentPolicy::default(),
        ));

        let (status, _) = send(state_with(handler), authenticated(r#"{"name":"test_echo"}"#)).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(sink.0.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    // -----------------------------------------------------------------------------------------
    // The tool list, the call id and cancellation
    // -----------------------------------------------------------------------------------------

    #[tokio::test]
    async fn the_tool_list_is_refused_without_the_token_and_answered_with_it() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool)).expect("a unique name");
        let state =
            Arc::new(ServerState::new(TOKEN, Arc::new(FakeHandler::default())).with_registry(Arc::new(registry)));

        let (status, _) = send(
            Arc::clone(&state),
            a_request("GET", TOOLS_PATH, &[(HOST.as_str(), "127.0.0.1:4711")], Body::empty()),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "the list names every tool an agent may call, so it needs the token like a call does"
        );

        let (status, _) = send(
            Arc::clone(&state),
            a_request(
                "GET",
                TOOLS_PATH,
                &[
                    (HOST.as_str(), "127.0.0.1:4711"),
                    (ORIGIN.as_str(), "http://evil.example"),
                    (AUTHORIZATION.as_str(), "Bearer the-token-of-this-run"),
                ],
                Body::empty(),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, body) = send(
            state,
            a_request(
                "GET",
                TOOLS_PATH,
                &[
                    (HOST.as_str(), "127.0.0.1:4711"),
                    (AUTHORIZATION.as_str(), "Bearer the-token-of-this-run"),
                ],
                Body::empty(),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let listed: Vec<Value> = serde_json::from_slice(&body).expect("a tool list");
        let names: Vec<&str> = listed
            .iter()
            .filter_map(|descriptor| descriptor["name"].as_str())
            .collect();
        assert_eq!(names, vec!["test_echo"]);
        assert!(
            listed[0].get("inputSchema").is_some(),
            "a client needs the schema to fill the arguments in"
        );
    }

    #[tokio::test]
    async fn the_call_id_the_bridge_sent_is_the_one_the_handler_sees() {
        let handler = Arc::new(FakeHandler::default());
        let state = state_with(handler.clone());

        let (status, _) = send(
            state,
            a_request(
                "POST",
                TOOL_CALL_PATH,
                &[
                    (HOST.as_str(), "127.0.0.1:4711"),
                    (AUTHORIZATION.as_str(), "Bearer the-token-of-this-run"),
                    (CONTENT_TYPE.as_str(), "application/json"),
                    (CALL_ID_HEADER, "call-42"),
                ],
                r#"{"name":"test_echo"}"#,
            ),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let calls = handler.calls.lock().expect("not poisoned");
        assert_eq!(
            calls[0].0, "call-42",
            "the id the bridge minted is the one a cancel names"
        );
    }

    #[tokio::test]
    async fn a_call_without_the_header_still_gets_an_id_of_its_own() {
        let handler = Arc::new(FakeHandler::default());
        let state = state_with(handler.clone());

        let (status, _) = send(state, authenticated(r#"{"name":"test_echo"}"#)).await;

        assert_eq!(status, StatusCode::OK);
        let calls = handler.calls.lock().expect("not poisoned");
        assert_eq!(
            calls[0].0.len(),
            36,
            "a fresh UUID, so two calls cannot answer to one id"
        );
    }

    #[tokio::test]
    async fn a_cancel_for_a_call_that_is_not_running_answers_false() {
        let state = state_with(Arc::new(FakeHandler::default()));

        let (status, body) = send(Arc::clone(&state), cancel_request("nobody")).await;
        assert_eq!(status, StatusCode::OK);
        let answer: Value = serde_json::from_slice(&body).expect("a cancel answer");
        assert_eq!(
            answer["cancelled"],
            json!(false),
            "a call that already answered is not stoppable"
        );

        let (status, _) = send(
            state,
            a_request(
                "POST",
                TOOL_CANCEL_PATH,
                &[
                    (HOST.as_str(), "127.0.0.1:4711"),
                    (CONTENT_TYPE.as_str(), "application/json"),
                ],
                r#"{"callId":"nobody"}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "stopping a call needs the token too");
    }

    #[tokio::test]
    async fn a_cancelled_call_ends_with_the_cancelled_code() {
        let calls = Arc::new(CallRegistry::new());
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(SlowTool)).expect("a unique name");
        let state = Arc::new(
            ServerState::new(TOKEN, registry_handler_with(registry, Arc::clone(&calls))).with_calls(Arc::clone(&calls)),
        );

        let running = tokio::spawn({
            let state = Arc::clone(&state);
            async move {
                send(
                    state,
                    a_request(
                        "POST",
                        TOOL_CALL_PATH,
                        &[
                            (HOST.as_str(), "127.0.0.1:4711"),
                            (AUTHORIZATION.as_str(), "Bearer the-token-of-this-run"),
                            (CONTENT_TYPE.as_str(), "application/json"),
                            (CALL_ID_HEADER, "call-to-stop"),
                        ],
                        r#"{"name":"test_slow"}"#,
                    ),
                )
                .await
            }
        });

        // The handler registers the call before it enters the tool; wait for that rather than for a
        // guessed number of milliseconds.
        for _ in 0..500 {
            if !calls.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(calls.len(), 1, "the call registered itself before it ran");

        let (status, body) = send(state, cancel_request("call-to-stop")).await;
        assert_eq!(status, StatusCode::OK);
        let answer: Value = serde_json::from_slice(&body).expect("a cancel answer");
        assert_eq!(answer["cancelled"], json!(true));

        let (status, body) = running.await.expect("the request task");
        assert_eq!(status, StatusCode::OK);
        let outcome: ToolOutcome = serde_json::from_slice(&body).expect("a tool outcome");
        assert_eq!(
            outcome.error().expect("the call was stopped").code(),
            "MCP-CANCELLED",
            "a stopped call is an outcome the bridge can render, not a broken connection"
        );
        assert!(calls.is_empty(), "the id is free once the call is over");
    }

    #[test]
    fn cancelling_everything_answers_how_many_calls_it_stopped() {
        let calls = CallRegistry::new();
        let first = calls.begin("one");
        let _second = calls.begin("two");
        assert_eq!(calls.len(), 2);

        assert_eq!(calls.cancel_all(), 2);
        assert!(first.is_cancelled(), "the handle a caller kept sees the cancellation");
        assert!(calls.cancel("one"), "a call that is still unwinding is still stoppable");

        calls.finish("one");
        calls.finish("two");
        assert!(!calls.cancel("one"), "a finished call cannot be cancelled");
        assert!(calls.is_empty());
    }

    /// A tool that waits until the call is cancelled, like a batch over a slow link.
    struct SlowTool;

    #[async_trait]
    impl Tool for SlowTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor {
                name: "test_slow".to_owned(),
                title: "Slow".to_owned(),
                description: "Waits to be cancelled.".to_owned(),
                input_schema: ToolInputSchema::object(json!({}), &[]),
                access: ToolAccess::Read,
            }
        }

        async fn call(&self, context: &ToolContext, _arguments: Value) -> Result<ToolResult, ToolError> {
            context
                .cancel()
                .guard(async {
                    // Long enough that a test which never cancels fails instead of hanging.
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    Ok(ToolResult::new(json!({ "late": true })))
                })
                .await
        }
    }

    fn cancel_request(call_id: &str) -> Request<Body> {
        a_request(
            "POST",
            TOOL_CANCEL_PATH,
            &[
                (HOST.as_str(), "127.0.0.1:4711"),
                (AUTHORIZATION.as_str(), "Bearer the-token-of-this-run"),
                (CONTENT_TYPE.as_str(), "application/json"),
            ],
            serde_json::json!({ "callId": call_id }).to_string(),
        )
    }
}
