use sevenshell_error::ShellError;
use sevenshell_types::AppInfo;
use tauri::WebviewWindow;

use crate::window_chrome::reveal_main_window;

/// Facts about the running application: name, version, platform, and the descriptor
/// tables the interface and Rust both derive from `descriptors/`.
#[tauri::command]
pub fn app_info() -> Result<AppInfo, ShellError> {
    Ok(AppInfo::current())
}

/// Shows the window, on the interface's word that it has drawn its first frame.
///
/// The window is created hidden so that what appears first is the interface rather than an empty
/// white webview. This is the only thing that reveals it early: `window_chrome`'s deadline reveals
/// it anyway when this never arrives, which is what keeps a frontend that fails to start from
/// leaving no window behind at all.
#[tauri::command]
pub fn app_ready(window: WebviewWindow) {
    reveal_main_window(&window);
}
