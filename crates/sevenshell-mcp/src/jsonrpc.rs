//! JSON-RPC 2.0, as much of it as an MCP client uses.
//!
//! One shape per direction, and no negotiation about it: a request is a method, an optional id and
//! optional parameters, and an answer is either a result or an error under the same id. The code
//! numbers are the ones the protocol reserves plus two of this bridge's own, and both are named here
//! so a test never asserts against a bare number.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The newest protocol revision this bridge speaks.
pub const LATEST_PROTOCOL_VERSION: &str = "2025-06-18";

/// Every revision this bridge can answer in, newest first.
///
/// Two, because they are the two whose tools surface is the one implemented here. A client that asks
/// for anything else is answered with [`LATEST_PROTOCOL_VERSION`] and decides for itself whether it
/// can go on — which is what the protocol asks a server to do, rather than refusing the handshake.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[LATEST_PROTOCOL_VERSION, "2024-11-05"];

/// The line was not JSON at all.
pub const PARSE_ERROR: i64 = -32700;
/// The line was JSON but not a request this protocol defines.
pub const INVALID_REQUEST: i64 = -32600;
/// The method does not exist.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// The method exists and the parameters do not fit it.
pub const INVALID_PARAMS: i64 = -32602;
/// A failure in the bridge itself.
pub const INTERNAL_ERROR: i64 = -32603;
/// The tool ran (or refused) and the answer is a failure. The `data` carries the
/// [`sevenshell_api::ToolError`], so an agent gets the stable code rather than a sentence.
pub const TOOL_FAILED: i64 = -32001;
/// The desktop is not running, is still locked, or its endpoint refused the request. Its own code
/// because the fix is a person's ("start 7Shell") rather than the agent's ("try another tool").
pub const DESKTOP_UNAVAILABLE: i64 = -32002;

/// A JSON-RPC id: a number or a string, and nothing else.
///
/// Kept typed rather than a raw `Value` so a response cannot be built with an id that was never a
/// request's — a float or an object is an invalid request, not an id to echo.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Id {
    Number(i64),
    String(String),
}

impl fmt::Display for Id {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(number) => write!(formatter, "{number}"),
            Self::String(value) => formatter.write_str(value),
        }
    }
}

/// One request, as it arrives on a line.
///
/// Unknown members are ignored, which JSON-RPC allows: a client that adds a field this bridge has
/// not learned about is still a client whose request should run.
#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    /// Present and equal to `"2.0"` in a valid request. Optional here so a missing one is reported as
    /// an invalid request rather than as unparsable JSON.
    #[serde(default)]
    pub jsonrpc: Option<String>,
    pub method: String,
    /// Absent on a notification, which is a message with no answer.
    #[serde(default)]
    pub id: Option<Id>,
    #[serde(default)]
    pub params: Option<Value>,
}

/// What went wrong, in the shape the protocol defines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    pub fn parse_error(detail: &str) -> Self {
        Self::new(PARSE_ERROR, format!("the line is not JSON: {detail}"))
    }

    pub fn invalid_request(detail: &str) -> Self {
        Self::new(INVALID_REQUEST, format!("not a JSON-RPC request: {detail}"))
    }

    pub fn method_not_found(method: &str) -> Self {
        Self::new(METHOD_NOT_FOUND, format!("no method is named `{method}`"))
    }

    pub fn invalid_params(detail: impl Into<String>) -> Self {
        Self::new(INVALID_PARAMS, detail)
    }

    pub fn internal(detail: impl Into<String>) -> Self {
        Self::new(INTERNAL_ERROR, detail)
    }

    /// The desktop could not be reached, or refused the token.
    ///
    /// The message says what to do and never where the file is: the endpoint's path is this
    /// machine's configuration layout, the bridge logs it to stderr, and an answer travelling into
    /// an agent's context does not need it.
    pub fn desktop_unavailable(detail: impl Into<String>) -> Self {
        Self::new(
            DESKTOP_UNAVAILABLE,
            format!("the 7Shell desktop is not available: {}", detail.into()),
        )
    }
}

impl From<sevenshell_api::ToolError> for RpcError {
    fn from(error: sevenshell_api::ToolError) -> Self {
        // The sentence is the tool's own when it has one; otherwise the stable code is the most
        // useful thing left, and it is what an agent can key behaviour off.
        let message = error
            .message()
            .map(str::to_owned)
            .unwrap_or_else(|| error.code().to_owned());
        match serde_json::to_value(&error) {
            Ok(data) => Self::new(TOOL_FAILED, message).with_data(data),
            Err(_) => Self::new(TOOL_FAILED, message),
        }
    }
}

/// One answer: a result or an error, under the id of the request it answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Result { id: Option<Id>, result: Value },
    Error { id: Option<Id>, error: RpcError },
}

impl Response {
    pub fn result(id: Option<Id>, result: Value) -> Self {
        Self::Result { id, result }
    }

    pub fn error(id: Option<Id>, error: RpcError) -> Self {
        Self::Error { id, error }
    }

    /// The line to write. A `None` id is written as `null`, which is what the protocol asks for an
    /// answer to something that had no id to echo.
    pub fn to_value(&self) -> Value {
        match self {
            Self::Result { id, result } => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result,
            }),
            Self::Error { id, error } => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": error,
            }),
        }
    }
}
