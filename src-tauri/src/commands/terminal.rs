//! Terminals: opening one, typing into it, resizing it, closing it.
//!
//! Keystrokes arrive and output leaves as base64 in JSON events. Bytes rather than text, on
//! purpose: a terminal stream is not UTF-8 at every boundary, and decoding it here would corrupt
//! exactly the multi-byte characters that arrive in two pieces.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Serialize;
use sevenshell_core::Core;
use sevenshell_error::ShellError;
use sevenshell_session::OpenTerminal;
use sevenshell_term::zmodem::{Detector, DetectorEvent, Direction, EndReason, FileInfo, ReceiveEvent};
use sevenshell_types::{HostSystem, LocalShell, OpenOutcome, OpenTunnel, SerialPortInfo};
use tauri::{AppHandle, Emitter as _, State};
use tauri_plugin_dialog::DialogExt;
use tokio::io::AsyncWriteExt;

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

/// What the host behind a terminal says it is: which system the session list should draw a mark for.
///
/// A question of its own rather than a field on the open, on purpose. Opening a terminal is what
/// somebody asked for, and it is answered the moment it can be — making it wait on a second round
/// trip that only decides which picture goes beside a row would put the answer to the question they
/// did not ask in front of the one they did. A host that does not answer costs that row its mark and
/// nothing else: the fields come back empty, and the one thing that does fail here — a handle whose
/// terminal has already ended — is a failure the interface drops on the floor.
#[tauri::command]
pub async fn session_system(state: State<'_, ShellState>, handle: String) -> Result<HostSystem, ShellError> {
    state.core().host_system(&handle).await
}

/// Calls off an open that is still going, and says whether there was one to call off.
///
/// A separate command rather than a handle to close: what is being stopped is an attempt, which has
/// no handle yet — that is the whole reason somebody wants to stop it.
#[tauri::command]
pub async fn session_open_cancel(state: State<'_, ShellState>, session_id: String) -> Result<bool, ShellError> {
    Ok(state.core().cancel_open(&session_id).await)
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

/// The shells this machine can start, for the local-terminal picker.
///
/// Identifiers and labels only: a shell is named `pwsh`, never by the path it happens to live at.
#[tauri::command]
pub async fn terminal_local_shells(state: State<'_, ShellState>) -> Result<Vec<LocalShell>, ShellError> {
    Ok(state.core().local_shells())
}

/// The WSL distributions this machine has. Empty where WSL is not installed.
#[tauri::command]
pub async fn terminal_wsl_distros(state: State<'_, ShellState>) -> Result<Vec<String>, ShellError> {
    Ok(state.core().wsl_distros())
}

/// The serial ports this machine has, for the serial picker.
#[tauri::command]
pub async fn terminal_serial_ports(state: State<'_, ShellState>) -> Result<Vec<SerialPortInfo>, ShellError> {
    Ok(state.core().serial_ports())
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

/// A ZMODEM session running on one terminal, and the file it is moving.
struct ZmodemSession {
    detector: Detector,
    incoming: Option<IncomingFile>,
    sending_name: Option<String>,
}

impl Default for ZmodemSession {
    fn default() -> Self {
        Self {
            detector: Detector::new(),
            incoming: None,
            sending_name: None,
        }
    }
}

struct IncomingFile {
    file: tokio::fs::File,
    name: String,
}

/// What the interface is told about a ZMODEM session.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ZmodemNotice {
    handle: String,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

fn notice(app: &AppHandle, handle: &str, kind: &'static str, name: Option<String>, detail: Option<String>) {
    let _ = app.emit(
        "zmodem:event",
        ZmodemNotice {
            handle: handle.to_owned(),
            kind,
            name,
            detail,
        },
    );
}

/// Turns the core's output stream into window events, and watches each terminal for ZMODEM.
///
/// The detector sits here rather than in the terminal stack: a received file needs a native save
/// dialog and a file to send needs a native picker, and this is the layer that owns both. The
/// protocol sees only bytes; the choice of where they land is made by a person, in this process.
///
/// The protocol itself is tested in `sevenshell-term` against scripted peers. A real `rz`/`sz`
/// session is a manual acceptance step: the lab has no `lrzsz`, and the two native dialogs cannot
/// be driven by Playwright.
pub fn pump_events(
    app: AppHandle,
    core: Arc<Core>,
    mut events: tokio::sync::mpsc::Receiver<sevenshell_core::CoreEvent>,
) {
    tauri::async_runtime::spawn(async move {
        let mut sessions: HashMap<String, ZmodemSession> = HashMap::new();
        while let Some(event) = events.recv().await {
            match event.event {
                sevenshell_session::TerminalEvent::Data(bytes) => {
                    let state = sessions.entry(event.handle.clone()).or_default();
                    feed_zmodem(&app, &core, &event.handle, state, bytes).await;
                }
                sevenshell_session::TerminalEvent::Closed { reason } => {
                    sessions.remove(&event.handle);
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

async fn feed_zmodem(app: &AppHandle, core: &Core, handle: &str, session: &mut ZmodemSession, bytes: Vec<u8>) {
    let events = match session.detector.feed(&bytes) {
        Ok(events) => events,
        Err(error) => {
            notice(app, handle, "failed", None, error.detail().map(str::to_owned));
            let abort = session.detector.abort();
            write_replies(core, handle, abort).await;
            session.incoming = None;
            return;
        }
    };

    let mut finish_sending = false;
    for event in events {
        match event {
            DetectorEvent::Output(bytes) => emit_data(app, handle, bytes),
            DetectorEvent::Reply(bytes) => {
                if let Err(error) = core.write(handle, &bytes).await {
                    log::warn!("a ZMODEM reply could not be written to {handle}: {error}");
                }
            }
            DetectorEvent::Started(direction) => {
                notice(app, handle, "started", None, None);
                if direction == Direction::Send {
                    let Some(path) = pick_file(app).await else {
                        let abort = session.detector.abort();
                        write_replies(core, handle, abort).await;
                        notice(app, handle, "cancelled", None, None);
                        return;
                    };
                    match send_from(app, core, handle, session, path).await {
                        Ok(()) => {}
                        Err(detail) => {
                            let abort = session.detector.abort();
                            write_replies(core, handle, abort).await;
                            notice(app, handle, "failed", None, Some(detail));
                            return;
                        }
                    }
                }
            }
            DetectorEvent::Received(ReceiveEvent::Offer(info)) => {
                let Some(path) = save_file(app, &info.name).await else {
                    let abort = session.detector.abort();
                    write_replies(core, handle, abort).await;
                    notice(app, handle, "cancelled", Some(info.name), None);
                    return;
                };
                match tokio::fs::File::create(&path).await {
                    Ok(file) => {
                        session.incoming = Some(IncomingFile {
                            file,
                            name: info.name.clone(),
                        });
                        notice(app, handle, "receiving", Some(info.name), None);
                    }
                    Err(error) => {
                        let abort = session.detector.abort();
                        write_replies(core, handle, abort).await;
                        notice(
                            app,
                            handle,
                            "failed",
                            Some(info.name),
                            Some(format!("the local file could not be created: {error}")),
                        );
                        return;
                    }
                }
            }
            DetectorEvent::Received(ReceiveEvent::Data(data)) => {
                let Some(incoming) = session.incoming.as_mut() else {
                    continue;
                };
                if let Err(error) = incoming.file.write_all(&data).await {
                    notice(
                        app,
                        handle,
                        "failed",
                        Some(incoming.name.clone()),
                        Some(format!("the local file could not be written: {error}")),
                    );
                    session.incoming = None;
                    let abort = session.detector.abort();
                    write_replies(core, handle, abort).await;
                    return;
                }
            }
            DetectorEvent::Received(ReceiveEvent::Complete(info)) => {
                if let Some(mut incoming) = session.incoming.take() {
                    let _ = incoming.file.flush().await;
                }
                notice(app, handle, "done", Some(info.name), None);
            }
            DetectorEvent::Received(ReceiveEvent::Skipped { name, reason }) => {
                notice(app, handle, "skipped", Some(name), Some(reason));
            }
            DetectorEvent::Sent { bytes } => {
                notice(
                    app,
                    handle,
                    "sent",
                    session.sending_name.clone(),
                    Some(bytes.to_string()),
                );
                finish_sending = true;
            }
            DetectorEvent::Ended(reason) => {
                session.incoming = None;
                let sent_name = session.sending_name.take();
                if reason == EndReason::Complete && sent_name.is_some() {
                    notice(app, handle, "finished", sent_name, None);
                }
            }
        }
    }

    if finish_sending && matches!(session.detector.active(), Some(Direction::Send)) {
        match session.detector.finish_sending() {
            Ok(events) => write_replies(core, handle, events).await,
            Err(error) => notice(app, handle, "failed", None, error.detail().map(str::to_owned)),
        }
    }
}

async fn send_from(
    app: &AppHandle,
    core: &Core,
    handle: &str,
    session: &mut ZmodemSession,
    path: PathBuf,
) -> Result<(), String> {
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|error| format!("the chosen file could not be read: {error}"))?;
    let size = metadata.len();
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| "the chosen path has no file name".to_owned())?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|age| age.as_secs());
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|error| format!("the chosen file could not be opened: {error}"))?;
    let events = session
        .detector
        .start_sending(
            FileInfo {
                name: name.clone(),
                size,
                modified,
                // The protocol's mode field is for the far side; this side's file keeps its own.
                mode: Some(0o100644),
            },
            file,
        )
        .map_err(|error| error.detail().unwrap_or("the ZMODEM sender could not start").to_owned())?;
    session.sending_name = Some(name.clone());
    notice(app, handle, "sending", Some(name), None);
    write_replies(core, handle, events).await;
    Ok(())
}

async fn write_replies(core: &Core, handle: &str, events: Vec<DetectorEvent>) {
    for event in events {
        if let DetectorEvent::Reply(bytes) = event
            && let Err(error) = core.write(handle, &bytes).await
        {
            log::warn!("a ZMODEM reply could not be written to {handle}: {error}");
        }
    }
}

fn emit_data(app: &AppHandle, handle: &str, bytes: Vec<u8>) {
    let payload = TerminalData {
        handle: handle.to_owned(),
        base64: BASE64.encode(bytes),
    };
    if let Err(error) = app.emit("terminal:data", payload) {
        log::warn!("terminal output could not be delivered: {error}");
    }
}

async fn save_file(app: &AppHandle, suggested: &str) -> Option<PathBuf> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog().file().set_file_name(suggested).save_file(move |path| {
        let _ = sender.send(path);
    });
    receiver.await.ok().flatten()?.into_path().ok()
}

async fn pick_file(app: &AppHandle) -> Option<PathBuf> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_file(move |path| {
        let _ = sender.send(path);
    });
    receiver.await.ok().flatten()?.into_path().ok()
}
