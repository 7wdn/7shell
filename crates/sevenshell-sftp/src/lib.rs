//! The remote file system, as seen through an SFTP channel.
//!
//! This crate knows about paths, bytes and the server's own refusals. It does not know about local
//! paths: a transfer is given where to read from or write to by whoever asked for it, and choosing
//! that is the caller's business — which is what keeps a download from being told where to land by
//! a script in a webview.
//!
//! Two rules shape a transfer, and both are about what is left behind when one does not finish:
//!
//! * **The name that was asked for is written last.** A transfer writes to a part file beside its
//!   target and renames it into place once every byte has arrived. A half-written file wearing the
//!   right name is the one kind of damage a file browser can do that is invisible afterwards — the
//!   name is what people check, not the size.
//! * **A part file is where a transfer stopped.** Its length is how much arrived, so a second
//!   attempt continues from there rather than starting again. Cancelling removes it, because "stop"
//!   is an answer about the file; failing keeps it, because a retry is the next thing somebody
//!   tries.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use russh_sftp::client::SftpSession as Channel;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileAttributes, OpenFlags, StatusCode};
use sevenshell_error::ShellError;
use sevenshell_types::{EntryKind, SftpEntry, SftpListing};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::OnceCell;

/// The largest file the editor will open.
///
/// Not a protocol limit: above this a file is not something somebody edits in a panel, and reading
/// it into memory to find that out would be the expensive way to learn it.
pub const MAX_EDIT_BYTES: u64 = 4 * 1024 * 1024;

/// How much is moved per read or write. Large enough to keep the channel busy, small enough that
/// progress moves visibly and a cancellation is noticed quickly.
const CHUNK_BYTES: usize = 64 * 1024;

/// What a transfer appends to the name it was asked for while it is still working.
///
/// Long enough that a file a person made would never be called this by accident, and visible when
/// one is left behind — which is what happens when a transfer fails and nobody retries it.
pub const PART_SUFFIX: &str = ".7shell-part";

/// The remote name a transfer writes to before it is finished.
pub fn part_of(path: &str) -> String {
    format!("{path}{PART_SUFFIX}")
}

/// The local name the same way, keeping the path's own encoding rather than going through a string.
///
/// Public because whoever starts a download is also whoever decides whether a part file left behind
/// by an earlier attempt still belongs to the file being copied now.
pub fn local_part_of(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(PART_SUFFIX);
    PathBuf::from(name)
}

/// How often a paused transfer looks to see whether it may go on.
const PAUSE_POLL: std::time::Duration = std::time::Duration::from_millis(100);

/// Where a POSIX server keeps the names for the numbers it reports as ownership.
const PASSWD_PATH: &str = "/etc/passwd";
const GROUP_PATH: &str = "/etc/group";

/// How far a transfer has come, and whether it has been asked to stop or to wait.
///
/// Shared with whoever started the transfer, which is why it is an `Arc` of atomics rather than a
/// callback: the caller polls it or reads it back, and nothing has to be sent over a channel just
/// to move a number.
pub struct TransferProgress {
    /// How much is coming, which is not always known when the transfer is asked for: a directory
    /// that has to be packed on the far side first is a size nobody can name until the packing is
    /// done, and the bar reads "still counting" until then.
    total: AtomicU64,
    transferred: AtomicU64,
    cancelled: AtomicBool,
    /// Set while somebody has asked the transfer to wait. The connection stays open and the part
    /// file stays where it is: pausing is not cancelling with a different word.
    paused: AtomicBool,
}

impl TransferProgress {
    pub fn new(total: u64) -> Arc<Self> {
        Arc::new(Self {
            total: AtomicU64::new(total),
            transferred: AtomicU64::new(0),
            cancelled: AtomicBool::new(false),
            paused: AtomicBool::new(false),
        })
    }

    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// Says how much is coming, once that is known.
    pub fn set_total(&self, total: u64) {
        self.total.store(total, Ordering::Relaxed);
    }

    pub fn transferred(&self) -> u64 {
        self.transferred.load(Ordering::Relaxed)
    }

    /// Says how much has arrived, for a transfer that started with part of its work already done.
    pub fn set_transferred(&self, bytes: u64) {
        self.transferred.store(bytes, Ordering::Relaxed);
    }

    /// Puts the counter and the flags back for another attempt at the same transfer.
    ///
    /// The counter goes to zero rather than to wherever it stopped: how much has arrived is a fact
    /// about the part file, and the transfer learns it when it opens one. Carrying the old number
    /// over would double-count the bytes that are about to be counted again.
    pub fn restart(&self) {
        self.transferred.store(0, Ordering::Relaxed);
        self.cancelled.store(false, Ordering::Relaxed);
        self.paused.store(false, Ordering::Relaxed);
    }

    /// Asks the transfer to stop. It stops at the next chunk boundary, so the caller sees a
    /// cancelled transfer rather than a half-written file with no explanation.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        // A transfer waiting on a pause has to look up from that wait to notice this.
        self.paused.store(false, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Asks the transfer to wait where it is, keeping its connection and its place.
    pub fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Waits until this transfer may move another chunk, answering `false` when it was cancelled.
    ///
    /// Pausing waits here rather than stopping: the person asked for a hold, not for the transfer to
    /// end, and everything about it — the connection, the part file, the row — is still there when
    /// they let it go on.
    async fn wait_until_allowed(&self) -> bool {
        loop {
            if self.is_cancelled() {
                return false;
            }
            if !self.is_paused() {
                return true;
            }
            tokio::time::sleep(PAUSE_POLL).await;
        }
    }

    fn advance(&self, bytes: u64) {
        self.transferred.fetch_add(bytes, Ordering::Relaxed);
    }
}

/// One SFTP channel, and the operations worth having on it.
pub struct SftpSession {
    inner: Channel,
    /// The server's own names for the numbers it reports as ownership. Read at most once per
    /// channel, on demand: a session that never lists anything never reads this.
    names: OnceCell<IdNames>,
}

/// What a server's account files say its numbers are called.
#[derive(Debug, Default)]
struct IdNames {
    users: HashMap<u32, String>,
    groups: HashMap<u32, String>,
}

impl SftpSession {
    /// Takes over a channel that has already been opened as the `sftp` subsystem.
    pub async fn new<S>(stream: S) -> Result<Self, ShellError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let inner = Channel::new(stream)
            .await
            .map_err(|error| ShellError::sftp_transfer_failed("the file channel", describe(&error)))?;

        Ok(Self {
            inner,
            names: OnceCell::new(),
        })
    }

    /// Reads a directory, with directories first and then names in a stable order.
    ///
    /// The server decides what order it sends; sorting here means the list does not shuffle
    /// between two reads of the same directory, which is the kind of thing that makes a file
    /// browser feel broken.
    pub async fn list(&self, path: &str) -> Result<SftpListing, ShellError> {
        let resolved = self.resolve(path).await;
        let names = self.id_names().await;

        let mut entries = Vec::new();
        let reader = self
            .inner
            .read_dir(resolved.as_str())
            .await
            .map_err(|error| failure(&error, &resolved))?;

        for entry in reader {
            let attributes = entry.metadata();
            // The listing names its own entries: a client that joined the strings itself would get
            // the root and a trailing slash wrong sooner or later.
            let entry_path = join(&resolved, &entry.file_name());
            entries.push(describe_entry(entry.file_name(), entry_path, &attributes, names));
        }

        entries.sort_by(|left, right| {
            let left_rank = u8::from(left.kind != EntryKind::Directory);
            let right_rank = u8::from(right.kind != EntryKind::Directory);
            left_rank
                .cmp(&right_rank)
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
        });

        Ok(SftpListing {
            path: resolved,
            entries,
        })
    }

    /// Resolves a path the way the server sees it, which is what the breadcrumbs show.
    pub async fn resolve(&self, path: &str) -> String {
        match self.inner.canonicalize(path.to_owned()).await {
            Ok(resolved) if !resolved.is_empty() => resolved,
            // An unresolvable path is still worth trying to read: the error that follows says why,
            // and it is a better message than anything this could invent.
            _ => path.to_owned(),
        }
    }

    /// The server's own names for the numbers it reports as ownership, read once per channel.
    ///
    /// Both account files are read at most once: accounts are added while a machine is being set
    /// up, not while a panel is open, and reading them on every listing would put two file reads in
    /// front of every directory. A server that will not hand them over — a Windows host, a locked
    /// down `/etc/passwd`, a chroot without either file — simply gets numbers in the listing, which
    /// is what the protocol itself said.
    async fn id_names(&self) -> &IdNames {
        self.names
            .get_or_init(|| async {
                IdNames {
                    users: self.read_accounts(PASSWD_PATH).await,
                    groups: self.read_accounts(GROUP_PATH).await,
                }
            })
            .await
    }

    async fn read_accounts(&self, path: &str) -> HashMap<u32, String> {
        match self.read_text(path).await {
            Ok(text) => parse_accounts(&text),
            Err(_) => HashMap::new(),
        }
    }

    pub async fn create_dir(&self, path: &str) -> Result<(), ShellError> {
        // Asked before it is attempted, because the server's own refusal for a name that is taken is
        // a bare `Failure` — and a person typing a name into a form is owed better than "the server
        // refused the operation".
        if self.exists(path).await? {
            return Err(ShellError::sftp_exists(path));
        }

        self.inner
            .create_dir(path.to_owned())
            .await
            .map_err(|error| failure(&error, path))
    }

    /// Whether something is already at this path. A link counts: what is being asked about is a name.
    pub async fn exists(&self, path: &str) -> Result<bool, ShellError> {
        self.inner
            .try_exists(path.to_owned())
            .await
            .map_err(|error| failure(&error, path))
    }

    /// Creates an empty file, and refuses when something with that name is already there.
    ///
    /// The refusal is most of the point: SFTP's plain create opens with truncate, so a "new file"
    /// that landed on an existing one would empty it without saying anything. The open below asks for
    /// `EXCLUDE` as well, which is the version of that promise two people typing the same name at the
    /// same moment cannot get past — the one above is only there to make the refusal readable.
    pub async fn create_file(&self, path: &str) -> Result<(), ShellError> {
        if self.exists(path).await? {
            return Err(ShellError::sftp_exists(path));
        }

        match self
            .inner
            .open_with_flags(
                path.to_owned(),
                OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
            )
            .await
        {
            // Nothing is written: the file exists from the moment the server answers the open.
            Ok(file) => file.close().await.map_err(|error| io_failure(&error, path)),
            Err(error) => Err(failure(&error, path)),
        }
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<(), ShellError> {
        self.inner
            .rename(from.to_owned(), to.to_owned())
            .await
            .map_err(|error| failure(&error, from))
    }

    pub async fn remove_file(&self, path: &str) -> Result<(), ShellError> {
        self.inner
            .remove_file(path.to_owned())
            .await
            .map_err(|error| failure(&error, path))
    }

    /// Removes a directory and everything under it.
    ///
    /// Recursive on purpose and by hand: SFTP has no recursive delete, and a half-deleted tree is
    /// worse than a refusal — so the children go first and a failure stops the parent from being
    /// removed while it still has contents.
    pub async fn remove_dir_recursive(&self, path: &str) -> Result<(), ShellError> {
        let listing = self.list(path).await?;

        for entry in listing.entries {
            match entry.kind {
                EntryKind::Directory => Box::pin(self.remove_dir_recursive(&entry.path)).await?,
                _ => self.remove_file(&entry.path).await?,
            }
        }

        self.inner
            .remove_dir(path.to_owned())
            .await
            .map_err(|error| failure(&error, path))
    }

    /// Reads a file as text, bounded and validated.
    pub async fn read_text(&self, path: &str) -> Result<String, ShellError> {
        let attributes = self
            .inner
            .metadata(path.to_owned())
            .await
            .map_err(|error| failure(&error, path))?;

        if let Some(size) = attributes.size
            && size > MAX_EDIT_BYTES
        {
            return Err(ShellError::sftp_transfer_failed(
                path,
                format!("the file is {size} bytes, larger than the editor will open"),
            ));
        }

        let bytes = self
            .inner
            .read(path.to_owned())
            .await
            .map_err(|error| failure(&error, path))?;

        String::from_utf8(bytes)
            .map_err(|_| ShellError::sftp_transfer_failed(path, "the file is not text this build can show"))
    }

    /// Writes text to a file, creating it or replacing what was there.
    ///
    /// `create` rather than the channel's own `write`, which opens for writing *only* and therefore
    /// refuses a path that does not exist yet — which is most of the paths a person types.
    pub async fn write_text(&self, path: &str, text: &str) -> Result<(), ShellError> {
        let mut file = self
            .inner
            .create(path.to_owned())
            .await
            .map_err(|error| failure(&error, path))?;
        file.write_all(text.as_bytes())
            .await
            .map_err(|error| io_failure(&error, path))?;
        file.close().await.map_err(|error| io_failure(&error, path))?;
        Ok(())
    }

    /// Copies a remote file to a local path, reporting progress and honouring cancellation.
    ///
    /// The local file that was named is **not** touched until the copy is complete: what this writes
    /// is the part file beside it, which is renamed into place at the end. A download that resumes
    /// therefore resumes into the part file, and the older local file — the one that is complete,
    /// possibly a different version — is what a person still has if the transfer never finishes.
    pub async fn download(
        &self,
        remote: &str,
        local: &Path,
        progress: &Arc<TransferProgress>,
    ) -> Result<(), ShellError> {
        let attributes = self
            .inner
            .metadata(remote.to_owned())
            .await
            .map_err(|error| failure(&error, remote))?;

        if attributes.is_dir() {
            return Err(ShellError::sftp_transfer_failed(
                remote,
                "this is a directory, not a file",
            ));
        }
        let total = attributes.size.unwrap_or(0);

        let part = local_part_of(local);
        let start = resume_offset(tokio::fs::metadata(&part).await.map(|found| found.len()).ok(), total);

        let mut file = self
            .inner
            .open(remote.to_owned())
            .await
            .map_err(|error| failure(&error, remote))?;
        if start > 0 {
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|error| io_failure(&error, remote))?;
        }

        let mut destination = tokio::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&part)
            .await
            .map_err(|error| {
                ShellError::sftp_transfer_failed(remote, format!("the local file could not be created: {error}"))
            })?;

        // The counter starts where the file does: a resumed transfer's bar is about the whole file,
        // not about the second half of it.
        progress.set_transferred(start);

        if let Err(error) = self
            .move_bytes(&mut file, &mut destination, progress, remote, "the local file")
            .await
        {
            if progress.is_cancelled() {
                let _ = tokio::fs::remove_file(&part).await;
            }
            return Err(error);
        }

        destination.flush().await.map_err(|error| {
            ShellError::sftp_transfer_failed(remote, format!("the local file could not be flushed: {error}"))
        })?;
        // The handle has to be closed before the rename: Windows will not rename a file that is
        // still open, and every platform wants the bytes on disk before the name changes.
        drop(destination);

        tokio::fs::rename(&part, local).await.map_err(|error| {
            ShellError::sftp_transfer_failed(remote, format!("the finished file could not be put in place: {error}"))
        })
    }

    /// Copies a local file to a remote path, reporting progress and honouring cancellation.
    ///
    /// The remote name that was asked for is written last, by the same rule as a download: what
    /// travels is the part file, and a rename puts it in place once it is whole. A file on a server
    /// that is half of what it should be is worse than a missing one, because the server will go on
    /// using it.
    pub async fn upload(&self, local: &Path, remote: &str, progress: &Arc<TransferProgress>) -> Result<(), ShellError> {
        let total = tokio::fs::metadata(local).await.map_err(|error| {
            ShellError::sftp_transfer_failed(remote, format!("the local file could not be read: {error}"))
        })?;
        if total.is_dir() {
            return Err(ShellError::sftp_transfer_failed(
                remote,
                "this is a directory, not a file",
            ));
        }
        let total = total.len();

        let part = part_of(remote);
        let existing = self
            .inner
            .metadata(part.clone())
            .await
            .ok()
            .and_then(|found| found.size);
        let start = resume_offset(existing, total);

        let mut source = tokio::fs::File::open(local).await.map_err(|error| {
            ShellError::sftp_transfer_failed(remote, format!("the local file could not be read: {error}"))
        })?;
        if start > 0 {
            source.seek(std::io::SeekFrom::Start(start)).await.map_err(|error| {
                ShellError::sftp_transfer_failed(remote, format!("the local file could not be read: {error}"))
            })?;
        }

        // Append rather than write: what is already in the part file is the beginning of this file,
        // and a resumed attempt has to go after it rather than over it.
        let mut file = self
            .inner
            .open_with_flags(part.clone(), OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::APPEND)
            .await
            .map_err(|error| failure(&error, remote))?;

        progress.set_transferred(start);

        if let Err(error) = self
            .move_bytes(&mut source, &mut file, progress, remote, "the local file")
            .await
        {
            if progress.is_cancelled() {
                let _ = self.remove_file(&part).await;
            }
            return Err(error);
        }

        file.shutdown().await.map_err(|error| io_failure(&error, remote))?;

        self.replace_with(&part, remote).await
    }

    /// Moves bytes from one end of a transfer to the other until there are none left.
    ///
    /// One implementation for both directions, because everything that has to be true of the loop —
    /// it notices a cancellation, it waits out a pause, it counts what it moved — is true of both,
    /// and two copies of that would eventually be one copy and one bug. The `where` names the side
    /// that is being read, so a failure says which end had the trouble.
    async fn move_bytes<R, W>(
        &self,
        reader: &mut R,
        writer: &mut W,
        progress: &Arc<TransferProgress>,
        remote: &str,
        reading: &str,
    ) -> Result<(), ShellError>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let mut buffer = vec![0_u8; CHUNK_BYTES];
        loop {
            if !progress.wait_until_allowed().await {
                return Err(cancelled(remote));
            }

            let read = reader
                .read(&mut buffer)
                .await
                .map_err(|error| io_failure(&error, reading))?;
            if read == 0 {
                return Ok(());
            }

            writer
                .write_all(&buffer[..read])
                .await
                .map_err(|error| io_failure(&error, remote))?;
            progress.advance(read as u64);
        }
    }

    /// Puts a finished part file in place of the name that was asked for.
    ///
    /// Two steps on purpose. SFTP's own rename is specified to fail when the destination exists,
    /// and servers differ about whether they honour that — doing the removal first means the outcome
    /// is the same everywhere rather than "works on OpenSSH, quietly replaces an older file on this
    /// other one". The gap is one instruction long, and what could be lost in it is a file that was
    /// about to be replaced anyway.
    async fn replace_with(&self, part: &str, remote: &str) -> Result<(), ShellError> {
        if self.exists(remote).await.unwrap_or(false) {
            self.remove_file(remote).await?;
        }

        self.rename(part, remote).await
    }

    /// The size of a remote file, for a progress bar that knows where it is going.
    pub async fn size_of(&self, path: &str) -> Result<u64, ShellError> {
        let attributes = self
            .inner
            .metadata(path.to_owned())
            .await
            .map_err(|error| failure(&error, path))?;
        Ok(attributes.size.unwrap_or(0))
    }

    /// How big a remote file is and when it last changed, for deciding whether an unfinished part
    /// file still belongs to it.
    ///
    /// The modification time in seconds, which is what SFTP reports. A server that does not say
    /// leaves that half empty rather than refusing: the size is still worth comparing.
    pub async fn stamp_of(&self, path: &str) -> Result<(u64, Option<u64>), ShellError> {
        let attributes = self
            .inner
            .metadata(path.to_owned())
            .await
            .map_err(|error| failure(&error, path))?;

        Ok((attributes.size.unwrap_or(0), attributes.mtime.map(u64::from)))
    }
}

/// How much of a transfer is already done, and whether the part file can be trusted at all.
///
/// A part file longer than the thing being copied is not a prefix of it — the source changed, or
/// the part is from something else entirely — so the answer is to start again rather than to append
/// somebody else's bytes. `None` means there is no part file yet.
fn resume_offset(part: Option<u64>, total: u64) -> u64 {
    match part {
        Some(bytes) if bytes <= total => bytes,
        Some(_) => 0,
        None => 0,
    }
}

/// Joins a directory and a name without ever producing a doubled separator.
fn join(directory: &str, name: &str) -> String {
    if directory.ends_with('/') {
        format!("{directory}{name}")
    } else {
        format!("{directory}/{name}")
    }
}

fn describe_entry(name: String, path: String, attributes: &FileAttributes, names: &IdNames) -> SftpEntry {
    let kind = if attributes.is_dir() {
        EntryKind::Directory
    } else if attributes.is_symlink() {
        EntryKind::Symlink
    } else if attributes.is_regular() {
        EntryKind::File
    } else {
        // A socket, a fifo, a device: worth showing, not worth downloading.
        EntryKind::Other
    };

    SftpEntry {
        name,
        path,
        kind,
        size: attributes.size.unwrap_or(0),
        // The protocol carries seconds since the epoch in a 32-bit field, so a file older than
        // 1970-and-a-bit or newer than 2038 arrives as numbers this build would misread as dates
        // far in the past. Times beyond that are reported as absent rather than wrong.
        modified_at: attributes.mtime.filter(|seconds| *seconds > 0).map(u64::from),
        mode: attributes.permissions,
        owner: attributes.uid.map(|uid| named(&names.users, uid)),
        group: attributes.gid.map(|gid| named(&names.groups, gid)),
        link_target: None,
    }
}

/// The name an account file gave a number, or the number itself when it gave none.
fn named(names: &HashMap<u32, String>, id: u32) -> String {
    match names.get(&id) {
        Some(name) => name.clone(),
        None => id.to_string(),
    }
}

/// The names in `/etc/passwd` or `/etc/group`, keyed by the number in the third field.
///
/// The two files differ in what follows the id and agree on everything before it, so one parser
/// reads both. A line that does not look like an account is skipped rather than guessed at: the
/// fallback for a missing name is the number, which is at least true.
fn parse_accounts(text: &str) -> HashMap<u32, String> {
    let mut names = HashMap::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut fields = line.split(':');
        let Some(name) = fields.next().filter(|name| !name.is_empty()) else {
            continue;
        };
        let Some(id) = fields.nth(1).and_then(|field| field.parse::<u32>().ok()) else {
            continue;
        };

        names.insert(id, name.to_owned());
    }

    names
}

fn failure(error: &SftpError, path: &str) -> ShellError {
    match error {
        SftpError::Status(status) => match status.status_code {
            StatusCode::NoSuchFile => ShellError::sftp_not_found(path),
            StatusCode::PermissionDenied => ShellError::sftp_permission_denied(path),
            other => ShellError::sftp_transfer_failed(path, format!("the server refused the operation: {other:?}")),
        },
        SftpError::Timeout => ShellError::sftp_transfer_failed(path, "the server stopped answering"),
        other => ShellError::sftp_transfer_failed(path, describe(other)),
    }
}

/// The channel's own failures arrive as I/O errors: the file is a stream, and the protocol errors
/// were already turned into envelopes on the way in.
fn io_failure(error: &std::io::Error, path: &str) -> ShellError {
    ShellError::sftp_transfer_failed(path, format!("the transfer failed: {error}"))
}

fn cancelled(path: &str) -> ShellError {
    ShellError::sftp_transfer_failed(path, "the transfer was cancelled")
}

fn describe(error: &SftpError) -> String {
    match error {
        SftpError::IO(message) => format!("the channel failed: {message}"),
        SftpError::Timeout => "the server stopped answering".to_owned(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_account_file_is_read_as_names_for_the_numbers_in_it() {
        let passwd = "\
# the accounts this machine knows
root:x:0:0:root:/root:/bin/bash

labuser:x:1000:1000::/config:/bin/bash
nobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin
a-line-with-no-id:x:nope:0::/:/bin/sh
";
        let names = parse_accounts(passwd);

        assert_eq!(names.get(&0).map(String::as_str), Some("root"));
        assert_eq!(names.get(&1000).map(String::as_str), Some("labuser"));
        assert_eq!(
            names.get(&65534).map(String::as_str),
            Some("nobody"),
            "a name after an empty field is still the name"
        );
        assert_eq!(
            names.len(),
            3,
            "the comment, the blank line and the unparsable one are not names"
        );
    }

    #[test]
    fn a_group_file_is_read_the_same_way_as_a_passwd_file() {
        let group = "root:x:0:\nlabuser:x:1000:labuser\n";

        let names = parse_accounts(group);

        assert_eq!(names.get(&1000).map(String::as_str), Some("labuser"));
    }

    #[test]
    fn a_number_nothing_names_is_shown_as_the_number() {
        let names = HashMap::new();

        assert_eq!(
            named(&names, 1000),
            "1000",
            "an unnamed id is still the id the server sent"
        );
    }
}
