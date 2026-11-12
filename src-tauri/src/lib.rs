//! The desktop shell.
//!
//! This crate owns the window and the command surface, and nothing else. Heavy lifting belongs
//! in the `sevenshell-*` crates so the same operations can later be driven from a command line
//! or an agent without going through a webview.

mod ai;
mod clipboard;
mod close;
mod commands;
mod dropped;
mod jsonl;
mod mcp;
mod mcp_client;
mod policy;
mod reveal;
mod state;
mod tray;
mod uploads;
mod window_chrome;

use std::sync::Arc;

use sevenshell_core::Core;
use tauri::{Emitter as _, Manager as _};

pub fn run() {
    // Before the builder, so that "the window is on screen after N ms" measures everything a person
    // waits for rather than everything after the shell was already up.
    window_chrome::mark_start();

    // One application per machine, and it is registered before every other plugin: it is the one that
    // decides a second launch has nothing to start, and a shell that had already built its window and
    // its tray would have paid for both before finding out.
    //
    // Only in a shipped build. Starting a second one in development is how somebody works — the
    // installed application and the one being edited side by side, or the test run that starts its own
    // — and a refusal there would leave them looking at nothing at all; two writers to one vault is a
    // choice a developer is allowed to make. What must never run twice is the application somebody
    // installed, and that is the build this is compiled into.
    #[cfg(debug_assertions)]
    let builder = tauri::Builder::default();
    #[cfg(not(debug_assertions))]
    let builder = tauri::Builder::default().plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
        // A second launch is somebody asking for the window — the same thing a click on the tray icon
        // is, out of the tray included.
        log::info!("a second launch asked for the window; the one already running is brought forward");
        tray::reveal(app);
    }));

    builder
        .plugin(tauri_plugin_log::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        // Registered for its Rust API alone: the interface reaches the channel through the two
        // `update_*` commands below rather than through the plugin's own JavaScript commands, which
        // is why no `updater:*` permission appears in the capability file. One place asks, and it is
        // ours — a permission the interface does not have is a path nobody can forget to audit.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            window_chrome::prepare_main_window(app.handle());
            window_chrome::silence_browser_keys(app.handle());
            apply_startup_window_theme(app.handle());
            window_chrome::watch_system_theme(app.handle());

            let (shell_state, events) = state::ShellState::new(app.handle());
            commands::terminal::pump_events(app.handle().clone(), shell_state.core_arc(), events);
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
                    // window was holding keep running while it is gone. Recorded rather than only
                    // done, because the two things that show the window by themselves — the
                    // interface's first frame and the reveal deadline — cannot tell this apart from
                    // a window nobody has shown yet, and would put it back on screen.
                    close::CloseAction::Hide => {
                        if let Some(shell_state) = window.try_state::<state::ShellState>() {
                            shell_state.set_window_put_away(true);
                        }
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
                let mcp = shell_state.mcp_arc();
                let mcp_client = shell_state.mcp_client_arc();
                let ai = shell_state.ai_arc();
                tauri::async_runtime::spawn(async move {
                    // First, because it is the one thing that might still be producing: a run whose
                    // window is gone is cancelled here rather than left to finish against a
                    // conversation nobody will read.
                    ai.shutdown();
                    // The endpoint file names a token this process is about to stop accepting, and a
                    // bridge that found it after the window was gone would report a desktop that is
                    // not answering. Removed before the last connection is closed.
                    mcp.stop().await;
                    // The external servers were opened with this window's vault; nothing is left to
                    // read their answers, so the sessions go too.
                    mcp_client.shutdown();
                    core.close_all().await
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::app::app_ready,
            commands::app::window_quit_guard,
            commands::app::window_quit,
            commands::update::update_check,
            commands::update::update_download,
            commands::update::update_install,
            commands::update::update_open_release_page,
            commands::background::background_image_choose,
            commands::background::background_image_read,
            commands::background::background_image_clear,
            commands::fonts::system_fonts_list,
            commands::settings::settings_get,
            commands::settings::settings_set,
            commands::settings::window_theme_apply,
            commands::settings::system_theme,
            commands::vault::vault_state,
            commands::vault::vault_create,
            commands::vault::vault_unlock,
            commands::vault::vault_lock,
            commands::vault::vault_change_password,
            commands::vault::vault_set_aside,
            commands::vault::vault_archives,
            commands::vault::vault_archive_verify,
            commands::vault::vault_archive_activate,
            commands::vault::vault_archive_import,
            commands::vault::vault_reveal_folder,
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
            commands::completion::completion_execute,
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
            commands::terminal::terminal_local_shells,
            commands::terminal::terminal_wsl_distros,
            commands::terminal::terminal_serial_ports,
            commands::terminal::tunnels_open,
            commands::terminal::tunnel_close,
            commands::desktop::desktop_open,
            commands::monitor::monitor_sample,
            commands::monitor::monitor_processes,
            commands::monitor::monitor_process_detail,
            commands::monitor::monitor_sockets,
            commands::files::files_open,
            commands::files::files_open_container,
            commands::files::files_open_pod,
            commands::files::files_list,
            commands::files::files_create_folder,
            commands::files::files_create_file,
            commands::files::files_rename,
            commands::files::files_set_permissions,
            commands::files::files_paste,
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
            commands::container::container_connect,
            commands::container::container_disconnect,
            commands::container::container_list,
            commands::container::container_inspect,
            commands::container::container_action,
            commands::container::container_rename,
            commands::container::container_create,
            commands::container::container_stats,
            commands::container::container_namespaces,
            commands::container::container_set_namespace,
            commands::container::container_runtime_info,
            commands::container::container_logs_start,
            commands::container::container_stream_stop,
            commands::container::container_exec_open,
            commands::container::container_images,
            commands::container::container_image_inspect,
            commands::container::container_image_history,
            commands::container::container_image_pull,
            commands::container::container_image_push,
            commands::container::container_image_remove,
            commands::container::container_image_prune,
            commands::container::container_image_tag,
            commands::container::container_image_load,
            commands::container::container_image_commit,
            commands::container::container_registry_login,
            commands::k8s::k8s_contexts,
            commands::k8s::k8s_connect,
            commands::k8s::k8s_disconnect,
            commands::k8s::k8s_request,
            commands::k8s::k8s_watch_start,
            commands::k8s::k8s_logs_start,
            commands::k8s::k8s_stream_stop,
            commands::k8s::k8s_exec_open,
            commands::mcp::mcp_status,
            commands::mcp::mcp_enable,
            commands::mcp::mcp_disable,
            commands::mcp::mcp_client_config,
            commands::mcp::mcp_audit_tail,
            commands::policy::policy_audit_tail,
            commands::mcp::mcp_approval_answer,
            commands::mcp::mcp_catalog,
            commands::mcp::mcp_servers_list,
            commands::mcp::mcp_server_save,
            commands::mcp::mcp_server_remove,
            commands::mcp::mcp_server_secrets_set,
            commands::mcp::mcp_server_refresh,
            commands::mcp::mcp_content_catalog,
            commands::mcp::mcp_resource_read,
            commands::mcp::mcp_prompt_get,
            commands::ai::ai_providers,
            commands::ai::ai_provider_key_set,
            commands::ai::ai_provider_remove,
            commands::ai::ai_provider_test,
            commands::ai::ai_chat_send,
            commands::ai::ai_chat_cancel,
            commands::ai::ai_chat_reset,
            commands::ai::ai_sessions_list,
            commands::ai::ai_session_get,
            commands::ai::ai_session_rename,
            commands::ai::ai_session_delete,
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
        Ok(settings) => {
            apply_shell_settings(app, &settings);
            if let Some(state) = app.try_state::<state::ShellState>() {
                // What an agent may see and do, before anything can ask. The vault is still locked
                // here, so a group's scope cannot be resolved yet — the visibility allowlist and the
                // session-level overrides are applied now, and the whole snapshot is rebuilt when the
                // vault opens. The tool registry's disabled set moves in the same pass, so a tool the
                // settings file switched off is never offered by a listener that was already up.
                state.apply_agent_policy(&settings);
                // The external servers first: the listener's `GET /tools` answers with whatever the
                // registry holds at the moment it is asked, and a configured server's tools should
                // be on the way before the first agent asks. Applied before the socket opens, and
                // opened on a task of its own — no handshake delays the window.
                if let Err(error) = state.mcp_client().apply_and_refresh(&settings) {
                    log::warn!("the external MCP servers could not be applied at startup: {error}");
                }
                // After the state is managed, because the listener keeps its runtime there. Spawned
                // rather than awaited: opening a socket is not a reason for the window to wait, and
                // the interface's first `mcp_status` reports what actually came up.
                crate::mcp::start_if_enabled(app, state.mcp_arc(), &settings);
            }
        }
        Err(error) => {
            log::warn!("settings could not be read at startup, so the shell keeps its defaults: {error}");
            // The defaults are a policy too — safe and caution run, the destructive levels ask — and a
            // matrix that decides without a trail is exactly what invariant 7 is about. Handing the
            // core the default matrix *and* the journal is what keeps a broken settings file from
            // also being a missing audit trail.
            if let Some(state) = app.try_state::<state::ShellState>() {
                state.apply_agent_policy(&sevenshell_types::AppSettings::default());
            }
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
        Ok(settings) => commands::settings::apply_window_theme(&window, settings.theme_mode.as_str()),
        Err(error) => {
            log::warn!("settings could not be read at startup, so the window keeps the system theme: {error}")
        }
    }
}
