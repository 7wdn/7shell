//! Remote files: the browser's session, and the transfers it starts.
//!
//! Local paths never appear in this module's public surface as something the interface chose: a
//! download is given where to write and an upload where to read, and both are decided by whoever
//! asks — the desktop shell opens a native dialog and passes the answer down.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::sessions::{HostKeyDecision, RouteOutcome};
use crate::{Core, TransferRecord};
use sevenshell_error::ShellError;
use sevenshell_sftp::{SftpSession, TransferProgress};
use sevenshell_ssh::{OpenRequest, Purpose, Route, SshSession};
use sevenshell_types::{EntryKind, SftpListing, SftpOutcome, TransferDirection, TransferInfo, TransferState};

/// One open file browser: the connection it rides on, and its SFTP channel.
pub struct FileSession {
    /// Kept alive for as long as the browser is: the channel dies with the connection.
    _connection: Arc<SshSession>,
    sftp: Arc<SftpSession>,
}

impl Core {
    /// Opens a file browser for a session, or reports which host key decision comes first.
    ///
    /// The channel rides on a connection of its own unless a terminal for the same session is
    /// already open, in which case that one is reused: one connection can carry both, and opening
    /// a second is a second authentication the server did not need to see.
    pub async fn open_files(&self, session_id: &str) -> Result<SftpOutcome, ShellError> {
        let session = self.session(session_id)?;
        let credential = self.credential(&session)?;
        let target = sevenshell_ssh::Target {
            host: session.host.clone(),
            port: session.port,
            user: session.user.clone(),
        };

        // Reuse a connection that is already open for this session, if there is one.
        let existing = self
            .registry
            .open_sessions()
            .await
            .into_iter()
            .find(|open| open.session_id == session_id)
            .map(|open| open.handle);

        let connection = match existing {
            Some(handle) => self.registry.session_of(&handle).await?,
            None => {
                let hop = match self.open_route(&session).await? {
                    RouteOutcome::Ready(hop) => hop,
                    RouteOutcome::Decision(decision) => return Ok(into_files_outcome(*decision)),
                };

                let presented = match &hop {
                    Some(hop) => {
                        sevenshell_ssh::probe_host_key_via(hop, &target.host, target.port, self.timeout).await?
                    }
                    None => sevenshell_ssh::probe_host_key(&target.host, target.port, self.timeout).await?,
                };

                match self.vault.find_known_host(&target.host, target.port)? {
                    None => return Ok(SftpOutcome::HostKeyUnknown { key: presented }),
                    Some(trusted) if trusted.public_key != presented.public_key => {
                        return Ok(SftpOutcome::HostKeyMismatch {
                            key: presented,
                            trusted,
                        });
                    }
                    Some(_) => {}
                }

                let route = match hop {
                    Some(hop) => Route::Through {
                        hop,
                        host: target.host.clone(),
                        port: target.port,
                    },
                    None => Route::Direct,
                };

                let request = OpenRequest {
                    target,
                    credential,
                    terminal_type: String::new(),
                    keepalive_seconds: session.terminal.keepalive_seconds,
                    timeout: self.timeout,
                    expected_host_key: presented,
                    size: sevenshell_ssh::TerminalSize::default(),
                    route,
                    // A carrier: a file browser needs a connection, not a shell on the server.
                    purpose: Purpose::Carrier,
                };

                let (output, _unused) = tokio::sync::mpsc::channel(1);
                Arc::new(SshSession::open(request, output).await?)
            }
        };

        let stream = connection.open_subsystem_stream("sftp").await?;
        let sftp = Arc::new(SftpSession::new(stream).await?);

        // The first listing is part of the answer: a browser that opened and then had to ask where
        // it was would have two ways to fail instead of one.
        let listing = sftp.list(".").await?;

        let handle = format!("f{}", self.next_file.fetch_add(1, Ordering::Relaxed));
        self.files.lock().await.insert(
            handle.clone(),
            Arc::new(FileSession {
                _connection: connection,
                sftp,
            }),
        );

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
            TransferDirection::Download,
            name,
            source.clone(),
            size,
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
            TransferDirection::Upload,
            name,
            remote,
            size,
            move |progress| async move { files.upload(&local, &destination, &progress).await },
        )
        .await
    }

    /// The transfers that are running or have run, newest last.
    pub async fn transfers(&self) -> Vec<TransferInfo> {
        let transfers = self.transfers.lock().await;
        let mut infos: Vec<TransferInfo> = transfers
            .values()
            .map(|record| TransferInfo {
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

    /// Runs a transfer on its own task, and remembers it until it is dismissed.
    async fn spawn_transfer<F, Fut>(
        &self,
        direction: TransferDirection,
        name: String,
        remote_path: String,
        size: u64,
        run: F,
    ) -> Result<String, ShellError>
    where
        F: FnOnce(Arc<TransferProgress>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), ShellError>> + Send + 'static,
    {
        let id = format!("x{}", self.next_transfer.fetch_add(1, Ordering::Relaxed));
        let progress = TransferProgress::new(size);

        let info = TransferInfo {
            id: id.clone(),
            direction,
            name,
            remote_path,
            size,
            transferred: 0,
            state: TransferState::Running,
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
