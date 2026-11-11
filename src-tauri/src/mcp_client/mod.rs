//! The client side of MCP, as the desktop owns it: the external servers this installation calls.
//!
//! The bridge in `sevenshell-mcp` speaks MCP *in* to this application; this tree talks *out* to
//! somebody else's server. Two things live here and nothing else:
//!
//! * [`McpClientSupervisor`] — the lifetime rules. It holds the one
//!   [`sevenshell_mcp_client::McpClientManager`] the window supervises, joins it to the one tool
//!   table every caller reads, and is the side that decides *when* a server is opened: at launch,
//!   after the vault opens, when the settings page saves, and never after the vault locks.
//! * [`McpServerView`] — one row of the settings page: the configuration, where its supervision has
//!   got to, and whether the vault holds secrets for it. There is no field a secret could arrive in.
//!
//! The split against the lower crate is the same one the rest of the shell follows: that crate knows
//! how to speak MCP, this one knows when it is allowed to. Every credential the lower crate needs
//! comes from the vault through [`McpClientSupervisor::apply`], which is also the one place the
//! settings file's list is handed over — so "what is configured" and "what is running" cannot be
//! written by two different paths.

mod supervisor;

use sevenshell_error::{ErrorCode, ShellError};
use sevenshell_mcp_client::McpClientError;
use sevenshell_types::{McpServerConfig, McpServerStatus};

pub use supervisor::{McpClientSupervisor, TauriServerStatusEvents};

/// One external server, as the settings page draws it.
///
/// `has_secrets` is `None` while the vault is locked — "cannot tell" is not "none stored", and a
/// page that drew the second over the first would invite somebody to type a token they had already
/// saved. `None` reaches the window as `null`, which is the shape the page matches on.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerView {
    pub config: McpServerConfig,
    pub status: McpServerStatus,
    pub has_secrets: Option<bool>,
}

/// A client failure as the envelope a command answers with.
///
/// Every client code is in the catalog, so the code and the message key cannot drift; the names a
/// sentence quotes (the server, the tool) are filled here, because this is the layer that knows
/// which variant carries them. The transport's own words are added as the detail, which is the
/// field the envelope scrubs and cuts — the client crate promises a detail carries no configuration
/// value.
pub(crate) fn shell_error(error: &McpClientError) -> ShellError {
    let Some(code) = ErrorCode::from_code(error.code()) else {
        // A newer client can report a code this host's catalog does not have yet. There is no
        // envelope code to carry its identity, so the sentence keeps it; `Display` names the code
        // and says what happened, and never quotes a configuration value.
        return ShellError::app_internal(error.to_string());
    };

    let envelope = crate::ai::catalog_error(code);
    let envelope = match error {
        McpClientError::DuplicateServer { id } => envelope.with_param("server", id),
        McpClientError::ConnectFailed { server, .. } => envelope.with_param("server", server),
        McpClientError::DuplicateTool { server, tool } | McpClientError::InvalidTool { server, tool, .. } => {
            envelope.with_param("server", server).with_param("tool", tool)
        }
        _ => envelope,
    };
    match error.detail() {
        Some(detail) => envelope.with_detail(detail),
        None => envelope,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sevenshell_mcp_client::McpClientError;

    #[test]
    fn a_catalog_failure_keeps_its_code_and_fills_the_names_its_sentence_quotes() {
        let connect = McpClientError::ConnectFailed {
            server: "files".to_owned(),
            detail: "connection refused".to_owned(),
        };
        let envelope = shell_error(&connect);
        assert_eq!(envelope.code().as_str(), "MCP-SERVER-CONNECT-FAILED");
        assert_eq!(envelope.message_key(), "errors.mcp.serverConnectFailed");
        assert_eq!(
            envelope.message_params().get("server").map(String::as_str),
            Some("files")
        );
        assert_eq!(envelope.detail(), Some("connection refused"));

        let invalid = McpClientError::InvalidConfig {
            server: "files".to_owned(),
            detail: "an stdio server needs a command to start".to_owned(),
        };
        let envelope = shell_error(&invalid);
        assert_eq!(envelope.code().as_str(), "MCP-INVALID-ARGUMENTS");
        assert_eq!(envelope.message_key(), "errors.mcp.invalidArguments");
        assert_eq!(envelope.detail(), Some("an stdio server needs a command to start"));
    }

    #[test]
    fn a_view_says_null_rather_than_omitting_a_secret_state_the_vault_cannot_answer() {
        use sevenshell_types::{McpServerState, McpServerStatus, McpServerTransport};

        let view = McpServerView {
            config: McpServerConfig::new("files", "Files", McpServerTransport::Stdio),
            status: McpServerStatus::new("files", McpServerState::Connecting),
            has_secrets: None,
        };
        let json = serde_json::to_value(&view).expect("a view is JSON");
        assert_eq!(json["hasSecrets"], serde_json::Value::Null);
        assert_eq!(json["status"]["serverId"], serde_json::json!("files"));
        assert_eq!(json["config"]["toolTimeoutSeconds"], serde_json::json!(60));
    }
}
