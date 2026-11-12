//! The client side of MCP: the external servers this installation may call tools on.
//!
//! This crate is the half of the MCP work that talks *out*: it opens a session with somebody
//! else's server, learns the tools that server offers, and adapts them to the same
//! [`sevenshell_api::Tool`] contract the built-in tools implement. The bridge in `sevenshell-mcp`
//! is the other half — it speaks MCP *in* to this application.
//!
//! Four pieces, in the order a call passes through them:
//!
//! * the transport layer — one message channel per transport (a child process's stdio, Streamable
//!   HTTP, or the older HTTP+SSE stream). It owns JSON-RPC ids and correlation and nothing else.
//! * [`McpSession`] — the handshake, the cached tool, resource and prompt lists, and one call under
//!   the server's own time budget.
//! * [`ExternalTool`] — one remote tool as this build's registry sees it: a name that says which
//!   server answers, the schema the server declared, and the approval every write goes through.
//! * [`McpClientManager`] — the set of configured servers, one session and state per server, and
//!   the [`sevenshell_api::ToolSource`] that joins them to the one tool table.
//!
//! Two rules hold throughout:
//!
//! * **No secret is held in a wire type.** A server's environment and headers live in the vault;
//!   [`McpServerSecrets`] is the in-memory form of that one vault value, and its `Debug` prints
//!   counts rather than values.
//! * **A tool name says which server answered.** `mcp__{server}__{tool}`, with the server id
//!   restricted so the name parses back into exactly two parts.
//!
//! A session also caches the server's resources and prompt templates, which are read rather than
//! called and so never become tools. They are the weaker half of a session: a server whose content
//! list cannot be read still offers its tools, because the tools are what a session is opened for.

mod adapter;
mod config;
mod manager;
mod session;
#[cfg(test)]
mod testing;
mod transport;

use std::fmt;

pub use adapter::{
    ExternalTool, MAX_DESCRIPTION_BYTES, MAX_INPUT_SCHEMA_BYTES, access_for_read_only_hint, descriptor_for,
};
pub use config::{
    McpServerSecrets, is_external_tool_name, is_valid_server_id, split_tool_name, tool_name, tool_timeout,
    validate_server,
};
pub use manager::McpClientManager;
pub use session::{
    ExternalCall, MAX_PROMPT_TEXT_CHARS, MAX_RESOURCE_TEXT_CHARS, McpSession, SUPPORTED_PROTOCOL_VERSIONS,
};
pub use transport::{
    CONNECT_TIMEOUT, DefaultTransportFactory, LIST_TIMEOUT, McpTransport, NotificationHandler, TransportFactory,
    connect,
};

/// What went wrong before, during, or instead of a call to an external server.
///
/// Every variant answers with a catalog code, so the interface can localize the sentence and the
/// host can build the same envelope it builds everywhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpClientError {
    /// This build does not speak that part yet.
    ///
    /// The sentence says what is missing so a log line is understandable; it never carries a
    /// configuration value.
    NotImplemented(&'static str),
    /// An id that cannot name a tool, or one no configured server has.
    InvalidServerId(String),
    /// Two configured servers answered to the same id.
    DuplicateServer { id: String },
    /// A server the caller named is not configured.
    UnknownServer { id: String },
    /// A server whose settings cannot be used at all: no command for stdio, no address for HTTP,
    /// an address that is not one. The detail names what is missing, never a configured value.
    InvalidConfig { server: String, detail: String },
    /// The server could not be reached, or refused the protocol handshake.
    ConnectFailed { server: String, detail: String },
    /// The server answered the request with an error.
    Failed { server: String, message: String },
    /// The server answered with a JSON-RPC error, whose code is kept because some of them mean
    /// "this optional method is not implemented" rather than "the session is broken".
    Rpc { server: String, code: i64, message: String },
    /// The call ran past the server's configured budget.
    Timeout { server: String, seconds: u64 },
    /// Two tools of one server produce the same name, so a call could reach either.
    DuplicateTool { server: String, tool: String },
    /// A tool listing this build cannot serve: a missing name, a schema that is not an object
    /// schema, or one larger than the limit.
    InvalidTool {
        server: String,
        tool: String,
        detail: String,
    },
    /// A person, or the window closing, called the call off.
    Cancelled,
}

impl McpClientError {
    /// The stable wire code, chosen from the catalog.
    pub fn code(&self) -> &'static str {
        use sevenshell_error::ErrorCode;
        match self {
            Self::NotImplemented(_) => ErrorCode::AppUnsupported.as_str(),
            Self::InvalidServerId(_) | Self::InvalidConfig { .. } => ErrorCode::McpInvalidArguments.as_str(),
            Self::DuplicateServer { .. } => ErrorCode::McpServerDuplicate.as_str(),
            Self::UnknownServer { .. } => ErrorCode::McpServerNotFound.as_str(),
            Self::ConnectFailed { .. } => ErrorCode::McpServerConnectFailed.as_str(),
            Self::Failed { .. } | Self::Rpc { .. } => ErrorCode::McpExternalFailed.as_str(),
            Self::Timeout { .. } => ErrorCode::McpExternalTimeout.as_str(),
            Self::DuplicateTool { .. } => ErrorCode::McpDuplicateTool.as_str(),
            Self::InvalidTool { .. } => ErrorCode::McpInvalidTool.as_str(),
            Self::Cancelled => ErrorCode::McpCancelled.as_str(),
        }
    }

    /// The catalog key for the sentence the interface draws.
    pub fn message_key(&self) -> &'static str {
        sevenshell_error::ErrorCode::from_code(self.code())
            .map(sevenshell_error::ErrorCode::message_key)
            .unwrap_or("errors.app.unsupported")
    }

    /// What the transport said, when it said anything.
    ///
    /// Kept out of `Display` on purpose: a sentence that travels into a log or an event should name
    /// the server and the code, and whoever needs the underlying refusal can ask for it here after
    /// it has been scrubbed.
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::InvalidConfig { detail, .. }
            | Self::ConnectFailed { detail, .. }
            | Self::InvalidTool { detail, .. } => Some(detail),
            Self::Failed { message, .. } | Self::Rpc { message, .. } => Some(message),
            _ => None,
        }
    }
}

impl fmt::Display for McpClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.code())?;
        match self {
            Self::NotImplemented(what) => write!(formatter, ": {what} is not implemented in this build yet"),
            Self::InvalidServerId(id) => write!(formatter, ": `{id}` is not a server id this build can use"),
            Self::DuplicateServer { id } => write!(formatter, ": two servers use the id `{id}`"),
            Self::UnknownServer { id } => write!(formatter, ": no server is configured with the id `{id}`"),
            Self::InvalidConfig { server, .. } => {
                write!(formatter, ": the server `{server}` is configured incompletely")
            }
            Self::ConnectFailed { server, .. } => write!(formatter, ": the server `{server}` could not be reached"),
            Self::Failed { server, .. } => write!(formatter, ": the server `{server}` answered with an error"),
            Self::Rpc { server, code, .. } => {
                write!(formatter, ": the server `{server}` answered with error {code}")
            }
            Self::Timeout { server, seconds } => {
                write!(
                    formatter,
                    ": the server `{server}` did not answer within {seconds} seconds"
                )
            }
            Self::DuplicateTool { server, tool } => {
                write!(formatter, ": the server `{server}` offers two tools named `{tool}`")
            }
            Self::InvalidTool { server, tool, .. } => {
                write!(
                    formatter,
                    ": the server `{server}` offers a tool this build cannot serve (`{tool}`)"
                )
            }
            Self::Cancelled => formatter.write_str(": the call was cancelled"),
        }
    }
}

impl std::error::Error for McpClientError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_names_a_code_and_says_which_server_without_any_configuration_value() {
        let failed = McpClientError::Failed {
            server: "files".to_owned(),
            message: "tools/call was refused".to_owned(),
        };
        assert_eq!(failed.code(), "MCP-EXTERNAL-FAILED");
        assert_eq!(failed.message_key(), "errors.mcp.externalFailed");
        assert!(failed.to_string().contains("files"));

        let timeout = McpClientError::Timeout {
            server: "files".to_owned(),
            seconds: 60,
        };
        assert_eq!(timeout.code(), "MCP-EXTERNAL-TIMEOUT");
        assert_eq!(timeout.message_key(), "errors.mcp.externalTimeout");

        assert_eq!(
            McpClientError::NotImplemented("the stdio transport").code(),
            "APP-UNSUPPORTED"
        );
    }

    #[test]
    fn the_detail_stays_out_of_the_sentence() {
        let error = McpClientError::ConnectFailed {
            server: "files".to_owned(),
            detail: "connection refused at 127.0.0.1:9000".to_owned(),
        };
        assert_eq!(error.detail(), Some("connection refused at 127.0.0.1:9000"));
        assert!(
            !error.to_string().contains("127.0.0.1"),
            "a sentence that names a socket is one that ends up in an event: {error}"
        );
    }

    #[test]
    fn a_listing_this_build_cannot_serve_has_its_own_code_and_detail() {
        let duplicate = McpClientError::DuplicateTool {
            server: "files".to_owned(),
            tool: "read".to_owned(),
        };
        assert_eq!(duplicate.code(), "MCP-DUPLICATE-TOOL");
        assert!(duplicate.to_string().contains("read"));

        let invalid = McpClientError::InvalidTool {
            server: "files".to_owned(),
            tool: "read".to_owned(),
            detail: "the input schema is not an object schema".to_owned(),
        };
        assert_eq!(invalid.code(), "MCP-INVALID-TOOL");
        assert_eq!(invalid.detail(), Some("the input schema is not an object schema"));

        let config = McpClientError::InvalidConfig {
            server: "files".to_owned(),
            detail: "a stdio server needs a command to start".to_owned(),
        };
        assert_eq!(config.code(), "MCP-INVALID-ARGUMENTS");
        assert_eq!(config.message_key(), "errors.mcp.invalidArguments");
        assert_eq!(config.detail(), Some("a stdio server needs a command to start"));

        let missing = McpClientError::UnknownServer { id: "files".to_owned() };
        assert_eq!(missing.code(), "MCP-SERVER-NOT-FOUND");
        assert_eq!(missing.message_key(), "errors.mcp.serverNotFound");

        assert_eq!(duplicate.message_key(), "errors.mcp.duplicateTool");
        assert_eq!(invalid.message_key(), "errors.mcp.invalidTool");
        assert_eq!(McpClientError::Cancelled.message_key(), "errors.mcp.cancelled");
    }
}
