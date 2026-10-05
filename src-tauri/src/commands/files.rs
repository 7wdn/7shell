//! Remote files: the browser's commands, and the dialogs that choose local paths.
//!
//! A download and an upload both need a local path, and both get it from a native dialog that runs
//! here. The interface names neither: it asks for a file to be saved or sent, and receives the id
//! of the transfer that does it — the same rule that keeps a script in a webview from choosing
//! where somebody's files land.
//!
//! An upload is the one that can replace something, so it looks before it sends: the names on the
//! far side are checked first, and a name that is taken comes back as a question rather than as a
//! transfer. The files that were chosen wait in `uploads` while the question is answered.

use std::path::PathBuf;

use sevenshell_error::ShellError;
use sevenshell_types::{EntryKind, SftpListing, SftpOutcome, TransferInfo, UploadAnswer, UploadOutcome};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::state::ShellState;
use crate::uploads::PendingUpload;

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

/// Creates an empty file, refused with `SFTP-EXISTS` when the name is already taken.
#[tauri::command]
pub async fn files_create_file(state: State<'_, ShellState>, handle: String, path: String) -> Result<(), ShellError> {
    state.core().create_file(&handle, &path).await
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

/// Asks where to put the chosen remote entries, then copies them there.
///
/// Answers with the transfer ids, or none at all when a dialog was dismissed — which is a person
/// changing their mind, not a failure.
///
/// The question follows from what was chosen, and that is why it is asked here rather than in the
/// interface: one file is asked for by name (a save dialog), several rows are asked for as a folder
/// to put them in, and a packed selection is asked for as a folder to open the archive into. A host
/// cannot be asked any of those, so this is the side that can.
#[tauri::command]
pub async fn files_download(
    app: AppHandle,
    state: State<'_, ShellState>,
    handle: String,
    paths: Vec<String>,
    title: String,
    suggested_name: String,
    packed: bool,
) -> Result<Vec<String>, ShellError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }

    // One archive, opened where somebody says: directories, files, or a mixture of the two.
    if packed || paths.len() > 1 {
        let chosen = spawn_dialog(move || app.dialog().file().set_title(title).blocking_pick_folder()).await?;
        let Some(chosen) = chosen else {
            return Ok(Vec::new());
        };
        let into: PathBuf = chosen.into_path().map_err(|error| {
            ShellError::sftp_transfer_failed(&paths[0], format!("that folder cannot be written to: {error}"))
        })?;

        if packed {
            return state
                .core()
                .start_packed_download(&handle, &paths, into)
                .await
                .map(|id| vec![id]);
        }

        // Several files, unpacked: one folder, one transfer each. A directory is not among them —
        // the interface only offers this when everything chosen is a file, because a directory has no
        // unpacked way down.
        let mut started = Vec::with_capacity(paths.len());
        for path in &paths {
            started.push(state.core().start_download_into(&handle, path, &into).await?);
        }
        return Ok(started);
    }

    let destination = spawn_dialog(move || {
        app.dialog()
            .file()
            .set_title(title)
            .set_file_name(suggested_name)
            .blocking_save_file()
    })
    .await?;

    let Some(destination) = destination else {
        return Ok(Vec::new());
    };

    let local: PathBuf = destination.into_path().map_err(|error| {
        ShellError::sftp_transfer_failed(&paths[0], format!("that location cannot be written: {error}"))
    })?;

    state
        .core()
        .start_download(&handle, &paths[0], local)
        .await
        .map(|id| vec![id])
}

/// Asks which local files to send, then copies them into a remote directory.
///
/// With `packed` set the dialog offers *several* files and what crosses the wire is one archive made
/// from them on this side — which is the whole point of the mode, and the reason the question it asks
/// is a different one. The far side unpacks it, so what lands there is the files themselves.
#[tauri::command]
pub async fn files_upload(
    app: AppHandle,
    state: State<'_, ShellState>,
    handle: String,
    directory: String,
    title: String,
    packed: bool,
) -> Result<UploadOutcome, ShellError> {
    if packed {
        let chosen = spawn_dialog(move || app.dialog().file().set_title(title).blocking_pick_files()).await?;
        let Some(chosen) = chosen else {
            return Ok(UploadOutcome::Cancelled);
        };

        let mut sources: Vec<PathBuf> = Vec::with_capacity(chosen.len());
        for entry in chosen {
            sources.push(entry.into_path().map_err(|error| {
                ShellError::sftp_transfer_failed(&directory, format!("that file cannot be read: {error}"))
            })?);
        }

        return start_or_ask(&state, handle, directory, sources, true).await;
    }

    let chosen = spawn_dialog(move || app.dialog().file().set_title(title).blocking_pick_file()).await?;

    let Some(chosen) = chosen else {
        return Ok(UploadOutcome::Cancelled);
    };

    let local: PathBuf = chosen
        .into_path()
        .map_err(|error| ShellError::sftp_transfer_failed(&directory, format!("that file cannot be read: {error}")))?;

    start_or_ask(&state, handle, directory, vec![local], false).await
}

/// The answer to the question an upload asked about the names already there.
#[tauri::command]
pub async fn files_upload_answer(
    state: State<'_, ShellState>,
    token: String,
    answer: UploadAnswer,
) -> Result<UploadOutcome, ShellError> {
    let upload = state.pending_uploads().take(&token).await?;

    let sources = match answer {
        UploadAnswer::Overwrite => upload.sources.clone(),
        UploadAnswer::Skip => upload.missing(),
        // Nothing is sent, and the files that were waiting are forgotten with the token.
        UploadAnswer::Cancel => return Ok(UploadOutcome::Cancelled),
    };

    if sources.is_empty() {
        return Ok(UploadOutcome::Cancelled);
    }

    start_uploads(&state, &upload.handle, &upload.directory, &sources, upload.packed).await
}

/// Sends the files, or stops and comes back with the names that are already there.
///
/// The check is a look at names rather than at contents, so it can say "this one would be replaced"
/// without having read anything: what is on the far side is only a decision for whoever is sending.
async fn start_or_ask(
    state: &ShellState,
    handle: String,
    directory: String,
    sources: Vec<PathBuf>,
    packed: bool,
) -> Result<UploadOutcome, ShellError> {
    if sources.is_empty() {
        return Ok(UploadOutcome::Cancelled);
    }

    let taken = state.core().upload_names_taken(&handle, &directory, &sources).await?;
    if taken.is_empty() {
        return start_uploads(state, &handle, &directory, &sources, packed).await;
    }

    // A single file has nothing to leave out — the answer is yes or no — and an archive is unpacked
    // as a whole, so there is nothing to skip inside it either.
    let skippable = !packed && sources.len() > 1;
    let token = state
        .pending_uploads()
        .remember(PendingUpload {
            handle,
            directory,
            sources,
            packed,
            taken: taken.clone(),
        })
        .await;

    Ok(UploadOutcome::Conflict {
        token,
        names: taken,
        skippable,
    })
}

/// Starts the transfers themselves: one per file, or one archive for a batch that travels packed.
async fn start_uploads(
    state: &ShellState,
    handle: &str,
    directory: &str,
    sources: &[PathBuf],
    packed: bool,
) -> Result<UploadOutcome, ShellError> {
    if packed {
        let id = state
            .core()
            .start_packed_upload(handle, sources.to_vec(), directory)
            .await?;
        return Ok(UploadOutcome::Started { ids: vec![id] });
    }

    let mut ids = Vec::with_capacity(sources.len());
    for source in sources {
        ids.push(state.core().start_upload(handle, source.clone(), directory).await?);
    }
    Ok(UploadOutcome::Started { ids })
}

/// Uploads files the operating system handed us — a drop, or a paste — into a directory.
///
/// The token stands for paths the interface never saw, and it is good for one upload. One transfer
/// per file: they are separate files, and a failure in one does not take the others down with it —
/// the ones already started stay in the transfer list, and the error that stopped the rest is
/// returned.
///
/// With `packed` set they go as one archive instead, made here: a folder dragged in from the desktop
/// is a batch like any other, and the switch means the same thing at every door. One transfer comes
/// back rather than one per file — which is the whole reason a batch is packed.
#[tauri::command]
pub async fn dropped_upload(
    state: State<'_, ShellState>,
    handle: String,
    directory: String,
    token: String,
    packed: bool,
) -> Result<UploadOutcome, ShellError> {
    let paths = state.pending_drops().take(&token).await?;
    start_or_ask(&state, handle, directory, paths, packed).await
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
