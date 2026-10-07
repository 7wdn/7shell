//! The tray icon: where a window that put itself away can be picked up again.
//!
//! Three things live here, and they are one story. The icon is created when the settings ask for it
//! and hidden when they stop asking; its menu holds the two things somebody can want from an
//! application whose window is not on screen, which are to see it and to be rid of it; and the words
//! on that menu are translated here rather than in the interface's translation files, because the menu
//! is drawn by the operating system at exactly the moment there is no interface: a window that is
//! hidden has no webview to ask for a string. Both languages this build ships are given, which is the
//! part of the rule about user-facing text that can apply here.

use sevenshell_types::AppSettings;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter as _, Manager as _};

use crate::state::ShellState;

/// What the icon is called, so that a settings change can find it again.
pub const TRAY_ID: &str = "main-tray";

/// The two words on the menu, in the language the interface is in.
///
/// A prefix match rather than a lookup table: this build ships two languages, and everything that
/// starts with `zh` is the one that is not English.
fn labels(locale: &str) -> (&'static str, &'static str) {
    if locale.starts_with("zh") {
        ("显示 7Shell", "退出 7Shell")
    } else {
        ("Show 7Shell", "Quit 7Shell")
    }
}

/// Puts the icon where the settings say it belongs: on screen, or away.
///
/// Called at start-up and again whenever the settings are written, because the icon is part of what a
/// settings file *says* rather than something that waits for a restart. The state flag is kept in step
/// here rather than asked of the icon itself: it is what the close decision reads, and the decision is
/// made in a place that has no icon to ask.
pub fn apply(app: &AppHandle, settings: &AppSettings) {
    let shown = if settings.show_tray_icon {
        match app.tray_by_id(TRAY_ID) {
            Some(tray) => match tray.set_visible(true) {
                Ok(()) => true,
                Err(error) => {
                    log::warn!("the tray icon could not be shown: {error}");
                    false
                }
            },
            None => match build(app, settings.locale.as_deref().unwrap_or("en")) {
                Ok(()) => true,
                Err(error) => {
                    log::warn!("the tray icon could not be created: {error}");
                    false
                }
            },
        }
    } else {
        if let Some(tray) = app.tray_by_id(TRAY_ID) {
            let _ = tray.set_visible(false);
        }
        false
    };

    if let Some(state) = app.try_state::<ShellState>() {
        state.set_tray_shown(shown);
    }
}

fn build(app: &AppHandle, locale: &str) -> tauri::Result<()> {
    let Some(icon) = app.default_window_icon().cloned() else {
        log::warn!("this build has no application icon, so it cannot show one in the tray");
        return Ok(());
    };

    let (show_label, quit_label) = labels(locale);
    let show_item = MenuItem::with_id(app, "show", show_label, true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", quit_label, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip("7Shell")
        .menu(&menu)
        // A click on the icon brings the window back, which is what a click on a tray icon is for;
        // the menu is what the right button is for.
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                reveal(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => reveal(app),
            "quit" => quit(app),
            _ => {}
        })
        .build(app)?;

    Ok(())
}

/// Brings the window back: shown, out of the way of nothing, and focused.
///
/// All three, because an icon that restores a window behind another one — or minimized — is an icon
/// somebody presses twice and then stops trusting.
pub fn reveal(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Asks to quit, the same way the window's own close button does.
///
/// The interface is the only thing that can see unsaved text or a transfer in flight, so with
/// something to lose the question goes to it and the answer comes back as a close request — which is
/// the path a close button already takes. With nothing to lose the window is closed here, and the
/// shell tears down what it holds on the way out.
pub fn quit(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    let asks_first = app.try_state::<ShellState>().is_some_and(|state| state.quit_guard());

    if asks_first {
        // Shown before the question is asked: a dialog over a window nobody can see is a question
        // nobody can answer.
        reveal(app);
        let _ = window.emit("quit-requested", ());
        return;
    }

    if let Some(state) = app.try_state::<ShellState>() {
        state.approve_quit();
    }
    let _ = window.close();
}
