use sevenshell_error::ShellError;
use sevenshell_types::AppSettings;
use tauri::{Manager as _, WebviewWindow};

use crate::window_chrome::{apply_menus_theme, effective_theme, theme_name, window_theme_for};

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
    apply_window_theme(&window, settings.theme_mode.as_str());
    // The two the shell itself obeys, which the window cannot apply by being restyled: whether there
    // is an icon, and what closing does.
    crate::apply_shell_settings(window.app_handle(), &settings);
    Ok(())
}

/// Makes the window's own surfaces (scrollbars, system dialogs) and the platform's menus follow the
/// chosen mode instead of contradicting it.
///
/// Two questions, answered differently on purpose. What the *window* is given is nothing for "follow
/// the system" — a window with a preference of its own stops hearing about the system changing — and
/// the mode itself otherwise. What the *application* is, which is what the menus are set from, is the
/// resolved theme: the platform's menus do not follow a window's theme, and Windows reads its own
/// setting rather than this one's, so a window switched to light kept a dark tray menu.
pub fn apply_window_theme(window: &WebviewWindow, mode: &str) {
    if let Err(error) = window.set_theme(window_theme_for(mode)) {
        log::warn!("the window theme could not be changed: {error}");
    }
    apply_menus_theme(effective_theme(mode, crate::window_chrome::system_theme()));
}

/// Applies the appearance to the surfaces the interface cannot restyle itself.
///
/// The window's chrome and the platform's menus belong to the platform, so they follow when the shell
/// is told — and the shell is otherwise told when the settings are *written*, which is a different
/// moment from when they are *chosen*: a theme clicked in the title bar is applied at once and
/// written when the settings page is left. In between, everything this command covers kept the
/// appearance of the mode before it, and the tray menu does not repaint itself on the next click.
#[tauri::command]
pub fn window_theme_apply(window: WebviewWindow, theme_mode: String) {
    apply_window_theme(&window, &theme_mode);
}

/// What the operating system says the application appearance is, for the interface to follow.
///
/// The interface asks rather than reading the webview's media query: WebView2 answers light whatever
/// the system is, so "follow the system" resolved to light on a machine whose system is dark. `None`
/// means this platform has nothing to say — a system that could not be read, or one of the platforms
/// whose webview follows the system by itself — and the media query is the answer there.
#[tauri::command]
pub fn system_theme() -> Option<&'static str> {
    theme_name(crate::window_chrome::system_theme())
}
