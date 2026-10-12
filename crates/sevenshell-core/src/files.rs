//! Remote files: the browser's session, and the transfers it starts.
//!
//! Local paths never appear in this module's public surface as something the interface chose: a
//! download is given where to write and an upload where to read, and both are decided by whoever
//! asks — the desktop shell opens a native dialog and passes the answer down.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::archive;
use crate::sessions::{ConnectionOutcome, HostKeyDecision, TransferAttempt, TransferRun};
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
    /// The session it was opened for, which is what every transfer started here belongs to.
    ///
    /// Held rather than looked up per transfer because the browser can be gone by the time somebody
    /// asks a transfer whose it is: the row in the list outlives the tab that started it.
    session_id: String,
}

impl FileSession {
    /// The connection underneath, which is what a packed transfer needs: `tar` is a command rather
    /// than a file operation, and SFTP has nowhere to run one.
    fn connection(&self) -> Arc<SshSession> {
        Arc::clone(&self.connection)
    }

    fn session_id(&self) -> String {
        self.session_id.clone()
    }
}

/// The local end of one transfer, for the two things only this side can do with it.
///
/// A named struct rather than a tuple because the second half is not obvious: whether the transfer
/// finished is what decides whether the file may be *opened*, while showing where it is stays
/// useful in every state — that is how somebody finds the part file of a transfer that stopped.
pub struct LocalEnd {
    pub path: PathBuf,
    pub done: bool,
}

/// What a transfer is, as the row that describes it — plus the two things the interface is not told.
///
/// Built by the caller rather than passed as arguments, because most of it is strings: the name, the
/// remote path and the session id would be three adjacent `String`s in a positional call, and two of
/// them swapping would still compile.
struct TransferFacts {
    direction: TransferDirection,
    name: String,
    remote_path: String,
    session_id: String,
    size: u64,
    packed: bool,
    /// Where the transfer's local end is, when it has one. Never serialized: see [`LocalEnd`].
    local: Option<PathBuf>,
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
        self.files.lock().await.insert(
            handle.clone(),
            Arc::new(FileSession {
                connection,
                sftp,
                session_id: session_id.to_owned(),
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
        let browser = self.file_session(handle).await?;
        let files = Arc::clone(&browser.sftp);
        let size = files.size_of(remote).await?;
        let name = file_name_of(remote);
        // Owned before the task takes it: the task outlives the borrow this call was given.
        let source = remote.to_owned();
        let witness = Arc::new(SourceWitness::default());

        self.spawn_transfer(
            self.reserve_transfer_id(),
            TransferFacts {
                direction: TransferDirection::Download,
                name,
                remote_path: source.clone(),
                session_id: browser.session_id(),
                size,
                packed: false,
                // Where it is landing, which is what the row can be pointed at afterwards.
                local: Some(local.clone()),
            },
            move |progress| {
                let files = Arc::clone(&files);
                let source = source.clone();
                let local = local.clone();
                let witness = Arc::clone(&witness);

                Box::pin(async move {
                    // What is here is a part file from an attempt that did not finish. It is worth
                    // continuing only if the file being copied is the one it came from.
                    if !witness.resume_allowed(files.stamp_of(&source).await.ok()) {
                        let _ = tokio::fs::remove_file(sevenshell_sftp::local_part_of(&local)).await;
                    }

                    // A download into a path that cannot be written is a failure the transfer reports.
                    files.download(&source, &local, &progress).await
                }) as TransferAttempt
            },
        )
        .await
    }

    /**
     * Which of these files would land on a name that is already there.
     *
     * The names only, so this is safe to hand to the interface, and one look each: it is the whole
     * input of the question an upload asks before it sends anything. A name that is taken is not a
     * failure — it is what would be *replaced* by what is sent, and that is somebody's decision.
     */
    pub async fn upload_names_taken(
        &self,
        handle: &str,
        directory: &str,
        sources: &[PathBuf],
    ) -> Result<Vec<String>, ShellError> {
        let files = Arc::clone(&self.file_session(handle).await?.sftp);
        let mut taken = Vec::new();

        for source in sources {
            let Some(name) = source.file_name().map(|name| name.to_string_lossy().into_owned()) else {
                continue;
            };
            if files.exists(&join_remote(directory, &name)).await? {
                taken.push(name);
            }
        }

        Ok(taken)
    }

    /// Starts copying a local file into a remote directory, and returns the transfer's id.
    pub async fn start_upload(&self, handle: &str, local: PathBuf, remote_dir: &str) -> Result<String, ShellError> {
        let browser = self.file_session(handle).await?;
        let files = Arc::clone(&browser.sftp);
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
        let witness = Arc::new(SourceWitness::default());

        self.spawn_transfer(
            self.reserve_transfer_id(),
            TransferFacts {
                direction: TransferDirection::Upload,
                name,
                remote_path: remote,
                session_id: browser.session_id(),
                size,
                packed: false,
                // Where it was read from, which is the end a person can still be shown.
                local: Some(local.clone()),
            },
            move |progress| {
                let files = Arc::clone(&files);
                let local = local.clone();
                let destination = destination.clone();
                let witness = Arc::clone(&witness);

                Box::pin(async move {
                    // The part file here is on the far side, and it is worth continuing into only if
                    // the file being sent is the one it came from.
                    if !witness.resume_allowed(local_stamp(&local).await) {
                        let _ = files.remove_file(&sevenshell_sftp::part_of(&destination)).await;
                    }

                    files.upload(&local, &destination, &progress).await
                }) as TransferAttempt
            },
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
            TransferFacts {
                direction: TransferDirection::Download,
                name: held,
                remote_path: origin,
                session_id: session.session_id(),
                size: 0,
                packed: true,
                // What lands is the entries themselves inside the folder that was chosen, so that
                // folder — not the archive, which is already gone by then — is the local end.
                local: Some(into.clone()),
            },
            move |progress| {
                let files = Arc::clone(&files);
                let connection = connection.clone();
                let staging = staging.clone();
                let label = label.clone();
                let parent = parent.clone();
                let names = names.clone();
                let archive_here = archive_here.clone();
                let into = into.clone();

                Box::pin(async move {
                    // An archive made on the far side is a new file every attempt, so a part file from
                    // the last one is not a prefix of this one: a packed transfer always starts clean.
                    let _ = std::fs::remove_file(sevenshell_sftp::local_part_of(&archive_here));

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

                    // Whether it worked or not, neither archive is left behind: one is as large as
                    // what was selected and the other is ours, and nobody else knows what either of
                    // them is.
                    let _ = files.remove_file(&staging).await;
                    let _ = std::fs::remove_file(&archive_here);
                    outcome
                }) as TransferAttempt
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
            TransferFacts {
                direction: TransferDirection::Upload,
                name,
                remote_path: directory.clone(),
                session_id: session.session_id(),
                size: 0,
                packed: true,
                // No local end to point at: what was sent is a pile of files, and the archive that
                // carried them across is deleted here the moment it has been read.
                local: None,
            },
            move |progress| {
                let files = Arc::clone(&files);
                let connection = connection.clone();
                let staging = staging.clone();
                let archive_here = archive_here.clone();
                let sources = sources.clone();
                let directory = directory.clone();

                Box::pin(async move {
                    // The archive is made again for every attempt, so nothing left from the last one
                    // is a prefix of this one.
                    let _ = files.remove_file(&sevenshell_sftp::part_of(&staging)).await;

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
                }) as TransferAttempt
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
    ///
    /// The row stays and says it was cancelled. That is the difference between this and
    /// [`Core::remove_transfer`]: stopping is a fact about the bytes, and a row that vanished the
    /// moment somebody asked it to stop would leave them unable to tell a cancellation from a
    /// failure they slept through.
    ///
    /// Waiting is what makes the flag and the row agree: the state is written by the task itself, so
    /// by the time this returns there is nothing left running that could write another one.
    pub async fn stop_transfer(&self, id: &str) -> Result<(), ShellError> {
        let (progress, task) = {
            let mut transfers = self.transfers.lock().await;
            let Some(record) = transfers.get_mut(id) else {
                // A transfer that is already gone is what a second click means.
                return Ok(());
            };
            // What has already stopped is not stopped again: a done transfer that was asked to
            // cancel would otherwise have its own record say it never finished.
            if !matches!(record.info.state, TransferState::Running | TransferState::Paused) {
                return Ok(());
            }

            (Arc::clone(&record.progress), record.task.take())
        };

        progress.cancel();
        if let Some(task) = task {
            let _ = task.await;
        }
        Ok(())
    }

    /// Takes a transfer off the list, stopping it first if it is still moving.
    ///
    /// This is the delete: what it means is "I am done looking at this row", and for a row that is
    /// still moving the only honest way to mean it is to stop the bytes as well. It waits for the
    /// task, so a record that is gone is a transfer that is really over — leaving a task behind to
    /// write into a map it is no longer in is how a row that was deleted comes back.
    pub async fn remove_transfer(&self, id: &str) -> Result<(), ShellError> {
        let (progress, task) = {
            let mut transfers = self.transfers.lock().await;
            let Some(record) = transfers.get_mut(id) else {
                return Ok(());
            };
            (Arc::clone(&record.progress), record.task.take())
        };

        progress.cancel();
        if let Some(task) = task {
            let _ = task.await;
        }

        let mut transfers = self.transfers.lock().await;
        // A retry that arrived while the task was being waited for may have started it again, and
        // what was asked to be removed is then something that is moving. It is left alone: the row is
        // on screen, and stopping it is one click — whereas a record dropped out from under a running
        // task is exactly the invisible transfer this list exists to prevent.
        if let Some(record) = transfers.get(id)
            && matches!(record.info.state, TransferState::Running | TransferState::Paused)
        {
            return Ok(());
        }
        transfers.remove(id);
        Ok(())
    }

    /// Asks everything that is still moving to stop, and leaves every row to say so.
    ///
    /// The rows are kept for the same reason one stop keeps its row, and it is the reason this is
    /// not a loop over [`Core::stop_transfer`]: the list would have to be read between the calls to
    /// find out what to stop, and a transfer started while that loop ran would be stopped by nobody.
    pub async fn stop_active_transfers(&self) -> Result<(), ShellError> {
        let (stops, tasks) = {
            let mut transfers = self.transfers.lock().await;
            let mut stops = Vec::new();
            let mut tasks = Vec::new();
            for record in transfers.values_mut() {
                if !matches!(record.info.state, TransferState::Running | TransferState::Paused) {
                    continue;
                }
                stops.push(Arc::clone(&record.progress));
                if let Some(task) = record.task.take() {
                    tasks.push(task);
                }
            }
            (stops, tasks)
        };

        for progress in stops {
            progress.cancel();
        }
        for task in tasks {
            let _ = task.await;
        }
        Ok(())
    }

    /// Takes every transfer that has stopped off the list, and nothing else.
    ///
    /// "Clear" is a tidying question, so it must not be a destructive answer: a transfer that is
    /// running, or waiting to go on, is one that is still going to happen, and it stays. Stopping
    /// what is moving is [`Core::stop_active_transfers`] — one call that says exactly that.
    pub async fn clear_settled_transfers(&self) -> Result<(), ShellError> {
        self.transfers
            .lock()
            .await
            .retain(|_, record| matches!(record.info.state, TransferState::Running | TransferState::Paused));
        Ok(())
    }

    /// Where a transfer's local end is, and whether it is finished there.
    ///
    /// `None` for an id this core does not know, and for a transfer with no local end of its own —
    /// a packed upload, whose archive is deleted the moment it has been read. The state comes back
    /// with the path because the two questions are asked together: a file that is finished may be
    /// opened, and one that is half-written may only be pointed at.
    pub async fn transfer_local_end(&self, id: &str) -> Option<LocalEnd> {
        let transfers = self.transfers.lock().await;
        let record = transfers.get(id)?;

        Some(LocalEnd {
            path: record.local.clone()?,
            done: record.info.state == TransferState::Done,
        })
    }

    /// Asks a transfer to wait, keeping its connection and the part of the file it has.
    ///
    /// Only a running transfer can be paused, and a transfer that has stopped is left alone: the two
    /// are different answers about the same row, and one of them is what the retry button is for.
    pub async fn pause_transfer(&self, id: &str) -> Result<(), ShellError> {
        let mut transfers = self.transfers.lock().await;
        let Some(record) = transfers.get_mut(id) else {
            return Ok(());
        };
        if record.info.state != TransferState::Running {
            return Ok(());
        }

        record.progress.pause();
        record.info.state = TransferState::Paused;
        Ok(())
    }

    /// Lets a paused transfer go on from where it is.
    pub async fn resume_transfer(&self, id: &str) -> Result<(), ShellError> {
        let mut transfers = self.transfers.lock().await;
        let Some(record) = transfers.get_mut(id) else {
            return Ok(());
        };
        if record.info.state != TransferState::Paused {
            return Ok(());
        }

        record.progress.resume();
        record.info.state = TransferState::Running;
        Ok(())
    }

    /// Runs a transfer that has stopped, from wherever it stopped.
    ///
    /// The same id, because it is the same transfer: the row somebody clicked is the row that starts
    /// moving again, and a retry that appeared as a second row would leave them wondering which one
    /// was real. How much has already been copied is not decided here — the transfer finds that out
    /// from the part file when it opens it, which is also what makes a retry resume rather than
    /// start again.
    pub async fn retry_transfer(&self, id: &str) -> Result<(), ShellError> {
        let (run, progress) = {
            let mut transfers = self.transfers.lock().await;
            let Some(record) = transfers.get_mut(id) else {
                return Ok(());
            };
            // A transfer that is running, or waiting, is already doing what this would ask for.
            if matches!(record.info.state, TransferState::Running | TransferState::Paused) {
                return Ok(());
            }

            record.progress.restart();
            record.info.state = TransferState::Running;
            (Arc::clone(&record.run), Arc::clone(&record.progress))
        };

        let task = self.run_transfer(id.to_owned(), Arc::clone(&run), Arc::clone(&progress));

        if let Some(record) = self.transfers.lock().await.get_mut(id) {
            record.task = Some(task);
        }
        Ok(())
    }

    /// Closes a file browser. Whatever it was moving keeps moving.
    ///
    /// A transfer is not the tab's: it was started from a browser, and it holds the channel it is
    /// using for as long as it runs, so closing the tab takes away the browsing and nothing else.
    /// The row stays in the list, which is the only place left that can stop it — and a 2 GB upload
    /// that died because somebody tidied away a panel would be a worse answer than a row that
    /// outlives the tab that started it.
    pub async fn close_files(&self, handle: &str) -> Result<(), ShellError> {
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

    /// Runs a transfer on its own task, and remembers it until it is taken off the list.
    async fn spawn_transfer<F, Fut>(&self, id: String, facts: TransferFacts, run: F) -> Result<String, ShellError>
    where
        F: Fn(Arc<TransferProgress>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), ShellError>> + Send + 'static,
    {
        let progress = TransferProgress::new(facts.size);

        let info = TransferInfo {
            id: id.clone(),
            direction: facts.direction,
            name: facts.name,
            remote_path: facts.remote_path,
            session_id: facts.session_id,
            size: facts.size,
            transferred: 0,
            state: TransferState::Running,
            has_local_path: facts.local.is_some(),
            packed: facts.packed,
        };

        // Boxed once, kept twice: the task that starts now and the retry button that may need it
        // later are the same work, and a transfer that could only be run once would have nothing to
        // retry.
        let run: TransferRun = Arc::new(move |progress| Box::pin(run(progress)));
        let task = self.run_transfer(id.clone(), Arc::clone(&run), Arc::clone(&progress));

        self.transfers.lock().await.insert(
            id.clone(),
            TransferRecord {
                info,
                progress,
                task: Some(task),
                local: facts.local,
                run,
            },
        );
        Ok(id)
    }

    /// Starts one attempt at a transfer and records what became of it.
    ///
    /// One implementation for the first attempt and every retry, because "what a finished transfer
    /// says about itself" must not be decided twice — the second copy is the one that forgets the
    /// cancelled case and leaves a row saying it worked.
    fn run_transfer(
        &self,
        id: String,
        run: TransferRun,
        progress: Arc<TransferProgress>,
    ) -> tokio::task::JoinHandle<()> {
        let shared = Arc::clone(&self.transfers);
        let running = Arc::clone(&progress);

        tokio::spawn(async move {
            let outcome = run(Arc::clone(&running)).await;

            let state = match outcome {
                Ok(()) => TransferState::Done,
                Err(_) if running.is_cancelled() => TransferState::Cancelled,
                Err(error) => TransferState::Failed {
                    detail: error.detail().map(str::to_owned).unwrap_or_else(|| error.to_string()),
                },
            };

            if let Some(record) = shared.lock().await.get_mut(&id) {
                record.info.state = state;
            }
        })
    }
}

/// What the file being copied looked like when this transfer last started.
///
/// A retry continues where the last attempt stopped, which is only correct for the *same* file:
/// appending the rest of a file that has changed under a part file would join two versions and
/// produce something with the right name and the wrong contents — the one failure a transfer must
/// not have, because nothing about it looks wrong afterwards.
///
/// So the witness is asked once per attempt: it answers whether the part file left over is still
/// this transfer's work. The first attempt never has an answer, which is what makes a stale part
/// file from a previous run of the application start again rather than be trusted.
#[derive(Default)]
struct SourceWitness {
    seen: std::sync::Mutex<Option<(u64, Option<u64>)>>,
}

impl SourceWitness {
    /// Whether an unfinished part file may be continued into, given how the source looks now.
    ///
    /// `None` is a source that cannot be looked at, which is not a reason to trust what is on disk:
    /// the transfer itself will report the failure properly a moment later.
    fn resume_allowed(&self, now: Option<(u64, Option<u64>)>) -> bool {
        let Ok(mut seen) = self.seen.lock() else {
            return false;
        };

        let same = now.is_some() && *seen == now;
        *seen = now;
        same
    }
}

/// How a local file looks now: its size, and when it last changed if the platform will say.
async fn local_stamp(path: &std::path::Path) -> Option<(u64, Option<u64>)> {
    let metadata = tokio::fs::metadata(path).await.ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_secs());

    Some((metadata.len(), modified))
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
