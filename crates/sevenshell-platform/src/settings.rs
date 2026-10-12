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

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sevenshell_error::ShellError;
use sevenshell_types::{
    AppLayout, AppSettings, BACKGROUND_BLUR_MAX, BACKGROUND_TRANSPARENCY_MAX, BackgroundDisplayMode, BackgroundImage,
    CornerStyle, CustomColors, FilesLayout, HomeLayout, SessionLayout, SnippetProvider, TabGroupMode, TabLayout,
    TabMaxWidth, TabPlacement, TabSortMode, ThemeMode, ThemePalette, background_blur_default,
    background_transparency_default, command_completion_on, monitor_on, normalize_sync_auto_upload_minutes,
    normalize_terminal_font_size, normalize_ui_scale, sidebar_on, sync_auto_upload_minutes_default,
    terminal_font_size_default, tray_icon_on, ui_scale_default, update_check_on,
};

/// The picture behind the window, as the file holds it.
///
/// Its own struct rather than a field of the settings, because it is a group of four values that only
/// mean anything together: a file name with a mode and two numbers. Stored as written, and normalised
/// on the way back in — a hand-edited file naming another mode, or a transparency past the end of its
/// slider, reads as the default rather than as a window nobody can work on.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredBackgroundImage {
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    display_mode: String,
    /// The two the picture is drawn with, defaulting to the same numbers the wire type does: a file
    /// that names a picture but not how much of it shows through reads as 15% rather than as an opaque
    /// window nobody chose.
    #[serde(default = "background_transparency_default")]
    transparency: u8,
    #[serde(default = "background_blur_default")]
    blur: u8,
}

impl StoredBackgroundImage {
    fn from_settings(image: &BackgroundImage) -> Self {
        Self {
            file_name: image.file_name.clone(),
            display_mode: image.display_mode.as_str().to_owned(),
            transparency: image.transparency,
            blur: image.blur,
        }
    }

    fn to_settings(&self) -> BackgroundImage {
        BackgroundImage {
            // A name the machine cannot use is dropped rather than kept: the interface would otherwise
            // draw a picture that is not there, which is worse than drawing none.
            file_name: self
                .file_name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
            display_mode: BackgroundDisplayMode::from_id(&self.display_mode).unwrap_or_default(),
            transparency: self.transparency.min(BACKGROUND_TRANSPARENCY_MAX),
            blur: self.blur.min(BACKGROUND_BLUR_MAX),
        }
    }
}

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
    /// Absent in a file written before there was a picture behind the window.
    #[serde(default)]
    background_image: StoredBackgroundImage,
    /// The two the shell itself obeys, absent in a file written before the shell had a tray. The
    /// first is on for the same reason it is on in the wire type: a window that puts itself away
    /// needs a way back, and an absent value means "nobody has chosen" rather than "chosen off".
    #[serde(default = "tray_icon_on")]
    show_tray_icon: bool,
    #[serde(default)]
    quit_on_close: bool,
    /// Absent in a file written before the update channel existed, which reads as on: the answer only
    /// arrives if somebody asks, and a build that shipped with the check off would be a build nobody
    /// hears about a new release from.
    #[serde(default = "update_check_on")]
    update_check_on_startup: bool,
    /// Absent in a file written before there was a packed transfer, which reads as off — the default
    /// the feature ships with. A file written by a build that has it cannot be read back by one that
    /// does not, but that is what the schema version is for.
    #[serde(default)]
    packed_transfer: bool,
    /// Absent in a file written before the command line completed, which reads as the default it ships
    /// with — on — rather than as a list somebody switched off.
    #[serde(default = "command_completion_on")]
    command_completion: bool,
    /// Changed keys, by action. Absent actions read as the default the build ships, which is what
    /// makes adding an action in a later build nothing a file has to be migrated for.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    keybindings: BTreeMap<String, String>,
    /// Absent in a file written before the sidebar could be resized, which reads as "the divider
    /// has never been moved".
    #[serde(default)]
    sidebar_width: Option<u32>,
    /// The two panels split down the middle, absent in a file written before either could be
    /// resized, which reads the same way: the divider has never been moved.
    #[serde(default)]
    files_tree_width: Option<u32>,
    #[serde(default)]
    commands_library_width: Option<u32>,
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
    /// Absent in a file written before the tab bar could wrap, which reads as the one row this build
    /// has always drawn.
    #[serde(default)]
    tab_layout: String,
    /// The four below are absent in the same way: a file written before the bar could be moved,
    /// grouped, ordered or narrowed reads as the bar this build has always drawn.
    #[serde(default)]
    tab_placement: String,
    #[serde(default)]
    tab_group_mode: String,
    #[serde(default)]
    tab_sort_mode: String,
    #[serde(default)]
    tab_max_width: String,
    /// Absent in a file written before a font could be chosen, which reads as the one this build
    /// ships with rather than as somebody having picked something.
    #[serde(default)]
    ui_font: Option<String>,
    #[serde(default)]
    terminal_font: Option<String>,
    /// Absent in a file written before the terminal's size was a choice, and clamped on the way back in:
    /// a hand-edited file asking for a two-point terminal reads as the default.
    #[serde(default = "terminal_font_size_default")]
    terminal_font_size: u8,
    /// Absent in a file written before the interface could be coloured by hand, which reads as
    /// "nobody has picked any" — the defaults a fresh install starts from.
    #[serde(default)]
    custom_colors: Option<CustomColors>,
    #[serde(default)]
    custom_colors_dark: Option<CustomColors>,
    #[serde(default)]
    locale: Option<String>,
    /// The sync server's address, user name and remote path. Absent in a file written before this
    /// application could talk to one, and read back as "no server configured".
    #[serde(default)]
    sync_endpoint: Option<String>,
    #[serde(default)]
    sync_username: Option<String>,
    #[serde(default)]
    sync_remote_path: Option<String>,
    #[serde(default)]
    sync_include_secrets: bool,
    #[serde(default)]
    sync_auto_upload: bool,
    #[serde(default = "sync_auto_upload_minutes_default")]
    sync_auto_upload_minutes: u32,
    /// A snippet backup's service, its GitLab address, and the file each account writes to. Absent in
    /// a file written before there was anywhere to park a snippet, and read back as "nothing chosen".
    ///
    /// The service is a string here for the same reason the theme is: a file written by a build with
    /// a fourth provider in it has to open in this one, and a name this build does not know reads as
    /// nobody having chosen rather than as a refusal to open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sync_snippet_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sync_snippet_instance: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    sync_snippet_ids: BTreeMap<String, String>,
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
            files_tree_width: settings.files_tree_width,
            commands_library_width: settings.commands_library_width,
            sidebar_open: settings.sidebar_open,
            monitor_enabled: settings.monitor_enabled,
            background_image: StoredBackgroundImage::from_settings(&settings.background_image),
            show_tray_icon: settings.show_tray_icon,
            quit_on_close: settings.quit_on_close,
            update_check_on_startup: settings.update_check_on_startup,
            packed_transfer: settings.packed_transfer,
            command_completion: settings.command_completion,
            keybindings: settings.keybindings.clone(),
            ui_scale: settings.ui_scale,
            corner_style: settings.corner_style.as_str().to_owned(),
            home_layout: settings.home_layout.as_str().to_owned(),
            app_layout: settings.app_layout.as_str().to_owned(),
            tab_layout: settings.tab_layout.as_str().to_owned(),
            tab_placement: settings.tab_placement.as_str().to_owned(),
            tab_group_mode: settings.tab_group_mode.as_str().to_owned(),
            tab_sort_mode: settings.tab_sort_mode.as_str().to_owned(),
            tab_max_width: settings.tab_max_width.as_str().to_owned(),
            ui_font: settings.ui_font.clone(),
            terminal_font: settings.terminal_font.clone(),
            terminal_font_size: settings.terminal_font_size,
            custom_colors: settings.custom_colors.clone(),
            custom_colors_dark: settings.custom_colors_dark.clone(),
            locale: settings.locale.clone(),
            sync_endpoint: settings.sync_endpoint.clone(),
            sync_username: settings.sync_username.clone(),
            sync_remote_path: settings.sync_remote_path.clone(),
            sync_include_secrets: settings.sync_include_secrets,
            sync_auto_upload: settings.sync_auto_upload,
            sync_auto_upload_minutes: settings.sync_auto_upload_minutes,
            sync_snippet_provider: settings
                .sync_snippet_provider
                .map(|provider| provider.as_str().to_owned()),
            sync_snippet_instance: settings.sync_snippet_instance.clone(),
            sync_snippet_ids: settings.sync_snippet_ids.clone(),
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
            files_tree_width: self.files_tree_width,
            commands_library_width: self.commands_library_width,
            sidebar_open: self.sidebar_open,
            monitor_enabled: self.monitor_enabled,
            background_image: self.background_image.to_settings(),
            show_tray_icon: self.show_tray_icon,
            quit_on_close: self.quit_on_close,
            update_check_on_startup: self.update_check_on_startup,
            packed_transfer: self.packed_transfer,
            command_completion: self.command_completion,
            keybindings: self.keybindings,
            ui_scale: normalize_ui_scale(self.ui_scale),
            corner_style: CornerStyle::from_id(&self.corner_style).unwrap_or_default(),
            home_layout: HomeLayout::from_id(&self.home_layout).unwrap_or_default(),
            app_layout: AppLayout::from_id(&self.app_layout).unwrap_or_default(),
            tab_layout: TabLayout::from_id(&self.tab_layout).unwrap_or_default(),
            tab_placement: TabPlacement::from_id(&self.tab_placement).unwrap_or_default(),
            tab_group_mode: TabGroupMode::from_id(&self.tab_group_mode).unwrap_or_default(),
            tab_sort_mode: TabSortMode::from_id(&self.tab_sort_mode).unwrap_or_default(),
            tab_max_width: TabMaxWidth::from_id(&self.tab_max_width).unwrap_or_default(),
            // A name the machine does not have is not an error and not worth a message: the stack
            // falls through to what follows it, which is exactly why the whole stack is stored.
            ui_font: self.ui_font.filter(|font| !font.trim().is_empty()),
            terminal_font: self.terminal_font.filter(|font| !font.trim().is_empty()),
            terminal_font_size: normalize_terminal_font_size(self.terminal_font_size),
            // A hand-coloured set that cannot be drawn is dropped whole rather than per colour: half
            // of somebody's palette with defaults filled in for the rest is a window nobody asked
            // for, and there is nothing in it they could recognise to fix.
            custom_colors: self.custom_colors.filter(CustomColors::is_drawable),
            custom_colors_dark: self.custom_colors_dark.filter(CustomColors::is_drawable),
            locale: self.locale.filter(|locale| !locale.is_empty()),
            sync_endpoint: self.sync_endpoint.filter(|endpoint| !endpoint.trim().is_empty()),
            sync_username: self.sync_username.filter(|name| !name.trim().is_empty()),
            sync_remote_path: self.sync_remote_path.filter(|path| !path.trim().is_empty()),
            sync_include_secrets: self.sync_include_secrets,
            sync_auto_upload: self.sync_auto_upload,
            sync_auto_upload_minutes: normalize_sync_auto_upload_minutes(self.sync_auto_upload_minutes),
            sync_snippet_provider: self.sync_snippet_provider.as_deref().and_then(SnippetProvider::from_id),
            sync_snippet_instance: self.sync_snippet_instance.filter(|value| !value.trim().is_empty()),
            sync_snippet_ids: self.sync_snippet_ids,
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

/// Writes bytes to `path`, atomically.
///
/// Shared with the other file this crate keeps, so that "an interrupted write leaves the previous
/// file readable" is one rule rather than one per file.
pub(crate) fn write_atomically(path: &Path, contents: &[u8]) -> Result<(), ShellError> {
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
            files_tree_width: None,
            commands_library_width: None,
            sidebar_open: sidebar_on(),
            monitor_enabled: monitor_on(),
            background_image: BackgroundImage::default(),
            show_tray_icon: tray_icon_on(),
            quit_on_close: false,
            update_check_on_startup: update_check_on(),
            packed_transfer: false,
            command_completion: command_completion_on(),
            keybindings: BTreeMap::new(),
            theme_palette: ThemePalette::default(),
            ui_scale: ui_scale_default(),
            corner_style: CornerStyle::default(),
            home_layout: HomeLayout::default(),
            app_layout: AppLayout::default(),
            tab_layout: TabLayout::default(),
            tab_placement: TabPlacement::default(),
            tab_group_mode: TabGroupMode::default(),
            tab_sort_mode: TabSortMode::default(),
            tab_max_width: TabMaxWidth::default(),
            ui_font: None,
            terminal_font: None,
            terminal_font_size: terminal_font_size_default(),
            custom_colors: None,
            custom_colors_dark: None,
            locale: locale.map(str::to_owned),
            sync_endpoint: None,
            sync_username: None,
            sync_remote_path: None,
            sync_include_secrets: false,
            sync_auto_upload: false,
            sync_auto_upload_minutes: sync_auto_upload_minutes_default(),
            sync_snippet_provider: None,
            sync_snippet_instance: None,
            sync_snippet_ids: BTreeMap::new(),
        }
    }

    #[test]
    fn a_sync_server_survives_a_round_trip_and_a_cleared_field_is_no_server() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");

        let mut written = settings(ThemeMode::Dark, ThemeMode::System, None);
        written.sync_endpoint = Some("https://dav.example.com/remote.php/dav/files/alice/".to_owned());
        written.sync_username = Some("alice".to_owned());
        written.sync_remote_path = Some("7shell/sync/backup.json".to_owned());
        written.sync_include_secrets = true;
        save_to(&path, &written).expect("write");

        let loaded = load_from(&path).expect("read");

        assert_eq!(
            loaded.sync_endpoint.as_deref(),
            Some("https://dav.example.com/remote.php/dav/files/alice/")
        );
        assert_eq!(loaded.sync_username.as_deref(), Some("alice"));
        assert_eq!(loaded.sync_remote_path.as_deref(), Some("7shell/sync/backup.json"));
        assert!(loaded.sync_include_secrets);

        // A field somebody cleared is nobody having a server, not a server at the empty address: the
        // panel's fields are filled in together or not at all, and a half-cleared form must not read
        // as a configuration to dial. Whitespace counts as cleared for the same reason a font name
        // does — it is what an empty text box leaves behind.
        std::fs::write(
            &path,
            r#"{"schemaVersion":1,"syncEndpoint":"   ","syncUsername":"","syncRemotePath":"x"}"#,
        )
        .expect("write");

        let loaded = load_from(&path).expect("read");

        assert_eq!(loaded.sync_endpoint, None);
        assert_eq!(loaded.sync_username, None);
        assert_eq!(
            loaded.sync_remote_path.as_deref(),
            Some("x"),
            "the fields are read one by one"
        );
        assert!(
            !loaded.sync_include_secrets,
            "a file written before this existed is not a person asking for it"
        );
    }

    #[test]
    fn a_snippet_setup_survives_a_round_trip_and_a_service_this_build_does_not_know_is_no_choice() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");

        let mut written = settings(ThemeMode::Dark, ThemeMode::System, None);
        written.sync_snippet_provider = Some(SnippetProvider::GitLab);
        written.sync_snippet_instance = Some("https://gitlab.example.com".to_owned());
        written.sync_snippet_ids = BTreeMap::from([
            ("gitlab:https://gitlab.example.com".to_owned(), "42".to_owned()),
            ("github".to_owned(), "abc123".to_owned()),
        ]);
        save_to(&path, &written).expect("write");

        let loaded = load_from(&path).expect("read");

        assert_eq!(loaded.sync_snippet_provider, Some(SnippetProvider::GitLab));
        assert_eq!(
            loaded.sync_snippet_instance.as_deref(),
            Some("https://gitlab.example.com")
        );
        assert_eq!(
            loaded.sync_snippet_ids.get("github").map(String::as_str),
            Some("abc123"),
            "the id of a service somebody is not currently on is not thrown away"
        );

        // A file naming a service this build has never heard of is not a refusal to open — the same
        // rule the theme follows. What it reads as is "nobody has chosen", which is a state the panel
        // can draw and a person can act on.
        std::fs::write(
            &path,
            r#"{"schemaVersion":1,"syncSnippetProvider":"bitbucket","syncSnippetInstance":"  "}"#,
        )
        .expect("write");

        let loaded = load_from(&path).expect("read");
        assert_eq!(loaded.sync_snippet_provider, None);
        assert_eq!(loaded.sync_snippet_instance, None);
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
        assert!(
            loaded.keybindings.is_empty(),
            "a file written before keys could be changed has none changed"
        );
        assert!(
            loaded.command_completion,
            "a file written before the command line completed is not a person asking for it to be off"
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
            // Not the defaults, so the round trip proves that the two widths a divider was left at
            // come back rather than the file forgetting half of what it was given.
            files_tree_width: Some(300),
            commands_library_width: Some(200),
            // Off, so the round trip proves a choice survives rather than the default coming back.
            monitor_enabled: false,
            // On, for the same reason in the other direction: the default is off.
            packed_transfer: true,
            // Off, for the same reason again: this one's default is on.
            command_completion: false,
            // A key somebody changed, so the round trip proves a changed one survives — and that an
            // action nobody touched is still the default rather than the default written down.
            keybindings: BTreeMap::from([("terminal.find".to_owned(), "Mod+G".to_owned())]),
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
            // The same, for the two the shell reads: the icon off and the window quitting when it is
            // closed, which are both the opposite of what a fresh install has.
            show_tray_icon: false,
            quit_on_close: true,
            // Not the default, so the round trip proves a choice survives.
            terminal_font_size: 20,
            // And one with a picture behind it, written at the far end of every slider.
            background_image: BackgroundImage {
                file_name: Some("wallpaper.png".to_owned()),
                display_mode: BackgroundDisplayMode::Tile,
                transparency: BACKGROUND_TRANSPARENCY_MAX,
                blur: BACKGROUND_BLUR_MAX,
            },
            // The same, for the rest of the words the bar is stored as.
            tab_layout: TabLayout::Wrap,
            tab_placement: TabPlacement::Right,
            tab_group_mode: TabGroupMode::Session,
            tab_sort_mode: TabSortMode::TitleAsc,
            tab_max_width: TabMaxWidth::Compact,
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
