//! The desktop shell.
//!
//! This crate owns the window and the command surface, and nothing else. Heavy
//! lifting belongs in the `sevenshell-*` crates so the same operations can later be
//! driven from a command line or an agent without going through a webview.

mod commands;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().build())
        .invoke_handler(tauri::generate_handler![commands::app::app_info])
        .run(tauri::generate_context!())
        .expect("failed to start the 7Shell desktop shell");
}
