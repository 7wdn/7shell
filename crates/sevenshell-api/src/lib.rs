//! The operations an agent may call, once.
//!
//! The desktop shell, a command line and an agent interface are three doors onto the same
//! operations. This crate is the one place a tool is *described and gated*, so that the three
//! doors cannot come to disagree about what a tool takes, what it may do, or what it leaves
//! behind in the audit trail.
//!
//! Three things are deliberately not here:
//!
//! * **The transport.** How a call reaches this process is the caller's business: the MCP bridge
//!   talks HTTP to a loopback endpoint the desktop writes down, and a future command line calls the
//!   registry directly. A tool is the same either way.
//! * **The approval window.** A tool asks an [`ApprovalBroker`], and what draws the question is the
//!   host's. A tool that could show a dialog would be a tool that cannot be called from anywhere
//!   else.
//! * **A second set of risk rules.** [`classify_command`] is the policy crate's own classifier, so
//!   an agent's command is judged by exactly the rules a person's command is.
//!
//! Dependencies point downwards only: this crate knows the core, the policy and the wire types, and
//! nothing here knows that a webview exists.

mod context;
mod endpoint;
mod error;
pub mod local;
mod registry;
mod tool;

pub use context::{
    ApprovalBroker, ApprovalDecision, ApprovalReason, ApprovalRecord, ApprovalRequest, AuditRecord, AuditSink,
    AuditStatus, Cancel, DenyingBroker, NoAuditSink, ToolContext,
};
pub use endpoint::{ENDPOINT_FILE_NAME, EndpointFile, MAX_BODY_BYTES, TOOL_CALL_PATH};
pub use error::ApiError;
pub use registry::{Tool, ToolRegistry, scope_allows};
pub use tool::{
    ToolAccess, ToolAudit, ToolCall, ToolDescriptor, ToolError, ToolInputSchema, ToolOutcome, ToolResult,
    parse_arguments,
};

/// Grades a command with the same rules the batch runner and the interface use.
///
/// Re-exported rather than reimplemented: an agent's command has to be judged by exactly the rules a
/// person's command is, and a second entry point is a second set of rules waiting to drift.
pub use sevenshell_policy::classify as classify_command;

/// The result limit a bridge enforces until the desktop tells it otherwise.
///
/// One authority for the number — the settings default — rather than a second literal here: a
/// bridge that truncated at a different size than the page configured would be a limit nobody can
/// predict.
pub const DEFAULT_MAX_RESULT_BYTES: u64 = sevenshell_types::MCP_MAX_RESULT_BYTES_DEFAULT;
