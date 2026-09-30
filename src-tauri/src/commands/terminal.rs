//! Terminals: opening one, typing into it, resizing it, closing it.
//!
//! Keystrokes arrive and output leaves as base64 in JSON events. Bytes rather than text, on
//! purpose: a terminal stream is not UTF-8 at every boundary, and decoding it here would corrupt
//! exactly the multi-byte characters that arrive in two pieces.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Serialize;
use sevenshell_error::ShellError;
use sevenshell_session::OpenTerminal;
use sevenshell_types::{OpenOutcome, OpenTunnel};
use tauri::{AppHandle, Emitter as _, State};

use crate::state::ShellState;

/// Output from a terminal, as it arrives at the interface.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalData {
    pub handle: String,
    pub base64: String,
}

/// A terminal that ended, with whatever the server said about it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalClosed {
    pub handle: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Opens a terminal for a saved session.
///
/// The answer may be "a host key decision is needed first" rather than a handle: nothing is sent
/// to the server until that decision is made.
#[tauri::command]
pub async fn session_open(state: State<'_, ShellState>, session_id: String) -> Result<OpenOutcome, ShellError> {
    state.core().open_terminal(&session_id).await
}

/// Sends keystrokes or pasted text, base64-encoded.
#[tauri::command]
pub async fn terminal_write(state: State<'_, ShellState>, handle: String, base64: String) -> Result<(), ShellError> {
    let data = BASE64
        .decode(base64.as_bytes())
        .map_err(|error| ShellError::app_internal(format!("the terminal input was not valid base64: {error}")))?;

    state.core().write(&handle, &data).await
}

#[tauri::command]
pub async fn terminal_resize(
    state: State<'_, ShellState>,
    handle: String,
    columns: u32,
    rows: u32,
) -> Result<(), ShellError> {
    state.core().resize(&handle, columns, rows).await
}

#[tauri::command]
pub async fn terminal_close(state: State<'_, ShellState>, handle: String) -> Result<(), ShellError> {
    state.core().close_terminal(&handle).await
}

/// The terminals that are open, so the interface can rebuild its tabs after a reload.
#[tauri::command]
pub async fn terminals_open(state: State<'_, ShellState>) -> Result<Vec<OpenTerminal>, ShellError> {
    Ok(state.core().open_terminals().await)
}

/// The forwards that are up, with how much each has carried.
#[tauri::command]
pub async fn tunnels_open(state: State<'_, ShellState>) -> Result<Vec<OpenTunnel>, ShellError> {
    Ok(state.core().tunnels().await)
}

/// Stops one forward. Asking to stop something that is already stopped is not a failure.
#[tauri::command]
pub async fn tunnel_close(state: State<'_, ShellState>, id: String) -> Result<(), ShellError> {
    state.core().close_tunnel(&id).await
}

/// Turns the core's output stream into window events.
///
/// This is the only place that knows the interface exists: everything below it deals in bytes and
/// handles, which is what keeps the same operations available to a command line later.
pub fn pump_events(app: AppHandle, mut events: tokio::sync::mpsc::Receiver<sevenshell_core::CoreEvent>) {
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            match event.event {
                sevenshell_session::TerminalEvent::Data(bytes) => {
                    let payload = TerminalData {
                        handle: event.handle,
                        base64: BASE64.encode(bytes),
                    };
                    if let Err(error) = app.emit("terminal:data", payload) {
                        log::warn!("terminal output could not be delivered: {error}");
                    }
                }
                sevenshell_session::TerminalEvent::Closed { reason } => {
                    let payload = TerminalClosed {
                        handle: event.handle,
                        reason,
                    };
                    if let Err(error) = app.emit("terminal:closed", payload) {
                        log::warn!("a terminal ending could not be delivered: {error}");
                    }
                }
            }
        }
    });
}
