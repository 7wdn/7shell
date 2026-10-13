//! The update channel: asking whether a newer release exists, and installing one.
//!
//! Two steps rather than one, because a person decides between them: the check says which version
//! is published and what it changed, and the install acts on the answer they saw. So the release a
//! check found waits here between the two calls — the same shape as the other pending answers in
//! the shell, and for the same reason: re-checking at install time could install a *different*
//! release than the one on screen.

use std::sync::{Arc, Mutex};

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
    /// Shared rather than owned, because a download holds the release while it runs: a guard held
    /// across an `await` is a guard this command cannot hold at all, and what the download needs is
    /// the release's name and address rather than the only copy of it.
    slot: Mutex<Option<Arc<Update>>>,
    /// The release downloaded and not yet installed, and the version it belongs to.
    ///
    /// Kept by version because the two steps are separate now: a check that finds something newer
    /// while an archive is waiting must not have that archive installed under the new version's name.
    downloaded: Mutex<Option<(String, Vec<u8>)>>,
}

impl PendingUpdate {
    /// Remembers the release a check found, replacing any earlier one.
    pub fn remember(&self, update: Update) {
        *self.lock() = Some(Arc::new(update));
    }

    /// Takes the release to install. Taking rather than borrowing, because installing consumes it:
    /// the two kinds of installer are both one-shot, and a second install of the same bytes would
    /// be a second installer run for nothing.
    pub fn take(&self) -> Option<Arc<Update>> {
        self.lock().take()
    }

    /// A look at what is waiting, without taking it: what a download needs.
    pub fn peek(&self) -> Option<Arc<Update>> {
        self.lock().clone()
    }

    /// Drops what was waiting. Called when a check finds nothing, so that a release the channel has
    /// since withdrawn is not installable from a stale answer.
    pub fn clear(&self) {
        *self.lock() = None;
        *self.downloaded.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    /// Remembers an archive downloaded for a version.
    pub fn remember_download(&self, version: String, bytes: Vec<u8>) {
        *self.downloaded.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((version, bytes));
    }

    /// Takes the archive downloaded for *this* version, if there is one.
    pub fn take_download(&self, version: &str) -> Option<Vec<u8>> {
        let mut slot = self.downloaded.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match slot.as_ref() {
            Some((downloaded, _)) if downloaded == version => slot.take().map(|(_, bytes)| bytes),
            _ => None,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Arc<Update>>> {
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

/// Downloads the release a check found and keeps the archive, installing nothing.
///
/// The two steps are separate because a person decides between them: what is downloaded is not what
/// is running, and "restart and update" is a decision somebody takes when they are ready for the
/// window to be replaced. The archive waits in the shell between the calls — the same shape as the
/// release waiting here for its install, and for the same reason.
///
/// The signature is checked while downloading, so an archive that does not match the key this build
/// carries never becomes something that could be installed later.
#[tauri::command]
pub async fn update_download(app: AppHandle, state: State<'_, ShellState>) -> Result<(), ShellError> {
    let Some(update) = state.pending_update().peek() else {
        return Err(ShellError::update_not_pending());
    };
    let version = update.version.clone();

    let progress_app = app.clone();
    let mut downloaded: u64 = 0;
    let bytes = update
        .download(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = progress_app.emit(PROGRESS_EVENT, UpdateProgress::Downloading { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|error| {
            log::warn!("the update could not be downloaded: {error}");
            ShellError::update_install_failed(error.to_string())
        })?;

    log::info!(
        "the update {} was downloaded ({} bytes) and waits to be installed",
        version,
        bytes.len()
    );
    state.pending_update().remember_download(version, bytes);
    Ok(())
}

/// Installs the release a check found, downloading it first when nobody has.
///
/// The archive already downloaded for this version is what gets installed — that is the whole point
/// of the two steps — and a build that was never asked to download one downloads it here, so a call
/// from anywhere still works. On Windows the installer takes over and this process is gone before the
/// call could return; on macOS and Linux the new version is in place and the application restarts
/// into it. Either way nothing here returns a success the interface could draw, which is why the
/// interface treats the call as the end of the conversation.
#[tauri::command]
pub async fn update_install(app: AppHandle, state: State<'_, ShellState>) -> Result<(), ShellError> {
    let Some(update) = state.pending_update().take() else {
        return Err(ShellError::update_not_pending());
    };

    let waiting = state.pending_update().take_download(&update.version);

    if let Some(bytes) = waiting {
        log::info!("the downloaded update {} is being installed", update.version);
        let _ = app.emit(PROGRESS_EVENT, UpdateProgress::Installing);
        return match update.install(bytes) {
            Ok(()) => {
                log::info!("the update is installed; restarting into it");
                app.restart()
            }
            Err(error) => {
                log::warn!("the update could not be installed: {error}");
                Err(ShellError::update_install_failed(error.to_string()))
            }
        };
    }

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

/// Opens the page this release is published on, in whatever the system uses for the web.
///
/// The address is built here rather than sent by the interface, and the version is checked before it
/// goes into the path: what arrives is a string from a release manifest, and a command that opened
/// whatever it was handed would be a way to make this application open anything at all.
#[tauri::command]
pub fn update_open_release_page(version: String) -> Result<(), ShellError> {
    let release_page = release_page_for(&version)
        .ok_or_else(|| ShellError::update_release_page_failed(format!("`{version}` is not a version")))?;

    crate::reveal::open_url(&release_page).map_err(|error| ShellError::update_release_page_failed(error.to_string()))
}

/// Where a version is published, or nothing when the string is not one.
///
/// Deliberately narrow: digits, dots and the two separators a pre-release uses, with something on
/// either side of every separator — enough for every version this channel can publish, and nothing a
/// path or a query could be hidden in.
pub fn release_page_for(version: &str) -> Option<String> {
    let trimmed = version.trim();
    let looks_like_a_version = !trimmed.is_empty()
        && trimmed.len() <= 64
        && trimmed.starts_with(|character: char| character.is_ascii_digit())
        && trimmed
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '+'))
        && !trimmed.contains("..");

    looks_like_a_version.then(|| format!("https://github.com/7wdn/7shell/releases/tag/v{trimmed}"))
}

#[cfg(test)]
mod tests {
    use super::release_page_for;

    #[test]
    fn a_version_becomes_the_page_it_is_published_on() {
        assert_eq!(
            release_page_for("0.2.0").as_deref(),
            Some("https://github.com/7wdn/7shell/releases/tag/v0.2.0")
        );
        // A pre-release is published on a page of its own, and the tag carries the suffix.
        assert_eq!(
            release_page_for("1.0.0-rc.1").as_deref(),
            Some("https://github.com/7wdn/7shell/releases/tag/v1.0.0-rc.1")
        );
    }

    #[test]
    fn a_string_that_is_not_a_version_is_not_a_page() {
        // The one thing that must not happen: a value out of a manifest becoming the address this
        // application opens. Everything here is either something else entirely or a way to reach
        // another host.
        for attempt in [
            "",
            "   ",
            "0.1.1/../../etc",
            "0.1.1 https://example.com",
            "javascript:alert(1)",
            "0.1.1?x=1",
            "0.1.1#fragment",
            "https://example.com",
            "../0.1.1",
            "0.1.1/0.2.0",
        ] {
            assert_eq!(release_page_for(attempt), None, "`{attempt}` was turned into a page");
        }
    }
}
