//! Saved sessions and the folders they live in.
//!
//! A session carries no secret: it names an authentication method and, for keys, a reference into
//! the vault. The password or passphrase that goes with it arrives separately, on its way in.
//!
//! Export and import are here, but only their outsides: the file is chosen with a native dialog in
//! Rust and read or written in Rust, so the interface is never handed a path. What travels to it is
//! a count and a file name — the same rule the key importer follows, and the reason a script in the
//! webview cannot read a session export or plant one.

use std::time::{SystemTime, UNIX_EPOCH};

use sevenshell_error::ShellError;
use sevenshell_types::{SecretInput, Session, SessionExportSummary, SessionGroup, SessionImportSummary};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::state::ShellState;

#[tauri::command]
pub fn sessions_list(state: State<'_, ShellState>) -> Result<Vec<Session>, ShellError> {
    state.vault().sessions()
}

/// Saves a session, and its secret when one is supplied.
///
/// `secret: None` means "leave whatever is stored alone", which is what the editor sends when the
/// user did not touch the password field: that is the difference between "unchanged" and "blank".
#[tauri::command]
pub fn session_save(
    state: State<'_, ShellState>,
    session: Session,
    secret: Option<SecretInput>,
) -> Result<Session, ShellError> {
    let secret = secret.map(|input| match session.auth.method {
        // A passphrase belongs to the key, and is stored with it during import; a secret sent with
        // a key session is therefore the key's passphrase and is treated as such.
        sevenshell_types::AuthMethod::PublicKey => sevenshell_vault::SessionSecret::KeyPassphrase(input.value),
        _ => sevenshell_vault::SessionSecret::Password(input.value),
    });

    state.vault().save_session(session, secret)
}

#[tauri::command]
pub fn session_delete(state: State<'_, ShellState>, id: String) -> Result<(), ShellError> {
    state.vault().delete_session(&id)
}

#[tauri::command]
pub fn groups_list(state: State<'_, ShellState>) -> Result<Vec<SessionGroup>, ShellError> {
    state.vault().groups()
}

#[tauri::command]
pub fn group_save(state: State<'_, ShellState>, group: SessionGroup) -> Result<SessionGroup, ShellError> {
    state.vault().save_group(group)
}

#[tauri::command]
pub fn group_delete(state: State<'_, ShellState>, id: String) -> Result<(), ShellError> {
    state.vault().delete_group(&id)
}

/// Writes the session library to a file the user picks, secrets only when they asked for them.
///
/// The bundle is built *before* the dialog is opened, so a locked vault is refused without a file
/// picker appearing and disappearing for no reason.
///
/// Returns `Ok(None)` when the dialog was dismissed, which is not a failure: it is somebody
/// changing their mind about where the file should go.
#[tauri::command]
pub async fn sessions_export(
    app: AppHandle,
    state: State<'_, ShellState>,
    title: String,
    file_name: String,
    include_secrets: bool,
) -> Result<Option<SessionExportSummary>, ShellError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let bundle = state.vault().export_bundle(include_secrets, now)?;
    let summary = SessionExportSummary {
        sessions: bundle.sessions.len() as u32,
        groups: bundle.groups.len() as u32,
        secrets: bundle.secrets.len() as u32,
        file_name: file_name.clone(),
    };
    let contents = bundle.to_json()?;

    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title(title)
            .set_file_name(file_name)
            .add_filter("7Shell sessions", &["json"])
            .blocking_save_file()
    })
    .await
    .map_err(|error| ShellError::app_internal(format!("the file dialog could not be opened: {error}")))?;

    let Some(file) = picked else {
        return Ok(None);
    };

    let path = file
        .into_path()
        .map_err(|error| ShellError::session_bundle_io(format!("that file cannot be written to: {error}")))?;
    std::fs::write(&path, contents)
        .map_err(|error| ShellError::session_bundle_io(format!("the file could not be written: {error}")))?;

    Ok(Some(summary))
}

/// Reads a session library out of a file the user picks and adds it to this vault.
///
/// Nothing already here is changed: an id the file shares with a saved session gets a fresh one on
/// the way in. Returns `Ok(None)` when the dialog was dismissed.
#[tauri::command]
pub async fn sessions_import(
    app: AppHandle,
    state: State<'_, ShellState>,
    title: String,
) -> Result<Option<SessionImportSummary>, ShellError> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title(title)
            .add_filter("7Shell sessions", &["json"])
            .blocking_pick_file()
    })
    .await
    .map_err(|error| ShellError::app_internal(format!("the file dialog could not be opened: {error}")))?;

    let Some(file) = picked else {
        return Ok(None);
    };

    let path = file
        .into_path()
        .map_err(|error| ShellError::session_bundle_invalid(format!("that file cannot be read: {error}")))?;
    let metadata = std::fs::metadata(&path)
        .map_err(|error| ShellError::session_bundle_io(format!("the file could not be read: {error}")))?;
    if metadata.len() > sevenshell_vault::MAX_BUNDLE_BYTES {
        return Err(ShellError::session_bundle_invalid(
            "that file is far too large to be a session export",
        ));
    }

    let contents = std::fs::read_to_string(&path)
        .map_err(|error| ShellError::session_bundle_io(format!("the file could not be read: {error}")))?;
    let bundle = sevenshell_vault::SessionBundle::parse(&contents)?;

    state.vault().import_bundle(bundle).map(Some)
}
