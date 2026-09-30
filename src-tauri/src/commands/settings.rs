use sevenshell_error::ShellError;
use sevenshell_types::AppSettings;
use tauri::WebviewWindow;

use crate::window_chrome::window_theme_for;

/// The non-secret preferences of this installation.
///
/// A missing file is the normal state of a fresh install and yields the defaults.
#[tauri::command]
pub fn settings_get() -> Result<AppSettings, ShellError> {
    sevenshell_platform::load()
}

/// Writes the preferences back and applies the part of them the window itself needs.
///
/// Secrets never travel through here: this file is readable by anyone who can read the user's
/// home directory, and it is treated as such.
#[tauri::command]
pub fn settings_set(window: WebviewWindow, settings: AppSettings) -> Result<(), ShellError> {
    sevenshell_platform::save(&settings)?;
    apply_window_theme(&window, &settings);
    Ok(())
}

/// Makes the window's own surfaces (scrollbars, system dialogs) follow the chosen mode instead
/// of contradicting it. "Follow the system" hands the decision back to the operating system.
pub fn apply_window_theme(window: &WebviewWindow, settings: &AppSettings) {
    if let Err(error) = window.set_theme(window_theme_for(settings.theme_mode.as_str())) {
        log::warn!("the window theme could not be changed: {error}");
    }
}
