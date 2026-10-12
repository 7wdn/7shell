//! The desktop shell.
//!
//! This crate owns the window and the command surface, and nothing else. Heavy lifting belongs
//! in the `sevenshell-*` crates so the same operations can later be driven from a command line
//! or an agent without going through a webview.

mod clipboard;
mod close;
mod commands;
mod dropped;
mod reveal;
mod state;
mod tray;
mod uploads;
mod window_chrome;

use std::sync::Arc;

use sevenshell_core::Core;
use tauri::{Emitter as _, Manager as _};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        // Registered for its Rust API alone: the interface reaches the channel through the two
        // `update_*` commands below rather than through the plugin's own JavaScript commands, which
        // is why no `updater:*` permission appears in the capability file. One place asks, and it is
        // ours — a permission the interface does not have is a path nobody can forget to audit.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            window_chrome::prepare_main_window(app.handle());
            apply_startup_window_theme(app.handle());
            window_chrome::watch_system_theme(app.handle());

            let (shell_state, events) = state::ShellState::new();
            commands::terminal::pump_events(app.handle().clone(), events);
            app.manage(shell_state);

            // After the state is managed rather than before: what the settings say about closing is
            // recorded *in* that state, and a shell that read them into nothing would decide every
            // close from the constructor's values instead of from the file.
            apply_startup_shell_settings(app.handle());

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::DragDrop(drag) = event {
                report_drag(window, drag);
            }

            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let action = match window.try_state::<state::ShellState>() {
                    Some(shell_state) => close::close_action(
                        shell_state.quit_approved(),
                        !shell_state.close_to_tray(),
                        shell_state.tray_shown(),
                        shell_state.quit_guard(),
                    ),
                    // No state means no window to reason about: whatever the shell is, a close that
                    // nothing objects to is a close.
                    None => close::CloseAction::Close,
                };

                log::info!("the window was asked to close: {action:?}");

                match action {
                    // Away and still running: the icon is what brings it back, and the sessions the
                    // window was holding keep running while it is gone.
                    close::CloseAction::Hide => {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                    // The interface is the only thing that can see what is unsaved, so it is asked.
                    close::CloseAction::Ask => {
                        api.prevent_close();
                        let _ = window.emit("quit-requested", ());
                    }
                    close::CloseAction::Close => {}
                }
            }

            // On the way out, tell the servers we are leaving rather than letting the sockets time
            // out. Fire and forget: closing the window must not wait for the network.
            if let tauri::WindowEvent::Destroyed = event
                && let Some(shell_state) = window.try_state::<state::ShellState>()
            {
                let core: Arc<Core> = shell_state.core_arc();
                tauri::async_runtime::spawn(async move { core.close_all().await });
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::app::app_ready,
            commands::app::window_quit_guard,
            commands::app::window_quit,
            commands::update::update_check,
            commands::update::update_install,
            commands::background::background_image_choose,
            commands::background::background_image_read,
            commands::background::background_image_clear,
            commands::fonts::system_fonts_list,
            commands::settings::settings_get,
            commands::settings::settings_set,
            commands::settings::system_theme,
            commands::vault::vault_state,
            commands::vault::vault_create,
            commands::vault::vault_unlock,
            commands::vault::vault_lock,
            commands::vault::vault_change_password,
            commands::vault::vault_set_aside,
            commands::sessions::sessions_list,
            commands::sessions::session_save,
            commands::sessions::session_delete,
            commands::sessions::groups_list,
            commands::sessions::group_save,
            commands::sessions::group_delete,
            commands::sessions::sessions_export,
            commands::sessions::sessions_import,
            commands::sync::sync_available,
            commands::sync::sync_status,
            commands::sync::sync_password,
            commands::sync::sync_passphrase,
            commands::sync::sync_test,
            commands::sync::sync_upload,
            commands::sync::sync_download,
            commands::sync::sync_apply,
            commands::sync::sync_backup_export,
            commands::sync::sync_backup_inspect,
            commands::sync::sync_backup_restore,
            commands::sync::snippet_sync_status,
            commands::sync::snippet_sync_token,
            commands::sync::snippet_sync_test,
            commands::sync::snippet_sync_upload,
            commands::sync::snippet_sync_download,
            commands::sync::snippet_sync_apply,
            commands::keys::keys_list,
            commands::keys::key_import,
            commands::keys::key_generate,
            commands::keys::key_public,
            commands::keys::key_forget,
            commands::known_hosts::known_hosts_list,
            commands::known_hosts::known_host_trust,
            commands::known_hosts::known_host_forget,
            commands::batch::batch_run,
            commands::snippets::snippets_open,
            commands::snippets::snippet_group_save,
            commands::snippets::snippet_group_delete,
            commands::snippets::snippet_save,
            commands::snippets::snippet_delete,
            commands::snippets::completion_words,
            commands::snippets::completion_word_save,
            commands::snippets::completion_word_delete,
            commands::snippets::completion_words_clear,
            commands::snippets::command_history,
            commands::snippets::command_history_clear,
            commands::snippets::command_classify,
            commands::snippets::command_send,
            commands::terminal::session_open,
            commands::terminal::session_open_cancel,
            commands::terminal::session_system,
            commands::terminal::terminal_write,
            commands::terminal::terminal_resize,
            commands::terminal::terminal_close,
            commands::terminal::terminals_open,
            commands::terminal::tunnels_open,
            commands::terminal::tunnel_close,
            commands::monitor::monitor_sample,
            commands::monitor::monitor_processes,
            commands::monitor::monitor_process_detail,
            commands::monitor::monitor_sockets,
            commands::files::files_open,
            commands::files::files_list,
            commands::files::files_create_folder,
            commands::files::files_create_file,
            commands::files::files_rename,
            commands::files::files_delete,
            commands::files::files_read_text,
            commands::files::files_write_text,
            commands::files::files_close,
            commands::files::files_download,
            commands::files::files_upload,
            commands::files::files_upload_answer,
            commands::files::download_folder_get,
            commands::files::download_folder_choose,
            commands::files::download_folder_select,
            commands::files::download_folder_ask,
            commands::files::download_folder_reveal,
            commands::files::transfers_open,
            commands::files::transfer_stop,
            commands::files::transfer_remove,
            commands::files::transfer_stop_all,
            commands::files::transfer_clear_settled,
            commands::files::transfer_open,
            commands::files::transfer_reveal,
            commands::files::transfer_pause,
            commands::files::transfer_resume,
            commands::files::transfer_retry,
            commands::files::dropped_upload,
            commands::files::clipboard_files,
            commands::clipboard::clipboard_text,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the 7Shell desktop shell");
}

/// What a drag over the window means to the interface.
///
/// The paths stay here: the interface is told whether a drop is hovering, and when one lands, a
/// token standing for the files. See `dropped` for why that matters.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DragPayload {
    hovering: bool,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DropPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pending: Option<dropped::PendingFiles>,
    /// Why the drop could not be used, when it could not. A drop of something that is not a file
    /// is worth saying out loud rather than ignoring.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<sevenshell_error::ShellError>,
}

fn report_drag<R: tauri::Runtime>(window: &tauri::Window<R>, event: &tauri::DragDropEvent) {
    use tauri::Emitter as _;

    match event {
        tauri::DragDropEvent::Enter { .. } | tauri::DragDropEvent::Over { .. } => {
            let _ = window.emit("files:drag", DragPayload { hovering: true });
        }
        tauri::DragDropEvent::Leave => {
            let _ = window.emit("files:drag", DragPayload { hovering: false });
        }
        tauri::DragDropEvent::Drop { paths, .. } => {
            let _ = window.emit("files:drag", DragPayload { hovering: false });

            let payload = match window.try_state::<state::ShellState>() {
                Some(shell_state) => match shell_state.pending_drops().remember(paths.clone()) {
                    Ok(pending) => DropPayload {
                        pending: Some(pending),
                        error: None,
                    },
                    Err(error) => DropPayload {
                        pending: None,
                        error: Some(error),
                    },
                },
                None => DropPayload {
                    pending: None,
                    error: None,
                },
            };

            let _ = window.emit("files:dropped", payload);
        }
        // The enum is `#[non_exhaustive]`: a future variant means a gesture this build does not
        // know about, and doing nothing is the right answer to that.
        _ => {}
    }
}

/// Makes the window start in the theme the user chose last time.
///
/// A settings file that cannot be read is not a reason to refuse to open a window: it is
/// reported, and the window follows the system until the interface offers to fix it.
/// Reads the settings once at start-up for the two things the shell itself obeys.
///
/// Not a second copy of the interface's settings: the same file, read at the one moment the interface
/// has not read it yet. A file that cannot be read leaves the defaults, which are the icon on and the
/// window closing the application — both of which are states a person can get out of.
fn apply_startup_shell_settings(app: &tauri::AppHandle) {
    match sevenshell_platform::load() {
        Ok(settings) => apply_shell_settings(app, &settings),
        Err(error) => {
            log::warn!("settings could not be read at startup, so the shell keeps its defaults: {error}");
        }
    }
}

/// Puts the two settings the shell obeys into effect: the tray icon, and what closing does.
pub fn apply_shell_settings(app: &tauri::AppHandle, settings: &sevenshell_types::AppSettings) {
    tray::apply(app, settings);
    if let Some(state) = app.try_state::<state::ShellState>() {
        state.set_close_to_tray(!settings.quit_on_close);
    }
}

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
