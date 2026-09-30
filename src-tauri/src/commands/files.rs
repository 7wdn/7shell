//! Remote files: the browser's commands, and the dialogs that choose local paths.
//!
//! A download and an upload both need a local path, and both get it from a native dialog that runs
//! here. The interface names neither: it asks for a file to be saved or sent, and receives the id
//! of the transfer that does it — the same rule that keeps a script in a webview from choosing
//! where somebody's files land.

use std::path::PathBuf;

use sevenshell_error::ShellError;
use sevenshell_types::{EntryKind, SftpListing, SftpOutcome, TransferInfo};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::state::ShellState;

/// Opens a file browser for a saved session.
#[tauri::command]
pub async fn files_open(state: State<'_, ShellState>, session_id: String) -> Result<SftpOutcome, ShellError> {
    state.core().open_files(&session_id).await
}

/// Reads a directory.
#[tauri::command]
pub async fn files_list(state: State<'_, ShellState>, handle: String, path: String) -> Result<SftpListing, ShellError> {
    state.core().list_files(&handle, &path).await
}

#[tauri::command]
pub async fn files_create_folder(state: State<'_, ShellState>, handle: String, path: String) -> Result<(), ShellError> {
    state.core().create_folder(&handle, &path).await
}

#[tauri::command]
pub async fn files_rename(
    state: State<'_, ShellState>,
    handle: String,
    from: String,
    to: String,
) -> Result<(), ShellError> {
    state.core().rename_entry(&handle, &from, &to).await
}

/// Deletes a file, or a directory and everything under it.
///
/// The kind is what the browser listed, so a directory a person right-clicked is deleted as a
/// directory even if something replaced it in between.
#[tauri::command]
pub async fn files_delete(
    state: State<'_, ShellState>,
    handle: String,
    path: String,
    kind: EntryKind,
) -> Result<(), ShellError> {
    state.core().delete_entry(&handle, &path, kind).await
}

/// Reads a text file for the editor.
#[tauri::command]
pub async fn files_read_text(state: State<'_, ShellState>, handle: String, path: String) -> Result<String, ShellError> {
    state.core().read_text_file(&handle, &path).await
}

#[tauri::command]
pub async fn files_write_text(
    state: State<'_, ShellState>,
    handle: String,
    path: String,
    text: String,
) -> Result<(), ShellError> {
    state.core().write_text_file(&handle, &path, &text).await
}

/// Closes a file browser, and cancels whatever it was still moving.
#[tauri::command]
pub async fn files_close(state: State<'_, ShellState>, handle: String) -> Result<(), ShellError> {
    state.core().close_files(&handle).await
}

/// The transfers that are running or have run.
#[tauri::command]
pub async fn transfers_open(state: State<'_, ShellState>) -> Result<Vec<TransferInfo>, ShellError> {
    Ok(state.core().transfers().await)
}

#[tauri::command]
pub async fn transfer_cancel(state: State<'_, ShellState>, id: String) -> Result<(), ShellError> {
    state.core().cancel_transfer(&id).await
}

/// Asks where to put a remote file, then copies it there.
///
/// Answers with the transfer's id, or `None` when the dialog was dismissed — which is a person
/// changing their mind, not a failure.
#[tauri::command]
pub async fn files_download(
    app: AppHandle,
    state: State<'_, ShellState>,
    handle: String,
    path: String,
    title: String,
    suggested_name: String,
) -> Result<Option<String>, ShellError> {
    let destination = spawn_dialog(move || {
        app.dialog()
            .file()
            .set_title(title)
            .set_file_name(suggested_name)
            .blocking_save_file()
    })
    .await?;

    let Some(destination) = destination else {
        return Ok(None);
    };

    let local: PathBuf = destination.into_path().map_err(|error| {
        ShellError::sftp_transfer_failed(&path, format!("that location cannot be written: {error}"))
    })?;

    state.core().start_download(&handle, &path, local).await.map(Some)
}

/// Asks which local file to send, then copies it into a remote directory.
#[tauri::command]
pub async fn files_upload(
    app: AppHandle,
    state: State<'_, ShellState>,
    handle: String,
    directory: String,
    title: String,
) -> Result<Option<String>, ShellError> {
    let chosen = spawn_dialog(move || app.dialog().file().set_title(title).blocking_pick_file()).await?;

    let Some(chosen) = chosen else {
        return Ok(None);
    };

    let local: PathBuf = chosen
        .into_path()
        .map_err(|error| ShellError::sftp_transfer_failed(&directory, format!("that file cannot be read: {error}")))?;

    state.core().start_upload(&handle, local, &directory).await.map(Some)
}

/// Uploads files the operating system handed us — a drop, or a paste — into a directory.
///
/// The token stands for paths the interface never saw, and it is good for one upload. One transfer
/// per file: they are separate files, and a failure in one does not take the others down with it —
/// the ones already started stay in the transfer list, and the error that stopped the rest is
/// returned.
#[tauri::command]
pub async fn dropped_upload(
    state: State<'_, ShellState>,
    handle: String,
    directory: String,
    token: String,
) -> Result<Vec<String>, ShellError> {
    let paths = state.pending_drops().take(&token).await?;

    let mut started = Vec::with_capacity(paths.len());
    for path in paths {
        started.push(state.core().start_upload(&handle, path, &directory).await?);
    }

    Ok(started)
}

/// The file list on the clipboard, as a token and the files' names.
///
/// Windows is the only platform this build can read that from, and the other ones say so instead
/// of pretending: `APP-UNSUPPORTED` names what is missing, which is what the interface shows.
#[tauri::command]
pub fn clipboard_files(state: State<'_, ShellState>) -> Result<crate::dropped::PendingFiles, ShellError> {
    let paths = crate::dropped::clipboard_files()?;
    state.pending_drops().remember(paths)
}

/// Runs a blocking file dialog off the async runtime's threads.
///
/// The dialog's own API blocks the thread it is called on, and a command may be running on one of
/// the runtime's — blocking those is how a UI stops responding.
async fn spawn_dialog<T, F>(dialog: F) -> Result<T, ShellError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(dialog)
        .await
        .map_err(|error| ShellError::app_internal(format!("the file dialog could not be opened: {error}")))
}
