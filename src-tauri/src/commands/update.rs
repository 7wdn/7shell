//! The update channel: asking whether a newer release exists, and installing one.
//!
//! Two steps rather than one, because a person decides between them: the check says which version
//! is published and what it changed, and the install acts on the answer they saw. So the release a
//! check found waits here between the two calls — the same shape as the other pending answers in
//! the shell, and for the same reason: re-checking at install time could install a *different*
//! release than the one on screen.

use std::sync::Mutex;

use sevenshell_error::ShellError;
use sevenshell_types::{AvailableRelease, UpdateCheck, UpdateProgress};
use tauri::{AppHandle, Emitter as _, State};
use tauri_plugin_updater::{Update, UpdaterExt as _};

use crate::state::ShellState;

/// The event a look at the channel produces while it runs.
pub const PROGRESS_EVENT: &str = "update:progress";

/// The release a check found, waiting for somebody to say install it.
///
/// One slot: a check replaces whatever the last one left, which is what makes "check, look, decide"
/// mean the release that was looked at.
#[derive(Default)]
pub struct PendingUpdate {
    slot: Mutex<Option<Update>>,
}

impl PendingUpdate {
    /// Remembers the release a check found, replacing any earlier one.
    pub fn remember(&self, update: Update) {
        *self.lock() = Some(update);
    }

    /// Takes the release to install. Taking rather than borrowing, because installing consumes it:
    /// the two kinds of installer are both one-shot, and a second install of the same bytes would
    /// be a second installer run for nothing.
    pub fn take(&self) -> Option<Update> {
        self.lock().take()
    }

    /// Drops what was waiting. Called when a check finds nothing, so that a release the channel has
    /// since withdrawn is not installable from a stale answer.
    pub fn clear(&self) {
        *self.lock() = None;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Update>> {
        match self.slot.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// Asks the update channel whether a newer release is published.
///
/// A development run answers `NotInstalled` without touching the network: its bytes come from
/// `target/`, there is no installed bundle to replace, and a request to a release server from every
/// run of the tests is a request nobody asked for.
#[tauri::command]
pub async fn update_check(app: AppHandle, state: State<'_, ShellState>) -> Result<UpdateCheck, ShellError> {
    let current_version = app.package_info().version.to_string();

    if cfg!(debug_assertions) {
        return Ok(UpdateCheck::NotInstalled { current_version });
    }

    let updater = app
        .updater()
        .map_err(|error| ShellError::update_check_failed(error.to_string()))?;
    let found = updater
        .check()
        .await
        .map_err(|error| ShellError::update_check_failed(error.to_string()))?;

    match found {
        Some(update) => {
            let release = AvailableRelease {
                version: update.version.clone(),
                notes: update.body.clone(),
                published_at: update.date.map(|date| date.unix_timestamp()),
            };
            log::info!("the update channel offers {} over {}", release.version, current_version);
            state.pending_update().remember(update);
            Ok(UpdateCheck::Available {
                current_version,
                release,
            })
        }
        None => {
            state.pending_update().clear();
            Ok(UpdateCheck::UpToDate { current_version })
        }
    }
}

/// Downloads the checked release, verifies its signature, and hands it to the system's installer.
///
/// On Windows the installer takes over and this process is gone before the call could return; on
/// macOS and Linux the new version is in place and the application restarts into it. Either way
/// nothing here returns a success the interface could draw, which is why the interface treats the
/// call as the end of the conversation rather than waiting for an answer to keep talking about.
#[tauri::command]
pub async fn update_install(app: AppHandle, state: State<'_, ShellState>) -> Result<(), ShellError> {
    let Some(update) = state.pending_update().take() else {
        return Err(ShellError::update_not_pending());
    };

    // The chunk length rather than a running total: the plugin reports each piece as it arrives, and
    // a progress line that counted the same bytes twice would move backwards.
    let mut downloaded: u64 = 0;
    let progress_app = app.clone();
    let result = update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = progress_app.emit(PROGRESS_EVENT, UpdateProgress::Downloading { downloaded, total });
            },
            {
                let progress_app = app.clone();
                move || {
                    let _ = progress_app.emit(PROGRESS_EVENT, UpdateProgress::Installing);
                }
            },
        )
        .await;

    match result {
        Ok(()) => {
            log::info!("the update is installed; restarting into it");
            app.restart()
        }
        Err(error) => {
            log::warn!("the update could not be installed: {error}");
            Err(ShellError::update_install_failed(error.to_string()))
        }
    }
}
