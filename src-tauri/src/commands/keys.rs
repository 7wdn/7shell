//! Importing a private key.
//!
//! The key file is chosen with a native dialog that runs here, in Rust, and is read here. The
//! interface never sees a path and never sees the key: it receives a reference, a file name, an
//! algorithm and a fingerprint — enough to show what is stored, and nothing that would let a
//! script in the webview use it.

use sevenshell_error::ShellError;
use sevenshell_types::KeyInfo;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::state::ShellState;

/// A key file larger than this is not a key.
const MAX_KEY_BYTES: u64 = 1024 * 1024;

/// Asks for a key file, reads it, and stores it in the vault.
///
/// Returns `Ok(None)` when the dialog was dismissed, which is not a failure: it is a person
/// changing their mind.
#[tauri::command]
pub async fn key_import(
    app: AppHandle,
    state: State<'_, ShellState>,
    title: String,
    passphrase: Option<String>,
) -> Result<Option<KeyInfo>, ShellError> {
    let picked =
        tauri::async_runtime::spawn_blocking(move || app.dialog().file().set_title(title).blocking_pick_file())
            .await
            .map_err(|error| ShellError::app_internal(format!("the file dialog could not be opened: {error}")))?;

    let Some(file) = picked else {
        return Ok(None);
    };

    let path = file
        .into_path()
        .map_err(|error| ShellError::ssh_key_invalid(format!("that file cannot be read as a path: {error}")))?;

    let metadata = std::fs::metadata(&path)
        .map_err(|error| ShellError::ssh_key_invalid(format!("the key file could not be read: {error}")))?;
    if metadata.len() > MAX_KEY_BYTES {
        return Err(ShellError::ssh_key_invalid(
            "that file is far too large to be a private key",
        ));
    }

    let contents = std::fs::read_to_string(&path)
        .map_err(|error| ShellError::ssh_key_invalid(format!("the key file could not be read: {error}")))?;

    let display_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "key".to_owned());

    // Parsing here means a bad file is refused before anything is written to the vault.
    let description = sevenshell_ssh::describe_key(&contents, passphrase.as_deref())?;

    let info = KeyInfo {
        key_ref: None,
        display_name,
        algorithm: description.algorithm,
        fingerprint: description.fingerprint,
        has_passphrase: passphrase.is_some() || description.has_passphrase,
    };

    state.vault().import_key(contents, passphrase, info).map(Some)
}

/// Removes a key that was imported and is no longer wanted.
#[tauri::command]
pub fn key_forget(state: State<'_, ShellState>, key_ref: String) -> Result<(), ShellError> {
    state.vault().forget_key(&key_ref)
}
