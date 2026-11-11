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
//!
//! The second half of the file is the *other* direction: the external servers this installation
//! calls. Those commands keep their logic in [`crate::mcp_client`] — what is saved, what is refused
//! and when a server is reopened is testable there without a webview — and only load the file, hand
//! it over and write it back here. A server's environment and headers go in through
//! `mcp_server_secrets_set` and never come out: every answer carries `hasSecrets`, never a value.

use std::collections::BTreeMap;

use sevenshell_api::{ApprovalDecision, AuditRecord};
use sevenshell_error::ShellError;
use sevenshell_mcp_client::McpServerSecrets;
use sevenshell_types::{AppSettings, McpServerConfig};
use tauri::{AppHandle, State};

use crate::mcp::{McpStatus, audit};
use crate::mcp_client::{McpServerView, shell_error};
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

/// The preferences, or the defaults when the file cannot be read.
///
/// The same rule the listener's status follows: a settings file that cannot be read is not a reason
/// to refuse to say what is configured. The defaults are an empty server list, which is honest.
fn settings_or_default() -> AppSettings {
    match sevenshell_platform::load() {
        Ok(settings) => settings,
        Err(error) => {
            log::warn!("settings could not be read, so the external servers report none: {error}");
            AppSettings::default()
        }
    }
}

/// An id no configured server answers to, as a sentence a page can show.
fn unknown_server(server_id: &str) -> ShellError {
    // The client's own code for this has no catalogue entry yet; the sentence names it so a log line
    // and the page still agree about what happened, and it quotes only what the caller sent.
    ShellError::app_internal(format!(
        "MCP-SERVER-NOT-FOUND: no external MCP server is configured with the id `{server_id}`"
    ))
}

/// Drops the entries a transport could not use, and trims the names.
///
/// A key with nothing in it is not an environment variable or a header. Values are left exactly as
/// typed: surrounding spaces can be part of a token, and a token this build trims is one the server
/// refuses.
fn key_values(values: BTreeMap<String, String>) -> BTreeMap<String, String> {
    values
        .into_iter()
        .filter_map(|(key, value)| {
            let key = key.trim().to_owned();
            (!key.is_empty()).then_some((key, value))
        })
        .collect()
}

/// Every external server, with where its supervision has got to and whether the vault holds secrets.
#[tauri::command]
pub async fn mcp_servers_list(state: State<'_, ShellState>) -> Result<Vec<McpServerView>, ShellError> {
    Ok(state.mcp_client().list(&settings_or_default()))
}

/// Writes one server's non-secret configuration and answers the row as the page draws it.
///
/// The order is deliberate: the manager accepts the list *first*, and only then is the file written.
/// A configuration the manager refuses is a page's answer, not a saved setting, and a list the
/// manager holds but the file does not would be reverted by the next save from any other page.
#[tauri::command]
pub async fn mcp_server_save(
    state: State<'_, ShellState>,
    config: McpServerConfig,
) -> Result<McpServerView, ShellError> {
    let mut settings = sevenshell_platform::load()?;
    let view = state
        .mcp_client()
        .save_server(&mut settings, config)
        .map_err(|error| shell_error(&error))?;

    sevenshell_platform::save(&settings)?;
    // The handshake is somebody else's server: the page must not wait on it. A server whose settings
    // did not change kept its session, so `refresh_if_needed` costs a connection only when the save
    // changed something.
    state.mcp_client().refresh_if_needed(&view.config.id);
    Ok(view)
}

/// Removes one server, with the secrets the vault files under it.
#[tauri::command]
pub async fn mcp_server_remove(state: State<'_, ShellState>, server_id: String) -> Result<(), ShellError> {
    let mut settings = sevenshell_platform::load()?;
    let removed = state
        .mcp_client()
        .remove_server(&mut settings, &server_id)
        .map_err(|error| shell_error(&error))?;

    if removed {
        sevenshell_platform::save(&settings)?;
    }
    // Nothing is emitted for a row that is gone: there is nothing left to draw, and the page learns
    // the server is not there any more from the list it asks for.
    Ok(())
}

/// Keeps the environment and headers one server is reached with, or forgets them.
///
/// Both objects travel whole and are written whole: `{}` in both is how the page clears what is
/// stored. The values go inwards only — the answer carries `hasSecrets`, never an entry.
#[tauri::command]
pub async fn mcp_server_secrets_set(
    state: State<'_, ShellState>,
    server_id: String,
    env: BTreeMap<String, String>,
    headers: BTreeMap<String, String>,
) -> Result<McpServerView, ShellError> {
    let settings = sevenshell_platform::load()?;
    let config = state
        .mcp_client()
        .server(&settings, &server_id)
        .ok_or_else(|| unknown_server(&server_id))?;

    let secrets = McpServerSecrets {
        env: key_values(env),
        headers: key_values(headers),
    };
    if secrets.is_empty() {
        state.vault().set_mcp_server_secrets(&server_id, None)?;
    } else {
        let encoded = serde_json::to_string(&secrets)
            .map_err(|error| ShellError::app_internal(format!("a server's secrets could not be encoded: {error}")))?;
        // An empty string is how the vault deletes; this is never empty here, so `Some` is a save.
        state.vault().set_mcp_server_secrets(&server_id, Some(encoded))?;
    }

    // The manager is handed the whole set again rather than the one entry: it reads the vault, so
    // there is one place secrets come from and no second copy to keep in step.
    state
        .mcp_client()
        .apply(&settings)
        .map_err(|error| shell_error(&error))?;
    // A server that failed for want of a token is retried now, which is what the person typing one
    // is asking for.
    state.mcp_client().refresh_in_background(config.id.clone());

    Ok(state
        .mcp_client()
        .view(&settings, &server_id)
        .expect("the server was checked to be configured above"))
}

/// Reconnects one server now, instead of waiting for its own backoff.
///
/// A failed attempt answers an error *and* leaves the row saying `failed` — the manager recorded it
/// and the event was sent before this returns, so the page draws the same thing whichever of the two
/// it looks at.
#[tauri::command]
pub async fn mcp_server_refresh(state: State<'_, ShellState>, server_id: String) -> Result<McpServerView, ShellError> {
    let settings = sevenshell_platform::load()?;
    if state.mcp_client().server(&settings, &server_id).is_none() {
        return Err(unknown_server(&server_id));
    }

    let attempt = state.mcp_client().refresh(&server_id).await;
    let view = state
        .mcp_client()
        .view(&settings, &server_id)
        .expect("the server was checked to be configured above");

    match attempt {
        Ok(_) => Ok(view),
        Err(error) => Err(shell_error(&error)),
    }
}
