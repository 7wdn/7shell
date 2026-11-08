//! Remote desktops: opening one, sending it input, and framing the pixels it sends back.
//!
//! Frames travel over a Tauri IPC channel as raw bytes rather than as JSON events. A frame is
//! already a dense pixel buffer; base64 in an event would inflate it by a third and make the
//! webview decode it again. The channel is per session and carries exactly one shape:
//!
//! ```text
//!   count: u32 LE
//!   repeated count times:
//!     x: u16 LE | y: u16 LE | width: u16 LE | height: u16 LE | RGBA8 ...
//! ```
//!
//! One message holds a whole processing step's changed rectangles. A Windows frame is often
//! dozens of small tiles, and one message per tile is what made the desktop feel slow.

use serde::Serialize;
use sevenshell_core::DesktopSessionEvent;
use sevenshell_error::ShellError;
use sevenshell_types::{DesktopInput, DesktopOutcome, OpenDesktop, PresentedCertificate};
use tauri::ipc::{Channel, Response};
use tauri::{AppHandle, Emitter as _, Manager as _, State};

use crate::state::ShellState;

/// Opens a desktop, or reports the certificate decision that has to be made first.
///
/// The channel is remembered before the connection attempt starts: the first frames can arrive
/// while this command is still returning, and a frame with nowhere to go would leave the canvas
/// blank until something on the far side happened to change.
#[tauri::command]
pub async fn desktop_open(
    state: State<'_, ShellState>,
    session_id: String,
    channel: Channel<Response>,
) -> Result<DesktopOutcome, ShellError> {
    state.remember_desktop_channel(&session_id, &channel);

    let result = state.core().open_desktop(&session_id).await;
    match &result {
        Ok(DesktopOutcome::Opened { handle, .. }) => state.attach_desktop_channel(&session_id, handle),
        Ok(_) => {
            // A decision is still waiting for a person; the channel stays under the session id so
            // the retry after trusting the certificate can use the same one.
        }
        Err(_) => state.forget_desktop_session(&session_id),
    }

    result
}

/// Records the certificate a person has just compared and accepted.
#[tauri::command]
pub async fn desktop_trust(state: State<'_, ShellState>, certificate: PresentedCertificate) -> Result<(), ShellError> {
    state.core().trust_desktop_certificate(&certificate)
}

#[tauri::command]
pub async fn desktop_input(
    state: State<'_, ShellState>,
    handle: String,
    input: DesktopInput,
) -> Result<(), ShellError> {
    state.core().desktop_input(&handle, input).await
}

#[tauri::command]
pub async fn desktop_resize(
    state: State<'_, ShellState>,
    handle: String,
    width: u16,
    height: u16,
) -> Result<(), ShellError> {
    state.core().desktop_resize(&handle, width, height).await
}

/// Asks for a full repaint, sent when the canvas mounts.
#[tauri::command]
pub async fn desktop_refresh(state: State<'_, ShellState>, handle: String) -> Result<(), ShellError> {
    state.core().desktop_refresh(&handle).await
}

#[tauri::command]
pub async fn desktop_close(state: State<'_, ShellState>, handle: String) -> Result<(), ShellError> {
    let result = state.core().desktop_close(&handle).await;
    state.forget_desktop_channel(&handle);
    result
}

/// The desktops that are open, so the interface can rebuild its tabs after a reload.
#[tauri::command]
pub async fn desktops_open(state: State<'_, ShellState>) -> Result<Vec<OpenDesktop>, ShellError> {
    Ok(state.core().open_desktops().await)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopResized {
    handle: String,
    width: u16,
    height: u16,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopEnded {
    handle: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopFailed {
    handle: String,
    error: ShellError,
}

/// Carries the core's desktop stream to the window.
pub fn pump_desktops(app: AppHandle, mut events: tokio::sync::mpsc::UnboundedReceiver<DesktopSessionEvent>) {
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                DesktopSessionEvent::Frame {
                    handle,
                    session_id,
                    bytes,
                } => {
                    let Some(channel) = app.try_state::<ShellState>().and_then(|state| {
                        state
                            .desktop_channel(&handle)
                            .or_else(|| state.desktop_channel_for_session(&session_id))
                    }) else {
                        continue;
                    };
                    let _ = channel.send(Response::new(bytes));
                }
                DesktopSessionEvent::Resized { handle, width, height } => {
                    let _ = app.emit("desktop:resized", DesktopResized { handle, width, height });
                }
                DesktopSessionEvent::Closed { handle } => {
                    if let Some(state) = app.try_state::<ShellState>() {
                        state.forget_desktop_channel(&handle);
                    }
                    let _ = app.emit("desktop:closed", DesktopEnded { handle });
                }
                DesktopSessionEvent::Failed { handle, error } => {
                    if let Some(state) = app.try_state::<ShellState>() {
                        state.forget_desktop_channel(&handle);
                    }
                    let _ = app.emit("desktop:failed", DesktopFailed { handle, error });
                }
            }
        }
    });
}
