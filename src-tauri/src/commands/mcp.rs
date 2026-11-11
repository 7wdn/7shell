//! What the interface is told and allowed to do about the agent interface.
//!
//! The window can read the status, turn the listener on and off, ask for the snippet it would paste
//! into an MCP client, read the tail of the audit trail and answer a pending approval. It cannot see
//! the token: that lives in the endpoint file beside the token's only reader, the bridge.
//!
//! Turning it on writes the setting *before* the socket is opened, and a start that then fails leaves
//! the setting on and the answer an error. That is deliberate: the switch must not claim a listener
//! that is not there, and the next launch (or the next click) tries again rather than silently
//! forgetting what the person asked for.

use sevenshell_api::{ApprovalDecision, AuditRecord};
use sevenshell_error::ShellError;
use tauri::{AppHandle, State};

use crate::mcp::{McpStatus, audit};
use crate::state::ShellState;

/// What is listening, if anything.
#[tauri::command]
pub async fn mcp_status(state: State<'_, ShellState>) -> Result<McpStatus, ShellError> {
    let settings = match sevenshell_platform::load() {
        Ok(settings) => settings,
        Err(error) => {
            // A settings file that cannot be read is not a reason to refuse to report a listener
            // that is running; the listener's own snapshot supplies the scope, and the defaults are
            // the narrowest tier.
            log::warn!("settings could not be read, so the agent interface reports its defaults: {error}");
            sevenshell_types::AppSettings::default()
        }
    };
    Ok(state.mcp().status(&settings).await)
}

/// Turns the listener on and reports what it bound.
#[tauri::command]
pub async fn mcp_enable(app: AppHandle, state: State<'_, ShellState>) -> Result<McpStatus, ShellError> {
    let mut settings = sevenshell_platform::load()?;
    settings.mcp_enabled = true;
    sevenshell_platform::save(&settings)?;
    crate::mcp::enable(&app, state.mcp(), &settings).await
}

/// Turns the listener off, removes its endpoint file and reports the state afterwards.
#[tauri::command]
pub async fn mcp_disable(app: AppHandle, state: State<'_, ShellState>) -> Result<McpStatus, ShellError> {
    // Stopped before the preference is written, because "turn it off" has to take effect even when
    // the settings file is the thing that is broken. A failed save is reported after the listener is
    // already gone: the next launch may try again, but nothing is listening now.
    let mut settings = match sevenshell_platform::load() {
        Ok(settings) => settings,
        Err(error) => {
            // Still off, still reported; the corrupt file is left alone rather than replaced by
            // defaults, because the rest of somebody's preferences are in it.
            crate::mcp::disable(&app, state.mcp(), &sevenshell_types::AppSettings::default()).await;
            return Err(error);
        }
    };
    settings.mcp_enabled = false;

    let status = crate::mcp::disable(&app, state.mcp(), &settings).await;
    sevenshell_platform::save(&settings)?;
    Ok(status)
}

/// The snippet an MCP client's configuration wants, without the token.
#[tauri::command]
pub fn mcp_client_config() -> Result<String, ShellError> {
    crate::mcp::client_config()
}

/// The last `limit` audit records, oldest first.
#[tauri::command]
pub fn mcp_audit_tail(state: State<'_, ShellState>, limit: usize) -> Result<Vec<AuditRecord>, ShellError> {
    let path = state.mcp().audit_path()?;
    Ok(audit::read_tail(&path, limit))
}

/// Answers one pending approval.
#[tauri::command]
pub fn mcp_approval_answer(state: State<'_, ShellState>, call_id: String, decision: String) -> Result<(), ShellError> {
    let decision = match decision.as_str() {
        "approved" => ApprovalDecision::Approved,
        "denied" => ApprovalDecision::Denied,
        other => {
            return Err(ShellError::app_internal(format!(
                "`{other}` is not an approval decision; expected `approved` or `denied`"
            )));
        }
    };
    // No error for an id that is not waiting: the window can answer a call that timed out a moment
    // before the click, and that is a race, not a mistake worth an error box.
    state.mcp().answer(&call_id, decision);
    Ok(())
}
