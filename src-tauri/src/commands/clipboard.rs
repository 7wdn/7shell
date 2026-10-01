//! The clipboard, for the one read the interface cannot do for itself.

use sevenshell_error::ShellError;

/// The text on the clipboard, or an empty string when it holds none.
///
/// The interface is told the text and nothing else about it: no owner, no format, no path. It is not
/// logged here and must not be — it is whatever the person last copied, which is sometimes a password
/// on its way into a terminal.
#[tauri::command]
pub fn clipboard_text() -> Result<String, ShellError> {
    crate::clipboard::text()
}
