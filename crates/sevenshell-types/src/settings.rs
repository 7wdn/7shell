//! Non-secret preferences the interface keeps between runs.
//!
//! Secrets do not live here — those belong to the vault. What lives here is what a user
//! would not mind being read over their shoulder: how the interface looks, and what
//! language it speaks.

use serde::{Deserialize, Serialize};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

impl AppSettings {
    /// Whether anything has been chosen yet. Used by the interface to decide whether the
    /// system language and theme still apply.
    pub fn is_untouched(&self) -> bool {
        self.theme_mode == ThemeMode::System && self.locale.is_none()
    }
}
