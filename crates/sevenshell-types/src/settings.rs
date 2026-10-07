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
    ///
    /// What a fresh install opens on, because a fresh install is a window with nothing connected in it,
    /// and that is who this page is for. Somebody with hosts in the library is one setting away from
    /// the working page.
    #[default]
    Intro,
    /// The working page: what the library holds, the hosts used last, and the actions that start
    /// something.
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

/// How the chrome around the panels is drawn.
///
/// Two shapes rather than a set of switches, because the two are answers to one question — how much
/// of the window should be the thing being looked at — and every difference between them falls out of
/// that answer: panels floating with the window showing between them against panels flush with a
/// single line between them, tabs that sit apart as boxes against tabs joined into one bar, and
/// headers with room around what they say against headers that give the row back to the panel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppLayout {
    /// Panels floating as cards with the window showing between them, tabs as boxes of their own, and
    /// headers with room around what they say.
    ///
    /// What a fresh install gets. Both of these shapes are this build's own — neither is a leftover —
    /// and the choice exists because which one somebody wants depends on whether they work in one panel
    /// or move between several.
    #[default]
    Separated,
    /// Panels flush against each other with a single line between them, one continuous tab bar, and
    /// tighter headers: the same information in fewer pixels.
    Classic,
}

impl AppLayout {
    pub const ALL: &'static [AppLayout] = &[AppLayout::Separated, AppLayout::Classic];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Separated => "separated",
            Self::Classic => "classic",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|layout| layout.as_str() == id)
    }
}

/// What the tab bar does when the tabs no longer fit in one row.
///
/// Two answers rather than a number, because the two answer a question the panels' own sizes do not:
/// whether the bar is allowed to take height from the panel underneath it. Scrolling keeps the bar one
/// row tall and puts the rest out of sight; wrapping keeps every tab in sight and pays for it in
/// vertical room.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TabLayout {
    /// One row that scrolls sideways. What this build has always done, so a fresh install gets it.
    #[default]
    Scroll,
    /// Rows that wrap until every tab is on screen.
    Wrap,
}

impl TabLayout {
    pub const ALL: &'static [TabLayout] = &[TabLayout::Scroll, TabLayout::Wrap];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scroll => "scroll",
            Self::Wrap => "wrap",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|layout| layout.as_str() == id)
    }
}

/// Which edge of the window the tab bar is drawn against.
///
/// The four edges rather than two, because a bar at the top and a bar at the side are answers to
/// different problems: a top bar costs height — the thing a terminal has least of — while a side bar
/// costs width and pays for it with a column of names instead of a row of tabs. A bar on the left or
/// the right is drawn as a column, which is also why wrapping is a choice only the horizontal ones
/// have.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TabPlacement {
    /// Across the top of the panel. What this build has always drawn, so a fresh install gets it.
    #[default]
    Top,
    /// Across the bottom, which is the same bar with the room it takes from the other end.
    Bottom,
    /// Down the left, as a column of names.
    Left,
    /// Down the right, which is the same column on the other side.
    Right,
}

impl TabPlacement {
    pub const ALL: &'static [TabPlacement] = &[
        TabPlacement::Top,
        TabPlacement::Bottom,
        TabPlacement::Left,
        TabPlacement::Right,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|edge| edge.as_str() == id)
    }

    /// Whether the bar is drawn as a column rather than as a row.
    pub const fn is_vertical(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

/// What the tabs are grouped by when they are drawn.
///
/// One axis rather than the three a database client has, because this build's tabs belong to sessions
/// and to nothing else: a terminal, a file browser and a host's pages opened from one session are the
/// same kind of togetherness the other application gets from a connection, and there is no second axis
/// under it to group by.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TabGroupMode {
    /// Every tab in one run. What this build has always drawn.
    #[default]
    None,
    /// One run per session, under the session's name.
    Session,
}

impl TabGroupMode {
    pub const ALL: &'static [TabGroupMode] = &[TabGroupMode::None, TabGroupMode::Session];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Session => "session",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|mode| mode.as_str() == id)
    }
}

/// The order the tabs are drawn in.
///
/// The order they were opened is the manual one, and it is what the bar has always shown. Sorting
/// reorders the drawing and never the tabs themselves, so going back to manual puts everything where
/// it was — and only the manual order is the one somebody can drag.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TabSortMode {
    /// The order they were opened in, which is the order they can be dragged into.
    #[default]
    Manual,
    /// Oldest first, by when each tab was opened.
    CreatedAsc,
    /// By the name each tab draws.
    TitleAsc,
}

impl TabSortMode {
    pub const ALL: &'static [TabSortMode] = &[TabSortMode::Manual, TabSortMode::CreatedAsc, TabSortMode::TitleAsc];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::CreatedAsc => "created-asc",
            Self::TitleAsc => "title-asc",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|mode| mode.as_str() == id)
    }
}

/// How wide a horizontal tab is allowed to be drawn.
///
/// A closed list of steps rather than a number of pixels, for the reason the interface scale is one:
/// it is a choice made by looking at the window, and a width nobody offers cannot be arrived at by
/// using the interface. A name longer than the step is truncated, and the whole of it is shown on
/// hover.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TabMaxWidth {
    /// As wide as the name asks for, up to what the bar can give it. A fresh install gets this, and so
    /// does a file written before the width was a choice.
    #[default]
    Unlimited,
    /// Ten rem: a name read by its first word.
    Compact,
    /// Twelve and a half rem.
    Medium,
    /// Fifteen rem.
    Standard,
    /// Twenty rem, for names that are sentences.
    Wide,
}

impl TabMaxWidth {
    pub const ALL: &'static [TabMaxWidth] = &[
        TabMaxWidth::Unlimited,
        TabMaxWidth::Compact,
        TabMaxWidth::Medium,
        TabMaxWidth::Standard,
        TabMaxWidth::Wide,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unlimited => "unlimited",
            Self::Compact => "compact",
            Self::Medium => "medium",
            Self::Standard => "standard",
            Self::Wide => "wide",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|width| width.as_str() == id)
    }
}

/// How the picture behind the window is painted when it does not match the window's shape.
///
/// The five the desktop has taught everybody: fill crops it to cover, fit keeps all of it and leaves
/// bars, stretch distorts it, center paints it at its own size, and tile repeats it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackgroundDisplayMode {
    #[default]
    Fill,
    Fit,
    Stretch,
    Center,
    Tile,
}

impl BackgroundDisplayMode {
    pub const ALL: &'static [BackgroundDisplayMode] = &[
        BackgroundDisplayMode::Fill,
        BackgroundDisplayMode::Fit,
        BackgroundDisplayMode::Stretch,
        BackgroundDisplayMode::Center,
        BackgroundDisplayMode::Tile,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fill => "fill",
            Self::Fit => "fit",
            Self::Stretch => "stretch",
            Self::Center => "center",
            Self::Tile => "tile",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|mode| mode.as_str() == id)
    }
}

/// The most the surfaces are allowed to let through, in percent.
///
/// Not all of it: a window whose own surfaces are entirely transparent is a window whose text floats
/// over a photograph, and the point of the setting is a picture you can still work on.
pub const BACKGROUND_TRANSPARENCY_MAX: u8 = 95;

/// The most blur this build will paint behind a window, in pixels.
pub const BACKGROUND_BLUR_MAX: u8 = 20;

/// The transparency a fresh install gets: a fifth of the way to the picture.
pub fn background_transparency_default() -> u8 {
    20
}

/// The picture drawn behind the whole window.
///
/// The file itself is not named here. It is copied into this installation's own directory when it is
/// chosen — so that moving or deleting the original does not empty the background — and the interface
/// reads it back through a command that takes no path at all: see `fileName`, which is a name to show
/// and nothing more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundImage {
    /// What the picture is called, as the person who chose it knows it. Absent or empty means there is
    /// no picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(default)]
    pub display_mode: BackgroundDisplayMode,
    /// How much the window's own surfaces let the picture through, in percent: 0 is an opaque window
    /// and [`BACKGROUND_TRANSPARENCY_MAX`] the most this build offers. The slider is labelled with this
    /// number, which is why the *transparency* is stored rather than the opacity it is the complement
    /// of: a settings file that says 20 and a label that says 20% are the same thing.
    #[serde(default = "background_transparency_default")]
    pub transparency: u8,
    /// How far the picture is blurred, in pixels.
    #[serde(default)]
    pub blur: u8,
}

impl Default for BackgroundImage {
    fn default() -> Self {
        Self {
            file_name: None,
            display_mode: BackgroundDisplayMode::default(),
            transparency: background_transparency_default(),
            blur: 0,
        }
    }
}

impl BackgroundImage {
    /// Whether there is a picture at all.
    ///
    /// An empty name is no picture: the field is optional on the wire, and a name that is only spaces is
    /// not one this build will try to read a file for.
    pub fn is_set(&self) -> bool {
        self.file_name.as_deref().is_some_and(|name| !name.trim().is_empty())
    }
}

/// The sizes the terminal may be drawn at, in pixels, and the one a fresh install has.
///
/// A closed range with a default inside it rather than a free number, for the reason the interface
/// scale is a list: the size is chosen by looking at the window, the two ends are what the slider
/// offers, and a file naming something outside them is read as the default rather than as a terminal
/// nobody can read.
pub const TERMINAL_FONT_SIZE_MIN: u8 = 10;
pub const TERMINAL_FONT_SIZE_MAX: u8 = 24;

/// The size this build draws a terminal at until somebody says otherwise.
///
/// Thirteen, which is small enough for a hundred columns and large enough to read: a terminal is read
/// in lines, and the number of lines is what the size trades against.
pub fn terminal_font_size_default() -> u8 {
    13
}

/// Keeps a size inside the range the slider offers.
pub fn normalize_terminal_font_size(size: u8) -> u8 {
    size.clamp(TERMINAL_FONT_SIZE_MIN, TERMINAL_FONT_SIZE_MAX)
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
    /// The picture drawn behind the whole window, if any.
    #[serde(default)]
    pub background_image: BackgroundImage,
    /// Whether the shell keeps an icon in the system tray.
    ///
    /// Absent in a file written before this was a choice, which reads as on for the reason the default
    /// above gives: with no icon and a window that hides, there would be nothing to bring it back.
    #[serde(default = "tray_icon_on")]
    pub show_tray_icon: bool,
    /// Whether closing the window ends the application instead of putting it away.
    ///
    /// Off unless somebody turns it on, and this one default is a *change* to what this build did
    /// before the choice existed: closing used to end the application because there was nowhere for it
    /// to go. Now there is a tray, so closing puts the window away — which is what the setting's own
    /// words describe — and this is the switch that gives the old behaviour back. A file written
    /// before the choice therefore opens onto the new behaviour, deliberately: it is the one this
    /// build ships with, and the other is one switch away.
    #[serde(default)]
    pub quit_on_close: bool,
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
    /// How the chrome around the panels is drawn. Absent in a file written before this was a choice,
    /// which reads as the shape this build has always drawn.
    #[serde(default)]
    pub app_layout: AppLayout,

    /// What the tab bar does when the tabs no longer fit in one row. Absent in a file written before
    /// this was a choice, which reads as the one row that scrolls this build has always drawn.
    #[serde(default)]
    pub tab_layout: TabLayout,
    /// Which edge the tab bar is drawn against.
    #[serde(default)]
    pub tab_placement: TabPlacement,
    /// What the tabs are grouped by when they are drawn.
    #[serde(default)]
    pub tab_group_mode: TabGroupMode,
    /// The order the tabs are drawn in.
    #[serde(default)]
    pub tab_sort_mode: TabSortMode,
    /// How wide a horizontal tab may be drawn.
    #[serde(default)]
    pub tab_max_width: TabMaxWidth,
    /// The family the interface draws its text in, as a `font-family` stack.
    ///
    /// The whole stack rather than one name, and absent means the one this build ships with: somebody
    /// picking a font is choosing the first family that resolves, and what follows it is how the
    /// interface stays readable when that family turns out not to be installed after all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_font: Option<String>,
    /// How large the terminal draws its text, in pixels.
    ///
    /// Its own setting rather than a share of the interface scale: a terminal is measured in columns
    /// and lines rather than in legs of a form, and the size somebody wants their shell at has nothing
    /// to do with the size they want the window's own text at.
    #[serde(default = "terminal_font_size_default")]
    pub terminal_font_size: u8,
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

/// Whether the shell keeps an icon in the system tray.
///
/// A named function for the same reason as the two above: it is a default two readers have to agree
/// on, and the default is "on" because a window that puts itself away when it is closed needs a way
/// back — the tray is that way, and turning it off is a choice somebody makes with the other setting
/// in view.
pub fn tray_icon_on() -> bool {
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
            show_tray_icon: tray_icon_on(),
            quit_on_close: false,
            background_image: BackgroundImage::default(),
            packed_transfer: false,
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
            && self.home_layout == HomeLayout::Intro
            && self.app_layout == AppLayout::Separated
            && self.tab_layout == TabLayout::Scroll
            && self.tab_placement == TabPlacement::Top
            && self.tab_group_mode == TabGroupMode::None
            && self.tab_sort_mode == TabSortMode::Manual
            && self.tab_max_width == TabMaxWidth::Unlimited
            && self.ui_font.is_none()
            && self.terminal_font.is_none()
            && self.terminal_font_size == terminal_font_size_default()
            && self.terminal_theme == ThemeMode::System
            && self.files_layout == FilesLayout::Tree
            && self.session_layout == SessionLayout::Split
            && self.terminal_panel_height.is_none()
            && self.sidebar_width.is_none()
            && self.sidebar_open
            && self.monitor_enabled
            && self.show_tray_icon
            && !self.quit_on_close
            && !self.background_image.is_set()
            && self.background_image.display_mode == BackgroundDisplayMode::Fill
            && self.background_image.transparency == background_transparency_default()
            && self.background_image.blur == 0
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
    fn the_default_terminal_size_is_one_the_slider_offers() {
        let size = terminal_font_size_default();
        assert!(
            (TERMINAL_FONT_SIZE_MIN..=TERMINAL_FONT_SIZE_MAX).contains(&size),
            "a default past the end of its own slider is one nobody could leave"
        );
        assert_eq!(normalize_terminal_font_size(0), TERMINAL_FONT_SIZE_MIN);
        assert_eq!(normalize_terminal_font_size(200), TERMINAL_FONT_SIZE_MAX);
        assert_eq!(normalize_terminal_font_size(16), 16);
    }

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
    fn every_app_layout_round_trips_through_its_own_name() {
        for layout in AppLayout::ALL {
            assert_eq!(
                AppLayout::from_id(layout.as_str()),
                Some(*layout),
                "a layout whose name does not read back is one the settings file cannot hold"
            );
        }
        assert_eq!(
            AppLayout::default(),
            AppLayout::Separated,
            "the default is the floating one, which is what a fresh install is meant to open onto"
        );
        assert_eq!(
            HomeLayout::default(),
            HomeLayout::Intro,
            "and the page it opens on is the one written for somebody with nothing connected yet"
        );
    }

    #[test]
    fn every_tab_layout_round_trips_through_its_own_name() {
        for layout in TabLayout::ALL {
            assert_eq!(
                TabLayout::from_id(layout.as_str()),
                Some(*layout),
                "a tab bar whose name does not read back is one the settings file cannot hold"
            );
        }
    }

    #[test]
    fn every_tab_edge_round_trips_through_its_own_name() {
        for edge in TabPlacement::ALL {
            assert_eq!(
                TabPlacement::from_id(edge.as_str()),
                Some(*edge),
                "an edge whose name does not read back is one the settings file cannot hold"
            );
        }
        assert!(
            TabPlacement::Left.is_vertical() && TabPlacement::Right.is_vertical(),
            "a bar down one side is drawn as a column"
        );
        assert!(
            !TabPlacement::Top.is_vertical() && !TabPlacement::Bottom.is_vertical(),
            "and a bar across the window is drawn as a row"
        );
    }

    #[test]
    fn every_tab_grouping_round_trips_through_its_own_name() {
        for mode in TabGroupMode::ALL {
            assert_eq!(
                TabGroupMode::from_id(mode.as_str()),
                Some(*mode),
                "a grouping whose name does not read back is one the settings file cannot hold"
            );
        }
    }

    #[test]
    fn every_tab_order_round_trips_through_its_own_name() {
        for mode in TabSortMode::ALL {
            assert_eq!(
                TabSortMode::from_id(mode.as_str()),
                Some(*mode),
                "an order whose name does not read back is one the settings file cannot hold"
            );
        }
    }

    #[test]
    fn every_display_mode_round_trips_through_its_own_name() {
        for mode in BackgroundDisplayMode::ALL {
            assert_eq!(
                BackgroundDisplayMode::from_id(mode.as_str()),
                Some(*mode),
                "a mode whose name does not read back is one the settings file cannot hold"
            );
        }
    }

    #[test]
    fn the_defaults_for_the_picture_are_ones_the_sliders_offer() {
        let image = BackgroundImage::default();
        assert_eq!(image.transparency, background_transparency_default());
        assert!(
            image.transparency <= BACKGROUND_TRANSPARENCY_MAX,
            "a default past the end of its own slider is one nobody could leave"
        );
        assert_eq!(image.blur, 0, "a fresh install has no picture, so nothing to blur");
        assert!(!image.is_set(), "and nothing to draw");
        assert!(
            !BackgroundImage {
                file_name: Some("   ".to_owned()),
                ..image.clone()
            }
            .is_set(),
            "a name of spaces is not a name"
        );
    }

    #[test]
    fn every_tab_width_round_trips_through_its_own_name() {
        for width in TabMaxWidth::ALL {
            assert_eq!(
                TabMaxWidth::from_id(width.as_str()),
                Some(*width),
                "a width whose name does not read back is one the settings file cannot hold"
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
