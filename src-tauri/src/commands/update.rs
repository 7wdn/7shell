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
    /// The archive a portable copy replaces itself with. Only a portable copy ever takes this:
    /// an installed build cannot unpack an archive over itself and is offered its installer.
    portable: Mutex<Option<crate::update_portable::PortableSource>>,
    /// The release downloaded and not yet installed, and the version it belongs to.
    ///
    /// Kept by version because the two steps are separate now: a check that finds something newer
    /// while an archive is waiting must not have that archive installed under the new version's name.
    downloaded: Mutex<Option<Downloaded>>,
}

/// Bytes downloaded for a version, and which kind of archive they are.
pub(crate) struct Downloaded {
    version: String,
    bytes: Vec<u8>,
    /// Whether these bytes are the portable archive rather than an installer. The installer is
    /// handed to the updater to run; the archive is unpacked over this copy.
    portable: bool,
}

impl PendingUpdate {
    /// Remembers the release a check found, replacing any earlier one.
    pub fn remember(&self, update: Update, portable: Option<crate::update_portable::PortableSource>) {
        *self.lock() = Some(Arc::new(update));
        self.set_portable(portable);
    }

    fn set_portable(&self, portable: Option<crate::update_portable::PortableSource>) {
        *self.lock_portable() = portable;
    }

    /// The portable archive the manifest named, when this copy is portable.
    pub fn peek_portable(&self) -> Option<crate::update_portable::PortableSource> {
        self.lock_portable().clone()
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
        self.set_portable(None);
        *self.downloaded.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    /// Remembers an archive downloaded for a version.
    pub fn remember_download(&self, version: String, bytes: Vec<u8>, portable: bool) {
        *self.downloaded.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Downloaded {
            version,
            bytes,
            portable,
        });
    }

    /// Takes the archive downloaded for *this* version, if there is one.
    pub fn take_download(&self, version: &str) -> Option<Downloaded> {
        let mut slot = self.downloaded.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match slot.as_ref() {
            Some(downloaded) if downloaded.version == version => slot.take(),
            _ => None,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Arc<Update>>> {
        match self.slot.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn lock_portable(&self) -> std::sync::MutexGuard<'_, Option<crate::update_portable::PortableSource>> {
        match self.portable.lock() {
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

            // A portable copy updates from its own archive; an installed one never looks at it.
            // The manifest entry is read here rather than at install time so the download and the
            // check name the same release — the slot below is what keeps them from drifting apart.
            let portable = if crate::update_portable::this_copy_is_portable() {
                let source = crate::update_portable::portable_source(&update);
                if source.is_none() {
                    log::warn!(
                        "the release {} names no portable archive for this platform",
                        release.version
                    );
                }
                source
            } else {
                None
            };

            state.pending_update().remember(update, portable);
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

    // A portable copy must never be handed the installer: it has no installation for one to
    // replace, and running it would put a second copy somewhere else instead of updating this one.
    let (bytes, portable) = if crate::update_portable::this_copy_is_portable() {
        let Some(source) = state.pending_update().peek_portable() else {
            return Err(ShellError::update_portable_unavailable());
        };
        let archive = crate::update_portable::archive_for(&update, &source);
        (download(&app, &archive).await?, true)
    } else {
        (download(&app, &update).await?, false)
    };

    log::info!(
        "the update {} was downloaded ({} bytes) and waits to be installed",
        version,
        bytes.len()
    );
    state.pending_update().remember_download(version, bytes, portable);
    Ok(())
}

/// Downloads whatever `update` points at, announcing progress while it runs.
///
/// The address and the signature are the only things a portable copy changes: the release is the
/// same, and so is the key its bytes are checked against.
async fn download(app: &AppHandle, update: &Update) -> Result<Vec<u8>, ShellError> {
    let progress_app = app.clone();
    let mut downloaded: u64 = 0;
    update
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
        })
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

    if let Some(downloaded) = waiting {
        let _ = app.emit(PROGRESS_EVENT, UpdateProgress::Installing);
        if downloaded.portable {
            log::info!("the downloaded portable archive {} is being unpacked", update.version);
            return install_portable(&app, &downloaded.bytes, &update.version);
        }
        log::info!("the downloaded update {} is being installed", update.version);
        return match update.install(downloaded.bytes) {
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

    // Nobody downloaded first. A portable copy fetches its own archive here and unpacks it; an
    // installed build hands the download straight to the installer as before.
    if crate::update_portable::this_copy_is_portable() {
        let Some(source) = state.pending_update().peek_portable() else {
            return Err(ShellError::update_portable_unavailable());
        };
        let archive = crate::update_portable::archive_for(&update, &source);
        let bytes = download(&app, &archive).await?;
        let _ = app.emit(PROGRESS_EVENT, UpdateProgress::Installing);
        return install_portable(&app, &bytes, &update.version);
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

/// Unpacks the portable archive over this copy and restarts into it.
///
/// The restart is the same call an installer update ends with: the executable at the path this
/// process started from is the new one by now, so restarting runs the new version.
fn install_portable(app: &AppHandle, archive: &[u8], version: &str) -> Result<(), ShellError> {
    crate::update_portable::install(archive)?;
    log::info!("the portable copy was replaced with {version}; restarting into it");
    app.restart()
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
