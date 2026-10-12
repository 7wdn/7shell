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

use tauri::{Emitter, Manager, Runtime};

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
/// means for native surfaces such as scrollbars and system dialogs. Deliberately still `None` for
/// that mode even though [`effective_theme`] can now say what the system is: a window with a
/// preference of its own stops being told when the system changes (the platform skips its
/// re-evaluation for a window that has been given a theme), and that notification is how the
/// interface hears about a system that changed while it was following it.
pub fn window_theme_for(mode: &str) -> Option<tauri::Theme> {
    match mode {
        "dark" => Some(tauri::Theme::Dark),
        "light" => Some(tauri::Theme::Light),
        _ => None,
    }
}

/// What the application *is*, given a chosen mode and what the system says.
///
/// The difference from [`window_theme_for`] is the "follow the system" mode: that one resolves to
/// the system's own answer here, and to nothing there. Both are needed, and they answer two
/// different questions — what to force on the window, and what the application is showing.
///
/// A mode this build does not know — an identifier from a newer version — resolves like "follow the
/// system" rather than contradicting it, which is what "not chosen" has always meant for a mode.
pub fn effective_theme(mode: &str, system: Option<tauri::Theme>) -> Option<tauri::Theme> {
    match mode {
        "dark" => Some(tauri::Theme::Dark),
        "light" => Some(tauri::Theme::Light),
        _ => system,
    }
}

/// What the operating system says the application appearance is, where this shell can find out.
///
/// Windows has to be read, and the webview cannot be asked instead: WebView2 answers light whatever
/// the system is. That is measured rather than assumed — with this machine in its dark appearance,
/// the media query inside the window answered light in all three modes, including while the window
/// itself was wearing dark. macOS and Linux answer nothing here, and the webview follows the system
/// on both.
#[cfg(windows)]
pub fn system_theme() -> Option<tauri::Theme> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    /// Where Windows writes down whether applications are in their light or dark appearance.
    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
    const VALUE: &str = "AppsUseLightTheme";

    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(KEY).ok()?;
    match key.get_value::<u32, _>(VALUE) {
        Ok(0) => Some(tauri::Theme::Dark),
        Ok(_) => Some(tauri::Theme::Light),
        // A value that is not there is not a value that is light: the interface is better off
        // falling back to the webview than being told something this shell does not know.
        Err(error) => {
            log::warn!("the system's own appearance could not be read: {error}");
            None
        }
    }
}

/// Nothing to read on the platforms whose webview follows the system by itself.
#[cfg(not(windows))]
pub fn system_theme() -> Option<tauri::Theme> {
    None
}

/// The name the interface knows a theme by.
pub fn theme_name(theme: Option<tauri::Theme>) -> Option<&'static str> {
    match theme {
        Some(tauri::Theme::Dark) => Some("dark"),
        Some(tauri::Theme::Light) => Some("light"),
        _ => None,
    }
}

/// Makes the platform's own menus follow the application's theme.
///
/// The tray icon's menu is the one this is for: Windows draws popup menus from the process's
/// "preferred app mode", and by default it reads that from the system rather than from the
/// application — so a window switched to light kept a dark tray menu.
///
/// There is no supported API for this and no crate in the tree that wraps it. The two functions
/// reached below are uxtheme ordinals this build already depends on: muda and tao, through Tauri,
/// call the same ones for their own reasons. Nothing happens when the ordinals are not there —
/// which has not been the case since Windows 10 1809 — and the menus keep the appearance they have.
#[cfg(windows)]
pub fn apply_menus_theme(theme: Option<tauri::Theme>) {
    use std::ffi::c_void;

    // Both lookups are kernel32's, and both take a pointer: a name for the module, and either a name
    // or, for an ordinal, the number itself in place of one.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryA(name: *const u8) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    }

    type SetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
    type FlushMenuThemes = unsafe extern "system" fn();

    /// `PreferredAppMode`: 135 is `SetPreferredAppMode`, and these are its two forcing values.
    const SET_PREFERRED_APP_MODE: u16 = 135;
    const FLUSH_MENU_THEMES: u16 = 136;
    const FORCE_DARK: i32 = 2;
    const FORCE_LIGHT: i32 = 3;

    let Some(theme) = theme else {
        return;
    };
    let mode = if theme == tauri::Theme::Dark {
        FORCE_DARK
    } else {
        FORCE_LIGHT
    };

    // Said out loud because the menu it decides is drawn by the system and looks like nothing this
    // application did: "the tray menu is still dark" is a report that needs this line to answer.
    log::info!(
        "the platform's menus follow the application: {}",
        if theme == tauri::Theme::Dark { "dark" } else { "light" }
    );

    // SAFETY: the module is loaded by name and the two ordinals are looked up in it; each is only
    // called after it was found, through a signature matching what uxtheme exports there. The
    // pointers are the library's own for the life of the process and are not freed, which is what
    // the platform's own documentation for this pair does as well.
    unsafe {
        let uxtheme = LoadLibraryA(c"uxtheme.dll".as_ptr() as *const u8);
        if uxtheme.is_null() {
            return;
        }
        let set_mode = GetProcAddress(uxtheme, SET_PREFERRED_APP_MODE as *const u8);
        let flush = GetProcAddress(uxtheme, FLUSH_MENU_THEMES as *const u8);
        if !set_mode.is_null() {
            let set_mode: SetPreferredAppMode = std::mem::transmute(set_mode);
            set_mode(mode);
        }
        if !flush.is_null() {
            let flush: FlushMenuThemes = std::mem::transmute(flush);
            flush();
        }
    }
}

/// Menus elsewhere are drawn by the toolkit, which already follows the application's theme.
#[cfg(not(windows))]
pub fn apply_menus_theme(_theme: Option<tauri::Theme>) {}

/// Tells the interface when the system's own appearance changes.
///
/// The webview cannot notice it — its answer never changes — and the window is only told while no
/// theme has been forced on it, which is exactly the state "follow the system" is in. So the event
/// is the signal and the system is read again rather than taken from the event: what that carries is
/// the window's theme, which is not the system's once a mode has been forced on the window.
pub fn watch_system_theme<R: Runtime>(app: &tauri::AppHandle<R>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    let app = app.clone();
    window.on_window_event(move |event| {
        if !matches!(event, tauri::WindowEvent::ThemeChanged(_)) {
            return;
        }

        let theme = system_theme();
        apply_menus_theme(theme);
        log::info!("the system's appearance changed; it is now {:?}", theme_name(theme));
        if let Err(error) = app.emit("system-theme-changed", theme_name(theme)) {
            log::warn!("the system's appearance could not be reported to the interface: {error}");
        }
    });
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
    fn following_the_system_resolves_to_what_the_system_says() {
        // The webview cannot be asked this on Windows: it answers light whatever the system is, so
        // what the interface follows has to come from the system's own setting.
        assert_eq!(
            effective_theme("system", Some(tauri::Theme::Dark)),
            Some(tauri::Theme::Dark)
        );
        assert_eq!(
            effective_theme("system", Some(tauri::Theme::Light)),
            Some(tauri::Theme::Light)
        );
    }

    #[test]
    fn a_system_that_could_not_be_read_leaves_the_resolution_to_the_webview() {
        // Nothing is better than a wrong answer: the interface falls back to the media query, which
        // follows the system on the platforms where this shell has no answer of its own.
        assert_eq!(effective_theme("system", None), None);
    }

    #[test]
    fn a_chosen_mode_does_not_change_when_the_system_does() {
        assert_eq!(
            effective_theme("dark", Some(tauri::Theme::Light)),
            Some(tauri::Theme::Dark)
        );
        assert_eq!(
            effective_theme("light", Some(tauri::Theme::Dark)),
            Some(tauri::Theme::Light)
        );
    }

    #[test]
    fn a_mode_from_a_newer_version_follows_the_system() {
        assert_eq!(
            effective_theme("midnight-aurora", Some(tauri::Theme::Dark)),
            Some(tauri::Theme::Dark)
        );
        assert_eq!(effective_theme("midnight-aurora", None), None);
    }

    #[test]
    fn a_theme_is_named_the_way_the_interface_names_it() {
        assert_eq!(theme_name(Some(tauri::Theme::Dark)), Some("dark"));
        assert_eq!(theme_name(Some(tauri::Theme::Light)), Some("light"));
        assert_eq!(theme_name(None), None);
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
