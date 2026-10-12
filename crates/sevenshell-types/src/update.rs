//! What the update channel said, and how far an installation has got.
//!
//! The channel itself is the plugin's business and its types stay in the shell: nothing here
//! carries a download URL or a signature, because nothing in the interface acts on one. What
//! crosses the wire is the answer a person reads — a version, what it says it changed, and whether
//! the check could be made at all.

use serde::{Deserialize, Serialize};

/// What a look at the update channel found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum UpdateCheck {
    /// Nothing newer than the running version is published.
    UpToDate { current_version: String },
    /// A newer release is published, and what it says about itself.
    Available {
        current_version: String,
        release: AvailableRelease,
    },
    /// This build is not an installed application, so there is nothing to update.
    ///
    /// A development run is the case: its bytes come from `target/`, and an update installed over
    /// them would replace a bundle that is not there. Said as its own answer rather than as a
    /// failure, because nothing went wrong — the question is the one that does not apply.
    NotInstalled { current_version: String },
}

/// A published release, as the update channel describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableRelease {
    pub version: String,
    /// The release notes, as published.
    ///
    /// Plain text, and shown as written: the notes come from a changelog somebody edited, and
    /// nothing in this build renders a markup document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// When the channel published it, in seconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<i64>,
}

/// How far an installation has got.
///
/// Emitted while it runs rather than returned when it ends: an update is a download of tens of
/// megabytes, and a window that says nothing for that long reads as a click that did not take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum UpdateProgress {
    /// The signed package is coming down.
    Downloading {
        downloaded: u64,
        /// The size the channel announced, when it announced one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        total: Option<u64>,
    },
    /// The bytes arrived and their signature checked out; the installer has them.
    Installing,
}
