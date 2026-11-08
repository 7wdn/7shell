//! Remote desktops: handed to the system client on Windows.
//!
//! The protocol does not live in this process; `Core::open_desktop` writes a temporary `.rdp` file
//! and starts the Windows client. Nothing pixel-shaped crosses this command surface.

use sevenshell_error::ShellError;
use tauri::State;

use crate::state::ShellState;

#[tauri::command]
pub async fn desktop_open(state: State<'_, ShellState>, session_id: String) -> Result<(), ShellError> {
    state.core().open_desktop(&session_id).await
}
