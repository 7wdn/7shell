//! The window's own chrome.
//!
//! On Windows and Linux the window is undecorated: the interface draws the title bar and the
//! window buttons itself, so the top strip of the window belongs to the application rather
//! than to the system. macOS keeps its native chrome in the overlay style configured in
//! `tauri.conf.json` — the traffic lights, their position and their behaviour are the
//! platform's to own, and moving or redrawing them goes wrong in ways users notice.
//!
//! The window is also created hidden (`visible: false` in `tauri.conf.json`) and revealed once the
//! interface has a frame to show. A webview that has not painted yet is an opaque white rectangle:
//! that is the browser control's own default background, which no stylesheet can reach, because the
//! stylesheet arrives with the very document that has not been painted. A window shown at creation
//! therefore spends its first few hundred milliseconds looking like a broken application. The
//! reveal is the interface's call to [`reveal_main_window`], and the deadline below is what keeps a
//! frontend that never gets that far from leaving a process with no window at all.

use std::time::Duration;

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

/// How long the window waits for the interface to report a drawn frame before it is shown anyway.
///
/// Long enough for a page load on a slow machine, short enough that a person expecting a window is
/// not left looking at nothing. It is the answer to the one hole that hiding the startup opens: a
/// frontend that fails to start would otherwise leave a process with nothing on screen to close.
pub const REVEAL_DEADLINE: Duration = Duration::from_secs(3);

/// Whether the deadline should put the window on screen.
///
/// Only a window nobody has shown yet: showing one that is already up would also take the focus back
/// from whatever the person switched to while the application was starting.
pub fn reveal_on_deadline(visible: bool) -> bool {
    !visible
}

/// Applies the platform's window chrome and arms the deadline that reveals the window.
///
/// The window is not shown here. It is shown when the interface says it has a frame, or by the
/// deadline when the interface never says anything at all.
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

    arm_reveal_deadline(&window);
}

/// Shows the window and gives it the focus: the interface has a frame on screen, or waited long
/// enough that it is not coming.
pub fn reveal_main_window<R: Runtime>(window: &tauri::WebviewWindow<R>) {
    if let Err(error) = window.show() {
        log::warn!("the window could not be shown: {error}");
    }
    if let Err(error) = window.set_focus() {
        log::warn!("the window could not be focused: {error}");
    }
}

/// Shows the window if the interface never reports a drawn frame.
fn arm_reveal_deadline<R: Runtime>(window: &tauri::WebviewWindow<R>) {
    let window = window.clone();

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(REVEAL_DEADLINE).await;

        // A window whose visibility cannot be read is treated as hidden: failing towards a window
        // somebody can see and close is the whole point of this deadline.
        let visible = window.is_visible().unwrap_or_else(|error| {
            log::warn!("the window's visibility could not be read: {error}");
            false
        });

        if reveal_on_deadline(visible) {
            log::warn!(
                "the interface did not report a drawn frame within {REVEAL_DEADLINE:?}, so the window is shown without one"
            );
            reveal_main_window(&window);
        }
    });
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

    #[test]
    fn the_deadline_shows_a_window_that_nothing_has_shown_yet() {
        assert!(reveal_on_deadline(false));
    }

    #[test]
    fn the_deadline_leaves_a_window_the_interface_already_showed_alone() {
        // Showing an already visible window would take the focus back from whatever the person
        // switched to while the application was starting.
        assert!(!reveal_on_deadline(true));
    }
}
