use sevenshell_error::ShellError;
use sevenshell_types::AppInfo;
use tauri::{Manager as _, State, WebviewWindow};

use crate::state::ShellState;
use crate::window_chrome::{reveal_main_window, reveal_on_first_frame};

/// Facts about the running application: name, version, platform, and the descriptor
/// tables the interface and Rust both derive from `descriptors/`.
#[tauri::command]
pub fn app_info() -> Result<AppInfo, ShellError> {
    let mut info = AppInfo::current();
    let (os_name, os_version) = support_os();
    info.os_name = os_name;
    info.os_version = os_version;
    Ok(info)
}

/// The platform name and version a support report needs, when the platform can name them.
///
/// The raw identifier (`windows`) is enough for the application's own decisions but not for a bug
/// report: two machines on the same identifier can be a decade apart. This is asked once, next to
/// the rest of the startup facts, and a platform or command that will not answer simply leaves the
/// identifier in place.
fn support_os() -> (String, Option<String>) {
    #[cfg(target_os = "windows")]
    {
        (
            "Windows".to_owned(),
            command_first_line("cmd", &["/C", "ver"]).and_then(|line| version_token(&line)),
        )
    }
    #[cfg(target_os = "macos")]
    {
        ("macOS".to_owned(), command_first_line("sw_vers", &["-productVersion"]))
    }
    #[cfg(target_os = "linux")]
    {
        linux_os()
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        (std::env::consts::OS.to_owned(), None)
    }
}

/// The longest dotted number in a line: `Microsoft Windows [Version 10.0.26300.9974]` in any
/// language, because the words around it are translated and the number is not.
#[cfg(any(target_os = "windows", test))]
fn version_token(line: &str) -> Option<String> {
    line.split(|ch: char| !ch.is_ascii_digit() && ch != '.')
        .filter(|token| {
            token.contains('.')
                && token
                    .split('.')
                    .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        })
        .max_by_key(|token| token.len())
        .map(str::to_owned)
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn command_first_line(program: &str, args: &[&str]) -> Option<String> {
    let mut command = std::process::Command::new(program);
    command.args(args);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW: asking must not flash a console.
    }
    let output = command.output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
}

#[cfg(target_os = "linux")]
fn linux_os() -> (String, Option<String>) {
    let text = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let value = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|rest| rest.strip_prefix('='))
            .map(|value| value.trim().trim_matches('"').to_owned())
            .filter(|value| !value.is_empty())
    };
    let name = value("NAME")
        .or_else(|| value("PRETTY_NAME"))
        .unwrap_or_else(|| "Linux".to_owned());
    (name, value("VERSION_ID"))
}

/// Shows the window, on the interface's word that it has drawn its first frame.
///
/// The window is created hidden so that what appears first is the interface rather than an empty
/// white webview. This is the only thing that reveals it early: `window_chrome`'s deadline reveals
/// it anyway when this never arrives, which is what keeps a frontend that fails to start from
/// leaving no window behind at all.
///
/// Not when the window is in the tray: somebody who closed it there while the interface was still
/// loading has said where they want it, and what brings it back is the icon they closed it to.
#[tauri::command]
pub fn app_ready(window: WebviewWindow) {
    let put_away = window
        .try_state::<ShellState>()
        .is_some_and(|state| state.window_put_away());

    if reveal_on_first_frame(put_away) {
        reveal_main_window(&window);
    }
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

#[cfg(test)]
mod tests {
    use super::version_token;

    #[test]
    fn the_windows_version_is_the_dotted_number_whatever_the_words_around_it() {
        assert_eq!(
            version_token("Microsoft Windows [Version 10.0.26300.9974]").as_deref(),
            Some("10.0.26300.9974")
        );
        assert_eq!(
            version_token("Microsoft Windows [版本 10.0.26300.9974]").as_deref(),
            Some("10.0.26300.9974")
        );
        assert_eq!(version_token("no version here"), None);
    }
}
