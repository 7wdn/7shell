//! The bridge an MCP client starts.
//!
//! Two halves, one direction each. [`server`] speaks the protocol on stdin and stdout and knows
//! nothing about HTTP; [`client`] knows the desktop's loopback endpoint and nothing about the
//! protocol. A call crosses from one to the other and an answer comes back, and neither half can
//! reach a vault, a server or a file on its own. [`cli`] is where the one path that decides between
//! them comes from: an argument, an environment variable, or this installation's configuration
//! directory, in that order.
//!
//! The bridge never starts the desktop. If the endpoint file is missing, the application is not
//! running; if the vault is locked, the answer is the vault's own code. Both are reported and left
//! for a person to act on.

pub mod cli;
pub mod client;
pub mod jsonrpc;
pub mod server;

pub use cli::{Command, ENDPOINT_FILE_ENV, endpoint_file_override};
pub use client::{HttpToolClient, endpoint_path};
pub use jsonrpc::{
    DESKTOP_UNAVAILABLE, INVALID_PARAMS, INVALID_REQUEST, Id, LATEST_PROTOCOL_VERSION, METHOD_NOT_FOUND, PARSE_ERROR,
    RpcError, SUPPORTED_PROTOCOL_VERSIONS, TOOL_FAILED,
};
pub use server::{DEFAULT_MAX_CONCURRENCY, McpServer, ToolClient};
