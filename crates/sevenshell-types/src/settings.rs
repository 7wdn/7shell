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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
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
            terminal_theme: ThemeMode::default(),
            files_layout: FilesLayout::default(),
            session_layout: SessionLayout::default(),
            terminal_panel_height: None,
            sidebar_width: None,
            sidebar_open: sidebar_on(),
            monitor_enabled: monitor_on(),
            packed_transfer: false,
            locale: None,
        }
    }
}

impl AppSettings {
    /// Whether anything has been chosen yet. Used by the interface to decide whether the
    /// system language and theme still apply.
    pub fn is_untouched(&self) -> bool {
        self.theme_mode == ThemeMode::System
            && self.terminal_theme == ThemeMode::System
            && self.files_layout == FilesLayout::Tree
            && self.session_layout == SessionLayout::Split
            && self.terminal_panel_height.is_none()
            && self.sidebar_width.is_none()
            && self.sidebar_open
            && self.monitor_enabled
            && !self.packed_transfer
            && self.locale.is_none()
    }
}
