//! The vault: creating it, opening it, locking it, changing its password, and the archives of
//! libraries past.
//!
//! Passwords travel *in* through these commands and never come back out. Nothing here returns a
//! secret, and nothing here formats one into an error.
//!
//! Everything about archives is by *name*. A library that could not be opened is renamed beside the
//! live one rather than deleted, and choosing one again is a message carrying that name; which
//! directory the two live in is decided here and never travels, the same way a chosen key file opens
//! in Rust and reaches the interface as a handle.

use std::time::{SystemTime, UNIX_EPOCH};

use sevenshell_error::ShellError;
use sevenshell_types::{SessionImportSummary, VaultArchive, VaultState};
use tauri::State;

use crate::reveal::{self, Show};
use crate::state::ShellState;

fn state_of(vault: &sevenshell_vault::Vault) -> VaultState {
    match vault.status() {
        sevenshell_vault::VaultStatus::Missing => VaultState::Missing,
        sevenshell_vault::VaultStatus::Locked => VaultState::Locked,
        sevenshell_vault::VaultStatus::Unlocked => VaultState::Unlocked,
    }
}

/// Whether a vault exists and whether it is open.
#[tauri::command]
pub fn vault_state(state: State<'_, ShellState>) -> VaultState {
    state_of(state.vault())
}

/// Creates a vault and leaves it unlocked, so the first run is one step rather than two.
#[tauri::command]
pub fn vault_create(state: State<'_, ShellState>, password: String) -> Result<VaultState, ShellError> {
    state.vault().create(&password)?;
    Ok(state_of(state.vault()))
}

#[tauri::command]
pub fn vault_unlock(state: State<'_, ShellState>, password: String) -> Result<VaultState, ShellError> {
    state.vault().unlock(&password)?;
    Ok(state_of(state.vault()))
}

/// Locks the vault. Open terminals are deliberately left alone: they are authenticated already,
/// and closing someone's session because they locked the password store would lose their work.
///
/// What locking means beyond that — including which waiting questions it drops — lives in
/// [`ShellState::lock_vault`], because the tray icon locks the same vault with no interface involved.
#[tauri::command]
pub fn vault_lock(state: State<'_, ShellState>) -> VaultState {
    state.lock_vault();
    state_of(state.vault())
}

#[tauri::command]
pub fn vault_change_password(state: State<'_, ShellState>, current: String, next: String) -> Result<(), ShellError> {
    state.vault().change_password(&current, &next)
}

/// Moves a vault that cannot be opened beside itself, so a new one can be created.
///
/// Answers with the file's new *name* only. The interface is never told where a file lives, and
/// nothing is deleted: the way back is a rename the person can do themselves.
#[tauri::command]
pub fn vault_set_aside(state: State<'_, ShellState>) -> Result<String, ShellError> {
    state.vault().set_aside()
}

/// The archived libraries that sit beside the live one, most recently written first.
#[tauri::command]
pub fn vault_archives(state: State<'_, ShellState>) -> Result<Vec<VaultArchive>, ShellError> {
    state.vault().archives()
}

/// Whether a password opens one archive, without making it the live one.
///
/// A refusal is the ordinary `VAULT-UNLOCK-FAILED`, so the field it was typed in is where the answer
/// belongs — trying a password is not a decision, and nothing about the live vault changes here.
#[tauri::command]
pub fn vault_archive_verify(state: State<'_, ShellState>, name: String, password: String) -> Result<(), ShellError> {
    state.vault().verify_archive(&name, &password)
}

/// Makes an archive the live vault, and archives the library that was live.
///
/// Answers with the name the replaced library was given, or nothing when there was no file in place.
/// Both halves are renames, so the answer is also how the person finds their way back. A question
/// waiting on the old library goes with it, the same way it does when the vault is locked.
#[tauri::command]
pub fn vault_archive_activate(state: State<'_, ShellState>, name: String) -> Result<Option<String>, ShellError> {
    state.pending_import().clear();
    state.pending_restore().clear();
    state.vault().activate_archive(&name)
}

/// Adds everything an archived library holds to the one that is open.
///
/// Requires an open vault: what arrives is added to a library, and a locked one has none to add to.
/// The rules are the ones an imported file follows — nothing already here is replaced — and the
/// counts that come back are what the person is told, including what could not be carried.
#[tauri::command]
pub fn vault_archive_import(
    state: State<'_, ShellState>,
    name: String,
    password: String,
) -> Result<SessionImportSummary, ShellError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();

    state.vault().import_archive(&name, &password, now)
}

/// Opens the folder the vault and its archives live in, in the system's file manager.
///
/// The vault file itself is marked where the platform can mark one, which puts the window on the
/// folder either way. The path is worked out here, so the interface asks for "where they are" rather
/// than naming somewhere: a page that could name a directory could name any directory.
#[tauri::command]
pub fn vault_reveal_folder(state: State<'_, ShellState>) -> Result<(), ShellError> {
    reveal::show(state.vault().path(), Show::Select)
}
