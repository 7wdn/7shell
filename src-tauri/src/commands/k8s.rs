//! The Kubernetes command surface.
//!
//! One generic request command carries the resource API; the typed commands are the two things a
//! generic request cannot express — a stream that outlives the call (logs) and a terminal (exec).
//! A write request carries the person's acknowledgement, because the policy verdict is made on
//! this side of the wire.
//!
//! A watch is the third shape: the call answers a stream id at once and the events arrive as they
//! happen. The events are emitted under the names the interface listens for — `k8s:watch` and
//! `k8s:stream-end` — so the page never has to ask whether a stream is still alive.

use sevenshell_error::ShellError;
use sevenshell_types::{
    K8sExecRequest, K8sLogRequest, K8sRequest, K8sResponse, K8sStreamEnd, K8sStreamLine, K8sWatchEvent,
    KubeContextInfo, SecretInput,
};
use tauri::{AppHandle, Emitter as _, State};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::state::ShellState;

#[tauri::command]
pub async fn k8s_contexts(
    state: State<'_, ShellState>,
    kubeconfig: Option<SecretInput>,
) -> Result<Vec<KubeContextInfo>, ShellError> {
    state.core().k8s_contexts(kubeconfig).await
}

#[tauri::command]
pub async fn k8s_connect(state: State<'_, ShellState>, session_id: String) -> Result<String, ShellError> {
    state.core().k8s_connect(&session_id).await
}

#[tauri::command]
pub async fn k8s_disconnect(state: State<'_, ShellState>, connection_id: String) -> Result<(), ShellError> {
    state.core().k8s_disconnect(&connection_id);
    Ok(())
}

#[tauri::command]
pub async fn k8s_request(
    state: State<'_, ShellState>,
    connection_id: String,
    request: K8sRequest,
) -> Result<K8sResponse, ShellError> {
    state.core().k8s_request(&connection_id, request).await
}

/// Starts following one list, and answers the stream id that stops it.
///
/// The channels are made here rather than in the core because the receiver is the window: the task
/// below turns what arrives into events, and it holds the only receiver there is.
#[tauri::command]
pub async fn k8s_watch_start(
    app: AppHandle,
    state: State<'_, ShellState>,
    connection_id: String,
    path: String,
) -> Result<String, ShellError> {
    let (events, event_receiver) = tokio::sync::mpsc::unbounded_channel();
    let (end, end_receiver) = tokio::sync::mpsc::unbounded_channel();

    let stream_id = state.core().k8s_watch_start(&connection_id, &path, events, end)?;
    forward_watch(app, event_receiver, end_receiver);
    Ok(stream_id)
}

#[tauri::command]
pub async fn k8s_logs_start(
    app: AppHandle,
    state: State<'_, ShellState>,
    connection_id: String,
    request: K8sLogRequest,
) -> Result<String, ShellError> {
    let (events, lines) = tokio::sync::mpsc::unbounded_channel();
    let (end, ends) = tokio::sync::mpsc::unbounded_channel();
    let stream_id = state
        .core()
        .k8s_logs_start(&connection_id, &request, events, end)
        .await?;
    forward_log(app, lines, ends);
    Ok(stream_id)
}

#[tauri::command]
pub async fn k8s_stream_stop(state: State<'_, ShellState>, stream_id: String) -> Result<(), ShellError> {
    state.core().k8s_stream_stop(&stream_id);
    Ok(())
}

#[tauri::command]
pub async fn k8s_exec_open(
    state: State<'_, ShellState>,
    connection_id: String,
    request: K8sExecRequest,
) -> Result<String, ShellError> {
    state.core().k8s_exec_open(&connection_id, &request, 80, 24).await
}

/// Delivers a watch's events to the window, and its end after them.
///
/// The end is drained *after* the events already queued: the two arrive on different channels, and
/// emitting the end first would tell the interface the stream is gone while its last events were
/// still on their way — the row would stop updating and the event would arrive for a table that no
/// longer exists.
fn forward_watch(
    app: AppHandle,
    mut events: UnboundedReceiver<K8sWatchEvent>,
    mut end: UnboundedReceiver<K8sStreamEnd>,
) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                Some(event) = events.recv() => {
                    if let Err(error) = app.emit("k8s:watch", event) {
                        log::warn!("a watch event could not be delivered: {error}");
                    }
                }
                Some(finished) = end.recv() => {
                    while let Ok(event) = events.try_recv() {
                        if let Err(error) = app.emit("k8s:watch", event) {
                            log::warn!("a watch event could not be delivered: {error}");
                        }
                    }
                    if let Err(error) = app.emit("k8s:stream-end", finished) {
                        log::warn!("a stream ending could not be delivered: {error}");
                    }
                    return;
                }
                else => return,
            }
        }
    });
}

/// Delivers a log's lines to the window, and its end after the last line.
fn forward_log(app: AppHandle, mut lines: UnboundedReceiver<K8sStreamLine>, mut end: UnboundedReceiver<K8sStreamEnd>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                Some(line) = lines.recv() => {
                    if let Err(error) = app.emit("k8s:stream", line) {
                        log::warn!("a log line could not be delivered: {error}");
                    }
                }
                Some(finished) = end.recv() => {
                    while let Ok(line) = lines.try_recv() {
                        if let Err(error) = app.emit("k8s:stream", line) {
                            log::warn!("a log line could not be delivered: {error}");
                        }
                    }
                    if let Err(error) = app.emit("k8s:stream-end", finished) {
                        log::warn!("a log ending could not be delivered: {error}");
                    }
                    return;
                }
                else => return,
            }
        }
    });
}
