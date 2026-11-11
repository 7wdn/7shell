//! The container command surface.
//!
//! Each command is a thin adapter over the core's container orchestration. The two commands that
//! outlive their call — a followed log and a pull/push — answer a stream id and deliver lines as
//! `container:stream` / `container:stream-end` events, which is the same shape the interface already
//! uses for transfers.

use sevenshell_error::ShellError;
use sevenshell_types::{
    ContainerCommitRequest, ContainerCreateRequest, ContainerDetail, ContainerImage, ContainerImageTransfer,
    ContainerRegistryLogin, ContainerRuntimeInfo, ContainerStats, ContainerStreamEnd, ContainerStreamLine,
    ContainerSummary, ImageLayer,
};
use tauri::{AppHandle, Emitter as _, State};
use tauri_plugin_dialog::DialogExt;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::state::ShellState;

#[tauri::command]
pub async fn container_connect(state: State<'_, ShellState>, session_id: String) -> Result<String, ShellError> {
    state.core().container_connect(&session_id).await
}

#[tauri::command]
pub async fn container_disconnect(state: State<'_, ShellState>, connection_id: String) -> Result<(), ShellError> {
    state.core().container_disconnect(&connection_id).await;
    Ok(())
}

#[tauri::command]
pub async fn container_list(
    state: State<'_, ShellState>,
    connection_id: String,
) -> Result<Vec<ContainerSummary>, ShellError> {
    state.core().container_list(&connection_id).await
}

#[tauri::command]
pub async fn container_inspect(
    state: State<'_, ShellState>,
    connection_id: String,
    container_id: String,
) -> Result<ContainerDetail, ShellError> {
    state.core().container_inspect(&connection_id, &container_id).await
}

#[tauri::command]
pub async fn container_action(
    state: State<'_, ShellState>,
    connection_id: String,
    container_id: String,
    action: String,
    acknowledged: bool,
) -> Result<(), ShellError> {
    state
        .core()
        .container_action(&connection_id, &container_id, &action, acknowledged)
        .await
}

#[tauri::command]
pub async fn container_rename(
    state: State<'_, ShellState>,
    connection_id: String,
    container_id: String,
    name: String,
) -> Result<(), ShellError> {
    state
        .core()
        .container_rename(&connection_id, &container_id, &name)
        .await
}

#[tauri::command]
pub async fn container_create(
    state: State<'_, ShellState>,
    connection_id: String,
    request: ContainerCreateRequest,
) -> Result<(), ShellError> {
    state.core().container_create(&connection_id, &request).await
}

#[tauri::command]
pub async fn container_stats(
    state: State<'_, ShellState>,
    connection_id: String,
) -> Result<Vec<ContainerStats>, ShellError> {
    state.core().container_stats(&connection_id).await
}

#[tauri::command]
pub async fn container_namespaces(
    state: State<'_, ShellState>,
    connection_id: String,
) -> Result<Vec<String>, ShellError> {
    state.core().container_namespaces(&connection_id).await
}

#[tauri::command]
pub async fn container_set_namespace(
    state: State<'_, ShellState>,
    connection_id: String,
    namespace: String,
) -> Result<(), ShellError> {
    state.core().container_set_namespace(&connection_id, &namespace)
}

#[tauri::command]
pub async fn container_runtime_info(
    state: State<'_, ShellState>,
    connection_id: String,
) -> Result<ContainerRuntimeInfo, ShellError> {
    state.core().container_runtime_info(&connection_id).await
}

#[tauri::command]
pub async fn container_logs_start(
    app: AppHandle,
    state: State<'_, ShellState>,
    connection_id: String,
    container_id: String,
    tail: u32,
    timestamps: bool,
) -> Result<String, ShellError> {
    let (events, lines) = tokio::sync::mpsc::unbounded_channel();
    let (end, ends) = tokio::sync::mpsc::unbounded_channel();
    let stream_id = state
        .core()
        .container_logs_start(&connection_id, &container_id, tail, timestamps, events, end)
        .await?;
    forward_stream(app, lines, ends);
    Ok(stream_id)
}

#[tauri::command]
pub async fn container_stream_stop(state: State<'_, ShellState>, stream_id: String) -> Result<(), ShellError> {
    state.core().container_stream_stop(&stream_id).await;
    Ok(())
}

#[tauri::command]
pub async fn container_exec_open(
    state: State<'_, ShellState>,
    connection_id: String,
    container_id: String,
    shell: String,
) -> Result<String, ShellError> {
    state
        .core()
        .container_exec_open(&connection_id, &container_id, &shell, 80, 24)
        .await
}

#[tauri::command]
pub async fn container_images(
    state: State<'_, ShellState>,
    connection_id: String,
) -> Result<Vec<ContainerImage>, ShellError> {
    state.core().container_images(&connection_id).await
}

#[tauri::command]
pub async fn container_image_inspect(
    state: State<'_, ShellState>,
    connection_id: String,
    image: String,
) -> Result<String, ShellError> {
    state.core().container_image_inspect(&connection_id, &image).await
}

#[tauri::command]
pub async fn container_image_history(
    state: State<'_, ShellState>,
    connection_id: String,
    image: String,
) -> Result<Vec<ImageLayer>, ShellError> {
    state.core().container_image_history(&connection_id, &image).await
}

#[tauri::command]
pub async fn container_image_pull(
    app: AppHandle,
    state: State<'_, ShellState>,
    connection_id: String,
    request: ContainerImageTransfer,
) -> Result<String, ShellError> {
    transfer(&app, &state, &connection_id, &request, false).await
}

#[tauri::command]
pub async fn container_image_push(
    app: AppHandle,
    state: State<'_, ShellState>,
    connection_id: String,
    request: ContainerImageTransfer,
) -> Result<String, ShellError> {
    transfer(&app, &state, &connection_id, &request, true).await
}

#[tauri::command]
pub async fn container_image_remove(
    state: State<'_, ShellState>,
    connection_id: String,
    image: String,
    acknowledged: bool,
) -> Result<(), ShellError> {
    state
        .core()
        .container_image_remove(&connection_id, &image, acknowledged)
        .await
}

#[tauri::command]
pub async fn container_image_prune(
    state: State<'_, ShellState>,
    connection_id: String,
    acknowledged: bool,
) -> Result<(), ShellError> {
    state.core().container_image_prune(&connection_id, acknowledged).await
}

#[tauri::command]
pub async fn container_image_tag(
    state: State<'_, ShellState>,
    connection_id: String,
    image: String,
    tag: String,
) -> Result<(), ShellError> {
    state.core().container_image_tag(&connection_id, &image, &tag).await
}

/// The archive shapes a runtime's `load` reads. The CLI detects the compression itself, so the
/// dialog offers every container rather than asking which one it is.
const ARCHIVE_EXTENSIONS: &[&str] = &["tar", "gz", "tgz", "bz2", "tbz2", "xz", "txz", "zst", "tzst"];

/// Asks for an image archive in a native dialog and loads it into the connected engine.
///
/// The dialog runs here, and the chosen file is read here: what the interface receives is the
/// runtime's own one-line answer ("Loaded image: busybox:1.36"), never a path and never the archive.
/// `Ok(None)` means the dialog was dismissed, which is a person changing their mind rather than a
/// failure.
#[tauri::command]
pub async fn container_image_load(
    app: AppHandle,
    state: State<'_, ShellState>,
    connection_id: String,
    title: String,
) -> Result<Option<String>, ShellError> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title(title)
            .add_filter("container image archive", ARCHIVE_EXTENSIONS)
            .blocking_pick_file()
    })
    .await
    .map_err(|error| ShellError::app_internal(format!("the file dialog could not be opened: {error}")))?;

    let Some(chosen) = picked else {
        return Ok(None);
    };
    let path = chosen.into_path().map_err(|error| {
        ShellError::container_command_failed("the image archive", format!("that file cannot be read: {error}"))
    })?;

    state.core().container_image_load(&connection_id, &path).await.map(Some)
}

#[tauri::command]
pub async fn container_image_commit(
    state: State<'_, ShellState>,
    connection_id: String,
    request: ContainerCommitRequest,
    acknowledged: bool,
) -> Result<String, ShellError> {
    state
        .core()
        .container_image_commit(&connection_id, &request, acknowledged)
        .await
}

#[tauri::command]
pub async fn container_registry_login(
    state: State<'_, ShellState>,
    connection_id: String,
    request: ContainerRegistryLogin,
) -> Result<(), ShellError> {
    state.core().container_registry_login(&connection_id, &request).await
}

async fn transfer(
    app: &AppHandle,
    state: &State<'_, ShellState>,
    connection_id: &str,
    request: &ContainerImageTransfer,
    push: bool,
) -> Result<String, ShellError> {
    let (events, lines) = tokio::sync::mpsc::unbounded_channel();
    let (end, ends) = tokio::sync::mpsc::unbounded_channel();
    let stream_id = state
        .core()
        .container_image_transfer_start(connection_id, request, push, events, end)
        .await?;
    forward_stream(app.clone(), lines, ends);
    Ok(stream_id)
}

/// Delivers a stream's lines to the window, and its end after them.
fn forward_stream(
    app: AppHandle,
    mut lines: UnboundedReceiver<ContainerStreamLine>,
    mut end: UnboundedReceiver<ContainerStreamEnd>,
) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                Some(line) = lines.recv() => {
                    if let Err(error) = app.emit("container:stream", line) {
                        log::warn!("a container stream line could not be delivered: {error}");
                    }
                }
                Some(finished) = end.recv() => {
                    // Lines already queued are delivered before the end: telling the interface the
                    // stream is over while the last line is still on its way would cut the tail off
                    // every operation.
                    while let Ok(line) = lines.try_recv() {
                        if let Err(error) = app.emit("container:stream", line) {
                            log::warn!("a container stream line could not be delivered: {error}");
                        }
                    }
                    if let Err(error) = app.emit("container:stream-end", finished) {
                        log::warn!("a container stream end could not be delivered: {error}");
                    }
                    break;
                }
            }
        }
    });
}
