//! The loopback control endpoint the stdio bridge talks to.
//!
//! It is deliberately the same three routes and the same order of checks as the desktop host: the
//! bridge is one program shared by both hosts, so the door it knocks on cannot have two shapes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, HOST, ORIGIN, WWW_AUTHENTICATE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use sevenshell_api::{CALL_ID_HEADER, Cancel, MAX_BODY_BYTES, TOOL_CALL_PATH, TOOL_CANCEL_PATH, TOOLS_PATH, ToolCall};
use sevenshell_types::AgentOrigin;

use super::auth::{bearer_value, host_is_loopback, is_json_content_type, tokens_match};
use crate::agent::ToolRuntime;

const MAX_CALL_ID_CHARS: usize = 128;

#[derive(Debug, Default)]
pub struct CallRegistry {
    calls: Mutex<HashMap<String, Cancel>>,
}

impl CallRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn begin(&self, call_id: &str) -> Cancel {
        let cancel = Cancel::new();
        self.lock().insert(call_id.to_owned(), cancel.clone());
        cancel
    }

    fn finish(&self, call_id: &str) {
        self.lock().remove(call_id);
    }

    pub fn cancel(&self, call_id: &str) -> bool {
        match self.lock().get(call_id) {
            Some(cancel) => {
                cancel.cancel();
                true
            }
            None => false,
        }
    }

    pub fn cancel_all(&self) -> usize {
        let guard = self.lock();
        for cancel in guard.values() {
            cancel.cancel();
        }
        guard.len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Cancel>> {
        match self.calls.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

pub struct ServerState {
    token: String,
    tools: Arc<ToolRuntime>,
    calls: Arc<CallRegistry>,
}

impl ServerState {
    pub fn new(token: impl Into<String>, tools: Arc<ToolRuntime>, calls: Arc<CallRegistry>) -> Self {
        Self {
            token: token.into(),
            tools,
            calls,
        }
    }
}

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
        Err(refusal) => return refusal,
    };
    let call_id = call_id_from(&headers);
    let cancel = state.calls.begin(&call_id);
    let context = state
        .tools
        .context_for_listener(AgentOrigin::Mcp, call_id.clone(), cancel);
    let outcome = state.tools.call(&context, call).await;
    state.calls.finish(&call_id);
    Json(outcome).into_response()
}

async fn handle_tools(State(state): State<Arc<ServerState>>, headers: HeaderMap) -> Response {
    if let Some(refusal) = refusal(&state, &headers) {
        return refusal;
    }
    Json(state.tools.descriptors()).into_response()
}

async fn handle_tool_cancel(State(state): State<Arc<ServerState>>, headers: HeaderMap, body: Body) -> Response {
    if let Some(refusal) = refusal(&state, &headers) {
        return refusal;
    }
    let request: CancelRequest = match read_json(&headers, body).await {
        Ok(request) => request,
        Err(refusal) => return refusal,
    };
    let cancelled = state.calls.cancel(&request.call_id);
    Json(serde_json::json!({ "cancelled": cancelled })).into_response()
}

async fn handle_health(State(_state): State<Arc<ServerState>>, headers: HeaderMap) -> Response {
    if let Some(refusal) = boundary_refusal(&headers) {
        return refusal;
    }
    Json(serde_json::json!({ "status": "ok" })).into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelRequest {
    call_id: String,
}

fn refusal(state: &ServerState, headers: &HeaderMap) -> Option<Response> {
    if let Some(refusal) = boundary_refusal(headers) {
        return Some(refusal);
    }
    let presented = bearer_value(headers.get(AUTHORIZATION).and_then(|value| value.to_str().ok()));
    let authorized = presented.is_some_and(|presented| tokens_match(&state.token, presented));
    if authorized {
        return None;
    }
    let mut response = StatusCode::UNAUTHORIZED.into_response();
    response
        .headers_mut()
        .insert(WWW_AUTHENTICATE, "Bearer".parse().expect("a valid header value"));
    Some(response)
}

fn boundary_refusal(headers: &HeaderMap) -> Option<Response> {
    if headers.contains_key(ORIGIN) {
        return Some(StatusCode::FORBIDDEN.into_response());
    }
    let host = headers.get(HOST).and_then(|value| value.to_str().ok()).unwrap_or("");
    if !host_is_loopback(host) {
        return Some(StatusCode::FORBIDDEN.into_response());
    }
    None
}

#[allow(clippy::result_large_err)]
async fn read_json<T: DeserializeOwned>(headers: &HeaderMap, body: Body) -> Result<T, Response> {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !is_json_content_type(content_type) {
        return Err(StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response());
    }
    if let Some(length) = headers.get(CONTENT_LENGTH).and_then(|value| value.to_str().ok())
        && length.parse::<usize>().is_ok_and(|length| length > MAX_BODY_BYTES)
    {
        return Err(StatusCode::PAYLOAD_TOO_LARGE.into_response());
    }
    let bytes = to_bytes(body, MAX_BODY_BYTES)
        .await
        .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE.into_response())?;
    serde_json::from_slice(&bytes).map_err(|_| StatusCode::BAD_REQUEST.into_response())
}

fn call_id_from(headers: &HeaderMap) -> String {
    headers
        .get(CALL_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|id| !id.is_empty() && id.len() <= MAX_CALL_ID_CHARS)
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}
