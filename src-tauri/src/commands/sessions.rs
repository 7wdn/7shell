//! Saved sessions and the folders they live in.
//!
//! A session carries no secret: it names an authentication method and, for keys, a reference into
//! the vault. The password or passphrase that goes with it arrives separately, on its way in.
//!
//! Export and import are here, but only their outsides: the file is chosen with a native dialog in
//! Rust and read or written in Rust, so the interface is never handed a path. What travels to it is
//! a count and a file name — the same rule the key importer follows, and the reason a script in the
//! webview cannot read a session export or plant one.
//!
//! An import that meets a sealed file stops and asks. The file has already been chosen by then, so
//! where it is waits in [`ShellState`] between the two calls rather than making the round trip: the
//! second call reads the file again from that remembered path, and the interface still never learns
//! one. Read again rather than held in memory for the same reason the vault re-reads nothing it does
//! not have to — and because a file that changed on disk between the two calls is a file the person
//! is entitled to have read.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sevenshell_error::ShellError;
use sevenshell_types::{SecretInput, Session, SessionExportSummary, SessionGroup, SessionImportOutcome};
use sevenshell_vault::{ExportProtection, OpenedBundle};
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
/// The bundle is built *before* the dialog is opened, so a locked vault — or a passphrase too short
/// to write a file under — is refused without a file picker appearing and disappearing for no
/// reason.
///
/// `passphrase` is the whole of what the interface decides about credentials, and the two answers
/// are a rule rather than a preference: given one, the file carries the stored logins sealed; given
/// none, it carries no credential at all. No value of this argument produces a password in the
/// clear, which is the point of it being an `Option<&str>` here and an enum below.
///
/// Returns `Ok(None)` when the dialog was dismissed, which is not a failure: it is somebody
/// changing their mind about where the file should go.
#[tauri::command]
pub async fn sessions_export(
    app: AppHandle,
    state: State<'_, ShellState>,
    title: String,
    file_name: String,
    passphrase: Option<String>,
) -> Result<Option<SessionExportSummary>, ShellError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let protection = match passphrase.as_deref() {
        Some(passphrase) => ExportProtection::Passphrase(passphrase),
        None => ExportProtection::WithoutCredentials,
    };
    let exported = state.vault().export_bundle(protection, now)?;
    let summary = SessionExportSummary {
        sessions: exported.sessions,
        groups: exported.groups,
        secrets: exported.secrets,
        encrypted: exported.encrypted,
        file_name: file_name.clone(),
    };
    let contents = exported.text;

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
/// the way in.
///
/// `passphrase` is `None` on the first call, and that call either imports what it read or answers
/// [`SessionImportOutcome::NeedsPassphrase`] and leaves the file waiting. The second call carries
/// the answer and reads that same file again. A dismissed dialog is `Ok(None)`.
///
/// A wrong passphrase is an error rather than a third outcome, and the file stays waiting: the
/// question is still open, and the dialog that asked it is the one that can say what went wrong.
#[tauri::command]
pub async fn sessions_import(
    app: AppHandle,
    state: State<'_, ShellState>,
    title: String,
    passphrase: Option<String>,
) -> Result<Option<SessionImportOutcome>, ShellError> {
    let path = match passphrase {
        Some(passphrase) => {
            let Some(path) = state.pending_import().path() else {
                return Err(ShellError::app_internal(
                    "there is no import waiting for a passphrase; start the import again",
                ));
            };
            return import_from(&state, path, Some(&passphrase));
        }
        None => pick_file(app, title).await?,
    };

    let Some(path) = path else {
        return Ok(None);
    };

    // Remembered before it is read: whether the file is sealed is only known once it has been
    // parsed, and the answer to that is what decides whether a question follows.
    state.pending_import().remember(path.clone());
    import_from(&state, path, None)
}

/// Asks the platform for a file, in Rust. `Ok(None)` is somebody closing the dialog.
async fn pick_file(app: AppHandle, title: String) -> Result<Option<PathBuf>, ShellError> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title(title)
            .add_filter("7Shell sessions", &["json"])
            .blocking_pick_file()
    })
    .await
    .map_err(|error| ShellError::app_internal(format!("the file dialog could not be opened: {error}")))?;

    match picked {
        Some(file) => file
            .into_path()
            .map(Some)
            .map_err(|error| ShellError::session_bundle_invalid(format!("that file cannot be read: {error}"))),
        None => Ok(None),
    }
}

/// Reads a chosen file — for a second time, when the first one stopped to ask — and decides whether
/// it is still waiting afterwards.
///
/// One exit for the pending slot, and it is here rather than on each path: the file is still waiting
/// only when a question is still open, and everything else — imported, refused, unreadable — is
/// done with it. Leaving a path behind on a refusal would be a question nobody can see.
fn import_from(
    state: &ShellState,
    path: PathBuf,
    passphrase: Option<&str>,
) -> Result<Option<SessionImportOutcome>, ShellError> {
    let outcome = read_and_import(state, path, passphrase);

    if !matches!(outcome, Ok(Some(SessionImportOutcome::NeedsPassphrase { .. }))) {
        state.pending_import().clear();
    }
    outcome
}

/// What the file holds, and what was done with it.
///
/// The size limit is checked from the file's own metadata, before a byte of it is read: a mistyped
/// path that lands on a disk image should be refused, not loaded.
fn read_and_import(
    state: &ShellState,
    path: PathBuf,
    passphrase: Option<&str>,
) -> Result<Option<SessionImportOutcome>, ShellError> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();

    let metadata = std::fs::metadata(&path)
        .map_err(|error| ShellError::session_bundle_io(format!("the file could not be read: {error}")))?;
    if metadata.len() > sevenshell_vault::MAX_BUNDLE_BYTES {
        return Err(ShellError::session_bundle_invalid(
            "that file is far too large to be a session export",
        ));
    }

    let contents = std::fs::read_to_string(&path)
        .map_err(|error| ShellError::session_bundle_io(format!("the file could not be read: {error}")))?;

    let bundle = match sevenshell_vault::open(&contents)? {
        OpenedBundle::Plain(bundle) => *bundle,
        OpenedBundle::Sealed(envelope) => match passphrase {
            Some(passphrase) => envelope.unseal(passphrase)?,
            // Still waiting: a wrong passphrase on the next call needs the same file.
            None => return Ok(Some(SessionImportOutcome::NeedsPassphrase { file_name })),
        },
    };

    state
        .vault()
        .import_bundle(bundle)
        .map(|summary| Some(SessionImportOutcome::Imported { summary }))
}
