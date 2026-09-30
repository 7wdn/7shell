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
#[tauri::command]
pub fn vault_lock(state: State<'_, ShellState>) -> VaultState {
    state.vault().lock();
    state_of(state.vault())
}

#[tauri::command]
pub fn vault_change_password(state: State<'_, ShellState>, current: String, next: String) -> Result<(), ShellError> {
    state.vault().change_password(&current, &next)
}
