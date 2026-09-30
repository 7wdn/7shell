//! The window's own chrome.
//!
//! On Windows and Linux the window is undecorated: the interface draws the title bar and the
//! window buttons itself, so the top strip of the window belongs to the application rather
//! than to the system. macOS keeps its native chrome in the overlay style configured in
//! `tauri.conf.json` — the traffic lights, their position and their behaviour are the
//! platform's to own, and moving or redrawing them goes wrong in ways users notice.

use tauri::{Manager, Runtime};

/// Whether a platform should give up its native window decorations.
///
/// `target_os` is a parameter rather than `std::env::consts::OS` so the decision can be tested
/// for every platform from any one of them.
pub fn native_decorations_override(target_os: &str) -> Option<bool> {
    match target_os {
        "windows" | "linux" => Some(false),
        _ => None,
    }
}

/// Which theme the window itself should wear for a chosen mode.
///
/// `None` hands the decision back to the operating system, which is what "follow the system"
/// means for native surfaces such as scrollbars and system dialogs.
pub fn window_theme_for(mode: &str) -> Option<tauri::Theme> {
    match mode {
        "dark" => Some(tauri::Theme::Dark),
        "light" => Some(tauri::Theme::Light),
        _ => None,
    }
}

/// Applies the platform's window chrome and shows the window.
///
/// The window is shown here rather than when the interface reports itself loaded: if the
/// frontend fails to start, the user still gets a window showing the failure instead of
/// nothing at all. Losing a frame of paint is the cheaper mistake.
pub fn prepare_main_window<R: Runtime>(app: &tauri::AppHandle<R>) {
    let Some(window) = app.get_webview_window("main") else {
        log::warn!("the main window was not found, so its chrome was left as configured");
        return;
    };

    if let Some(decorations) = native_decorations_override(std::env::consts::OS)
        && let Err(error) = window.set_decorations(decorations)
    {
        log::warn!("the window could not be undecorated: {error}");
    }

    // A test run points the application at a temporary data directory, so it starts by offering to
    // create a vault. Saying so in the title is what keeps that from looking like a wiped vault to
    // whoever is looking at the screen.
    if std::env::var_os(sevenshell_platform::paths::CONFIG_DIR_OVERRIDE).is_some()
        && let Err(error) = window.set_title("7Shell — test data")
    {
        log::warn!("the window title could not be changed: {error}");
    }

    if let Err(error) = window.show() {
        log::warn!("the window could not be shown: {error}");
    }
    if let Err(error) = window.set_focus() {
        log::warn!("the window could not be focused: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_and_linux_draw_their_own_title_bar() {
        assert_eq!(native_decorations_override("windows"), Some(false));
        assert_eq!(native_decorations_override("linux"), Some(false));
    }

    #[test]
    fn anything_unexpected_keeps_its_native_chrome() {
        // macOS owns its traffic lights, and an unknown platform is not a place to guess: a
        // window with no system chrome and no working custom title bar cannot be moved or closed.
        assert_eq!(native_decorations_override("macos"), None);
        assert_eq!(native_decorations_override("freebsd"), None);
    }

    #[test]
    fn following_the_system_leaves_the_window_theme_to_the_system() {
        assert_eq!(window_theme_for("system"), None);
        assert_eq!(window_theme_for("dark"), Some(tauri::Theme::Dark));
        assert_eq!(window_theme_for("light"), Some(tauri::Theme::Light));
        // An identifier from a newer version is treated as "not chosen", not as an error.
        assert_eq!(window_theme_for("midnight-aurora"), None);
    }
}
