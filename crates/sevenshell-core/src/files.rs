//! Remote files: the browser's session, and the transfers it starts.
//!
//! Local paths never appear in this module's public surface as something the interface chose: a
//! download is given where to write and an upload where to read, and both are decided by whoever
//! asks — the desktop shell opens a native dialog and passes the answer down.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::archive;
use crate::sessions::{ConnectionOutcome, HostKeyDecision};
use crate::{Core, TransferRecord};
use sevenshell_error::ShellError;
use sevenshell_sftp::{SftpSession, TransferProgress};
use sevenshell_ssh::SshSession;
use sevenshell_types::{EntryKind, SftpListing, SftpOutcome, TransferDirection, TransferInfo, TransferState};

/// One open file browser: the connection it rides on, and its SFTP channel.
pub struct FileSession {
    /// Kept alive for as long as the browser is: the channel dies with the connection.
    connection: Arc<SshSession>,
    sftp: Arc<SftpSession>,
}

impl FileSession {
    /// The connection underneath, which is what a packed transfer needs: `tar` is a command rather
    /// than a file operation, and SFTP has nowhere to run one.
    fn connection(&self) -> Arc<SshSession> {
        Arc::clone(&self.connection)
    }
}

impl Core {
    /// Opens a file browser for a session, or reports which host key decision comes first.
    ///
    /// The channel rides on a connection of its own unless a terminal for the same session is
    /// already open, in which case that one is reused: one connection can carry both, and opening
    /// a second is a second authentication the server did not need to see.
    pub async fn open_files(&self, session_id: &str) -> Result<SftpOutcome, ShellError> {
        let session = self.session(session_id)?;

        let connection = match self.open_carrier(&session).await? {
            ConnectionOutcome::Ready(connection) => connection,
            ConnectionOutcome::Decision(decision) => return Ok(into_files_outcome(*decision)),
        };

        let stream = connection.open_subsystem_stream("sftp").await?;
        let sftp = Arc::new(SftpSession::new(stream).await?);

        // The first listing is part of the answer: a browser that opened and then had to ask where
        // it was would have two ways to fail instead of one.
        let listing = sftp.list(".").await?;

        let handle = format!("f{}", self.next_file.fetch_add(1, Ordering::Relaxed));
        self.files
            .lock()
            .await
            .insert(handle.clone(), Arc::new(FileSession { connection, sftp }));

        Ok(SftpOutcome::Opened {
            handle,
            session_id: session_id.to_owned(),
            listing,
        })
    }

    /// Reads a directory.
    pub async fn list_files(&self, handle: &str, path: &str) -> Result<SftpListing, ShellError> {
        self.file_session(handle).await?.sftp.list(path).await
    }

    pub async fn create_folder(&self, handle: &str, path: &str) -> Result<(), ShellError> {
        self.file_session(handle).await?.sftp.create_dir(path).await
    }

    /// Creates an empty file, unless something with that name is already there.
    pub async fn create_file(&self, handle: &str, path: &str) -> Result<(), ShellError> {
        self.file_session(handle).await?.sftp.create_file(path).await
    }

    pub async fn rename_entry(&self, handle: &str, from: &str, to: &str) -> Result<(), ShellError> {
        self.file_session(handle).await?.sftp.rename(from, to).await
    }

    /// Deletes a file, or a directory and everything under it.
    ///
    /// The kind comes from what the browser listed rather than from a second look: the entry a
    /// person right-clicked is the entry they meant, even if something replaced it since.
    pub async fn delete_entry(&self, handle: &str, path: &str, kind: EntryKind) -> Result<(), ShellError> {
        let files = self.file_session(handle).await?;

        match kind {
            EntryKind::Directory => files.sftp.remove_dir_recursive(path).await,
            _ => files.sftp.remove_file(path).await,
        }
    }

    pub async fn read_text_file(&self, handle: &str, path: &str) -> Result<String, ShellError> {
        self.file_session(handle).await?.sftp.read_text(path).await
    }

    pub async fn write_text_file(&self, handle: &str, path: &str, text: &str) -> Result<(), ShellError> {
        self.file_session(handle).await?.sftp.write_text(path, text).await
    }

    /// Starts copying a remote file to a local path, and returns the transfer's id.
    pub async fn start_download(&self, handle: &str, remote: &str, local: PathBuf) -> Result<String, ShellError> {
        let files = Arc::clone(&self.file_session(handle).await?.sftp);
        let size = files.size_of(remote).await?;
        let name = file_name_of(remote);
        // Owned before the task takes it: the task outlives the borrow this call was given.
        let source = remote.to_owned();

        self.spawn_transfer(
            self.reserve_transfer_id(),
            TransferDirection::Download,
            name,
            source.clone(),
            size,
            false,
            move |progress| async move {
                // A download into a path that cannot be written is a failure the transfer reports.
                files.download(&source, &local, &progress).await
            },
        )
        .await
    }

    /// Starts copying a local file into a remote directory, and returns the transfer's id.
    pub async fn start_upload(&self, handle: &str, local: PathBuf, remote_dir: &str) -> Result<String, ShellError> {
        let files = Arc::clone(&self.file_session(handle).await?.sftp);
        let name = local
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| ShellError::sftp_transfer_failed(remote_dir, "that path has no file name"))?;
        let remote = join_remote(remote_dir, &name);
        let size = tokio::fs::metadata(&local)
            .await
            .map_err(|error| {
                ShellError::sftp_transfer_failed(&name, format!("the local file could not be read: {error}"))
            })?
            .len();

        let destination = remote.clone();
        self.spawn_transfer(
            self.reserve_transfer_id(),
            TransferDirection::Upload,
            name,
            remote,
            size,
            false,
            move |progress| async move { files.upload(&local, &destination, &progress).await },
        )
        .await
    }

    /**
     * Starts packing the remote entries somebody selected and putting them on this machine.
     *
     * What lands is those entries themselves — a folder, a file, or a mixture — inside the folder that
     * was chosen. The archive is a vehicle that exists on three sides in turn, the far side's
     * temporary directory, the wire, and this machine's, and it is deleted from all three: a download
     * that left a `.tar.gz` behind would be handing somebody the second half of the job.
     *
     * How large the archive is is nobody's business until the far side has finished making it, so the
     * transfer starts with a total of zero — which the list already draws as "still counting" — and
     * the real number arrives when `tar` is done.
     */
    pub async fn start_packed_download(
        &self,
        handle: &str,
        entries: &[String],
        into: PathBuf,
    ) -> Result<String, ShellError> {
        let session = self.file_session(handle).await?;
        let files = Arc::clone(&session.sftp);
        let connection = session.connection();
        let id = self.reserve_transfer_id();

        let (parent, names) = common_parent(entries)?;
        let staging = archive::staging_path(&files.resolve(".").await, &id);
        // Named for the person rather than for the far side: one folder keeps its own name, and a
        // selection gets one that says what it was.
        let held = match names.as_slice() {
            [only] => archive::archive_name(only),
            several => archive::downloaded_name(several.len()),
        };
        // What a failure is reported against: the thing itself when there is one, and the directory
        // they all came from when there are several.
        let label = match entries {
            [only] => only.clone(),
            _ => parent.clone(),
        };
        let archive_here = archive::local_staging_path(&id);
        // Where they came from, which is what the row is about. A packed download does not read the
        // listing again when it finishes — nothing on the far side changed.
        let origin = parent.clone();

        self.spawn_transfer(
            id,
            TransferDirection::Download,
            held,
            origin,
            0,
            true,
            move |progress| async move {
                let taken = take_archive(
                    &files,
                    &connection,
                    &staging,
                    &label,
                    &parent,
                    &names,
                    &archive_here,
                    &progress,
                )
                .await;
                let outcome = match taken {
                    Ok(()) => unpack_local(archive_here.clone(), into).await,
                    Err(error) => Err(error),
                };

                // Whether it worked or not, neither archive is left behind: one is as large as what
                // was selected and the other is ours, and nobody else knows what either of them is.
                let _ = files.remove_file(&staging).await;
                let _ = std::fs::remove_file(&archive_here);
                outcome
            },
        )
        .await
    }

    /**
     * Starts copying one remote file into a local directory, under the name it has on the far side.
     *
     * The plain download asks where to put *this* file. This is the other case: several files were
     * chosen and the folder was asked for once, so each of them keeps its own name inside it.
     */
    pub async fn start_download_into(&self, handle: &str, remote: &str, into: &Path) -> Result<String, ShellError> {
        self.start_download(handle, remote, into.join(file_name_of(remote)))
            .await
    }

    /**
     * Starts packing local files here and putting them on the far side.
     *
     * The packing happens on this machine, which is the half that makes this a packed transfer rather
     * than a request to go and make an archive first: a person chooses files, and one archive crosses
     * the wire. What arrives is unpacked there, so the far side ends up with the files themselves.
     *
     * Until the archive exists nobody knows how large it is — that is what packing *does* — so the
     * row starts with a total of zero and the progress bar says "still counting" while it is made.
     */
    pub async fn start_packed_upload(
        &self,
        handle: &str,
        sources: Vec<PathBuf>,
        remote_dir: &str,
    ) -> Result<String, ShellError> {
        let session = self.file_session(handle).await?;
        let files = Arc::clone(&session.sftp);
        let connection = session.connection();
        let id = self.reserve_transfer_id();

        let name = archive::uploaded_name(sources.len());
        let staging = archive::staging_path(&files.resolve(".").await, &id);
        let archive_here = archive::local_staging_path(&id);
        // Where the contents land, which is also what the panel reads again when this finishes: a
        // packed upload puts files *in* this directory rather than a file inside it.
        let directory = remote_dir.to_owned();

        self.spawn_transfer(
            id,
            TransferDirection::Upload,
            name,
            directory.clone(),
            0,
            true,
            move |progress| async move {
                let outcome = async {
                    progress.set_total(pack_local(archive_here.clone(), sources).await?);
                    files.upload(&archive_here, &staging, &progress).await?;
                    if progress.is_cancelled() {
                        return Err(cancelled(&directory));
                    }
                    archive::run(&archive::unpack_command(&staging, &directory), &connection, &directory).await
                }
                .await;

                let _ = files.remove_file(&staging).await;
                let _ = std::fs::remove_file(&archive_here);
                outcome
            },
        )
        .await
    }

    /// The transfers that are running or have run, newest last.
    ///
    /// The size and the count come from the progress object rather than from what the record was
    /// created with: a packed transfer learns its size part-way through, and a row that went on
    /// saying "unknown" after the archive existed would be a row nobody could believe.
    pub async fn transfers(&self) -> Vec<TransferInfo> {
        let transfers = self.transfers.lock().await;
        let mut infos: Vec<TransferInfo> = transfers
            .values()
            .map(|record| TransferInfo {
                size: record.progress.total(),
                transferred: record.progress.transferred(),
                ..record.info.clone()
            })
            .collect();
        infos.sort_by(|left, right| left.id.cmp(&right.id));
        infos
    }

    /// Asks a transfer to stop, and waits for it to notice.
    pub async fn cancel_transfer(&self, id: &str) -> Result<(), ShellError> {
        let record = self.transfers.lock().await.remove(id);
        let Some(record) = record else {
            // A transfer that is already gone is what a second click means.
            return Ok(());
        };

        record.progress.cancel();
        let _ = record.task.await;
        Ok(())
    }

    /// Closes a file browser, and cancels whatever it was still moving.
    pub async fn close_files(&self, handle: &str) -> Result<(), ShellError> {
        let ids: Vec<String> = {
            let transfers = self.transfers.lock().await;
            transfers
                .values()
                .filter(|record| record.info.id.starts_with(handle))
                .map(|record| record.info.id.clone())
                .collect()
        };

        for id in ids {
            let _ = self.cancel_transfer(&id).await;
        }

        self.files.lock().await.remove(handle);
        Ok(())
    }

    async fn file_session(&self, handle: &str) -> Result<Arc<FileSession>, ShellError> {
        self.files
            .lock()
            .await
            .get(handle)
            .cloned()
            .ok_or_else(|| ShellError::app_internal(format!("the file browser {handle} is no longer open")))
    }

    /// The next transfer's id, taken now rather than when the task starts.
    ///
    /// A packed transfer needs it before its task exists: the archive it makes on the far side is
    /// named after it, and two transfers that picked the same name would overwrite each other's work.
    fn reserve_transfer_id(&self) -> String {
        format!("x{}", self.next_transfer.fetch_add(1, Ordering::Relaxed))
    }

    /// Runs a transfer on its own task, and remembers it until it is dismissed.
    #[allow(clippy::too_many_arguments)]
    async fn spawn_transfer<F, Fut>(
        &self,
        id: String,
        direction: TransferDirection,
        name: String,
        remote_path: String,
        size: u64,
        packed: bool,
        run: F,
    ) -> Result<String, ShellError>
    where
        F: FnOnce(Arc<TransferProgress>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), ShellError>> + Send + 'static,
    {
        let progress = TransferProgress::new(size);

        let info = TransferInfo {
            id: id.clone(),
            direction,
            name,
            remote_path,
            size,
            transferred: 0,
            state: TransferState::Running,
            packed,
        };

        let shared = Arc::clone(&self.transfers);
        let finished = id.clone();
        let running = Arc::clone(&progress);

        let task = tokio::spawn(async move {
            let outcome = run(Arc::clone(&running)).await;

            let state = match outcome {
                Ok(()) => TransferState::Done,
                Err(_) if running.is_cancelled() => TransferState::Cancelled,
                Err(error) => TransferState::Failed {
                    detail: error.detail().map(str::to_owned).unwrap_or_else(|| error.to_string()),
                },
            };

            if let Some(record) = shared.lock().await.get_mut(&finished) {
                record.info.state = state;
            }
        });

        self.transfers
            .lock()
            .await
            .insert(id.clone(), TransferRecord { info, progress, task });
        Ok(id)
    }
}

/// The last segment of a remote path, which is what a person recognises.
fn file_name_of(path: &str) -> String {
    path.rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or(path)
        .to_owned()
}

/// A remote path split into the directory that holds it and its own name.
///
/// `tar` is run from the parent, because the entries it stores are the names themselves: unpacking
/// what it produces gives back `logs/` rather than the contents of `logs/` strewn into wherever
/// somebody happened to be standing.
fn split_remote(path: &str) -> (String, String) {
    let trimmed = path.trim_end_matches('/');

    match trimmed.rsplit_once('/') {
        Some(("", name)) => ("/".to_owned(), name.to_owned()),
        Some((parent, name)) => (parent.to_owned(), name.to_owned()),
        // A relative path: the parent is wherever the session already is.
        None => (".".to_owned(), trimmed.to_owned()),
    }
}

/// The directory a selection lives in, and the names of the entries inside it.
///
/// A selection comes from one listing, so this cannot fail in the interface — but `-C <one directory>
/// <names…>` is the shape the far side's `tar` needs, and a caller that handed over paths from two
/// directories has to be told rather than quietly packed from one of them.
fn common_parent(entries: &[String]) -> Result<(String, Vec<String>), ShellError> {
    let mut parent: Option<String> = None;
    let mut names: Vec<String> = Vec::with_capacity(entries.len());

    for entry in entries {
        let (holder, name) = split_remote(entry);
        match &parent {
            Some(known) if *known != holder => {
                return Err(ShellError::sftp_archive_failed(
                    entry,
                    "these entries are not all in one directory, so they cannot be packed together",
                ));
            }
            Some(_) => {}
            None => parent = Some(holder),
        }
        names.push(name);
    }

    parent
        .map(|parent| (parent, names))
        .ok_or_else(|| ShellError::sftp_archive_failed("(nothing)", "there is nothing to pack"))
}

/// Packs the selected entries on the far side and brings the archive down.
///
/// The cancellation check between the two steps is not decoration: a `tar` that has already finished
/// cannot be stopped, but a download that has not begun can — and somebody who asked to stop should
/// not have a file appear on their disk afterwards.
#[allow(clippy::too_many_arguments)]
async fn take_archive(
    files: &Arc<SftpSession>,
    connection: &SshSession,
    staging: &str,
    label: &str,
    parent: &str,
    names: &[String],
    local: &Path,
    progress: &Arc<TransferProgress>,
) -> Result<(), ShellError> {
    archive::run(&archive::pack_command(staging, parent, names), connection, label).await?;

    if progress.is_cancelled() {
        return Err(cancelled(label));
    }
    // Only now is there a number to count towards.
    progress.set_total(files.size_of(staging).await?);

    files.download(staging, local, progress).await
}

/// Packs on this machine without blocking the runtime.
///
/// An archive of a few gigabytes is a long synchronous write, and the task it runs in is one the
/// interface is waiting on for its next frame — so it is handed to a thread that is allowed to be
/// busy.
async fn pack_local(archive: PathBuf, sources: Vec<PathBuf>) -> Result<u64, ShellError> {
    tokio::task::spawn_blocking(move || archive::pack_local(&archive, &sources))
        .await
        .unwrap_or_else(|error| {
            Err(ShellError::sftp_archive_failed(
                "(packing)",
                format!("the packing task did not finish: {error}"),
            ))
        })
}

/// Unpacks on this machine, for the same reason and in the same way.
async fn unpack_local(archive: PathBuf, into: PathBuf) -> Result<(), ShellError> {
    tokio::task::spawn_blocking(move || archive::unpack_local(&archive, &into))
        .await
        .unwrap_or_else(|error| {
            Err(ShellError::sftp_archive_failed(
                "(unpacking)",
                format!("the unpacking task did not finish: {error}"),
            ))
        })
}

/// The same sentence the SFTP layer uses when a transfer is stopped part-way, so the state a person
/// sees does not depend on which layer noticed.
fn cancelled(path: &str) -> ShellError {
    ShellError::sftp_transfer_failed(path, "the transfer was cancelled")
}

fn join_remote(directory: &str, name: &str) -> String {
    if directory.ends_with('/') {
        format!("{directory}{name}")
    } else {
        format!("{directory}/{name}")
    }
}

/// The file browser's own answer to a decision, which is the terminal's answer with its own kind.
fn into_files_outcome(decision: HostKeyDecision) -> SftpOutcome {
    match decision {
        HostKeyDecision::Unknown(key) => SftpOutcome::HostKeyUnknown { key },
        HostKeyDecision::Mismatch { key, trusted } => SftpOutcome::HostKeyMismatch { key, trusted },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_splits_into_the_directory_that_holds_it_and_its_name() {
        assert_eq!(
            split_remote("/var/log/nginx"),
            ("/var/log".to_owned(), "nginx".to_owned())
        );
        assert_eq!(split_remote("/logs"), ("/".to_owned(), "logs".to_owned()));
    }

    #[test]
    fn a_trailing_slash_does_not_become_part_of_the_name() {
        assert_eq!(
            split_remote("/var/log/nginx/"),
            ("/var/log".to_owned(), "nginx".to_owned())
        );
    }

    #[test]
    fn a_relative_path_is_taken_as_seen_from_where_the_session_is() {
        assert_eq!(split_remote("logs"), (".".to_owned(), "logs".to_owned()));
    }

    #[test]
    fn a_selection_shares_the_directory_it_was_listed_in() {
        let entries = vec![
            "/var/log/nginx".to_owned(),
            "/var/log/syslog".to_owned(),
            "/var/log/journal".to_owned(),
        ];

        let (parent, names) = common_parent(&entries).expect("one directory");

        assert_eq!(parent, "/var/log");
        assert_eq!(
            names,
            vec!["nginx".to_owned(), "syslog".to_owned(), "journal".to_owned()]
        );
    }

    #[test]
    fn entries_from_two_directories_are_refused_rather_than_packed_from_one() {
        let entries = vec!["/var/log/nginx".to_owned(), "/etc/hosts".to_owned()];

        let error = common_parent(&entries).expect_err("two directories");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SftpArchiveFailed);
        assert_eq!(
            error.message_params().get("path").map(String::as_str),
            Some("/etc/hosts"),
            "the refusal names the entry that is not where the others are"
        );
    }

    #[test]
    fn there_is_nothing_to_pack_in_an_empty_selection() {
        assert!(common_parent(&[]).is_err());
    }
}
