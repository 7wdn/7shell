//! Keys: importing one, making one, and what the interface is told about either.
//!
//! A key file is chosen with a native dialog that runs here, in Rust, and is read here. The
//! interface never sees a path and never sees key material: it receives a reference, a name, an
//! algorithm and a fingerprint — enough to show what is stored, and nothing that would let a script
//! in the webview use it.
//!
//! The one key-shaped thing that does cross that line is a **public** key, and only ever that: it is
//! the half meant to be copied onto machines, and handing it over is the whole of "export the public
//! key". The private half is written nowhere but the vault.

use sevenshell_error::ShellError;
use sevenshell_types::{CreatedKey, KeyAlgorithm, KeyInfo};
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

/// Makes a key, keeps it in the vault, and hands back the line that goes on a server.
///
/// The private half is written nowhere but the vault — it is not offered as a file, because this
/// application is the thing that uses it — and what comes back is the public line, which is the half
/// somebody pastes into `authorized_keys`. A passphrase, when one is given, protects the private half
/// wherever it is stored and is kept beside the key so that a session can use it.
#[tauri::command]
pub async fn key_generate(
    state: State<'_, ShellState>,
    algorithm: KeyAlgorithm,
    comment: String,
    passphrase: Option<String>,
) -> Result<CreatedKey, ShellError> {
    let stored = passphrase.clone();
    let making = passphrase;

    // Making a key is arithmetic, and for RSA it is seconds of it: the window has to keep drawing
    // while this runs, so it runs on a thread of its own.
    let generated = tauri::async_runtime::spawn_blocking(move || {
        sevenshell_ssh::generate(algorithm, comment.trim(), making.as_deref())
    })
    .await
    .map_err(|error| ShellError::app_internal(format!("the key could not be generated: {error}")))??;

    let description = generated.description().clone();
    let info = KeyInfo {
        key_ref: None,
        display_name: next_name(&state, algorithm)?,
        algorithm: description.algorithm,
        fingerprint: description.fingerprint,
        has_passphrase: description.has_passphrase,
    };

    let info = state
        .vault()
        .import_key(generated.private_text().to_owned(), stored, info)?;

    Ok(CreatedKey {
        info,
        public_key: generated.public_line().to_owned(),
    })
}

/// The `authorized_keys` line for a key the vault holds, so it can be put on a server.
///
/// A public key is not a secret — copying it onto machines is what it is for — which is why this is
/// the one direction key material travels outwards, and why it is a line rather than a file.
#[tauri::command]
pub fn key_public(state: State<'_, ShellState>, key_ref: String) -> Result<String, ShellError> {
    let material = state
        .vault()
        .key_material(&key_ref)?
        .ok_or_else(|| ShellError::ssh_key_invalid("that key is no longer stored"))?;

    sevenshell_ssh::public_line(&material.contents, material.passphrase.as_deref())
}

/// What to call a key that was just made: the name OpenSSH would have given the file.
///
/// `id_ed25519` and its siblings are the names a person recognises, and two keys of one kind are
/// told apart by a number rather than by a fingerprint nobody reads twice.
fn next_name(state: &ShellState, algorithm: KeyAlgorithm) -> Result<String, ShellError> {
    let taken: Vec<String> = state.vault().keys()?.into_iter().map(|key| key.display_name).collect();
    let base = format!("id_{}", algorithm.as_str());

    if !taken.contains(&base) {
        return Ok(base);
    }

    let mut number = 2;
    loop {
        let candidate = format!("{base}-{number}");
        if !taken.contains(&candidate) {
            return Ok(candidate);
        }
        number += 1;
    }
}

/// Removes a key that was imported and is no longer wanted.
#[tauri::command]
pub fn key_forget(state: State<'_, ShellState>, key_ref: String) -> Result<(), ShellError> {
    state.vault().forget_key(&key_ref)
}

/// The imported keys, for the page that lists them.
///
/// The fingerprints and the names, never the material: what travels out of the vault about a key is
/// what identifies it, and that is all this returns.
#[tauri::command]
pub fn keys_list(state: State<'_, ShellState>) -> Result<Vec<KeyInfo>, ShellError> {
    state.core().keys()
}
