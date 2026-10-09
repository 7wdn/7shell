//! The vault: creating it, opening it, locking it, and changing its password.
//!
//! Passwords travel *in* through these commands and never come back out. Nothing here returns a
//! secret, and nothing here formats one into an error.

use sevenshell_error::ShellError;
use sevenshell_types::VaultState;
use tauri::State;

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
/// An import that was waiting for a passphrase is dropped with it. Nothing about the path is secret,
/// but a question that can no longer be answered — the next call would write into a locked vault —
/// should not be left standing.
#[tauri::command]
pub fn vault_lock(state: State<'_, ShellState>) -> VaultState {
    state.pending_import().clear();
    state.vault().lock();
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
