//! The places on this machine an agent may name, picked in a native dialog.
//!
//! The dialog runs in Rust, and what comes back to the interface is a token and a display name. The
//! path stays in the core for as long as the application runs; nothing on the wire, in an agent's
//! prompt or in an audit line is a path.

use sevenshell_error::ShellError;
use sevenshell_types::{HostLocationKind, HostLocationView};
use tauri::{State, WebviewWindow};
use tauri_plugin_dialog::DialogExt as _;

use crate::state::ShellState;

/// Opens the system's picker and records what it returned.
///
/// `Ok(None)` is somebody closing the picker without choosing: not a failure, and not a change.
#[tauri::command]
pub async fn host_location_choose(
    window: WebviewWindow,
    state: State<'_, ShellState>,
    kind: HostLocationKind,
    title: String,
) -> Result<Option<HostLocationView>, ShellError> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let dialog = window.dialog().file().set_title(title);
        match kind {
            HostLocationKind::File => dialog.blocking_pick_file(),
            HostLocationKind::Folder => dialog.blocking_pick_folder(),
        }
    })
    .await
    .map_err(|error| ShellError::app_internal(format!("the picker could not be opened: {error}")))?;

    let Some(chosen) = picked else {
        return Ok(None);
    };
    let path = chosen
        .into_path()
        .map_err(|error| ShellError::app_internal(format!("the chosen location cannot be read: {error}")))?;
    state.core().host_location_add(path, kind).map(Some)
}

#[tauri::command]
pub fn host_locations_list(state: State<'_, ShellState>) -> Result<Vec<HostLocationView>, ShellError> {
    state.core().host_locations()
}

#[tauri::command]
pub fn host_location_forget(state: State<'_, ShellState>, token: String) -> Result<bool, ShellError> {
    state.core().host_location_forget(&token)
}
