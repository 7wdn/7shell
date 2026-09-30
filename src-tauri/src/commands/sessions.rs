//! Saved sessions and the folders they live in.
//!
//! A session carries no secret: it names an authentication method and, for keys, a reference into
//! the vault. The password or passphrase that goes with it arrives separately, on its way in.

use sevenshell_error::ShellError;
use sevenshell_types::{SecretInput, Session, SessionGroup};
use tauri::State;

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
