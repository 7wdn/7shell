//! The desktop shell.
//!
//! This crate owns the window and the command surface, and nothing else. Heavy lifting belongs
//! in the `sevenshell-*` crates so the same operations can later be driven from a command line
//! or an agent without going through a webview.

mod commands;
mod window_chrome;

use tauri::Manager as _;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().build())
        .setup(|app| {
            window_chrome::prepare_main_window(app.handle());
            apply_startup_window_theme(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::settings::settings_get,
            commands::settings::settings_set
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the 7Shell desktop shell");
}

/// Makes the window start in the theme the user chose last time.
///
/// A settings file that cannot be read is not a reason to refuse to open a window: it is
/// reported, and the window follows the system until the interface offers to fix it.
fn apply_startup_window_theme(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    match sevenshell_platform::load() {
        Ok(settings) => commands::settings::apply_window_theme(&window, &settings),
        Err(error) => {
            log::warn!("settings could not be read at startup, so the window keeps the system theme: {error}")
        }
    }
}
