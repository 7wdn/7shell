//! The endpoint file: how a bridge in another process finds the desktop.
//!
//! The desktop listens on loopback and writes down four things in one file under its configuration
//! directory. The bridge reads that file, presents the token on every call, and never starts the
//! desktop itself: a bridge that could launch a second application would be a bridge that decides
//! when a vault is open.
//!
//! The file is the contract between the two halves and nothing else is:
//!
//! * `POST {TOOL_CALL_PATH}` with a [`crate::ToolCall`] as the body, `Authorization: Bearer <token>`,
//!   `Content-Type: application/json`, and a body no larger than [`MAX_BODY_BYTES`].
//! * `200` with a [`crate::ToolOutcome`]; `401` for a token that is not the one in the file; `413`
//!   for a body over the limit. Everything else is a desktop that is not answering properly.
//! * The desktop checks the `Host` header names its own loopback address and refuses a request that
//!   carries an `Origin` it did not expect, so a page in a browser cannot reach the endpoint even if
//!   it learned the port.
//!
//! The token is a secret, and this type is deliberately *not* one of the wire types in
//! `sevenshell-types`: nothing that crosses to the interface may hold a credential. It is read by
//! the bridge and written by the desktop, and it never travels further than the loopback request it
//! authorises.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The file's name inside the configuration directory. Fixed, because both halves look for it.
pub const ENDPOINT_FILE_NAME: &str = "mcp-endpoint.json";

/// Where a call is posted.
pub const TOOL_CALL_PATH: &str = "/tools/call";

/// Where the tool list is read from.
///
/// Added for the second wave: the bridge starts with the built-in list compiled in and asks the
/// desktop for the whole table once it is reachable, so external servers' tools reach a client
/// without the bridge having to know what an external server is. The desktop is authoritative; a
/// bridge that cannot reach it falls back to the compiled list rather than to a partial one.
pub const TOOLS_PATH: &str = "/tools";

/// Where a call in flight can be called off.
///
/// The body names the call by id; the caller's connection is not the thing being cancelled, because
/// a tool call may outlive the request that started it (an approval waits for a person).
pub const TOOL_CANCEL_PATH: &str = "/tools/cancel";

/// The header that carries a call's own id.
///
/// A header rather than a field of [`crate::ToolCall`]: that type is frozen and refuses unknown
/// fields, and adding one would make an older desktop reject every call from a newer bridge. A
/// header is ignored by anything that does not know it, which is the compatibility this needs.
pub const CALL_ID_HEADER: &str = "x-sshell-call-id";

/// The largest request body the endpoint accepts.
///
/// Two mebibytes. A tool argument is a command line, a path or a file's text: large enough for a
/// text file worth writing, small enough that a mistake is a refusal rather than a request that
/// swallows the application's memory.
pub const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

/// What the desktop writes down when it starts listening.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointFile {
    /// The bound port. Never zero in a written file: zero asks the system for one, and the answer is
    /// what gets recorded.
    pub port: u16,
    /// The bearer token for this run. Rotated on every start: a file left over from last time is a
    /// token that no longer opens anything.
    pub token: String,
    /// The process that wrote it, so a stale file can be told apart from a live one.
    pub pid: u32,
    /// Seconds since the Unix epoch, for the same reason.
    pub started_at: u64,
    /// The largest result the bridge will put on its stdout.
    ///
    /// The desktop writes the configured ceiling here; a bridge that started before the file existed
    /// uses the compiled default. Zero reads as the default rather than as "truncate everything".
    #[serde(default = "default_max_result_bytes")]
    pub max_result_bytes: u64,
}

fn default_max_result_bytes() -> u64 {
    crate::DEFAULT_MAX_RESULT_BYTES
}

impl EndpointFile {
    pub fn new(port: u16, token: impl Into<String>, pid: u32, started_at: u64) -> Self {
        Self {
            port,
            token: token.into(),
            pid,
            started_at,
            max_result_bytes: default_max_result_bytes(),
        }
    }

    /// The configured ceiling, which may be lower or higher than the compiled default.
    pub fn with_max_result_bytes(mut self, bytes: u64) -> Self {
        self.max_result_bytes = bytes;
        self
    }

    /// The base URL the endpoint answers on.
    ///
    /// Built from the port rather than stored, so a file cannot name another host: the listener is
    /// loopback, and a bridge that could be pointed elsewhere would be a client for something else.
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

/// Debug never prints the token. A file's contents end up in logs and issue reports, which is the
/// one place a bearer token must not be.
impl fmt::Debug for EndpointFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EndpointFile")
            .field("port", &self.port)
            .field("token", &"<redacted>")
            .field("pid", &self.pid)
            .field("started_at", &self.started_at)
            .field("max_result_bytes", &self.max_result_bytes)
            .finish()
    }
}
