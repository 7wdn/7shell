//! The settings file: reading and writing the preferences described by
//! [`sevenshell_types::AppSettings`].
//!
//! Three rules, all of them about not losing data:
//!
//! 1. **A missing file is not an error.** It means nobody has chosen anything yet.
//! 2. **A file from a newer version is refused.** Dropping fields a future version needs is
//!    how settings get silently destroyed; refusing to start quietly would be worse, so it
//!    reports `STORE-SCHEMA-NEWER` and the interface says so.
//! 3. **Writes are atomic.** Temporary file, flush, rename. An interrupted write leaves the
//!    previous settings readable, which is invariant 5 applied to something less scary than
//!    the vault but no less annoying to lose.

use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sevenshell_error::ShellError;
use sevenshell_types::{AppSettings, FilesLayout, SessionLayout, ThemeMode, monitor_on, sidebar_on};

/// Bumped only when the stored layout changes in a way older builds cannot read.
pub const SETTINGS_SCHEMA_VERSION: u32 = 1;

/// What is actually on disk.
///
/// Strings rather than enums for the values the interface chooses: a file written by a newer
/// version may name a theme this build has never heard of, and falling back to the default is
/// better than refusing to open.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSettings {
    schema_version: u32,
    #[serde(default)]
    theme_mode: String,
    /// Absent in a file written before the terminal had a palette of its own, which reads as
    /// "follow the interface" — the honest answer for somebody who never chose.
    #[serde(default)]
    terminal_theme: String,
    #[serde(default)]
    files_layout: String,
    /// Absent in a file written before a session could open as two halves, which reads as the
    /// default — the split — rather than as the terminal alone.
    #[serde(default)]
    session_layout: String,
    /// Absent in a file written before the terminal had a file panel, which reads as "the divider
    /// has never been moved".
    #[serde(default)]
    terminal_panel_height: Option<u32>,
    /// Absent in a file written before there was a monitor, which reads as the default the feature
    /// ships with — on — rather than as a monitor somebody switched off before it existed.
    #[serde(default = "monitor_on")]
    monitor_enabled: bool,
    /// Absent in a file written before there was a packed transfer, which reads as off — the default
    /// the feature ships with. A file written by a build that has it cannot be read back by one that
    /// does not, but that is what the schema version is for.
    #[serde(default)]
    packed_transfer: bool,
    /// Absent in a file written before the sidebar could be resized, which reads as "the divider
    /// has never been moved".
    #[serde(default)]
    sidebar_width: Option<u32>,
    /// Absent in a file written before the sidebar could be put away, which reads as the default it
    /// ships with — on — rather than as a sidebar somebody closed before it could be closed.
    #[serde(default = "sidebar_on")]
    sidebar_open: bool,
    #[serde(default)]
    locale: Option<String>,
}

impl StoredSettings {
    fn from_settings(settings: &AppSettings) -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            theme_mode: settings.theme_mode.as_str().to_owned(),
            terminal_theme: settings.terminal_theme.as_str().to_owned(),
            files_layout: settings.files_layout.as_str().to_owned(),
            session_layout: settings.session_layout.as_str().to_owned(),
            terminal_panel_height: settings.terminal_panel_height,
            sidebar_width: settings.sidebar_width,
            sidebar_open: settings.sidebar_open,
            monitor_enabled: settings.monitor_enabled,
            packed_transfer: settings.packed_transfer,
            locale: settings.locale.clone(),
        }
    }

    fn into_settings(self) -> AppSettings {
        AppSettings {
            theme_mode: ThemeMode::from_id(&self.theme_mode).unwrap_or_default(),
            terminal_theme: ThemeMode::from_id(&self.terminal_theme).unwrap_or_default(),
            files_layout: FilesLayout::from_id(&self.files_layout).unwrap_or_default(),
            session_layout: SessionLayout::from_id(&self.session_layout).unwrap_or_default(),
            terminal_panel_height: self.terminal_panel_height,
            sidebar_width: self.sidebar_width,
            sidebar_open: self.sidebar_open,
            monitor_enabled: self.monitor_enabled,
            packed_transfer: self.packed_transfer,
            locale: self.locale.filter(|locale| !locale.is_empty()),
        }
    }
}

/// Reads settings from `path`. An absent file yields the defaults.
pub fn load_from(path: &Path) -> Result<AppSettings, ShellError> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(AppSettings::default()),
        Err(error) => return Err(ShellError::store_io(format!("cannot read {}: {error}", path.display()))),
    };

    let stored: StoredSettings = serde_json::from_str(&raw)
        .map_err(|error| ShellError::store_io(format!("{} is not valid settings JSON: {error}", path.display())))?;

    if stored.schema_version > SETTINGS_SCHEMA_VERSION {
        return Err(
            ShellError::store_schema_newer(stored.schema_version, SETTINGS_SCHEMA_VERSION)
                .with_detail(format!("{} was written by a newer version of 7Shell", path.display())),
        );
    }

    Ok(stored.into_settings())
}

/// Writes settings to `path`, atomically.
pub fn save_to(path: &Path, settings: &AppSettings) -> Result<(), ShellError> {
    let stored = StoredSettings::from_settings(settings);
    let json = serde_json::to_string_pretty(&stored)
        .map_err(|error| ShellError::app_internal(format!("settings could not be serialized: {error}")))?;

    write_atomically(path, json.as_bytes())
}

/// The settings of the running application, from the platform configuration directory.
pub fn load() -> Result<AppSettings, ShellError> {
    match crate::paths::settings_file() {
        Some(path) => load_from(&path),
        None => Ok(AppSettings::default()),
    }
}

/// Writes the settings of the running application, creating the directory if needed.
pub fn save(settings: &AppSettings) -> Result<(), ShellError> {
    let path = crate::paths::settings_file()
        .ok_or_else(|| ShellError::store_io("the configuration directory could not be located on this system"))?;

    save_to(&path, settings)
}

fn write_atomically(path: &Path, contents: &[u8]) -> Result<(), ShellError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| ShellError::store_io(format!("cannot create {}: {error}", parent.display())))?;
    }

    let temporary = path.with_extension("json.tmp");
    let write = || -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        Ok(())
    };

    if let Err(error) = write() {
        // Leave nothing half-written behind for the next read to trip over.
        let _ = std::fs::remove_file(&temporary);
        return Err(ShellError::store_io(format!(
            "cannot write {}: {error}",
            temporary.display()
        )));
    }

    std::fs::rename(&temporary, path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        ShellError::store_io(format!("cannot replace {}: {error}", path.display()))
    })
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    fn settings(theme_mode: ThemeMode, terminal_theme: ThemeMode, locale: Option<&str>) -> AppSettings {
        AppSettings {
            theme_mode,
            terminal_theme,
            files_layout: FilesLayout::default(),
            session_layout: SessionLayout::default(),
            terminal_panel_height: None,
            sidebar_width: None,
            sidebar_open: sidebar_on(),
            monitor_enabled: monitor_on(),
            packed_transfer: false,
            locale: locale.map(str::to_owned),
        }
    }

    #[test]
    fn a_file_written_before_the_terminal_had_a_palette_still_loads() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        std::fs::write(&path, r#"{"schemaVersion":1,"themeMode":"dark","locale":"zh-CN"}"#).expect("write");

        let loaded = load_from(&path).expect("an older file is not an error");

        assert_eq!(loaded.theme_mode, ThemeMode::Dark);
        assert_eq!(
            loaded.terminal_theme,
            ThemeMode::System,
            "absent means follow the interface"
        );
        assert_eq!(
            loaded.session_layout,
            SessionLayout::Split,
            "absent means the default, and the default opens both halves"
        );
        assert_eq!(
            loaded.terminal_panel_height, None,
            "absent means the divider has never been moved"
        );
        assert!(
            loaded.monitor_enabled,
            "a file written before the monitor existed is not a person asking for it to be off"
        );
        assert!(
            !loaded.packed_transfer,
            "a file written before packed transfers existed is not a person asking for one"
        );
        assert_eq!(loaded.locale.as_deref(), Some("zh-CN"));
    }

    #[test]
    fn an_absent_file_means_nothing_has_been_chosen_yet() {
        let directory = tempdir().expect("temp dir");
        let loaded = load_from(&directory.path().join("settings.json")).expect("defaults");

        assert_eq!(loaded, AppSettings::default());
        assert!(loaded.is_untouched());
    }

    #[test]
    fn settings_survive_a_round_trip() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        let written = AppSettings {
            session_layout: SessionLayout::Terminal,
            terminal_panel_height: Some(320),
            // Off, so the round trip proves a choice survives rather than the default coming back.
            monitor_enabled: false,
            // On, for the same reason in the other direction: the default is off.
            packed_transfer: true,
            ..settings(ThemeMode::Light, ThemeMode::System, Some("zh-CN"))
        };

        save_to(&path, &written).expect("save");
        assert_eq!(load_from(&path).expect("load"), written);

        // Nothing is left over from writing.
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn a_theme_this_build_does_not_know_falls_back_instead_of_failing() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"schemaVersion":1,"themeMode":"midnight-aurora","locale":"en"}"#,
        )
        .expect("write");

        let loaded = load_from(&path).expect("a future theme is not a hard failure");

        assert_eq!(loaded.theme_mode, ThemeMode::System);
        assert_eq!(loaded.locale.as_deref(), Some("en"));
    }

    #[test]
    fn a_newer_schema_is_refused_rather_than_downgraded() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        std::fs::write(&path, r#"{"schemaVersion":2,"themeMode":"dark","locale":"en"}"#).expect("write");

        let error = load_from(&path).expect_err("a newer file must not be read as if it were ours");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::StoreSchemaNewer);
        assert_eq!(error.message_params().get("found").map(String::as_str), Some("2"));
    }

    #[test]
    fn broken_json_is_reported_and_not_swallowed() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        std::fs::write(&path, "{ not json").expect("write");

        let error = load_from(&path).expect_err("broken settings must be visible");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::StoreIo);
    }

    #[test]
    fn a_failed_write_leaves_the_previous_file_alone() {
        let directory = tempdir().expect("temp dir");
        let blocker = directory.path().join("blocker");
        std::fs::write(&blocker, "do not touch").expect("write");

        // The parent of the settings path is a file, so creating the directory fails.
        let error =
            save_to(&blocker.join("settings.json"), &AppSettings::default()).expect_err("cannot write through a file");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::StoreIo);
        assert_eq!(
            std::fs::read_to_string(&blocker).expect("still readable"),
            "do not touch"
        );
    }
}
