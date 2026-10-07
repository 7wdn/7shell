//! The window's background picture, as the shell sees it.
//!
//! Three commands, and the shape of all three is the same promise: the interface chooses a picture and
//! is told what it was *called*, never where it lives. The picker is the system's own (opened here, in
//! Rust, as every native dialog in this build is), the copy belongs to this installation, and reading
//! it back takes no argument at all — there is no path for a frontend to name, so there is none for one
//! to name wrongly.
//!
//! The filter's name is passed in rather than written here: it is a word somebody reads, and every word
//! the interface shows comes from the interface's own translations (see `sessions_import`, which asks
//! for its title the same way).

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use sevenshell_error::ShellError;
use sevenshell_platform::background;
use tauri::WebviewWindow;
use tauri_plugin_dialog::DialogExt as _;

/// Opens the system's file picker and copies what comes back.
///
/// `Ok(None)` is somebody closing the picker without choosing: not a failure, and not a change.
#[tauri::command]
pub async fn background_image_choose(window: WebviewWindow, title: String) -> Result<Option<String>, ShellError> {
    let directory = background_directory()?;

    let picked = tauri::async_runtime::spawn_blocking(move || {
        window
            .dialog()
            .file()
            .set_title(title)
            .add_filter("image", &background::EXTENSIONS)
            .blocking_pick_file()
    })
    .await
    .map_err(|error| ShellError::store_io(format!("the file picker could not be opened: {error}")))?;

    let Some(chosen) = picked else {
        return Ok(None);
    };

    let source = chosen
        .into_path()
        .map_err(|error| ShellError::store_io(format!("the chosen file cannot be read: {error}")))?;
    let name = source
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_owned();

    sevenshell_platform::background::store(&directory, &source)?;
    Ok(Some(name))
}

/// The stored picture, base64-encoded for the webview to turn into something it can draw.
#[tauri::command]
pub async fn background_image_read() -> Result<String, ShellError> {
    let directory = background_directory()?;
    tauri::async_runtime::spawn_blocking(move || background::read(&directory).map(|bytes| BASE64.encode(bytes)))
        .await
        .map_err(|error| ShellError::store_io(format!("reading the background picture failed: {error}")))?
}

/// Takes the stored picture away. Nothing stored is nothing to do.
#[tauri::command]
pub async fn background_image_clear() -> Result<(), ShellError> {
    let directory = background_directory()?;
    tauri::async_runtime::spawn_blocking(move || background::clear(&directory))
        .await
        .map_err(|error| ShellError::store_io(format!("removing the background picture failed: {error}")))?
}

fn background_directory() -> Result<std::path::PathBuf, ShellError> {
    sevenshell_platform::config_dir()
        .ok_or_else(|| ShellError::store_io("the configuration directory could not be located on this system"))
}
