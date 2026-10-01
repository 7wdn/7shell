//! What the operating system has on its clipboard, for the one thing the webview cannot ask it.
//!
//! Reading text is the opposite of the problem `dropped.rs` solves: text is not a path, so it can be
//! handed to the interface as it is and needs no token. It is read here rather than in the webview
//! for a reason a probe found and no documentation said: WebView2 answers a script's clipboard read
//! with a permission request that nothing is waiting to answer, so the promise never settles. A paste
//! that went that way would hang with nothing to show — no error, no message, no sign it was pressed.
//!
//! What comes back is the person's own clipboard. It is never logged, never put into an error, and
//! never stored: it goes straight to the terminal it was pasted into.

use sevenshell_error::ShellError;

/// The text on the clipboard, or an empty string when it holds none.
///
/// A clipboard holding files, a picture, or nothing at all is not an error here — there is simply no
/// text, which is what an empty string says. That is the only way reading this format fails on
/// Windows, so an empty answer is the honest one rather than a refusal the interface would have to
/// dress up.
#[cfg(windows)]
pub fn text() -> Result<String, ShellError> {
    use clipboard_win::{formats::Unicode, get_clipboard};

    let read: Result<String, _> = get_clipboard(Unicode);
    Ok(read.unwrap_or_default())
}

#[cfg(not(windows))]
pub fn text() -> Result<String, ShellError> {
    Err(ShellError::app_unsupported(
        "pasting text from the clipboard on this platform",
    ))
}
