use sevenshell_error::ShellError;
use sevenshell_types::AppInfo;

/// Facts about the running application: name, version, platform, and the descriptor
/// tables the interface and Rust both derive from `descriptors/`.
#[tauri::command]
pub fn app_info() -> Result<AppInfo, ShellError> {
    Ok(AppInfo::current())
}
