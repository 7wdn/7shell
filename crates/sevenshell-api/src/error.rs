//! How a failure becomes something an agent can read.
//!
//! Two directions meet here. A failure from below is a [`ShellError`]: the envelope the whole
//! application reports through, with a stable code and a message key. A failure above is a
//! [`ToolError`]: the same facts plus a sentence an agent can act on, because an MCP client has no
//! locale and no catalog to look a key up in.
//!
//! Nothing is reinterpreted on the way across: the code, the key, its parameters and the scrubbed
//! detail are the envelope's own values, and a test proves no field is dropped.

use std::collections::BTreeMap;

use sevenshell_error::{MAX_DETAIL_CHARS, ShellError, scrub_bounded};
use thiserror::Error;

use crate::tool::{ToolAudit, ToolError, ToolErrorFields};

/// A failure that belongs to this layer rather than to a subsystem below it.
///
/// Its own type so the constructor of a tool reads as what went wrong, and the wire code is chosen
/// in one place.
#[derive(Debug, Error)]
pub enum ApiError {
    #[error("no tool is named `{name}`")]
    UnknownTool { name: String },
    #[error("`{name}` is declared but not implemented in this build")]
    NotImplemented { name: String },
    #[error("the arguments do not match the tool's input: {detail}")]
    InvalidArguments { detail: String },
    #[error("the call was called off before it finished")]
    Cancelled,
    #[error("a tool named `{name}` is already registered")]
    DuplicateTool { name: String },
    #[error("a tool must have a name")]
    EmptyToolName,
}

impl ApiError {
    /// The wire code this failure travels under. Stable: an agent may key behaviour off it.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnknownTool { .. } => "MCP-UNKNOWN-TOOL",
            Self::NotImplemented { .. } => "MCP-NOT-IMPLEMENTED",
            Self::InvalidArguments { .. } => "MCP-INVALID-ARGUMENTS",
            Self::Cancelled => "MCP-CANCELLED",
            Self::DuplicateTool { .. } => "MCP-DUPLICATE-TOOL",
            Self::EmptyToolName => "MCP-INVALID-TOOL",
        }
    }
}

impl From<ApiError> for ToolError {
    fn from(error: ApiError) -> Self {
        Self::from_fields(ToolErrorFields {
            code: error.code().to_owned(),
            message: Some(scrub_bounded(&error.to_string(), MAX_DETAIL_CHARS)),
            message_key: None,
            message_params: BTreeMap::new(),
            detail: None,
            audit: None,
        })
    }
}

impl From<ShellError> for ToolError {
    fn from(error: ShellError) -> Self {
        Self::from_shell(error)
    }
}

impl ToolError {
    /// An error with a code and nothing else. Prefer a named constructor.
    pub fn new(code: impl Into<String>) -> Self {
        Self::from_fields(ToolErrorFields {
            code: code.into(),
            message: None,
            message_key: None,
            message_params: BTreeMap::new(),
            detail: None,
            audit: None,
        })
    }

    pub fn unknown_tool(name: &str) -> Self {
        ApiError::UnknownTool { name: name.to_owned() }.into()
    }

    pub fn not_implemented(name: &str) -> Self {
        ApiError::NotImplemented { name: name.to_owned() }.into()
    }

    pub fn invalid_arguments(detail: impl Into<String>) -> Self {
        ApiError::InvalidArguments { detail: detail.into() }.into()
    }

    pub fn cancelled() -> Self {
        ApiError::Cancelled.into()
    }

    /// A tool the settings page switched off. It is not offered, and a stale agent that calls it
    /// anyway gets this rather than a "no such tool" that suggests a broken build.
    pub fn tool_disabled(name: &str) -> Self {
        Self::from_fields(ToolErrorFields {
            code: "MCP-TOOL-DISABLED".to_owned(),
            message: Some(format!("the tool `{name}` is switched off in Agent access")),
            message_key: Some("errors.mcp.toolDisabled".to_owned()),
            message_params: BTreeMap::from([("tool".to_owned(), name.to_owned())]),
            detail: None,
            audit: None,
        })
    }

    /// A saved session the access policy hides. Its existence is not advertised in listings, but a
    /// caller that names it is told why the call did not reach it.
    pub fn session_hidden(id: &str) -> Self {
        Self::from_fields(ToolErrorFields {
            code: "MCP-SESSION-HIDDEN".to_owned(),
            message: Some(format!(
                "the session `{id}` is not visible under the current agent access rules"
            )),
            message_key: Some("errors.mcp.sessionHidden".to_owned()),
            message_params: BTreeMap::from([("session".to_owned(), id.to_owned())]),
            detail: None,
            audit: None,
        })
    }

    /// A tool the configured scope does not offer. The message key is the catalog's, so the desktop
    /// can show the same sentence it shows for any other refused action.
    pub fn denied(action: &str) -> Self {
        ShellError::policy_denied(action).into()
    }

    /// A destructive action that nobody has agreed to yet.
    pub fn approval_required(action: &str) -> Self {
        ShellError::policy_approval_required(action).into()
    }

    /// Carries the envelope's own facts across, unchanged.
    pub fn from_shell(error: ShellError) -> Self {
        Self::from_fields(ToolErrorFields {
            code: error.code().as_str().to_owned(),
            // The envelope's own rendering: the code, the catalog key and the scrubbed detail. A
            // sentence would have to be localized, and a bridge has no locale to localize with.
            message: Some(error.to_string()),
            message_key: Some(error.message_key().to_owned()),
            message_params: error.message_params().clone(),
            detail: error.detail().map(str::to_owned),
            audit: None,
        })
    }

    pub fn with_audit(mut self, audit: ToolAudit) -> Self {
        self.0.audit = Some(audit);
        self
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.0.message = Some(scrub_bounded(message.into().as_str(), MAX_DETAIL_CHARS));
        self
    }
}

/// A serde failure, bounded and scrubbed before it is put in an envelope.
///
/// The values in the arguments are not credentials — the tools take none — but a message that echoes
/// a file's text back is still a message nobody asked for. Scrubbing masks the shapes that look like
/// a secret and the bound keeps one long line from filling an error box.
pub(crate) fn describe(error: &serde_json::Error) -> String {
    scrub_bounded(&error.to_string(), MAX_DETAIL_CHARS)
}
