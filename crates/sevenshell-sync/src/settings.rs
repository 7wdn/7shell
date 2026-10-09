//! The settings that travel, and the ones that stay.
//!
//! A separately shaped struct rather than `AppSettings` with fields blanked, for two reasons. The
//! file should not carry twenty nulls meaning "this build has a window size and chose not to send
//! it", and a new setting has to be *classified* here rather than travelling by accident — which is
//! the only mechanism that keeps a value describing one machine on that machine.
//!
//! What is deliberately not here: the window geometry, the interface scale, the fonts, the tray
//! icon and whether closing quits. Each describes this installation's display or one machine's
//! habits; carrying them would change how somebody else's window looks, which is not what "the same
//! setup" means.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sevenshell_types::{
    AppLayout, AppSettings, CornerStyle, CustomColors, FilesLayout, HomeLayout, SessionLayout, TabGroupMode, TabLayout,
    TabMaxWidth, TabPlacement, TabSortMode, ThemeMode, ThemePalette,
};

/// How many settings travel when nothing optional is set. Pinned by a test against the field list
/// below, because a count a dialog reads out loud is a number somebody will notice being wrong.
const REQUIRED_SETTINGS: u32 = 18;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableSettings {
    pub theme_mode: ThemeMode,
    pub theme_palette: ThemePalette,
    pub terminal_theme: ThemeMode,
    pub files_layout: FilesLayout,
    pub session_layout: SessionLayout,
    pub sidebar_open: bool,
    pub monitor_enabled: bool,
    pub packed_transfer: bool,
    pub command_completion: bool,
    /// Which key does which action. A map rather than a whole list, so an action this build does
    /// not know about is not a file this build refuses.
    pub keybindings: BTreeMap<String, String>,
    pub corner_style: CornerStyle,
    pub home_layout: HomeLayout,
    pub app_layout: AppLayout,
    pub tab_layout: TabLayout,
    pub tab_placement: TabPlacement,
    pub tab_group_mode: TabGroupMode,
    pub tab_sort_mode: TabSortMode,
    pub tab_max_width: TabMaxWidth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_colors: Option<CustomColors>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_colors_dark: Option<CustomColors>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

impl PortableSettings {
    /// Reads the settings that travel out of the ones this machine has.
    pub fn of(settings: &AppSettings) -> Self {
        Self {
            theme_mode: settings.theme_mode,
            theme_palette: settings.theme_palette,
            terminal_theme: settings.terminal_theme,
            files_layout: settings.files_layout,
            session_layout: settings.session_layout,
            sidebar_open: settings.sidebar_open,
            monitor_enabled: settings.monitor_enabled,
            packed_transfer: settings.packed_transfer,
            command_completion: settings.command_completion,
            keybindings: settings.keybindings.clone(),
            corner_style: settings.corner_style,
            home_layout: settings.home_layout,
            app_layout: settings.app_layout,
            tab_layout: settings.tab_layout,
            tab_placement: settings.tab_placement,
            tab_group_mode: settings.tab_group_mode,
            tab_sort_mode: settings.tab_sort_mode,
            tab_max_width: settings.tab_max_width,
            custom_colors: settings.custom_colors.clone(),
            custom_colors_dark: settings.custom_colors_dark.clone(),
            locale: settings.locale.clone(),
        }
    }

    /// Writes these over the settings this machine already has.
    ///
    /// Everything that describes the installation is left exactly as it was: a restore may change
    /// how the application looks, never where its window sits or how large its text is on a display
    /// it has never seen. The caller reads the settings that are here, applies this over them, and
    /// writes them back — which is also why this takes the settings rather than a path.
    pub fn apply_to(&self, settings: &mut AppSettings) {
        settings.theme_mode = self.theme_mode;
        settings.theme_palette = self.theme_palette;
        settings.terminal_theme = self.terminal_theme;
        settings.files_layout = self.files_layout;
        settings.session_layout = self.session_layout;
        settings.sidebar_open = self.sidebar_open;
        settings.monitor_enabled = self.monitor_enabled;
        settings.packed_transfer = self.packed_transfer;
        settings.command_completion = self.command_completion;
        settings.keybindings = self.keybindings.clone();
        settings.corner_style = self.corner_style;
        settings.home_layout = self.home_layout;
        settings.app_layout = self.app_layout;
        settings.tab_layout = self.tab_layout;
        settings.tab_placement = self.tab_placement;
        settings.tab_group_mode = self.tab_group_mode;
        settings.tab_sort_mode = self.tab_sort_mode;
        settings.tab_max_width = self.tab_max_width;
        settings.custom_colors = self.custom_colors.clone();
        settings.custom_colors_dark = self.custom_colors_dark.clone();
        settings.locale = self.locale.clone();
    }

    /// How many values this carries, which is what a catalog reads out.
    pub fn count(&self) -> u32 {
        REQUIRED_SETTINGS
            + u32::from(self.custom_colors.is_some())
            + u32::from(self.custom_colors_dark.is_some())
            + u32::from(self.locale.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_count_is_the_number_of_fields_that_are_always_there() {
        // If a field is added above and not counted, a dialog says one number too few and nobody
        // finds out until somebody counts them by hand.
        let settings = PortableSettings::of(&AppSettings::default());
        let counted = REQUIRED_SETTINGS
            + u32::from(settings.custom_colors.is_some())
            + u32::from(settings.custom_colors_dark.is_some())
            + u32::from(settings.locale.is_some());

        assert_eq!(settings.count(), counted);
        assert_eq!(
            serde_json::to_value(&settings)
                .expect("the settings serialize")
                .as_object()
                .expect("an object")
                .len() as u32,
            settings.count(),
            "the count has to be the number of values that were actually written"
        );
    }

    #[test]
    fn a_setting_that_describes_this_machine_does_not_travel() {
        let settings = AppSettings {
            ui_scale: 150,
            terminal_font_size: 20,
            terminal_panel_height: Some(400),
            ..AppSettings::default()
        };

        let travelling = PortableSettings::of(&settings);
        let mut destination = AppSettings::default();
        travelling.apply_to(&mut destination);

        assert_eq!(destination.ui_scale, AppSettings::default().ui_scale);
        assert_eq!(
            destination.terminal_font_size,
            AppSettings::default().terminal_font_size
        );
        assert_eq!(
            destination.terminal_panel_height,
            AppSettings::default().terminal_panel_height
        );
    }

    #[test]
    fn what_does_travel_arrives() {
        let settings = AppSettings {
            theme_palette: ThemePalette::Sage,
            tab_placement: TabPlacement::Left,
            locale: Some("zh-CN".to_owned()),
            ..AppSettings::default()
        };

        let mut destination = AppSettings::default();
        PortableSettings::of(&settings).apply_to(&mut destination);

        assert_eq!(destination.theme_palette, ThemePalette::Sage);
        assert_eq!(destination.tab_placement, TabPlacement::Left);
        assert_eq!(destination.locale.as_deref(), Some("zh-CN"));
    }
}
