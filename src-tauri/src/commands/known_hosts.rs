//! Trusted host keys.
//!
//! Listing them and forgetting them is how a person recovers from a server that was rebuilt, so
//! both are first-class operations rather than something hidden behind editing a file.

use sevenshell_error::ShellError;
use sevenshell_types::{KnownHostEntry, PresentedHostKey};
use tauri::State;

use crate::state::ShellState;

#[tauri::command]
pub fn known_hosts_list(state: State<'_, ShellState>) -> Result<Vec<KnownHostEntry>, ShellError> {
    state.vault().known_hosts()
}

/// Records the key a person has just accepted, which is what makes the next connection quiet.
#[tauri::command]
pub fn known_host_trust(state: State<'_, ShellState>, key: PresentedHostKey) -> Result<(), ShellError> {
    state.core().trust_host_key(&key)
}

/// Forgets the trusted key for a host, so the next connection asks again.
#[tauri::command]
pub fn known_host_forget(state: State<'_, ShellState>, host: String, port: u16) -> Result<(), ShellError> {
    state.vault().forget_host(&host, port)
}
