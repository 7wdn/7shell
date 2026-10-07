//! The font families this machine has, for the two settings that choose one.
//!
//! Read on demand rather than at start-up: the list is three hundred names on an ordinary Windows
//! machine, and most runs of this application never open the picker that shows them.

use sevenshell_error::ShellError;

/// Every family this machine has, sorted. An empty list is a machine whose fonts could not be read,
/// which is not an error: the picker still offers the families that ship with the application and
/// still accepts a name typed into it.
///
/// Reading the database means reading several hundred font files, so it runs off the main thread —
/// the interface asks for this while a dropdown is opening, and a window that stops drawing for a
/// third of a second at that moment is a window that looks broken.
#[tauri::command]
pub async fn system_fonts_list() -> Result<Vec<String>, ShellError> {
    tauri::async_runtime::spawn_blocking(sevenshell_platform::list_families)
        .await
        .map_err(|error| ShellError::app_internal(format!("the font list could not be read: {error}")))
}
