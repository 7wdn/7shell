//! The desktop side of the agent interface.
//!
//! The desktop is the only process with the vault, the known hosts and the live sessions, so it is
//! the process the MCP bridge talks to. It listens on `127.0.0.1` — the configured port or one the
//! system assigns — exposes three routes on loopback (`POST /tools/call`, `GET /tools` and
//! `POST /tools/cancel`), and writes down the port and the token for this run in
//! `<config_dir>/mcp-endpoint.json`. The bridge reads that file; the interface never does.
//!
//! The pieces, and why they are apart:
//!
//! * [`auth`] — the token and the header checks. Pure functions, so the boundary is testable without
//!   a socket.
//! * [`endpoint`] — the file the bridge reads. Owner-only and replaced atomically.
//! * [`server`] — the HTTP layer and the adapter onto `sevenshell_api`'s registry. The handler is a
//!   trait, so the layer can be exercised with a fake tool and no window.
//! * [`approval`] — the question a destructive call asks, answered by id through the `mcp:approval`
//!   event.
//! * [`audit`] — one JSONL line per call, rotated by size and pruned by age.
//! * [`runtime`] — the listener's lifetime, owned by [`crate::state::ShellState`].
//!
//! Two things belong to this layer rather than to a tool:
//!
//! * **The connection cache.** One [`sevenshell_api::local::ConnectionCache`] per run, shared by
//!   every call, so a conversation is one authenticated connection per session. The runtime owns it
//!   and empties it when the vault locks or the listener stops.
//! * **The approval window.** Destructive calls ask through `mcp:approval` and wait for
//!   `mcp_approval_answer`. Nothing is remembered: "yes, always" is a settings decision, not a side
//!   effect of one click.
//!
//! The token is a secret and never crosses to the interface. [`McpStatus`] names the port, the scope
//! and the audit file's path — the things a person needs to understand what is on — and nothing else.

pub mod approval;
pub mod audit;
pub mod auth;
pub mod endpoint;
mod runtime;
pub mod server;

use std::sync::Arc;

use serde::Serialize;
use sevenshell_error::ShellError;
use sevenshell_types::{AppSettings, McpScope};
use tauri::{AppHandle, Emitter as _};

pub use runtime::McpRuntime;

/// The event the interface listens on for the listener's own state.
pub const STATUS_EVENT: &str = "mcp:status";

/// The agent interface, as the settings page sees it.
///
/// `enabled` means *listening*, not "the setting says it should be": a start that failed to bind is
/// a switch that must show off, because an agent cannot reach it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    pub enabled: bool,
    /// The bound port, `None` when nothing is listening.
    pub port: Option<u16>,
    pub scope: McpScope,
    /// The snippet to paste into an MCP client, without the token. Empty when the bridge cannot be
    /// located; [`client_config`] is where that becomes a sentence.
    pub client_config: String,
    pub audit_path: String,
    pub pending_approvals: usize,
}

/// The client configuration snippet, without the token.
///
/// The bridge is looked for beside this executable — that is where a packaged build puts it and where
/// a development build leaves it. When it is not there this fails rather than naming a path that is
/// not a program: a snippet pointing at nothing is a client that starts nothing, which is a worse
/// answer than "not installed".
pub fn client_config() -> Result<String, ShellError> {
    let executable = std::env::current_exe()
        .map_err(|error| ShellError::app_internal(format!("this executable's own path could not be read: {error}")))?;
    let directory = executable
        .parent()
        .ok_or_else(|| ShellError::app_internal("this executable has no directory"))?;

    let bridge = directory.join(if cfg!(windows) { "sshell-mcp.exe" } else { "sshell-mcp" });
    let installed = std::fs::metadata(&bridge)
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false);
    if !installed {
        return Err(ShellError::app_internal(format!(
            "the MCP bridge is missing or empty beside this application (looked for {}); run `pnpm build:sidecar` or install a packaged 7Shell",
            bridge.display()
        )));
    }

    let endpoint = endpoint::path()?;
    Ok(serde_json::json!({
        "mcpServers": {
            "7shell": {
                "command": bridge.display().to_string(),
                "env": {
                    "SEVENSHELL_ENDPOINT_FILE": endpoint.display().to_string(),
                },
            },
        },
    })
    .to_string())
}

/// Starts the listener and tells the interface.
pub async fn enable(app: &AppHandle, runtime: &McpRuntime, settings: &AppSettings) -> Result<McpStatus, ShellError> {
    let status = runtime.start(app, settings).await?;
    emit_status(app, &status);
    Ok(status)
}

/// Stops the listener and tells the interface.
pub async fn disable(app: &AppHandle, runtime: &McpRuntime, settings: &AppSettings) -> McpStatus {
    runtime.stop().await;
    let status = runtime.status(settings).await;
    emit_status(app, &status);
    status
}

/// Makes the listener follow a settings change.
///
/// Called from `settings_set`, which is the one place the file is written. Only the five things that
/// describe the listener restart it; a change to the font or the window must not interrupt a call.
pub async fn apply_settings(
    app: &AppHandle,
    runtime: &McpRuntime,
    previous: &AppSettings,
    next: &AppSettings,
) -> Result<McpStatus, ShellError> {
    if !next.mcp_enabled {
        return Ok(disable(app, runtime, next).await);
    }

    // A start that failed leaves the setting on and the listener off; the next settings write is an
    // opportunity to try again rather than a reason to keep pretending.
    if !previous.mcp_enabled || listener_changed(previous, next) || !runtime.is_running().await {
        runtime.stop().await;
        return enable(app, runtime, next).await;
    }

    Ok(runtime.status(next).await)
}

/// Starts the listener when the settings asked for it at launch.
///
/// Spawned rather than awaited: `setup` is synchronous and the window should not wait on a socket.
/// A failure is logged and shown as "off" by the first status the interface asks for.
pub fn start_if_enabled(app: &AppHandle, runtime: Arc<McpRuntime>, settings: &AppSettings) {
    if !settings.mcp_enabled {
        return;
    }

    let app = app.clone();
    let settings = settings.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = enable(&app, &runtime, &settings).await {
            log::warn!("the agent interface could not start: {error}");
        }
    });
}

/// Whether two settings describe a different listener.
fn listener_changed(previous: &AppSettings, next: &AppSettings) -> bool {
    previous.mcp_port != next.mcp_port
        || previous.mcp_scope != next.mcp_scope
        || previous.mcp_approval_timeout_seconds != next.mcp_approval_timeout_seconds
        || previous.mcp_audit_retention_days != next.mcp_audit_retention_days
        || previous.mcp_max_result_bytes != next.mcp_max_result_bytes
}

fn emit_status(app: &AppHandle, status: &McpStatus) {
    if let Err(error) = app.emit(STATUS_EVENT, status) {
        log::warn!("the agent interface's status could not be sent to the window: {error}");
    }
}
