use sevenshell_error::ShellError;
use sevenshell_types::AppInfo;
use tauri::{State, WebviewWindow};

use crate::state::ShellState;
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

/// Says whether closing the window would throw something away.
///
/// Armed while an editor holds unsaved text or a transfer is in flight, and disarmed the moment
/// there is nothing left to lose — the window then closes on the first request, exactly as it did
/// before. It is deliberately the interface's call: only the interface can see what is unsaved.
#[tauri::command]
pub fn window_quit_guard(state: State<'_, ShellState>, armed: bool) {
    state.set_quit_guard(armed);
}

/// Closes the window for real, on the interface's word that the question has been answered.
///
/// The window's own close request goes through the shell's decision — putting the window away, or
/// asking — so an interface that has *already* asked needs a way past it. This is that way, and it is
/// the only one: the flag it sets is what the decision reads.
#[tauri::command]
pub fn window_quit(window: WebviewWindow, state: State<'_, ShellState>) {
    state.approve_quit();
    let _ = window.close();
}
