//! Remote files: the browser's session, and the transfers it starts.
//!
//! Local paths never appear in this module's public surface as something the interface chose: a
//! download is given where to write and an upload where to read, and both are decided by whoever
//! asks — the desktop shell opens a native dialog and passes the answer down.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::archive;
use crate::sessions::{ConnectionOutcome, HostKeyDecision, TransferAttempt, TransferRun};
use crate::{Core, TransferRecord};
use sevenshell_container::{
    ByteStream, CommandFuture, CommandTransport, ContainerFiles, InputStream, StreamState, script_of,
};
use sevenshell_error::ShellError;
use sevenshell_ftp::{FtpConnectionOptions, FtpEntry, FtpSession};
use sevenshell_k8s::{K8sManager, RunOutput, RunSession};
use sevenshell_sftp::{SftpSession, TransferProgress};
use sevenshell_ssh::{ExecOutcome, ShellListing, SshSession, TransferOptions};
use sevenshell_sync::{DavEntry, WebDavFiles, normalize_dav_path};
use sevenshell_types::{
    ContainerRuntime, EntryKind, K8sExecRequest, Protocol, Session, SftpEntry, SftpListing, SftpOutcome, SftpPasteItem,
    SftpPasteOutcome, TransferDirection, TransferInfo, TransferState,
};
use sevenshell_vault::ServicePassword;
use tokio::sync::mpsc;

/// One open file browser: the connection it rides on, and the backend it speaks.
pub struct FileSession {
    backend: FileBackend,
    /// The session it was opened for, which is what every transfer started here belongs to.
    ///
    /// Held rather than looked up per transfer because the browser can be gone by the time somebody
    /// asks a transfer whose it is: the row in the list outlives the tab that started it.
    session_id: String,
}

enum FileBackend {
    /// An SFTP channel over an SSH connection.
    Sftp {
        /// Kept alive for as long as the browser is: the channel dies with the connection.
        connection: Arc<SshSession>,
        sftp: Arc<SftpSession>,
    },
    /// One folder on an HTTP server, with no connection object beyond the client it carries.
    WebDav(Arc<WebDavFiles>),
    /// One FTP control connection, serialized: the protocol has one session and one data
    /// connection at a time, so every command and transfer takes the lock.
    Ftp(Arc<Mutex<FtpSession>>),
    /// The shell of an SSH server with no SFTP subsystem: listing and file operations go through
    /// commands, and file bodies through SCP.
    Scp { connection: Arc<SshSession> },
    /// Files inside a running container, reached through a shell in it.
    ///
    /// There is no channel to keep open and no path on this machine: every operation is one `sh -c`
    /// script run inside the container, so what is held here is the handle that builds the scripts.
    /// A pod's container is the same backend: the script is the same one, and only the transport
    /// that runs it differs (a runtime's `exec` there, the API server's `exec` channel here).
    Container(Arc<ContainerFiles>),
}

impl FileSession {
    fn sftp(&self) -> Result<&Arc<SftpSession>, ShellError> {
        match &self.backend {
            FileBackend::Sftp { sftp, .. } => Ok(sftp),
            FileBackend::WebDav(_) | FileBackend::Ftp(_) | FileBackend::Scp { .. } | FileBackend::Container(_) => {
                Err(ShellError::app_unsupported("this file browser is not an SFTP channel"))
            }
        }
    }

    /// The connection underneath, which is what a packed transfer needs: `tar` is a command rather
    /// than a file operation, and SFTP has nowhere to run one.
    fn connection(&self) -> Result<Arc<SshSession>, ShellError> {
        match &self.backend {
            FileBackend::Sftp { connection, .. } => Ok(Arc::clone(connection)),
            FileBackend::Scp { connection } => Ok(Arc::clone(connection)),
            FileBackend::WebDav(_) | FileBackend::Ftp(_) | FileBackend::Container(_) => Err(
                ShellError::app_unsupported("this file browser has no shell to run a command in"),
            ),
        }
    }

    fn session_id(&self) -> String {
        self.session_id.clone()
    }

    /// Removes one entry without caring which kind it is, for the overwrite half of a paste.
    async fn remove_any(&self, path: &str) -> Result<(), ShellError> {
        match &self.backend {
            FileBackend::Sftp { sftp, .. } => sftp.remove_path(path).await,
            FileBackend::WebDav(dav) => dav.remove(path).await,
            FileBackend::Ftp(ftp) => {
                let mut ftp = ftp.lock().await;
                match ftp.remove_file(path).await {
                    Ok(()) => Ok(()),
                    Err(_) => ftp_remove_tree(&mut ftp, path).await,
                }
            }
            FileBackend::Scp { connection } => scp_remove(connection, path).await,
            FileBackend::Container(container) => container.remove(path, true).await,
        }
    }

    async fn list(&self, path: &str) -> Result<SftpListing, ShellError> {
        match &self.backend {
            FileBackend::Sftp { sftp, .. } => sftp.list(path).await,
            FileBackend::WebDav(dav) => Ok(dav_listing(path, dav.list(path).await?)),
            FileBackend::Ftp(ftp) => {
                let entries = ftp.lock().await.list(path).await?;
                Ok(ftp_listing(path, entries))
            }
            FileBackend::Scp { connection } => {
                let listing = connection.list_via_shell(path, SHELL_TIMEOUT).await?;
                Ok(scp_listing(listing))
            }
            FileBackend::Container(container) => container.list(path).await,
        }
    }

    async fn create_dir(&self, path: &str) -> Result<(), ShellError> {
        match &self.backend {
            FileBackend::Sftp { sftp, .. } => sftp.create_dir(path).await,
            FileBackend::WebDav(dav) => dav.create_dir(path).await,
            FileBackend::Ftp(ftp) => {
                let mut ftp = ftp.lock().await;
                if ftp.exists(path).await? {
                    return Err(ShellError::ftp_exists(path));
                }
                ftp.mkdir(path).await
            }
            FileBackend::Scp { connection } => {
                if scp_exists(connection, path).await? {
                    return Err(ShellError::scp_exists(path));
                }
                scp_exec_ok(connection, &format!("mkdir -- {}", archive::shell_quote(path)), path).await
            }
            // `mkdir -p` inside the container, and idempotent by design: the backend refuses a name
            // that is a file, and a folder that is already there is the folder the person asked for.
            FileBackend::Container(container) => container.create_dir(path).await,
        }
    }

    async fn create_file(&self, path: &str) -> Result<(), ShellError> {
        match &self.backend {
            FileBackend::Sftp { sftp, .. } => sftp.create_file(path).await,
            FileBackend::WebDav(dav) => dav.create_file(path).await,
            FileBackend::Ftp(ftp) => {
                let mut ftp = ftp.lock().await;
                if ftp.exists(path).await? {
                    return Err(ShellError::ftp_exists(path));
                }
                let mut empty = tokio::io::empty();
                ftp.upload(path, &mut empty).await.map(|_| ())
            }
            FileBackend::Scp { connection } => {
                if scp_exists(connection, path).await? {
                    return Err(ShellError::scp_exists(path));
                }
                scp_exec_ok(connection, &format!("set -C; : > {}", archive::shell_quote(path)), path).await
            }
            // `touch` and never a redirection: a name that is already there is left with its content
            // rather than emptied, which is the one thing "create" must not do by surprise.
            FileBackend::Container(container) => container.create_file(path).await,
        }
    }

    async fn rename(&self, from: &str, to: &str) -> Result<(), ShellError> {
        match &self.backend {
            FileBackend::Sftp { sftp, .. } => sftp.rename(from, to).await,
            FileBackend::WebDav(dav) => dav.rename(from, to).await,
            FileBackend::Ftp(ftp) => ftp.lock().await.rename(from, to).await,
            FileBackend::Scp { connection } => {
                scp_exec_ok(
                    connection,
                    &format!("mv -f -- {} {}", archive::shell_quote(from), archive::shell_quote(to)),
                    to,
                )
                .await
            }
            FileBackend::Container(container) => {
                // `mv` replaces whatever is at the target, and a rename onto an existing name is a
                // whole file gone. The question is asked here because the backend cannot ask it
                // without a second command, and a name that is taken is a refusal, not a rename.
                if container_exists(container, to).await? {
                    return Err(ShellError::sftp_exists(to));
                }
                container.rename(from, to).await
            }
        }
    }

    async fn set_permissions(&self, path: &str, mode: u32) -> Result<(), ShellError> {
        match &self.backend {
            FileBackend::Sftp { sftp, .. } => sftp.set_permissions(path, mode).await,
            FileBackend::WebDav(_) => Err(ShellError::app_unsupported("a WebDAV folder has no permission bits")),
            FileBackend::Ftp(_) => Err(ShellError::app_unsupported("an FTP server has no chmod reply")),
            FileBackend::Scp { connection } => {
                scp_exec_ok(
                    connection,
                    &format!("chmod {:04o} -- {}", mode, archive::shell_quote(path)),
                    path,
                )
                .await
            }
            FileBackend::Container(container) => container.set_mode(path, mode).await,
        }
    }

    async fn remove(&self, path: &str, kind: EntryKind) -> Result<(), ShellError> {
        match &self.backend {
            FileBackend::Sftp { sftp, .. } => match kind {
                EntryKind::Directory => sftp.remove_dir_recursive(path).await,
                _ => sftp.remove_file(path).await,
            },
            FileBackend::WebDav(dav) => dav.remove(path).await,
            FileBackend::Ftp(ftp) => {
                let mut ftp = ftp.lock().await;
                match kind {
                    EntryKind::Directory => ftp_remove_tree(&mut ftp, path).await,
                    _ => ftp.remove_file(path).await,
                }
            }
            FileBackend::Scp { connection } => match kind {
                EntryKind::Directory => {
                    scp_exec_ok(connection, &format!("rm -rf -- {}", archive::shell_quote(path)), path).await
                }
                _ => scp_exec_ok(connection, &format!("rm -f -- {}", archive::shell_quote(path)), path).await,
            },
            FileBackend::Container(container) => container.remove(path, kind == EntryKind::Directory).await,
        }
    }

    async fn read_text(&self, path: &str) -> Result<String, ShellError> {
        match &self.backend {
            FileBackend::Sftp { sftp, .. } => sftp.read_text(path).await,
            FileBackend::WebDav(dav) => String::from_utf8(dav.read(path).await?)
                .map_err(|_| ShellError::webdav_transfer_failed(path, "the file is not UTF-8 text")),
            FileBackend::Ftp(ftp) => {
                let mut writer = VecWriter::default();
                ftp.lock().await.download(path, &mut writer).await?;
                String::from_utf8(writer.bytes)
                    .map_err(|_| ShellError::ftp_operation_failed("the file is not UTF-8 text"))
            }
            FileBackend::Scp { connection } => scp_read_text(connection, path).await,
            FileBackend::Container(container) => container.read_text(path).await,
        }
    }

    async fn write_text(&self, path: &str, text: &str) -> Result<(), ShellError> {
        match &self.backend {
            FileBackend::Sftp { sftp, .. } => sftp.write_text(path, text).await,
            FileBackend::WebDav(dav) => dav.write(path, text.as_bytes().to_vec()).await,
            FileBackend::Ftp(ftp) => {
                let mut input = text.as_bytes();
                ftp.lock().await.upload(path, &mut input).await.map(|_| ())
            }
            FileBackend::Scp { connection } => scp_write_text(connection, path, text).await,
            FileBackend::Container(container) => container.write_text(path, text).await,
        }
    }

    async fn exists(&self, path: &str) -> Result<bool, ShellError> {
        match &self.backend {
            FileBackend::Sftp { sftp, .. } => sftp.exists(path).await,
            FileBackend::WebDav(dav) => dav.exists(path).await,
            FileBackend::Ftp(ftp) => ftp.lock().await.exists(path).await,
            FileBackend::Scp { connection } => scp_exists(connection, path).await,
            FileBackend::Container(container) => container_exists(container, path).await,
        }
    }

    async fn size_of(&self, path: &str) -> Result<u64, ShellError> {
        match &self.backend {
            FileBackend::Sftp { sftp, .. } => sftp.size_of(path).await,
            FileBackend::WebDav(dav) => Ok(dav.stat(path).await?.map(|entry| entry.size).unwrap_or(0)),
            FileBackend::Ftp(ftp) => Ok(ftp.lock().await.stat(path).await?.map(|entry| entry.size).unwrap_or(0)),
            FileBackend::Scp { connection } => scp_size(connection, path).await,
            FileBackend::Container(container) => container_size_of(container, path).await,
        }
    }
}

/// How long a remote-side copy or move may run before it is treated as hung.
///
/// Copying a directory is a long command rather than a stuck one; this is the budget for the far
/// side's `cp`/`mv`, not a hurry for a copy that is working.
const PASTE_TIMEOUT: Duration = Duration::from_secs(600);

/// How long one shell command from a file browser may take.
const SHELL_TIMEOUT: Duration = Duration::from_secs(60);

/// How long one SCP transfer may take.
const SCP_TIMEOUT: Duration = Duration::from_secs(600);

/// How much text the editor will read out of one file over a shell.
const SHELL_TEXT_LIMIT: usize = 16 * 1024 * 1024;

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
    /// Whether the backend behind this transfer can pause it.
    pausable: bool,
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
        if session.protocol == Protocol::WebDav {
            return self.open_webdav_files(&session).await;
        }
        if session.protocol == Protocol::Ftp {
            return self.open_ftp_files(&session).await;
        }

        let connection = match self.open_carrier(&session).await? {
            ConnectionOutcome::Ready(connection) => connection,
            ConnectionOutcome::Decision(decision) => return Ok(into_files_outcome(*decision)),
        };

        let stream = match connection.open_subsystem_stream("sftp").await {
            Ok(stream) => stream,
            Err(sftp_error) => {
                // A server old enough to have no SFTP subsystem is the case this fallback exists
                // for. The probe is what tells a missing subsystem apart from a connection that is
                // already gone: only a working shell with `scp` can browse this way.
                let ready = matches!(
                    connection.probe_shell_capabilities(self.timeout).await,
                    Ok(capabilities) if capabilities.scp && capabilities.can_list()
                );
                if ready {
                    return self.open_scp_files(&session, connection).await;
                }
                return Err(sftp_error);
            }
        };
        let sftp = match SftpSession::new(stream).await {
            Ok(sftp) => Arc::new(sftp),
            Err(sftp_error) => {
                // A server can accept the subsystem request and then say nothing: the channel is
                // open and the protocol never starts. That is the same missing subsystem from this
                // side, so the same fallback applies.
                let ready = matches!(
                    connection.probe_shell_capabilities(self.timeout).await,
                    Ok(capabilities) if capabilities.scp && capabilities.can_list()
                );
                if ready {
                    return self.open_scp_files(&session, connection).await;
                }
                return Err(sftp_error);
            }
        };

        // The first listing is part of the answer: a browser that opened and then had to ask where
        // it was would have two ways to fail instead of one.
        let listing = sftp.list(".").await?;

        let handle = format!("f{}", self.next_file.fetch_add(1, Ordering::Relaxed));
        self.files.lock().await.insert(
            handle.clone(),
            Arc::new(FileSession {
                backend: FileBackend::Sftp { connection, sftp },
                session_id: session_id.to_owned(),
            }),
        );

        Ok(SftpOutcome::Opened {
            handle,
            session_id: session_id.to_owned(),
            listing,
            packed: true,
        })
    }

    /// Opens a WebDAV folder as a file browser.
    ///
    /// There is no host key and no SSH connection: the session's address is the folder, and the
    /// first listing is as much a proof that the server is a WebDAV one as the opening is.
    async fn open_webdav_files(&self, session: &Session) -> Result<SftpOutcome, ShellError> {
        let password = self
            .vault
            .secret_for_session(&session.id)?
            .map(|secret| ServicePassword::new(secret.into_value()));
        let username = Some(session.user.trim().to_owned()).filter(|user| !user.is_empty());
        let dav = Arc::new(WebDavFiles::new(&session.webdav.url, username, password)?);

        let entries = dav.list("/").await?;
        let handle = format!("f{}", self.next_file.fetch_add(1, Ordering::Relaxed));
        self.files.lock().await.insert(
            handle.clone(),
            Arc::new(FileSession {
                backend: FileBackend::WebDav(dav),
                session_id: session.id.clone(),
            }),
        );

        Ok(SftpOutcome::Opened {
            handle,
            session_id: session.id.clone(),
            listing: dav_listing("/", entries),
            packed: false,
        })
    }

    /// Opens an FTP/FTPS session as a file browser.
    ///
    /// There is no host key: the protocol has no server identity to pin, and FTPS certificates are
    /// verified against the platform trust store by the client crate. The login directory is the
    /// root the browser shows.
    async fn open_ftp_files(&self, session: &Session) -> Result<SftpOutcome, ShellError> {
        let password = self
            .vault
            .secret_for_session(&session.id)?
            .map(|secret| ServicePassword::new(secret.into_value()))
            .unwrap_or_else(|| ServicePassword::new(String::new()));
        let username = if session.user.trim().is_empty() {
            "anonymous".to_owned()
        } else {
            session.user.trim().to_owned()
        };
        let options = FtpConnectionOptions::new(session.host.clone(), username, password)
            .with_port(session.port)
            .with_secure(session.ftp.secure);
        let ftp = Arc::new(Mutex::new(FtpSession::connect(options).await?));

        let entries = ftp.lock().await.list("/").await?;
        let handle = format!("f{}", self.next_file.fetch_add(1, Ordering::Relaxed));
        self.files.lock().await.insert(
            handle.clone(),
            Arc::new(FileSession {
                backend: FileBackend::Ftp(ftp),
                session_id: session.id.clone(),
            }),
        );

        Ok(SftpOutcome::Opened {
            handle,
            session_id: session.id.clone(),
            listing: ftp_listing("/", entries),
            packed: false,
        })
    }

    /// Opens a shell-backed browser for an SSH server with no SFTP subsystem.
    ///
    /// The listing command is the probe's proof that this path works; the backend keeps the
    /// connection alive for SCP transfers and shell-side commands.
    async fn open_scp_files(&self, session: &Session, connection: Arc<SshSession>) -> Result<SftpOutcome, ShellError> {
        // Where a login shell starts is where the SFTP browser would have started. A shell cannot
        // say that from a listing, so it is asked once.
        let home = connection
            .exec_capture("pwd", self.timeout, 4096)
            .await
            .ok()
            .map(|captured| captured.stdout.trim().to_owned())
            .filter(|home| !home.is_empty())
            .unwrap_or_else(|| "/".to_owned());
        let listing = connection.list_via_shell(&home, self.timeout).await?;
        let handle = format!("f{}", self.next_file.fetch_add(1, Ordering::Relaxed));
        self.files.lock().await.insert(
            handle.clone(),
            Arc::new(FileSession {
                backend: FileBackend::Scp { connection },
                session_id: session.id.clone(),
            }),
        );

        Ok(SftpOutcome::Opened {
            handle,
            session_id: session.id.clone(),
            listing: scp_listing(listing),
            packed: false,
        })
    }

    /// Opens a browser for the files inside a running container.
    ///
    /// One `exec` per operation, on the runner the container connection was opened with: a second
    /// `Arc` of the same SSH session is the same connection, which is why the provider hands its
    /// runner out rather than the caller finding one. The first listing is part of the answer and is
    /// what resolves the path — a symlinked directory is shown where it leads, not where it was asked
    /// about — and it is also what refuses a container that is not running, before a handle exists.
    pub async fn open_container_files(
        &self,
        connection_id: &str,
        container_id: &str,
        path: &str,
    ) -> Result<SftpOutcome, ShellError> {
        let provider = self.containers.provider(connection_id)?;
        let runner = provider.runner()?;
        let files = Arc::new(ContainerFiles::for_provider(&provider, container_id, runner)?);

        // A container has no home directory the way a login shell does; its filesystem root is the
        // one place every container has, so an empty starting path means that.
        let start = if path.trim().is_empty() { "/" } else { path };
        let listing = files.list(start).await?;

        let handle = format!("f{}", self.next_file.fetch_add(1, Ordering::Relaxed));
        self.files.lock().await.insert(
            handle.clone(),
            Arc::new(FileSession {
                backend: FileBackend::Container(files),
                // The container connection is what a transfer started here belongs to: it is the
                // tab the browser was opened from, and the one thing that outlives the panel.
                session_id: connection_id.to_owned(),
            }),
        );

        Ok(SftpOutcome::Opened {
            handle,
            session_id: connection_id.to_owned(),
            listing,
            // A selection is fetched entry by entry rather than as one archive, but the browser
            // offers packing the same way: what lands is the entries themselves either way.
            packed: true,
        })
    }

    /// Opens a browser for the files inside one container of one pod.
    ///
    /// The same file backend the container path uses, with a transport that runs its scripts
    /// through the API server's `exec` channel instead of a runtime's CLI: every path, every
    /// quoting rule and every tar check is therefore the one the container browser already made.
    /// Nothing here is a path on this machine, and the first listing is part of the answer — it is
    /// what resolves the path and what refuses a pod that is not reachable before a handle exists.
    pub async fn open_pod_files(
        &self,
        connection_id: &str,
        namespace: &str,
        pod: &str,
        container: &str,
        path: &str,
    ) -> Result<SftpOutcome, ShellError> {
        let transport = PodTransport {
            clusters: Arc::clone(&self.clusters),
            connection_id: connection_id.to_owned(),
            namespace: namespace.trim().to_owned(),
            pod: pod.trim().to_owned(),
            container: container.trim().to_owned(),
        };
        // The runtime name shapes the argv a pod never sees — the transport takes the script out of
        // it — and `Docker` is the shape every runtime shares, so it is also the shape a pod's
        // script is spelled in. The namespace of a pod is the API's, not a runtime's, so it stays
        // empty here and the pod name is what the backend validates as the container it addresses.
        let pod_name = transport.pod.clone();
        let files = Arc::new(ContainerFiles::from_transport(
            ContainerRuntime::Docker,
            "",
            &pod_name,
            Arc::new(transport),
        )?);

        // A pod container has no home directory either; its filesystem root is the one place every
        // container has, so an empty starting path means that.
        let start = if path.trim().is_empty() { "/" } else { path };
        let listing = files.list(start).await?;

        let handle = format!("f{}", self.next_file.fetch_add(1, Ordering::Relaxed));
        self.files.lock().await.insert(
            handle.clone(),
            Arc::new(FileSession {
                backend: FileBackend::Container(files),
                // The cluster connection is what a transfer started here belongs to: it is the tab
                // the browser was opened from, and the one thing that outlives the panel.
                session_id: connection_id.to_owned(),
            }),
        );

        Ok(SftpOutcome::Opened {
            handle,
            session_id: connection_id.to_owned(),
            listing,
            // A selection is fetched entry by entry rather than as one archive, but the browser
            // offers packing the same way: what lands is the entries themselves either way.
            packed: true,
        })
    }

    /// Reads a directory.
    pub async fn list_files(&self, handle: &str, path: &str) -> Result<SftpListing, ShellError> {
        self.file_session(handle).await?.list(path).await
    }

    pub async fn create_folder(&self, handle: &str, path: &str) -> Result<(), ShellError> {
        self.file_session(handle).await?.create_dir(path).await
    }

    /// Creates an empty file, unless something with that name is already there.
    pub async fn create_file(&self, handle: &str, path: &str) -> Result<(), ShellError> {
        self.file_session(handle).await?.create_file(path).await
    }

    pub async fn rename_entry(&self, handle: &str, from: &str, to: &str) -> Result<(), ShellError> {
        self.file_session(handle).await?.rename(from, to).await
    }

    /// Changes the permission bits of one entry.
    pub async fn set_permissions(&self, handle: &str, path: &str, mode: u32) -> Result<(), ShellError> {
        if mode & !0o7777 != 0 {
            return Err(ShellError::sftp_operation_failed(
                path,
                "permission bits must be between 0000 and 7777",
            ));
        }
        self.file_session(handle).await?.set_permissions(path, mode).await
    }

    /// Copies or moves a clipboard's paths into one directory.
    ///
    /// The far side does the work: a recursive copy through SFTP would read every byte back and
    /// send it out again, while the server's own `cp -a` keeps it local and preserves the metadata
    /// an archive-mode copy is expected to preserve. Paths are quoted for the shell one at a time —
    /// a file may be called `x; rm -rf /`.
    ///
    /// Nothing is changed when a target already exists: the conflicts come back first, and the
    /// caller asks whether to replace them. Answering yes runs the same request with `overwrite`.
    pub async fn paste_entries(
        &self,
        handle: &str,
        sources: &[String],
        destination: &str,
        moving: bool,
        overwrite: bool,
    ) -> Result<SftpPasteOutcome, ShellError> {
        let files = self.file_session(handle).await?;
        let destination = destination.trim_end_matches('/');
        let destination = if destination.is_empty() { "/" } else { destination };

        let mut planned = Vec::with_capacity(sources.len());
        for source in sources {
            let source = source.trim_end_matches('/');
            if source.is_empty() || source == "/" {
                return Err(ShellError::sftp_operation_failed(
                    source,
                    "the filesystem root cannot be copied or moved",
                ));
            }
            let name = source.rsplit('/').next().unwrap_or(source);
            let target = join_remote(destination, name);
            planned.push((source.to_owned(), target));
        }

        let mut settled = Vec::new();
        let mut conflicts = Vec::new();
        let mut runnable = Vec::new();
        for (source, target) in planned {
            if source == target {
                // Pasting a cut back into the directory it came from is already where it belongs.
                settled.push(SftpPasteItem {
                    source,
                    target,
                    ok: moving,
                    error: (!moving).then(|| "the source and target are the same path".to_owned()),
                });
                continue;
            }
            if !overwrite && files.exists(&target).await? {
                conflicts.push(target);
                continue;
            }
            runnable.push((source, target));
        }

        if !conflicts.is_empty() {
            // The question half of this answer: nothing has been executed, so a "no" has left the
            // directory exactly as it was.
            return Ok(SftpPasteOutcome {
                conflicts,
                results: Vec::new(),
            });
        }

        let mut results = settled;
        for (source, target) in runnable {
            if overwrite
                && files.exists(&target).await?
                && let Err(error) = files.remove_any(&target).await
            {
                results.push(SftpPasteItem {
                    source,
                    target,
                    ok: false,
                    error: Some(error.to_string()),
                });
                continue;
            }

            let outcome = match &files.backend {
                FileBackend::WebDav(dav) => {
                    let outcome = if moving {
                        dav.rename(&source, &target).await
                    } else {
                        dav.copy(&source, &target).await
                    };
                    outcome.map_err(|error| error.to_string())
                }
                FileBackend::Sftp { .. } | FileBackend::Scp { .. } => {
                    // The far side does the work: a recursive copy through SFTP would read every
                    // byte back and send it out again, while the server's own `cp -a` keeps it
                    // local and preserves the metadata an archive-mode copy is expected to
                    // preserve. Paths are quoted one at a time — a file may be called
                    // `x; rm -rf /`.
                    let command = if moving {
                        format!(
                            "mv -f -- {} {}",
                            archive::shell_quote(&source),
                            archive::shell_quote(&target)
                        )
                    } else {
                        format!(
                            "cp -a -- {} {}",
                            archive::shell_quote(&source),
                            archive::shell_quote(&target)
                        )
                    };
                    match files.connection()?.exec(&command, PASTE_TIMEOUT).await {
                        Ok(done) if done.exit_code == Some(0) => Ok(()),
                        Ok(done) => Err(brief_report(&done)),
                        Err(error) => Err(error.to_string()),
                    }
                }
                FileBackend::Ftp(ftp) => {
                    if moving {
                        ftp.lock()
                            .await
                            .rename(&source, &target)
                            .await
                            .map_err(|error| error.to_string())
                    } else {
                        Err("an FTP server cannot copy a file without transferring its bytes".to_owned())
                    }
                }
                FileBackend::Container(container) => {
                    // The container's own `mv`/`cp -a`: the far side already has the bytes, and a
                    // recursive copy through this channel would carry every one of them back here.
                    let outcome = if moving {
                        container.rename(&source, &target).await
                    } else {
                        container.copy(&source, &target).await
                    };
                    outcome.map_err(|error| error.to_string())
                }
            };

            match outcome {
                Ok(()) => results.push(SftpPasteItem {
                    source,
                    target,
                    ok: true,
                    error: None,
                }),
                Err(error) => results.push(SftpPasteItem {
                    source,
                    target,
                    ok: false,
                    error: Some(error),
                }),
            }
        }

        Ok(SftpPasteOutcome {
            conflicts: Vec::new(),
            results,
        })
    }

    /// Deletes a file, or a directory and everything under it.
    ///
    /// The kind comes from what the browser listed rather than from a second look: the entry a
    /// person right-clicked is the entry they meant, even if something replaced it since.
    pub async fn delete_entry(&self, handle: &str, path: &str, kind: EntryKind) -> Result<(), ShellError> {
        self.file_session(handle).await?.remove(path, kind).await
    }

    pub async fn read_text_file(&self, handle: &str, path: &str) -> Result<String, ShellError> {
        self.file_session(handle).await?.read_text(path).await
    }

    pub async fn write_text_file(&self, handle: &str, path: &str, text: &str) -> Result<(), ShellError> {
        self.file_session(handle).await?.write_text(path, text).await
    }

    /// Starts copying a remote file to a local path, and returns the transfer's id.
    pub async fn start_download(&self, handle: &str, remote: &str, local: PathBuf) -> Result<String, ShellError> {
        let browser = self.file_session(handle).await?;
        let size = browser.size_of(remote).await?;
        let name = file_name_of(remote);
        // Owned before the task takes it: the task outlives the borrow this call was given.
        let source = remote.to_owned();
        let id = self.reserve_transfer_id();
        let session_id = browser.session_id();
        // A container download is one `tar | base64` answered as text by the runner: there is no
        // stream to hold where it stopped, so a paused row could not be continued from it.
        let pausable = !matches!(&browser.backend, FileBackend::Scp { .. } | FileBackend::Container(_));
        let facts = TransferFacts {
            direction: TransferDirection::Download,
            name,
            remote_path: source.clone(),
            session_id,
            size,
            packed: false,
            pausable,
            // Where it is landing, which is what the row can be pointed at afterwards.
            local: Some(local.clone()),
        };

        match &browser.backend {
            FileBackend::Sftp { sftp, .. } => {
                let files = Arc::clone(sftp);
                let witness = Arc::new(SourceWitness::default());

                self.spawn_transfer(id, facts, move |progress| {
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
                })
                .await
            }
            FileBackend::WebDav(dav) => {
                let dav = Arc::clone(dav);

                self.spawn_transfer(id, facts, move |progress| {
                    let dav = Arc::clone(&dav);
                    let source = source.clone();
                    let local = local.clone();

                    Box::pin(async move { webdav_download(&dav, &source, &local, &progress).await }) as TransferAttempt
                })
                .await
            }
            FileBackend::Ftp(ftp) => {
                let ftp = Arc::clone(ftp);

                self.spawn_transfer(id, facts, move |progress| {
                    let ftp = Arc::clone(&ftp);
                    let source = source.clone();
                    let local = local.clone();

                    Box::pin(async move { ftp_download(&ftp, &source, &local, progress).await }) as TransferAttempt
                })
                .await
            }
            FileBackend::Scp { connection } => {
                let connection = Arc::clone(connection);

                self.spawn_transfer(id, facts, move |progress| {
                    let connection = Arc::clone(&connection);
                    let source = source.clone();
                    let local = local.clone();

                    Box::pin(async move { scp_download_to(&connection, &source, &local, progress).await })
                        as TransferAttempt
                })
                .await
            }
            FileBackend::Container(container) => {
                let container = Arc::clone(container);
                let total = size;

                self.spawn_transfer(id, facts, move |progress| {
                    let container = Arc::clone(&container);
                    let source = source.clone();
                    let local = local.clone();

                    Box::pin(async move {
                        // Nothing of the tree is written here until the archive inside the container
                        // is complete: the bytes arrive as one answer, so a stop during the wait
                        // leaves no half-written file behind.
                        tokio::select! {
                            outcome = container.download(&source, &local) => outcome?,
                            _ = wait_cancelled(Arc::clone(&progress)) => return Err(cancelled(&source)),
                        }
                        progress.set_transferred(total);
                        Ok(())
                    }) as TransferAttempt
                })
                .await
            }
        }
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
        let files = self.file_session(handle).await?;
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
        let id = self.reserve_transfer_id();
        let session_id = browser.session_id();
        // A container upload is one `tar` handed to `sh` on standard input, with no stream of its
        // own: there is nowhere to hold it, so it cannot be paused.
        let pausable = !matches!(&browser.backend, FileBackend::Scp { .. } | FileBackend::Container(_));
        let facts = TransferFacts {
            direction: TransferDirection::Upload,
            name,
            remote_path: remote,
            session_id,
            size,
            packed: false,
            pausable,
            // Where it was read from, which is the end a person can still be shown.
            local: Some(local.clone()),
        };

        match &browser.backend {
            FileBackend::Sftp { sftp, .. } => {
                let files = Arc::clone(sftp);
                let witness = Arc::new(SourceWitness::default());

                self.spawn_transfer(id, facts, move |progress| {
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
                })
                .await
            }
            FileBackend::WebDav(dav) => {
                let dav = Arc::clone(dav);

                self.spawn_transfer(id, facts, move |progress| {
                    let dav = Arc::clone(&dav);
                    let local = local.clone();
                    let destination = destination.clone();

                    Box::pin(async move { webdav_upload(&dav, &local, &destination, &progress).await })
                        as TransferAttempt
                })
                .await
            }
            FileBackend::Ftp(ftp) => {
                let ftp = Arc::clone(ftp);

                self.spawn_transfer(id, facts, move |progress| {
                    let ftp = Arc::clone(&ftp);
                    let local = local.clone();
                    let destination = destination.clone();

                    Box::pin(async move { ftp_upload(&ftp, &local, &destination, progress).await }) as TransferAttempt
                })
                .await
            }
            FileBackend::Scp { connection } => {
                let connection = Arc::clone(connection);

                self.spawn_transfer(id, facts, move |progress| {
                    let connection = Arc::clone(&connection);
                    let local = local.clone();
                    let destination = destination.clone();

                    Box::pin(async move {
                        let (parent, _) = destination.rsplit_once('/').unwrap_or(("", destination.as_str()));
                        let parent = if parent.is_empty() { "/" } else { parent };
                        let callback_progress = Arc::clone(&progress);
                        let mut callback = move |bytes: u64| callback_progress.set_transferred(bytes);
                        let options = TransferOptions::default();
                        let transfer =
                            connection.scp_upload(&local, parent, &options, SCP_TIMEOUT, Some(&mut callback));
                        tokio::select! {
                            result = transfer => match result {
                                Ok(_) => Ok(()),
                                Err(error) => {
                                    let _ = scp_remove(&connection, &destination).await;
                                    Err(error)
                                }
                            },
                            _ = wait_cancelled(Arc::clone(&progress)) => {
                                let _ = scp_remove(&connection, &destination).await;
                                Ok(())
                            }
                        }
                    }) as TransferAttempt
                })
                .await
            }
            FileBackend::Container(container) => {
                let container = Arc::clone(container);
                let directory = remote_dir.to_owned();
                let total = size;

                self.spawn_transfer(id, facts, move |progress| {
                    let container = Arc::clone(&container);
                    let local = local.clone();
                    let destination = destination.clone();
                    // The archive carries the chosen name, so it is unpacked *into* the folder that
                    // was being browsed rather than onto the path the file lands at: `tar xf - -C`
                    // naming a file would make a folder of that name and put the file inside it.
                    let directory = directory.clone();

                    Box::pin(async move {
                        tokio::select! {
                            outcome = container.upload(&local, &directory) => outcome?,
                            _ = wait_cancelled(Arc::clone(&progress)) => return Err(cancelled(&destination)),
                        }
                        progress.set_transferred(total);
                        Ok(())
                    }) as TransferAttempt
                })
                .await
            }
        }
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
        if let FileBackend::Container(container) = &session.backend {
            return self
                .start_container_download(Arc::clone(container), &session.session_id(), entries, into)
                .await;
        }
        let files = Arc::clone(session.sftp()?);
        let connection = session.connection()?;
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
                pausable: true,
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
        if let FileBackend::Container(container) = &session.backend {
            return self
                .start_container_upload(Arc::clone(container), &session.session_id(), sources, remote_dir)
                .await;
        }
        let files = Arc::clone(session.sftp()?);
        let connection = session.connection()?;
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
                pausable: true,
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

    /// Fetches a selection out of a container, one entry at a time.
    ///
    /// There is no archive to make on this side: a container's `tar` writes its bytes to the
    /// runner's answer, so several entries would be several archives — and the container's file
    /// handle already turns one entry into one archive. What lands is what the SFTP path lands: the
    /// entries themselves, under their own names, inside the folder that was chosen.
    async fn start_container_download(
        &self,
        container: Arc<ContainerFiles>,
        session_id: &str,
        entries: &[String],
        into: PathBuf,
    ) -> Result<String, ShellError> {
        let Some(first) = entries.first() else {
            return Err(ShellError::sftp_archive_failed("(nothing)", "there is nothing to pack"));
        };
        let id = self.reserve_transfer_id();
        let held = match entries {
            [only] => archive::archive_name(&file_name_of(only)),
            several => archive::downloaded_name(several.len()),
        };
        // Where the selection came from, which is what the row is about. A download has no directory
        // that is read again afterwards, so a selection spread over two folders is described by the
        // first entry's parent rather than refused the way one archive would have to.
        let origin = split_remote(first).0;
        let label = first.clone();
        let plan: Vec<(String, PathBuf)> = entries
            .iter()
            .map(|entry| (entry.clone(), into.join(file_name_of(entry))))
            .collect();

        self.spawn_transfer(
            id,
            TransferFacts {
                direction: TransferDirection::Download,
                name: held,
                remote_path: origin,
                session_id: session_id.to_owned(),
                size: 0,
                packed: true,
                pausable: false,
                local: Some(into),
            },
            move |progress| {
                let container = Arc::clone(&container);
                let plan = plan.clone();
                let label = label.clone();

                Box::pin(async move {
                    for (entry, destination) in &plan {
                        if progress.is_cancelled() {
                            return Err(cancelled(&label));
                        }
                        container.download(entry, destination).await?;
                    }
                    Ok(())
                }) as TransferAttempt
            },
        )
        .await
    }

    /// Sends a selection into a container, one source at a time.
    ///
    /// Each source crosses as its own archive, because a container `exec` carries one standard input
    /// and the only `tar` that can unpack it is the container's own. The row is one transfer, the way
    /// the SFTP path's packed upload is — with one difference worth knowing: a failure part-way
    /// leaves what already arrived, where one archive would have been all or nothing.
    async fn start_container_upload(
        &self,
        container: Arc<ContainerFiles>,
        session_id: &str,
        sources: Vec<PathBuf>,
        directory: &str,
    ) -> Result<String, ShellError> {
        let id = self.reserve_transfer_id();
        let name = archive::uploaded_name(sources.len());
        // The folder the files land in, which is what the panel reads again when this finishes: a
        // packed upload puts them *in* it rather than one file inside it.
        let directory = directory.to_owned();

        self.spawn_transfer(
            id,
            TransferFacts {
                direction: TransferDirection::Upload,
                name,
                remote_path: directory.clone(),
                session_id: session_id.to_owned(),
                size: 0,
                packed: true,
                pausable: false,
                local: None,
            },
            move |progress| {
                let container = Arc::clone(&container);
                let sources = sources.clone();
                let directory = directory.clone();

                Box::pin(async move {
                    for source in &sources {
                        if progress.is_cancelled() {
                            return Err(cancelled(&directory));
                        }
                        container.upload(source, &directory).await?;
                    }
                    Ok(())
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
        if record.info.state != TransferState::Running || !record.info.pausable {
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
            pausable: facts.pausable,
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

/// What a WebDAV listing looks like to the panel: the same shape SFTP produces, with the columns
/// WebDAV does not have left empty rather than filled with zeroes.
fn dav_listing(path: &str, entries: Vec<DavEntry>) -> SftpListing {
    let mut entries: Vec<SftpEntry> = entries
        .into_iter()
        .map(|entry| SftpEntry {
            name: entry.name,
            path: entry.path,
            kind: if entry.is_dir {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            size: entry.size,
            modified_at: entry.modified_at,
            mode: None,
            owner: None,
            group: None,
            uid: None,
            gid: None,
            link_target: None,
        })
        .collect();

    sort_entries(&mut entries);

    SftpListing {
        path: normalize_dav_path(path),
        entries,
    }
}

/// What an FTP listing looks like to the panel.
fn ftp_listing(path: &str, entries: Vec<FtpEntry>) -> SftpListing {
    let mut entries: Vec<SftpEntry> = entries
        .into_iter()
        .map(|entry| SftpEntry {
            name: entry.name,
            path: entry.path,
            kind: if entry.is_dir {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            size: entry.size,
            modified_at: entry.modified.and_then(|seconds| u64::try_from(seconds).ok()),
            mode: None,
            owner: None,
            group: None,
            uid: None,
            gid: None,
            link_target: None,
        })
        .collect();

    sort_entries(&mut entries);

    SftpListing {
        path: path.to_owned(),
        entries,
    }
}

fn sort_entries(entries: &mut [SftpEntry]) {
    entries.sort_by(|left, right| {
        let left_dir = left.kind == EntryKind::Directory;
        let right_dir = right.kind == EntryKind::Directory;
        right_dir
            .cmp(&left_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
}

/// Collects a downloaded file in memory, for the text editor and the small-file paths.
#[derive(Default)]
struct VecWriter {
    bytes: Vec<u8>,
}

impl AsyncWrite for VecWriter {
    fn poll_write(mut self: Pin<&mut Self>, _context: &mut Context<'_>, buffer: &[u8]) -> Poll<std::io::Result<usize>> {
        self.bytes.extend_from_slice(buffer);
        Poll::Ready(Ok(buffer.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// Removes one FTP directory and everything under it.
///
/// FTP has no recursive delete, so the tree is walked here: every child is removed before the
/// directory that held it. The stack is explicit because an async recursive call cannot be boxed
/// without a second allocation per level, and a deep tree is not worth that.
async fn ftp_remove_tree(ftp: &mut FtpSession, path: &str) -> Result<(), ShellError> {
    let mut stack = vec![(path.to_owned(), false)];
    while let Some((current, expanded)) = stack.pop() {
        if expanded {
            ftp.remove_dir(&current).await?;
            continue;
        }

        let entries = ftp.list(&current).await?;
        stack.push((current, true));
        for entry in entries {
            if entry.is_dir {
                stack.push((entry.path, false));
            } else {
                ftp.remove_file(&entry.path).await?;
            }
        }
    }
    Ok(())
}

/// What a shell listing looks like to the panel.
fn scp_listing(listing: ShellListing) -> SftpListing {
    let mut entries: Vec<SftpEntry> = listing
        .entries
        .into_iter()
        .map(|entry| SftpEntry {
            name: entry.name.clone(),
            path: join_remote(&listing.path, &entry.name),
            kind: entry.kind,
            size: entry.size,
            modified_at: entry.modified_at,
            mode: entry.mode,
            owner: entry.owner,
            group: entry.group,
            uid: entry.uid,
            gid: entry.gid,
            link_target: entry.link_target,
        })
        .collect();

    sort_entries(&mut entries);

    SftpListing {
        path: listing.path,
        entries,
    }
}

async fn scp_exec_ok(connection: &SshSession, command: &str, path: &str) -> Result<(), ShellError> {
    let outcome = connection.exec(command, SHELL_TIMEOUT).await?;
    if outcome.exit_code == Some(0) {
        Ok(())
    } else {
        Err(ShellError::scp_operation_failed(path, brief_report(&outcome)))
    }
}

async fn scp_exists(connection: &SshSession, path: &str) -> Result<bool, ShellError> {
    let command = format!("ls -d -- {} >/dev/null 2>&1", archive::shell_quote(path));
    let outcome = connection.exec(&command, SHELL_TIMEOUT).await?;
    Ok(outcome.exit_code == Some(0))
}

async fn scp_size(connection: &SshSession, path: &str) -> Result<u64, ShellError> {
    let command = format!("wc -c < {}", archive::shell_quote(path));
    let captured = connection.exec_capture(&command, SHELL_TIMEOUT, 1024).await?;
    if captured.exit_code != Some(0) {
        return Ok(0);
    }
    Ok(captured.stdout.trim().parse().unwrap_or(0))
}

/// Whether a container's directory listing carries this name.
///
/// The container backend has no `stat`, and a listing of the parent is the only look it offers. That
/// parent therefore has to exist, which it does for every question asked here: a rename target's
/// folder, a paste destination, and the directory an upload is aimed at.
async fn container_exists(files: &ContainerFiles, path: &str) -> Result<bool, ShellError> {
    let (parent, name) = split_remote(path);
    if name.is_empty() {
        // Only the container's root has no name of its own, and it is always there.
        return Ok(true);
    }
    let listing = files.list(&parent).await?;
    Ok(listing.entries.iter().any(|entry| entry.name == name))
}

/// The size a container listing gave a path, when it is a file.
///
/// A folder has no single size, and the tar this backend carries it in is base64 of the whole tree,
/// so a folder download starts without a number the way a packed transfer does.
async fn container_size_of(files: &ContainerFiles, path: &str) -> Result<u64, ShellError> {
    let (parent, name) = split_remote(path);
    let listing = files.list(&parent).await?;
    Ok(listing
        .entries
        .iter()
        .find(|entry| entry.name == name && entry.kind == EntryKind::File)
        .map_or(0, |entry| entry.size))
}

/// Files inside one pod container, run through the API server's `exec` channel.
///
/// The container crate's file backend is reused whole — its paths, its quoting and its tar checks —
/// and this is only the answer to "where does the script run". The argv it is handed names a
/// runtime and a container because that is how the backend spells a command; nothing before the
/// script is read here, and [`script_of`] refuses anything that is not the `sh -c` tail the backend
/// builds, so a change there cannot quietly turn into a different command inside the pod.
///
/// Every call opens its own channel rather than sharing one: a file browser's operations are
/// independent, and a shared socket would need a queue to keep one upload from interleaving with a
/// listing, which is a second scheduler nobody asked for.
struct PodTransport {
    clusters: Arc<K8sManager>,
    connection_id: String,
    namespace: String,
    pod: String,
    container: String,
}

impl PodTransport {
    /// The exec request one script becomes.
    ///
    /// The three names come from the interface and are percent-escaped into their path segments by
    /// the URL builder, so none of them can spell a different resource. An empty container is the
    /// pod's own annotation, which is what "no container chosen" means in the API.
    fn request(&self, script: &str) -> K8sExecRequest {
        K8sExecRequest {
            namespace: self.namespace.clone(),
            pod: self.pod.clone(),
            container: self.container.clone(),
            command: script.to_owned(),
        }
    }

    async fn open(&self, argv: &[String]) -> Result<RunSession, ShellError> {
        let command = script_of(argv)?;
        self.clusters
            .exec_run(&self.connection_id, &self.request(command.script), command.stdin)
            .await
    }
}

impl CommandTransport for PodTransport {
    fn run<'a>(&'a self, argv: &'a [String]) -> CommandFuture<'a, String> {
        Box::pin(async move {
            let session = self.open(argv).await?;
            finished(session.run_to_end(&[]).await?)
        })
    }

    fn run_stdin<'a>(&'a self, argv: &'a [String], input: Vec<u8>) -> CommandFuture<'a, String> {
        Box::pin(async move {
            let session = self.open(argv).await?;
            finished(session.run_to_end(&input).await?)
        })
    }

    fn stream_stdin<'a>(&'a self, argv: &'a [String], mut input: InputStream) -> CommandFuture<'a, String> {
        Box::pin(async move {
            let session = self.open(argv).await?;
            let mut stdout = session.take_stdout();
            // The command's own output is read beside the input: an unread answer queue stops the
            // session reading its socket, and the socket is how the input is getting there.
            let drained = tokio::spawn(async move {
                let mut collected = Vec::new();
                if let Some(mut stdout) = stdout.take() {
                    while let Some(chunk) = stdout.recv().await {
                        collected.extend_from_slice(&chunk);
                    }
                }
                collected
            });

            let mut chunks = input.take_chunks();
            let mut write_failure = None;
            while let Some(chunk) = chunks.recv().await {
                if let Err(error) = session.write(&chunk).await {
                    write_failure = Some(error);
                    break;
                }
            }
            session.end_stdin();
            let collected = drained.await.unwrap_or_default();

            // The input's own failure comes first, the way the runner reports it: a command fed half
            // an archive can only complain about the archive.
            if let Some(detail) = input.failure() {
                return Err(ShellError::container_command_failed("the standard input", detail));
            }
            if let Some(error) = write_failure {
                return Err(error);
            }
            let ending = session.wait_ended().await;
            if let Some(error) = ending {
                return Err(error);
            }
            finished(RunOutput {
                stdout: collected,
                stderr: session.stderr(),
                exit_status: session.exit_status(),
                // An upload is the one file command whose input has to be ended by closing the
                // channel: the container's `tar` reads to the end, and the protocol has no other
                // way to say there is no more.
                input_ended_by_close: session.input_ended_by_close(),
            })
        })
    }

    fn stream_bytes<'a>(&'a self, argv: &'a [String]) -> CommandFuture<'a, ByteStream> {
        Box::pin(async move {
            let session = self.open(argv).await?;
            let mut stdout = session
                .take_stdout()
                .ok_or_else(|| ShellError::app_internal("the pod exec session has no standard output"))?;
            let (sender, chunks) = mpsc::channel(POD_STREAM_QUEUE);
            let state = StreamState::new();
            let recorded = state.clone();

            tokio::spawn(async move {
                while let Some(chunk) = stdout.recv().await {
                    if sender.send(chunk).await.is_err() {
                        // Nobody is reading any more: the command should not keep streaming into
                        // this process for a destination that has already closed.
                        session.close();
                        break;
                    }
                }
                let error = session.wait_ended().await;
                let status = session.exit_status().and_then(|code| u32::try_from(code).ok());
                // A zero status means the command's own complaints are warnings, and a stream is
                // only failed by the reason it stopped: the reader is mid-archive and has no room
                // for a note.
                let detail = match (error, status) {
                    (Some(error), _) => error.detail().map(str::to_owned).unwrap_or_else(|| error.to_string()),
                    (None, Some(0)) => String::new(),
                    (None, _) => session.stderr(),
                };
                // Written before the last chunk is handed over, which is what makes the reader's
                // question about the ending answerable.
                recorded.finish(status, detail);
            });

            Ok(ByteStream::from_receiver(chunks, &state))
        })
    }
}

/// How many chunks of a pod's tar stream may wait for the reader.
const POD_STREAM_QUEUE: usize = 8;

/// The answer of a command that is over, as text, or why it was not an answer.
fn finished(output: RunOutput) -> Result<String, ShellError> {
    match output.failure() {
        Some(detail) => Err(ShellError::container_command_failed("the container command", detail)),
        None => Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
    }
}

async fn scp_read_text(connection: &SshSession, path: &str) -> Result<String, ShellError> {
    let command = format!("cat -- {}", archive::shell_quote(path));
    let captured = connection
        .exec_capture(&command, SHELL_TIMEOUT, SHELL_TEXT_LIMIT)
        .await?;
    if captured.truncated {
        return Err(ShellError::scp_operation_failed(
            path,
            "the file is larger than the editor will read over SCP",
        ));
    }
    if captured.exit_code != Some(0) {
        return Err(ShellError::scp_operation_failed(
            path,
            format!("exit {:?}: {}", captured.exit_code, captured.stderr.trim()),
        ));
    }
    Ok(captured.stdout)
}

async fn scp_write_text(connection: &SshSession, path: &str, text: &str) -> Result<(), ShellError> {
    let temporary = std::env::temp_dir().join(format!("7shell-write-{}.tmp", uuid::Uuid::new_v4()));
    tokio::fs::write(&temporary, text.as_bytes())
        .await
        .map_err(|error| ShellError::scp_operation_failed(path, format!("the local temporary file: {error}")))?;

    let parent = path
        .rsplit_once('/')
        .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
        .unwrap_or("/");
    let Some(name) = temporary.file_name().and_then(|name| name.to_str()) else {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(ShellError::scp_operation_failed(
            path,
            "the temporary file has no usable name",
        ));
    };
    let remote_temporary = join_remote(parent, name);

    let outcome = async {
        connection
            .scp_upload(&temporary, parent, &TransferOptions::default(), SCP_TIMEOUT, None)
            .await?;
        scp_exec_ok(
            connection,
            &format!(
                "mv -f -- {} {}",
                archive::shell_quote(&remote_temporary),
                archive::shell_quote(path)
            ),
            path,
        )
        .await
    }
    .await;

    if outcome.is_err() {
        let _ = scp_exec_ok(
            connection,
            &format!("rm -f -- {}", archive::shell_quote(&remote_temporary)),
            path,
        )
        .await;
    }
    let _ = tokio::fs::remove_file(&temporary).await;
    outcome
}

async fn scp_remove(connection: &SshSession, path: &str) -> Result<(), ShellError> {
    scp_exec_ok(connection, &format!("rm -rf -- {}", archive::shell_quote(path)), path).await
}

/// Copies one file over SCP into the exact local name the caller chose.
///
/// The SCP engine writes into a directory and names the file after the remote one, so a staging
/// directory beside the target is what lets the chosen name win while keeping the write on the
/// same file system for the final rename.
async fn scp_download_to(
    connection: &SshSession,
    remote: &str,
    local: &Path,
    progress: Arc<TransferProgress>,
) -> Result<(), ShellError> {
    let parent = local.parent().unwrap_or_else(|| Path::new("."));
    let staging = parent.join(format!(".7shell-download-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&staging)
        .await
        .map_err(|error| ShellError::scp_transfer_failed(remote, format!("the staging directory: {error}")))?;

    let callback_progress = Arc::clone(&progress);
    let mut callback = move |bytes: u64| callback_progress.set_transferred(bytes);
    let options = TransferOptions::default();
    let transfer = connection.scp_download(remote, &staging, &options, SCP_TIMEOUT, Some(&mut callback));

    let outcome = tokio::select! {
        result = transfer => match result {
            Ok(_) if !progress.is_cancelled() => {
                let landed = staging.join(file_name_of(remote));
                tokio::fs::rename(&landed, local)
                    .await
                    .map_err(|error| ShellError::scp_transfer_failed(remote, format!("the download could not be moved into place: {error}")))
            }
            Ok(_) => Ok(()),
            Err(error) => Err(error),
        },
        _ = wait_cancelled(Arc::clone(&progress)) => Ok(()),
    };

    let _ = tokio::fs::remove_dir_all(&staging).await;
    outcome
}

/// Waits until a transfer has been asked to stop, for futures that cannot be paused directly.
async fn wait_cancelled(progress: Arc<TransferProgress>) {
    while !progress.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Copies one FTP file into a local path through a progress-aware writer.
async fn ftp_download(
    ftp: &Mutex<FtpSession>,
    remote: &str,
    local: &Path,
    progress: Arc<TransferProgress>,
) -> Result<(), ShellError> {
    let part = sevenshell_sftp::local_part_of(local);
    let _ = tokio::fs::remove_file(&part).await;

    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&part)
        .await
        .map_err(|error| {
            ShellError::ftp_operation_failed(format!("the local part file could not be written: {error}"))
        })?;
    let mut writer = ProgressWriter::new(file, Arc::clone(&progress));

    let mut session = ftp.lock().await;
    let downloaded = session.download(remote, &mut writer).await;
    if progress.is_cancelled() {
        drop(session);
        drop(writer);
        let _ = tokio::fs::remove_file(&part).await;
        return Ok(());
    }
    if let Err(error) = downloaded {
        drop(session);
        drop(writer);
        let _ = tokio::fs::remove_file(&part).await;
        return Err(error);
    }
    writer.flush().await.map_err(|error| {
        ShellError::ftp_operation_failed(format!("the local part file could not be written: {error}"))
    })?;
    drop(writer);
    drop(session);

    tokio::fs::rename(&part, local).await.map_err(|error| {
        ShellError::ftp_operation_failed(format!("the downloaded file could not be moved into place: {error}"))
    })?;
    Ok(())
}

/// Sends one local file to an FTP folder through a temporary name beside the target.
async fn ftp_upload(
    ftp: &Mutex<FtpSession>,
    local: &Path,
    remote: &str,
    progress: Arc<TransferProgress>,
) -> Result<(), ShellError> {
    let part = transfer_part_of(remote);
    let file = tokio::fs::File::open(local)
        .await
        .map_err(|error| ShellError::ftp_operation_failed(format!("the local file could not be read: {error}")))?;
    let mut reader = ProgressReader::new(file, Arc::clone(&progress));

    let mut session = ftp.lock().await;
    let uploaded = session.upload(&part, &mut reader).await;
    if progress.is_cancelled() {
        drop(session);
        let _ = ftp.lock().await.remove_file(&part).await;
        return Ok(());
    }
    if let Err(error) = uploaded {
        drop(session);
        let _ = ftp.lock().await.remove_file(&part).await;
        return Err(error);
    }

    if session.exists(remote).await?
        && let Err(error) = session.remove_file(remote).await
    {
        let _ = session.remove_file(&part).await;
        return Err(error);
    }
    if let Err(error) = session.rename(&part, remote).await {
        let _ = session.remove_file(&part).await;
        return Err(error);
    }
    Ok(())
}

/// An existing file reader that reports how much it has handed over and waits while paused.
struct ProgressReader<R> {
    inner: R,
    progress: Arc<TransferProgress>,
    read: u64,
    pause: Pin<Box<tokio::time::Sleep>>,
}

impl<R> ProgressReader<R> {
    fn new(inner: R, progress: Arc<TransferProgress>) -> Self {
        Self {
            inner,
            progress,
            read: 0,
            pause: Box::pin(tokio::time::sleep(Duration::from_millis(50))),
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for ProgressReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.progress.is_cancelled() {
            return Poll::Ready(Err(std::io::Error::other("the transfer was cancelled")));
        }
        if self.progress.is_paused() {
            if self.pause.as_mut().poll(context).is_ready() {
                self.pause.as_mut().reset(Instant::now() + Duration::from_millis(50));
                let _ = self.pause.as_mut().poll(context);
            }
            return Poll::Pending;
        }

        let before = buffer.filled().len();
        let outcome = Pin::new(&mut self.inner).poll_read(context, buffer);
        if matches!(outcome, Poll::Ready(Ok(()))) {
            let read = (buffer.filled().len() - before) as u64;
            self.read += read;
            self.progress.set_transferred(self.read);
        }
        outcome
    }
}

/// A local file writer that reports how much it has received and waits while paused.
struct ProgressWriter<W> {
    inner: W,
    progress: Arc<TransferProgress>,
    written: u64,
    pause: Pin<Box<tokio::time::Sleep>>,
}

impl<W> ProgressWriter<W> {
    fn new(inner: W, progress: Arc<TransferProgress>) -> Self {
        Self {
            inner,
            progress,
            written: 0,
            pause: Box::pin(tokio::time::sleep(Duration::from_millis(50))),
        }
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for ProgressWriter<W> {
    fn poll_write(mut self: Pin<&mut Self>, context: &mut Context<'_>, buffer: &[u8]) -> Poll<std::io::Result<usize>> {
        if self.progress.is_cancelled() {
            return Poll::Ready(Err(std::io::Error::other("the transfer was cancelled")));
        }
        if self.progress.is_paused() {
            if self.pause.as_mut().poll(context).is_ready() {
                self.pause.as_mut().reset(Instant::now() + Duration::from_millis(50));
                let _ = self.pause.as_mut().poll(context);
            }
            return Poll::Pending;
        }

        let outcome = Pin::new(&mut self.inner).poll_write(context, buffer);
        if let Poll::Ready(Ok(written)) = outcome {
            self.written += written as u64;
            self.progress.set_transferred(self.written);
        }
        outcome
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(context)
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(context)
    }
}

/// Copies one WebDAV file into a local path, resuming a part file when the server honors a range.
async fn webdav_download(
    dav: &WebDavFiles,
    remote: &str,
    local: &Path,
    progress: &TransferProgress,
) -> Result<(), ShellError> {
    let part = sevenshell_sftp::local_part_of(local);
    // A WebDAV listing has no cheap, stable identity for a file — no inode, no SFTP-style stamp —
    // so a part file cannot be proved to be a prefix of this version. Starting clean is the only
    // answer that cannot join two versions into a file that looks right and is not. Pausing keeps
    // the open response and the part file; only a retry after a failure starts again.
    let _ = tokio::fs::remove_file(&part).await;

    let mut download = dav.open_download(remote, 0).await?;
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&part)
        .await
        .map_err(|error| {
            ShellError::sftp_transfer_failed(remote, format!("the local part file could not be written: {error}"))
        })?;

    progress.set_transferred(0);
    let mut received = 0u64;
    loop {
        if progress.is_cancelled() {
            let _ = tokio::fs::remove_file(&part).await;
            return Ok(());
        }
        while progress.is_paused() {
            if progress.is_cancelled() {
                let _ = tokio::fs::remove_file(&part).await;
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        let Some(chunk) = download.next().await? else {
            break;
        };
        file.write_all(&chunk).await.map_err(|error| {
            ShellError::sftp_transfer_failed(remote, format!("the local part file could not be written: {error}"))
        })?;
        received += chunk.len() as u64;
        progress.set_transferred(received);
    }

    file.flush().await.map_err(|error| {
        ShellError::sftp_transfer_failed(remote, format!("the local part file could not be written: {error}"))
    })?;
    drop(file);

    if progress.is_cancelled() {
        let _ = tokio::fs::remove_file(&part).await;
        return Ok(());
    }
    tokio::fs::rename(&part, local).await.map_err(|error| {
        ShellError::sftp_transfer_failed(
            remote,
            format!("the downloaded file could not be moved into place: {error}"),
        )
    })?;
    Ok(())
}

/// Sends one local file to a WebDAV folder through a temporary name beside the target.
///
/// A PUT that dies half-way leaves whatever the server kept; writing to a part name first means the
/// target only changes when the whole file arrived, the same rule the SFTP upload follows.
async fn webdav_upload(
    dav: &WebDavFiles,
    local: &Path,
    remote: &str,
    progress: &TransferProgress,
) -> Result<(), ShellError> {
    let part = transfer_part_of(remote);
    let mut file = tokio::fs::File::open(local).await.map_err(|error| {
        ShellError::sftp_transfer_failed(remote, format!("the local file could not be read: {error}"))
    })?;
    let (sender, receiver) = tokio::sync::mpsc::channel::<Vec<u8>>(4);

    let produced = async {
        let mut buffer = vec![0u8; 64 * 1024];
        let mut sent = 0u64;
        loop {
            if progress.is_cancelled() {
                break;
            }
            while progress.is_paused() {
                if progress.is_cancelled() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if progress.is_cancelled() {
                break;
            }

            let read = file.read(&mut buffer).await.map_err(|error| {
                ShellError::sftp_transfer_failed(remote, format!("the local file could not be read: {error}"))
            })?;
            if read == 0 {
                break;
            }
            if sender.send(buffer[..read].to_vec()).await.is_err() {
                break;
            }
            sent += read as u64;
            progress.set_transferred(sent);
        }
        Ok::<(), ShellError>(())
    };

    let (uploaded, produced) = tokio::join!(dav.write_stream(&part, receiver, Some(progress.total())), produced);
    if progress.is_cancelled() {
        let _ = dav.remove(&part).await;
        return Ok(());
    }
    produced?;
    if let Err(error) = uploaded {
        let _ = dav.remove(&part).await;
        return Err(error);
    }

    if dav.exists(remote).await?
        && let Err(error) = dav.remove(remote).await
    {
        let _ = dav.remove(&part).await;
        return Err(error);
    }
    if let Err(error) = dav.rename(&part, remote).await {
        let _ = dav.remove(&part).await;
        return Err(error);
    }
    Ok(())
}

/// The temporary name an upload lands under before it is moved into place.
fn transfer_part_of(remote: &str) -> String {
    let (parent, name) = remote.rsplit_once('/').unwrap_or(("", remote));
    let parent = if parent.is_empty() { "/" } else { parent };
    join_remote(parent, &format!(".{name}.7shell-part"))
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
    if directory == "/" {
        format!("/{name}")
    } else if directory.ends_with('/') {
        format!("{directory}{name}")
    } else {
        format!("{directory}/{name}")
    }
}

/// What a failed remote command said, kept to the first line or two.
fn brief_report(outcome: &ExecOutcome) -> String {
    let text = if outcome.stderr.trim().is_empty() {
        outcome.stdout.trim()
    } else {
        outcome.stderr.trim()
    };
    let status = outcome
        .exit_code
        .map_or_else(|| "no exit status".to_owned(), |code| code.to_string());
    let mut report = if text.is_empty() {
        format!("exit {status}")
    } else {
        format!("exit {status}: {text}")
    };
    if report.chars().count() > 400 {
        report = report.chars().take(400).collect::<String>() + "…";
    }
    report
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

    #[test]
    fn a_shell_listing_becomes_the_panel_shape_with_joined_paths() {
        let listing = ShellListing {
            path: "/home/alice".to_owned(),
            entries: vec![sevenshell_ssh::ShellEntry {
                name: "notes.txt".to_owned(),
                kind: EntryKind::File,
                size: 5,
                mode: Some(0o644),
                owner: Some("alice".to_owned()),
                group: Some("users".to_owned()),
                uid: Some(1000),
                gid: Some(100),
                modified_at: Some(1_700_000_000),
                link_target: None,
            }],
            skipped: 0,
            format: sevenshell_ssh::ListingFormat::Find,
        };

        let mapped = scp_listing(listing);

        assert_eq!(mapped.path, "/home/alice");
        assert_eq!(mapped.entries[0].path, "/home/alice/notes.txt");
        assert_eq!(mapped.entries[0].kind, EntryKind::File);
        assert_eq!(mapped.entries[0].size, 5);
        assert_eq!(mapped.entries[0].mode, Some(0o644));
        assert_eq!(mapped.entries[0].modified_at, Some(1_700_000_000));
    }

    #[test]
    fn a_root_listing_joins_a_name_without_a_double_slash() {
        assert_eq!(join_remote("/", "notes.txt"), "/notes.txt");
    }

    // --- A container browser behind the same commands -------------------------

    /// A core with a vault of its own, and nothing else: every test here stops at a refusal the
    /// container backend makes before a process exists, so no runtime has to be installed.
    fn container_core() -> (Arc<Core>, tempfile::TempDir) {
        let directory = tempfile::tempdir().expect("a temporary folder");
        let vault = Arc::new(sevenshell_vault::Vault::open_at(directory.path().join("vault.json")));
        vault.create("the password of the vault").expect("create the vault");
        let (core, _events) = Core::new(vault);
        (core, directory)
    }

    #[tokio::test]
    async fn a_container_browser_for_an_unknown_connection_names_the_connection() {
        let (core, _directory) = container_core();

        let error = core
            .open_container_files("missing", "cid", "/")
            .await
            .expect_err("no connection means nothing to browse");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::ContainerNotConnected);
        assert_eq!(
            error.message_params().get("connection").map(String::as_str),
            Some("missing"),
            "the refusal names the connection that is not open"
        );
        assert!(
            core.files.lock().await.is_empty(),
            "nothing was opened, so nothing is kept"
        );
    }

    #[tokio::test]
    async fn a_container_browser_that_cannot_list_its_first_directory_registers_no_handle() {
        let (core, _directory) = container_core();
        core.containers
            .connect(
                "c1",
                sevenshell_types::ContainerRuntime::Docker,
                "",
                sevenshell_container::Runner::Local,
            )
            .expect("a local connection is a name until something runs");

        // A relative path is refused by the container backend itself before any process exists,
        // which is what says this request reached that backend rather than another one.
        let error = core
            .open_container_files("c1", "cid", "relative/path")
            .await
            .expect_err("a relative path is not a container path");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::ContainerCommandFailed);
        assert!(
            core.files.lock().await.is_empty(),
            "an open that could not list must not leave a handle behind"
        );
    }

    #[tokio::test]
    async fn the_file_commands_reach_the_container_backend_only_through_an_open_handle() {
        let (core, _directory) = container_core();
        let files = Arc::new(
            ContainerFiles::new(
                sevenshell_types::ContainerRuntime::Docker,
                "",
                "cid",
                sevenshell_container::Runner::Local,
            )
            .expect("a container id with no namespace is valid"),
        );
        core.files.lock().await.insert(
            "f7".to_owned(),
            Arc::new(FileSession {
                backend: FileBackend::Container(files),
                session_id: "c1".to_owned(),
            }),
        );

        let error = core
            .list_files("f7", "relative")
            .await
            .expect_err("a relative path never becomes a script");
        assert_eq!(
            error.code(),
            sevenshell_error::ErrorCode::ContainerCommandFailed,
            "the handle must route to the container backend, whose refusal this is"
        );

        let error = core.list_files("f8", "/").await.expect_err("no such handle is open");
        assert_eq!(error.code(), sevenshell_error::ErrorCode::AppInternal);
        assert!(
            error.to_string().contains("f8"),
            "the refusal names the handle that is gone: {error}"
        );

        core.close_files("f7").await.expect("closing a browser is always fine");
        assert_eq!(
            core.list_files("f7", "/").await.expect_err("closed").code(),
            sevenshell_error::ErrorCode::AppInternal,
            "a closed handle is as unknown as one that was never opened"
        );
    }

    // --- A pod browser, before a cluster is involved -------------------------

    fn pod_transport() -> PodTransport {
        PodTransport {
            clusters: Arc::new(K8sManager::new()),
            connection_id: "k1".to_owned(),
            namespace: "sevenshell-lab".to_owned(),
            pod: "lab-shell".to_owned(),
            container: "app".to_owned(),
        }
    }

    #[test]
    fn a_pod_file_command_names_the_pod_and_the_script_it_was_given() {
        let request = pod_transport().request("cd '/' && pwd -P && LC_ALL=C ls -la");
        assert_eq!(request.namespace, "sevenshell-lab");
        assert_eq!(request.pod, "lab-shell");
        assert_eq!(request.container, "app");
        assert_eq!(
            request.command, "cd '/' && pwd -P && LC_ALL=C ls -la",
            "the backend's script travels as it was built, marker and all added below this line"
        );
    }

    #[tokio::test]
    async fn a_pod_browser_refuses_a_relative_path_before_it_reaches_the_cluster() {
        let (core, _directory) = container_core();
        let error = core
            .open_pod_files("missing", "sevenshell-lab", "lab-shell", "", "relative/path")
            .await
            .expect_err("a relative path is not a container path");

        assert_eq!(
            error.code(),
            sevenshell_error::ErrorCode::ContainerCommandFailed,
            "the refusal is the file backend's, made before any channel was opened"
        );
        assert!(
            core.files.lock().await.is_empty(),
            "an open that could not list must not leave a handle behind"
        );
    }

    #[tokio::test]
    async fn a_pod_browser_for_an_unknown_connection_names_the_connection() {
        let (core, _directory) = container_core();
        let error = core
            .open_pod_files("missing", "sevenshell-lab", "lab-shell", "lab-shell", "/")
            .await
            .expect_err("no connection means nothing to reach");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::K8sNotConnected);
        assert!(
            core.files.lock().await.is_empty(),
            "nothing was opened, so nothing is kept"
        );
    }
}
