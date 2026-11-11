use sevenshell_error::ShellError;
use sevenshell_types::AppSettings;
use tauri::{Manager as _, State, WebviewWindow};

use crate::state::ShellState;
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
pub async fn settings_set(
    window: WebviewWindow,
    state: State<'_, ShellState>,
    settings: AppSettings,
) -> Result<(), ShellError> {
    // Read before the write: the listener restarts only when the *listener's* settings changed, and
    // a font or a window width must not interrupt a call an agent is in the middle of.
    let previous = sevenshell_platform::load().unwrap_or_default();

    sevenshell_platform::save(&settings)?;
    apply_window_theme(&window, settings.theme_mode.as_str());
    // The two the shell itself obeys, which the window cannot apply by being restyled: whether there
    // is an icon, and what closing does.
    crate::apply_shell_settings(window.app_handle(), &settings);

    // The external servers are a list in this same file, and the settings page is not the only place
    // it can change (the server commands write it too). Compared rather than applied every time: a
    // save that touched the font must not reconnect five servers, and `replace` keeps the sessions
    // of entries that did not move — but the comparison is what keeps the emit quiet as well. The
    // result bound travels with the same apply because it, too, belongs to the manager.
    if (previous.mcp_client_servers != settings.mcp_client_servers
        || previous.mcp_max_result_bytes != settings.mcp_max_result_bytes)
        && let Err(error) = state.mcp_client().apply_and_refresh(&settings)
    {
        // The settings page has already been written, so the refusal is the manager's: say what it
        // is and leave the file alone rather than reverting somebody's other preferences.
        log::warn!("the external MCP servers could not be updated from the settings page: {error}");
    }

    // A failure to bind is returned rather than hidden: the settings page asked for a listener and
    // has to be told there is none, even though the preference itself was saved.
    crate::mcp::apply_settings(window.app_handle(), state.mcp(), &previous, &settings).await?;
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
