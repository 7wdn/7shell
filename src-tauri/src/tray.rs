//! The tray icon: where a window that put itself away can be picked up again.
//!
//! Three things live here, and they are one story. The icon is created when the settings ask for it
//! and hidden when they stop asking; its menu holds the three things somebody can want from an
//! application whose window is not on screen, which are to see it, to lock what it is holding and to
//! be rid of it; and the words on that menu are translated here rather than in the interface's
//! translation files, because the menu is drawn by the operating system at exactly the moment there is
//! no interface: a window that is hidden has no webview to ask for a string. Both languages this build
//! ships are given, which is the part of the rule about user-facing text that can apply here.

use sevenshell_types::AppSettings;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter as _, Manager as _};

use crate::state::ShellState;

/// What the icon is called, so that a settings change can find it again.
pub const TRAY_ID: &str = "main-tray";

/// The event the interface listens for when the vault is locked from here.
///
/// Locking from the tray happens with no webview involved, so the interface has to be *told* rather
/// than asked: without this the window would come back from the tray still drawing the library of a
/// vault that is shut, and the next save would fail with `VAULT-LOCKED` for no visible reason.
pub const LOCKED_EVENT: &str = "vault-locked";

/// The words on the menu, in the language the interface is in.
///
/// A prefix match rather than a lookup table: this build ships two languages, and everything that
/// starts with `zh` is the one that is not English. Public within the crate so that the mapping can be
/// checked without an application to put a menu on.
pub(crate) fn labels(locale: &str) -> (&'static str, &'static str, &'static str) {
    if locale.starts_with("zh") {
        ("显示 7Shell", "锁定保险库", "退出 7Shell")
    } else {
        ("Show 7Shell", "Lock the vault", "Quit 7Shell")
    }
}

/// The three items the menu is made of, kept because the words on them change with the language.
///
/// A menu is built once and lives for as long as the process does, so a label written at build time is
/// a label that never follows the language somebody picks afterwards — which is exactly what happened
/// before this was kept: the interface switched to another language and the tray went on saying the
/// old one until the next launch.
pub struct TrayMenu {
    show: MenuItem<tauri::Wry>,
    lock: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
}

impl TrayMenu {
    /// Rewrites every label.
    ///
    /// Every settings write comes through here, so this is where the language of a running application
    /// reaches the menu — and it is logged because that hand-over is invisible: the menu is drawn by
    /// the system, and nothing in this process can read it back to check.
    fn retitle(&self, locale: &str) {
        let (show, lock, quit) = labels(locale);
        log::info!("the tray menu follows the language: `{locale}`");
        if let Err(error) = self
            .show
            .set_text(show)
            .and_then(|()| self.lock.set_text(lock))
            .and_then(|()| self.quit.set_text(quit))
        {
            log::warn!("the tray menu could not be translated: {error}");
        }
    }
}

/// Puts the icon where the settings say it belongs: on screen, or away.
///
/// Called at start-up and again whenever the settings are written, because the icon is part of what a
/// settings file *says* rather than something that waits for a restart. The state flag is kept in step
/// here rather than asked of the icon itself: it is what the close decision reads, and the decision is
/// made in a place that has no icon to ask.
pub fn apply(app: &AppHandle, settings: &AppSettings) {
    let locale = settings.locale.as_deref().unwrap_or("en");
    let shown = if settings.show_tray_icon {
        match app.tray_by_id(TRAY_ID) {
            Some(tray) => match tray.set_visible(true) {
                Ok(()) => {
                    // The language may have changed since the menu was built, and a menu is not a
                    // thing that redraws itself.
                    if let Some(menu) = app.try_state::<TrayMenu>() {
                        menu.retitle(locale);
                    }
                    true
                }
                Err(error) => {
                    log::warn!("the tray icon could not be shown: {error}");
                    false
                }
            },
            None => match build(app, locale) {
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

    let (show_label, lock_label, quit_label) = labels(locale);
    let show_item = MenuItem::with_id(app, "show", show_label, true, None::<&str>)?;
    let lock_item = MenuItem::with_id(app, "lock", lock_label, true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", quit_label, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &lock_item, &quit_item])?;

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
            "lock" => lock(app),
            "quit" => quit(app),
            _ => {}
        })
        .build(app)?;

    // Managed rather than dropped: the labels have to be reachable from the next settings change.
    let _ = app.manage(TrayMenu {
        show: show_item,
        lock: lock_item,
        quit: quit_item,
    });

    log::info!("the tray menu was built in `{locale}`");

    Ok(())
}

/// Locks the vault from the tray, and tells the interface that it happened.
///
/// The one entry to locking that has no interface behind it: nothing was pressed in a window, so
/// nothing in a window knows. What locking *means* — including the questions it drops — is
/// [`ShellState::lock_vault`], the same call the command makes, and the event is what puts the gate
/// on screen the next time the window is looked at. Open terminals are left running, exactly as they
/// are when the command locks: they are authenticated already.
pub fn lock(app: &AppHandle) {
    let Some(state) = app.try_state::<ShellState>() else {
        return;
    };
    state.lock_vault();
    log::info!("the vault was locked from the tray menu");

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit(LOCKED_EVENT, ());
    }
}

/// Brings the window back: shown, out of the way of nothing, and focused.
///
/// All three, because an icon that restores a window behind another one — or minimized — is an icon
/// somebody presses twice and then stops trusting. This is also the one thing that undoes having put
/// the window away, which is what the close decision leaves behind it (see `ShellState`): asking for
/// the window is exactly what the flag is waiting to hear.
pub fn reveal(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if let Some(state) = app.try_state::<ShellState>() {
            state.set_window_put_away(false);
        }
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

#[cfg(test)]
mod tests {
    use super::labels;

    #[test]
    fn the_menu_is_written_in_the_language_the_interface_is_in() {
        assert_eq!(labels("zh-CN"), ("显示 7Shell", "锁定保险库", "退出 7Shell"));
        assert_eq!(labels("zh"), ("显示 7Shell", "锁定保险库", "退出 7Shell"));
        assert_eq!(labels("en"), ("Show 7Shell", "Lock the vault", "Quit 7Shell"));
        // Anything this build does not ship reads as English, which is what the interface does with an
        // unknown language as well: it is the language of the code, and the one that is always there.
        assert_eq!(labels("fr-FR"), ("Show 7Shell", "Lock the vault", "Quit 7Shell"));
        assert_eq!(labels(""), ("Show 7Shell", "Lock the vault", "Quit 7Shell"));
    }
}
