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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// How tall the file panel under a terminal is, in pixels.
    ///
    /// A wish rather than a measurement: the interface clamps it against the window it has, so a
    /// height that no longer fits is honoured as far as it can be rather than refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_panel_height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

impl AppSettings {
    /// Whether anything has been chosen yet. Used by the interface to decide whether the
    /// system language and theme still apply.
    pub fn is_untouched(&self) -> bool {
        self.theme_mode == ThemeMode::System
            && self.terminal_theme == ThemeMode::System
            && self.files_layout == FilesLayout::Tree
            && self.terminal_panel_height.is_none()
            && self.locale.is_none()
    }
}
