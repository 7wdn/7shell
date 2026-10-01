//! The remote file system, as seen through an SFTP channel.
//!
//! This crate knows about paths, bytes and the server's own refusals. It does not know about local
//! paths: a transfer is given where to read from or write to by whoever asked for it, and choosing
//! that is the caller's business — which is what keeps a download from being told where to land by
//! a script in a webview.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use russh_sftp::client::SftpSession as Channel;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileAttributes, OpenFlags, StatusCode};
use sevenshell_error::ShellError;
use sevenshell_types::{EntryKind, SftpEntry, SftpListing};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// The largest file the editor will open.
///
/// Not a protocol limit: above this a file is not something somebody edits in a panel, and reading
/// it into memory to find that out would be the expensive way to learn it.
pub const MAX_EDIT_BYTES: u64 = 4 * 1024 * 1024;

/// How much is moved per read or write. Large enough to keep the channel busy, small enough that
/// progress moves visibly and a cancellation is noticed quickly.
const CHUNK_BYTES: usize = 64 * 1024;

/// How far a transfer has come, and whether it has been asked to stop.
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
}

impl TransferProgress {
    pub fn new(total: u64) -> Arc<Self> {
        Arc::new(Self {
            total: AtomicU64::new(total),
            transferred: AtomicU64::new(0),
            cancelled: AtomicBool::new(false),
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

    /// Asks the transfer to stop. It stops at the next chunk boundary, so the caller sees a
    /// cancelled transfer rather than a half-written file with no explanation.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    fn advance(&self, bytes: u64) {
        self.transferred.fetch_add(bytes, Ordering::Relaxed);
    }
}

/// One SFTP channel, and the operations worth having on it.
pub struct SftpSession {
    inner: Channel,
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

        Ok(Self { inner })
    }

    /// Reads a directory, with directories first and then names in a stable order.
    ///
    /// The server decides what order it sends; sorting here means the list does not shuffle
    /// between two reads of the same directory, which is the kind of thing that makes a file
    /// browser feel broken.
    pub async fn list(&self, path: &str) -> Result<SftpListing, ShellError> {
        let resolved = self.resolve(path).await;

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
            entries.push(describe_entry(entry.file_name(), entry_path, &attributes));
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

        let mut file = self
            .inner
            .open(remote.to_owned())
            .await
            .map_err(|error| failure(&error, remote))?;

        // The local end is written fresh: a partial download that resumed over an older file would
        // mix two versions, and hiding that from the person is worse than starting over.
        let mut destination = tokio::fs::File::create(local).await.map_err(|error| {
            ShellError::sftp_transfer_failed(remote, format!("the local file could not be created: {error}"))
        })?;

        let mut buffer = vec![0_u8; CHUNK_BYTES];
        loop {
            if progress.is_cancelled() {
                return Err(cancelled(remote));
            }

            let read = file
                .read(&mut buffer)
                .await
                .map_err(|error| io_failure(&error, remote))?;
            if read == 0 {
                break;
            }

            destination.write_all(&buffer[..read]).await.map_err(|error| {
                ShellError::sftp_transfer_failed(remote, format!("the local file could not be written: {error}"))
            })?;
            progress.advance(read as u64);
        }

        destination.flush().await.map_err(|error| {
            ShellError::sftp_transfer_failed(remote, format!("the local file could not be flushed: {error}"))
        })?;

        Ok(())
    }

    /// Copies a local file to a remote path, reporting progress and honouring cancellation.
    pub async fn upload(&self, local: &Path, remote: &str, progress: &Arc<TransferProgress>) -> Result<(), ShellError> {
        let mut source = tokio::fs::File::open(local).await.map_err(|error| {
            ShellError::sftp_transfer_failed(remote, format!("the local file could not be read: {error}"))
        })?;

        let mut file = self
            .inner
            .create(remote.to_owned())
            .await
            .map_err(|error| failure(&error, remote))?;

        let mut buffer = vec![0_u8; CHUNK_BYTES];
        loop {
            if progress.is_cancelled() {
                return Err(cancelled(remote));
            }

            let read = source.read(&mut buffer).await.map_err(|error| {
                ShellError::sftp_transfer_failed(remote, format!("the local file could not be read: {error}"))
            })?;
            if read == 0 {
                break;
            }

            file.write_all(&buffer[..read])
                .await
                .map_err(|error| io_failure(&error, remote))?;
            progress.advance(read as u64);
        }

        file.shutdown().await.map_err(|error| io_failure(&error, remote))?;

        Ok(())
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
}

/// Joins a directory and a name without ever producing a doubled separator.
fn join(directory: &str, name: &str) -> String {
    if directory.ends_with('/') {
        format!("{directory}{name}")
    } else {
        format!("{directory}/{name}")
    }
}

fn describe_entry(name: String, path: String, attributes: &FileAttributes) -> SftpEntry {
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
        link_target: None,
    }
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
