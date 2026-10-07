//! Non-secret preferences the interface keeps between runs.
//!
//! Secrets do not live here — those belong to the vault. What lives here is what a user
//! would not mind being read over their shoulder: how the interface looks, and what
//! language it speaks.

use serde::{Deserialize, Serialize};

/// How the file browser is laid out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilesLayout {
    /// A directory tree beside the listing: the shape a remote file system is easiest to move
    /// around in, and what somebody who works with servers expects.
    #[default]
    Tree,
    /// The listing alone, for a narrow window or a directory that is already where you want to be.
    List,
}

impl FilesLayout {
    pub const ALL: &'static [FilesLayout] = &[FilesLayout::Tree, FilesLayout::List];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tree => "tree",
            Self::List => "list",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|layout| layout.as_str() == id)
    }
}

/// How a session opens: a terminal on its own, or a terminal with its file panel under it.
///
/// The two are one tab either way — the panel draws the same file browser a file tab would, so
/// nothing about the session differs but its shape on screen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionLayout {
    /// The terminal alone. The file panel is one toggle away, on the terminal's own strip.
    Terminal,
    /// The terminal and the files for the same host, one above the other, in one tab. The panel
    /// follows the shell's directory, so a `cd` in the terminal takes it along.
    #[default]
    Split,
}

impl SessionLayout {
    pub const ALL: &'static [SessionLayout] = &[SessionLayout::Terminal, SessionLayout::Split];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Split => "split",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|layout| layout.as_str() == id)
    }
}

/// How the interface picks its colours.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    /// Follow the operating system, and keep following it while it changes.
    #[default]
    System,
    Dark,
    Light,
}

impl ThemeMode {
    pub const ALL: &'static [ThemeMode] = &[ThemeMode::System, ThemeMode::Dark, ThemeMode::Light];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|mode| mode.as_str() == id)
    }
}

impl std::fmt::Display for ThemeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The colour the interface is made of, on top of whichever of the two themes it is in.
///
/// A palette is not a third theme: light and dark are decided by the operating system or by the
/// person, and either can be worn in any of these. What a palette changes is the tint — the
/// background, the borders and the accent all move together to one hue — so the two settings are
/// orthogonal on purpose and are chosen in the interface as two separate fields.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePalette {
    /// The palette this build has always drawn in, and what a fresh install wears.
    #[default]
    Default,
    /// Cool grey-blue, softer than the greys of the default palette.
    Mist,
    /// Almost colourless, and the darkest of the light palettes — grey with the blue taken out.
    Graphite,
    /// Blue, saturated: the default palette's own family, taken further.
    Cobalt,
    /// Green-grey.
    Sage,
    /// Warm, for a window that should not read as a terminal at midnight.
    Amber,
    /// Rose.
    Blush,
    /// The editor's palette, for somebody who has been looking at it all day.
    Vscode,
    /// The other editor's palette.
    Idea,
    /// The Xcode palette: a bright blue accent on near-white.
    Xcode,
    /// The JetBrains palette: violet-leaning.
    Jetbrains,
    /// The Cursor palette.
    Cursor,
    /// Warm sand, for a window that reads as paper rather than as a screen.
    Claude,
    /// Whatever the person picked: the five colours in [`CustomColors`], drawn in place of one of
    /// the palettes above.
    Custom,
}

/// The five colours a custom palette is built from.
///
/// Deliberately few. Everything else a window is made of — its panels, its hovered rows, its muted
/// text, whether the text on the accent is white or black — is derived from these, so that a person
/// choosing colours cannot arrive at a combination nobody can read. The derivation lives in the
/// interface, in `apps/desktop/src/lib/settings/customColors.ts`, because that is what has to draw
/// it; this side only remembers what was picked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomColors {
    /// `#rrggbb`, which is the only shape a colour picker hands out and the only one accepted here.
    pub background: String,
    pub foreground: String,
    pub primary: String,
    pub border: String,
    pub card: String,
}

impl CustomColors {
    /// Whether every one of the five is a colour this build can draw.
    ///
    /// Six hex digits and nothing else: a name like `red` or a fourth channel would reach a
    /// stylesheet that cannot use it, and the window would come out half-drawn rather than wrong.
    pub fn is_drawable(&self) -> bool {
        [
            &self.background,
            &self.foreground,
            &self.primary,
            &self.border,
            &self.card,
        ]
        .iter()
        .all(|colour| is_hex_colour(colour))
    }

    /// What a custom palette is before anybody has chosen one, in the light theme.
    pub fn light_default() -> Self {
        Self {
            background: "#ffffff".to_owned(),
            foreground: "#0a0a0a".to_owned(),
            primary: "#171717".to_owned(),
            border: "#e5e5e5".to_owned(),
            card: "#fafafa".to_owned(),
        }
    }

    /// The same in the dark one, which is a different set rather than a darkening: the two are
    /// chosen apart, so one custom palette is really two.
    pub fn dark_default() -> Self {
        Self {
            background: "#131416".to_owned(),
            foreground: "#d7d7db".to_owned(),
            primary: "#d0d0d6".to_owned(),
            border: "#6e6e72".to_owned(),
            card: "#19191c".to_owned(),
        }
    }
}

fn is_hex_colour(value: &str) -> bool {
    value.len() == 7 && value.starts_with('#') && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

impl ThemePalette {
    pub const ALL: &'static [ThemePalette] = &[
        ThemePalette::Default,
        ThemePalette::Mist,
        ThemePalette::Graphite,
        ThemePalette::Cobalt,
        ThemePalette::Sage,
        ThemePalette::Amber,
        ThemePalette::Blush,
        ThemePalette::Vscode,
        ThemePalette::Idea,
        ThemePalette::Xcode,
        ThemePalette::Jetbrains,
        ThemePalette::Cursor,
        ThemePalette::Claude,
        ThemePalette::Custom,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Mist => "mist",
            Self::Graphite => "graphite",
            Self::Cobalt => "cobalt",
            Self::Sage => "sage",
            Self::Amber => "amber",
            Self::Blush => "blush",
            Self::Vscode => "vscode",
            Self::Idea => "idea",
            Self::Xcode => "xcode",
            Self::Jetbrains => "jetbrains",
            Self::Cursor => "cursor",
            Self::Claude => "claude",
            Self::Custom => "custom",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|palette| palette.as_str() == id)
    }
}

/// How round the interface's corners are.
///
/// One setting for the whole window rather than a radius per component: a corner is a property of
/// the way this build draws, and a component that chose its own roundness would be the one thing
/// that did not change when somebody asked for squarer corners. The scale itself lives in the
/// stylesheet — three steps, and this says which one is worn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CornerStyle {
    /// Nothing is rounded: every corner is a right angle.
    None,
    /// A small radius, for somebody who wants the boxes to look like boxes.
    Small,
    /// What this build has always drawn, and therefore what a fresh install gets.
    #[default]
    Large,
}

impl CornerStyle {
    pub const ALL: &'static [CornerStyle] = &[CornerStyle::None, CornerStyle::Small, CornerStyle::Large];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Small => "small",
            Self::Large => "large",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|style| style.as_str() == id)
    }
}

/// What the home page is for.
///
/// Two pages rather than one page with parts that can be turned off, because the two answer
/// different questions: somebody who has never connected anything wants to know what this is, and
/// somebody who has wants their own hosts and the thing they were doing. A page made of optional
/// pieces would be a third answer nobody asked for, and would have to guess which pieces belong
/// together — the counters and the host names are the same answer, not two.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HomeLayout {
    /// What this build is, and the steps of a first connection: no names, no counts, no history.
    Intro,
    /// The working page: what the library holds, the hosts used last, and the actions that start
    /// something. What this build has always drawn, so a fresh install opens on it.
    #[default]
    Workspace,
}

impl HomeLayout {
    pub const ALL: &'static [HomeLayout] = &[HomeLayout::Intro, HomeLayout::Workspace];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Intro => "intro",
            Self::Workspace => "workspace",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|layout| layout.as_str() == id)
    }
}

/// The interface scales this build offers, in percent of its own size.
///
/// A closed list rather than a free number, because the setting is a choice made by looking at the
/// window: a scale nobody offers cannot be arrived at by using the interface, and honouring one
/// found in the file would leave the menu showing a value it does not contain. A file naming one of
/// these is honoured; anything else reads as the default.
pub const UI_SCALE_PERCENTS: &[u32] = &[75, 90, 100, 110, 125, 150, 175, 200];

/// Everything the settings file holds.
///
/// Both fields are optional in effect: absent means "not chosen yet", which is different
/// from "chosen to be the default". The interface follows the system until someone says
/// otherwise, so a fresh install needs no first-run dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
    pub theme_mode: ThemeMode,
    /// The tint worn on top of the theme above.
    ///
    /// Absent in a file written before there was more than one way for the interface to look, which
    /// reads as the palette this build has always drawn in.
    #[serde(default)]
    pub theme_palette: ThemePalette,
    /// Which palette the terminal draws in.
    ///
    /// The same three values as the interface's theme, with one meaning changed: `system` here is
    /// "follow the interface", because a terminal that follows the operating system while the
    /// window around it does not would be the one thing on screen that disagrees. A light
    /// interface with a dark terminal is a common and deliberate choice, so it is kept.
    #[serde(default)]
    pub terminal_theme: ThemeMode,
    /// How the file browser is laid out.
    #[serde(default)]
    pub files_layout: FilesLayout,
    /// How a session opens: the terminal on its own, or the terminal with its file panel under it.
    #[serde(default)]
    pub session_layout: SessionLayout,
    /// How tall the file panel under a terminal is, in pixels.
    ///
    /// A wish rather than a measurement: the interface clamps it against the window it has, so a
    /// height that no longer fits is honoured as far as it can be rather than refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_panel_height: Option<u32>,
    /// How wide the session sidebar is, in pixels.
    ///
    /// The same kind of wish as the height above and clamped by the same rule: a sidebar wider than
    /// the window it is beside is honoured as far as the workspace allows rather than refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar_width: Option<u32>,
    /// Whether the session sidebar is on screen.
    ///
    /// On unless somebody puts it away: the sessions live there, and a window that opens with an
    /// empty left edge reads as a build that has lost them.
    #[serde(default = "sidebar_on")]
    pub sidebar_open: bool,
    /// Whether the host monitor runs beside the workspace.
    ///
    /// On unless somebody says otherwise, unlike the fields above: those describe a choice the
    /// interface would otherwise have to guess at, while this one is a feature that is either
    /// there or gone. Turning it off stops the sampling as well as the drawing — a hidden panel
    /// that went on running commands on the far side would be worse than one that is on screen.
    #[serde(default = "monitor_on")]
    pub monitor_enabled: bool,
    /// Whether a directory travels as one archive instead of file by file.
    ///
    /// Off unless somebody turns it on, unlike the monitor above: this one changes what a transfer
    /// *is* — a `.tar.gz` on disk rather than the files themselves — and a build that started packing
    /// by itself would surprise somebody who only wanted their folder.
    #[serde(default)]
    pub packed_transfer: bool,
    /// How much the whole interface is scaled, in percent of the size it was drawn at.
    ///
    /// The window is the same size either way: this is the interface inside it growing, which is
    /// what a high-density display needs and what the operating system's own scaling cannot know
    /// about a window that is already open. A wish like the panel height above — the webview clamps
    /// what it is given — rather than a measurement of anything.
    #[serde(default = "ui_scale_default")]
    pub ui_scale: u32,
    /// How round the interface's corners are. Absent in a file written before this was a choice,
    /// which reads as the roundness this build has always drawn.
    #[serde(default)]
    pub corner_style: CornerStyle,
    /// Which of the two home pages is drawn. Absent in a file written before there were two, which
    /// reads as the one this build has always drawn.
    #[serde(default)]
    pub home_layout: HomeLayout,
    /// The family the interface draws its text in, as a `font-family` stack.
    ///
    /// The whole stack rather than one name, and absent means the one this build ships with: somebody
    /// picking a font is choosing the first family that resolves, and what follows it is how the
    /// interface stays readable when that family turns out not to be installed after all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_font: Option<String>,
    /// The same for the terminal, which is a choice of its own: a window is read in a proportional
    /// face and a terminal in a fixed-width one, and somebody who wants their editor's font under the
    /// shell prompt rarely wants it in the file listing beside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_font: Option<String>,
    /// The five colours the `custom` palette is drawn from, for the light theme. Absent means
    /// nobody has picked any, and the ones a fresh install starts from apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_colors: Option<CustomColors>,
    /// The same for the dark theme. Two sets rather than one because the two are chosen apart: a
    /// custom palette that only held one would be unreadable in whichever theme it was not made for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_colors_dark: Option<CustomColors>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

/// The interface's own size when nobody has changed it.
///
/// A named function rather than a constant because `serde` wants a path, like the two below. It is
/// 100 — the size the interface is drawn at by the code that draws it — and it is deliberately
/// written out rather than derived from the list above: "the middle of the menu" would be a
/// different number every time the menu grew. That it is one of the choices is a test's business,
/// not arithmetic's.
pub fn ui_scale_default() -> u32 {
    100
}

/// Reads a scale out of a settings file, refusing one no menu offers.
///
/// Garbage is not an error here any more than an unknown theme is: the interface opens at its own
/// size and says nothing, because a display preference nobody can have chosen is not worth a
/// message that a user cannot act on.
pub fn normalize_ui_scale(percent: u32) -> u32 {
    if UI_SCALE_PERCENTS.contains(&percent) {
        percent
    } else {
        ui_scale_default()
    }
}

/// The monitor's default, which is "on".
///
/// A named function rather than a constant because `serde` wants a path. It is public because
/// there are two readers of it — this type's own default and the settings file's — and a default
/// written down twice is a default that drifts.
pub fn monitor_on() -> bool {
    true
}

/// The sidebar's default, which is "on screen".
///
/// A named function for the same reason as the monitor's above.
pub fn sidebar_on() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme_mode: ThemeMode::default(),
            theme_palette: ThemePalette::default(),
            terminal_theme: ThemeMode::default(),
            files_layout: FilesLayout::default(),
            session_layout: SessionLayout::default(),
            terminal_panel_height: None,
            sidebar_width: None,
            sidebar_open: sidebar_on(),
            monitor_enabled: monitor_on(),
            packed_transfer: false,
            ui_scale: ui_scale_default(),
            corner_style: CornerStyle::default(),
            home_layout: HomeLayout::default(),
            ui_font: None,
            terminal_font: None,
            custom_colors: None,
            custom_colors_dark: None,
            locale: None,
        }
    }
}

impl AppSettings {
    /// Whether anything has been chosen yet. Used by the interface to decide whether the
    /// system language and theme still apply.
    pub fn is_untouched(&self) -> bool {
        self.theme_mode == ThemeMode::System
            && self.theme_palette == ThemePalette::Default
            && self.ui_scale == ui_scale_default()
            && self.corner_style == CornerStyle::Large
            && self.home_layout == HomeLayout::Workspace
            && self.ui_font.is_none()
            && self.terminal_font.is_none()
            && self.terminal_theme == ThemeMode::System
            && self.files_layout == FilesLayout::Tree
            && self.session_layout == SessionLayout::Split
            && self.terminal_panel_height.is_none()
            && self.sidebar_width.is_none()
            && self.sidebar_open
            && self.monitor_enabled
            && !self.packed_transfer
            && self.custom_colors.is_none()
            && self.custom_colors_dark.is_none()
            && self.locale.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_scale_is_one_the_menu_offers() {
        assert!(
            UI_SCALE_PERCENTS.contains(&ui_scale_default()),
            "a default the interface cannot show or take back is one nobody could leave"
        );
    }

    #[test]
    fn a_custom_colour_this_build_cannot_draw_is_refused() {
        let mut colours = CustomColors::light_default();
        assert!(colours.is_drawable(), "the defaults are drawable");

        for undrawable in ["red", "#fff", "#12345g", "#1234567", ""] {
            colours.background = undrawable.to_owned();
            assert!(
                !colours.is_drawable(),
                "`{undrawable}` would reach a stylesheet that cannot use it"
            );
        }
    }

    #[test]
    fn every_home_layout_round_trips_through_its_own_name() {
        for layout in HomeLayout::ALL {
            assert_eq!(
                HomeLayout::from_id(layout.as_str()),
                Some(*layout),
                "a home page whose name does not read back is one the settings file cannot hold"
            );
        }
    }

    #[test]
    fn every_corner_style_round_trips_through_its_own_name() {
        for style in CornerStyle::ALL {
            assert_eq!(
                CornerStyle::from_id(style.as_str()),
                Some(*style),
                "a corner style whose name does not read back is one the settings file cannot hold"
            );
        }
    }

    #[test]
    fn every_palette_round_trips_through_its_own_name() {
        for palette in ThemePalette::ALL {
            assert_eq!(
                ThemePalette::from_id(palette.as_str()),
                Some(*palette),
                "a palette whose name does not read back is one the settings file cannot hold"
            );
        }
    }
}
