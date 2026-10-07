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
use sevenshell_types::{
    AppLayout, AppSettings, CornerStyle, CustomColors, FilesLayout, HomeLayout, SessionLayout, ThemeMode, ThemePalette,
    monitor_on, normalize_ui_scale, sidebar_on, ui_scale_default,
};

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
    /// Absent in a file written before the interface had more than one palette, which reads as the
    /// one it has always drawn in.
    #[serde(default)]
    theme_palette: String,
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
    /// Absent in a file written before the interface could be scaled, which reads as the size it
    /// has always been drawn at rather than as somebody having asked for something else.
    #[serde(default = "ui_scale_default")]
    ui_scale: u32,
    /// Absent in a file written before the corners were a choice, which reads as the roundness this
    /// build has always drawn rather than as somebody having asked for squares.
    #[serde(default)]
    corner_style: String,
    /// Absent in a file written before there were two home pages, which reads as the one this build
    /// has always drawn.
    #[serde(default)]
    home_layout: String,
    /// Absent in a file written before the chrome was a choice, which reads as the shape this build
    /// has always drawn.
    #[serde(default)]
    app_layout: String,
    /// Absent in a file written before a font could be chosen, which reads as the one this build
    /// ships with rather than as somebody having picked something.
    #[serde(default)]
    ui_font: Option<String>,
    #[serde(default)]
    terminal_font: Option<String>,
    /// Absent in a file written before the interface could be coloured by hand, which reads as
    /// "nobody has picked any" — the defaults a fresh install starts from.
    #[serde(default)]
    custom_colors: Option<CustomColors>,
    #[serde(default)]
    custom_colors_dark: Option<CustomColors>,
    #[serde(default)]
    locale: Option<String>,
}

impl StoredSettings {
    fn from_settings(settings: &AppSettings) -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            theme_mode: settings.theme_mode.as_str().to_owned(),
            theme_palette: settings.theme_palette.as_str().to_owned(),
            terminal_theme: settings.terminal_theme.as_str().to_owned(),
            files_layout: settings.files_layout.as_str().to_owned(),
            session_layout: settings.session_layout.as_str().to_owned(),
            terminal_panel_height: settings.terminal_panel_height,
            sidebar_width: settings.sidebar_width,
            sidebar_open: settings.sidebar_open,
            monitor_enabled: settings.monitor_enabled,
            packed_transfer: settings.packed_transfer,
            ui_scale: settings.ui_scale,
            corner_style: settings.corner_style.as_str().to_owned(),
            home_layout: settings.home_layout.as_str().to_owned(),
            app_layout: settings.app_layout.as_str().to_owned(),
            ui_font: settings.ui_font.clone(),
            terminal_font: settings.terminal_font.clone(),
            custom_colors: settings.custom_colors.clone(),
            custom_colors_dark: settings.custom_colors_dark.clone(),
            locale: settings.locale.clone(),
        }
    }

    fn into_settings(self) -> AppSettings {
        AppSettings {
            theme_mode: ThemeMode::from_id(&self.theme_mode).unwrap_or_default(),
            theme_palette: ThemePalette::from_id(&self.theme_palette).unwrap_or_default(),
            terminal_theme: ThemeMode::from_id(&self.terminal_theme).unwrap_or_default(),
            files_layout: FilesLayout::from_id(&self.files_layout).unwrap_or_default(),
            session_layout: SessionLayout::from_id(&self.session_layout).unwrap_or_default(),
            terminal_panel_height: self.terminal_panel_height,
            sidebar_width: self.sidebar_width,
            sidebar_open: self.sidebar_open,
            monitor_enabled: self.monitor_enabled,
            packed_transfer: self.packed_transfer,
            ui_scale: normalize_ui_scale(self.ui_scale),
            corner_style: CornerStyle::from_id(&self.corner_style).unwrap_or_default(),
            home_layout: HomeLayout::from_id(&self.home_layout).unwrap_or_default(),
            app_layout: AppLayout::from_id(&self.app_layout).unwrap_or_default(),
            // A name the machine does not have is not an error and not worth a message: the stack
            // falls through to what follows it, which is exactly why the whole stack is stored.
            ui_font: self.ui_font.filter(|font| !font.trim().is_empty()),
            terminal_font: self.terminal_font.filter(|font| !font.trim().is_empty()),
            // A hand-coloured set that cannot be drawn is dropped whole rather than per colour: half
            // of somebody's palette with defaults filled in for the rest is a window nobody asked
            // for, and there is nothing in it they could recognise to fix.
            custom_colors: self.custom_colors.filter(CustomColors::is_drawable),
            custom_colors_dark: self.custom_colors_dark.filter(CustomColors::is_drawable),
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
            theme_palette: ThemePalette::default(),
            ui_scale: ui_scale_default(),
            corner_style: CornerStyle::default(),
            home_layout: HomeLayout::default(),
            app_layout: AppLayout::default(),
            ui_font: None,
            terminal_font: None,
            custom_colors: None,
            custom_colors_dark: None,
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
        assert_eq!(
            loaded.theme_palette,
            ThemePalette::Default,
            "a file written before there was a choice of palette reads as the one that was always there"
        );
        assert_eq!(
            loaded.ui_scale,
            ui_scale_default(),
            "and the interface is the size it has always been"
        );
        assert_eq!(
            loaded.custom_colors, None,
            "a file written before colours could be picked by hand has picked none"
        );
        assert_eq!(loaded.locale.as_deref(), Some("zh-CN"));
    }

    #[test]
    fn a_palette_this_build_does_not_know_falls_back_instead_of_failing() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"schemaVersion":1,"themeMode":"dark","themePalette":"aubergine","uiScale":125}"#,
        )
        .expect("write");

        let loaded = load_from(&path).expect("a palette from a newer build is not a hard failure");

        assert_eq!(loaded.theme_palette, ThemePalette::Default);
        assert_eq!(
            loaded.ui_scale, 125,
            "the fields either side of the unknown one are still read"
        );
    }

    #[test]
    fn a_hand_coloured_set_is_read_back_and_a_broken_one_is_dropped() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        let chosen = CustomColors {
            background: "#101820".to_owned(),
            foreground: "#e8e8e8".to_owned(),
            primary: "#4f9cf0".to_owned(),
            border: "#2a3340".to_owned(),
            card: "#161f2b".to_owned(),
        };
        let written = AppSettings {
            theme_palette: ThemePalette::Custom,
            custom_colors: Some(chosen.clone()),
            ..AppSettings::default()
        };
        save_to(&path, &written).expect("save");
        assert_eq!(
            load_from(&path).expect("load").custom_colors,
            Some(chosen),
            "a colour somebody picked comes back as it was picked"
        );

        // One colour that is not a colour takes the whole set with it: half a palette with defaults
        // filled in is a window nobody chose and nothing in it to recognise.
        std::fs::write(
            &path,
            // `r##` rather than `r#`: every colour in this JSON opens with `"#`, which is where the
            // shorter delimiter ends.
            r##"{"schemaVersion":1,"themePalette":"custom","customColors":{"background":"rebeccapurple","foreground":"#e8e8e8","primary":"#4f9cf0","border":"#2a3340","card":"#161f2b"}}"##,
        )
        .expect("write");
        assert_eq!(load_from(&path).expect("load").custom_colors, None);
    }

    #[test]
    fn a_scale_no_menu_offers_reads_as_the_default() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        // The interface cannot reach a scale outside the list, so one in the file was either typed
        // by hand or written by a build with a different list — and either way it is not a choice
        // this build can show or take back.
        std::fs::write(&path, r#"{"schemaVersion":1,"uiScale":400}"#).expect("write");

        let loaded = load_from(&path).expect("an unusable scale is not a hard failure");

        assert_eq!(loaded.ui_scale, ui_scale_default());
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
            // Neither is the default, for the same reason again: a palette and a size both have to
            // come back as the ones that were chosen.
            theme_palette: ThemePalette::Claude,
            ui_scale: 150,
            // Not the default, so the round trip proves a choice survives.
            corner_style: CornerStyle::None,
            // Not the default, so the round trip proves a choice survives.
            home_layout: HomeLayout::Intro,
            // Not the default, so the round trip proves a choice survives.
            app_layout: AppLayout::Classic,
            ui_font: Some("'JetBrains Mono', monospace".to_owned()),
            terminal_font: Some("'Fira Code', monospace".to_owned()),
            custom_colors: Some(CustomColors::light_default()),
            custom_colors_dark: Some(CustomColors::dark_default()),
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
