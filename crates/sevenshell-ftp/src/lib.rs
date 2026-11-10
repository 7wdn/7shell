//! The far side of an FTP or FTPS file browser: one session, and the verbs over its tree.
//!
//! This crate is a protocol client and nothing more. It knows how to reach a server, prove who is
//! asking, list a directory, and move a file one way or the other; it does not know about tabs,
//! transfers or local paths. What it returns is a directory as entries, which the orchestration
//! layer maps onto the same wire shape the SFTP browser uses.
//!
//! The rules it is held to:
//!
//! * **A password reaches the wire and never a log line.** FTP has no header to put it in, so the
//!   protocol sends it as a command; the client that carries it is built without logging
//!   (`suppaftp`'s `no-log`), and no error this crate produces contains it. Errors carry the numeric
//!   server status and this side's description of the failure, never the server's own reply text,
//!   which may quote the command.
//! * **FTPS means TLS on the control and data connections.** A session configured for TLS that
//!   cannot negotiate it fails; it does not quietly fall back to a cleartext login. Certificates are
//!   verified against the platform trust store and the host name must match — there is no
//!   "accept anything" switch in this crate. Plain FTP happens only when the caller asked for it.
//! * **Passive mode only.** Active mode asks the server to connect back to this machine, which a
//!   firewall on either end turns into a hang nobody can explain.
//! * **Paths stay inside the session.** The directory the login landed in is the root of everything
//!   this crate addresses; `..` cannot walk out of it, and a line break in an argument is refused
//!   rather than sent as a second command. The rules live in the module that owns them, next to the
//!   tests that hold them.
//!
//! # What a listing contains
//!
//! `MLSD` is asked for first, because it is the only listing with a defined format. When the server
//! does not implement it, `LIST` is parsed in its Unix and Windows shapes, and a server whose
//! `LIST` is neither is reported as unreadable rather than shown as an empty directory. Directories
//! list in server order. A symlink is an entry, not its target. `modified` is UTC seconds since the
//! Unix epoch, or `None` when the server did not state a time.
//!
//! # What the caller owns
//!
//! Downloads and uploads stream through the caller's reader or writer; nothing here holds a file in
//! memory. A transfer that fails is left for the next command to clean up on the control
//! connection, so the session remains usable; whether a half-written remote file should be removed
//! or a half-written local file resumed is a decision above this crate.
//!
//! # Verification
//!
//! The tests drive a scripted server: login, MLSD/LIST, RETR/STOR, renames and the FTPS ordering are
//! checked without a network. A real FTPS certificate chain and a real server's passive data ports
//! are **not** exercised here — the development machine's Docker registry refuses FTP images.
//! A lab recipe is still required before this can be called live-tested.

mod entry;
mod error;
mod path;

use std::fmt;

pub use entry::FtpEntry;
use sevenshell_error::{OperationOutcome, ShellError};
use sevenshell_vault::ServicePassword;
use suppaftp::FtpError;
use suppaftp::async_native_tls::TlsConnector;
use suppaftp::tokio::{AsyncNativeTlsConnector, AsyncNativeTlsFtpStream};
use suppaftp::types::FileType;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

/// The port an FTP server listens on when nobody says otherwise.
pub const DEFAULT_FTP_PORT: u16 = 21;

/// What is needed to open one session.
///
/// The password is a [`ServicePassword`] rather than a `String` so it is wiped when this is dropped
/// and cannot be printed by accident. `secure` chooses explicit FTPS (`AUTH TLS` on the normal port);
/// there is no separate implicit-FTPS mode and no way to ask for "TLS if available".
pub struct FtpConnectionOptions {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: ServicePassword,
    /// Whether to require TLS before the login is sent.
    pub secure: bool,
}

impl FtpConnectionOptions {
    pub fn new(host: impl Into<String>, username: impl Into<String>, password: ServicePassword) -> Self {
        Self {
            host: host.into(),
            port: DEFAULT_FTP_PORT,
            username: username.into(),
            password,
            secure: false,
        }
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Requires FTPS. A server that cannot negotiate it is an error, never a cleartext login.
    pub fn with_secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }
}

/// The endpoint, the user and the password, but not in a form that reconstructs an address.
///
/// The port is left out with the password: a log line has no use for the pair, and the host alone
/// is what makes a failure recognizable to whoever reads it.
impl fmt::Debug for FtpConnectionOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FtpConnectionOptions")
            .field("host", &self.host)
            .field("username", &self.username)
            .field("secure", &self.secure)
            .field("password", &self.password)
            .finish_non_exhaustive()
    }
}

/// How a directory listing is obtained, in the order it is tried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Listing {
    /// `FEAT` did not say; try `MLSD` and fall back if the server refuses the verb.
    Unknown,
    /// `MLST`/`MLSD` is advertised.
    Machine,
    /// `FEAT` answered without `MLST`: go straight to `LIST`.
    Human,
}

/// One logged-in FTP or FTPS session.
///
/// A session is not `Clone` and does not reconnect. Commands are sequential: FTP has one control
/// connection and one data connection at a time, so concurrent callers must serialize above this.
pub struct FtpSession {
    stream: AsyncNativeTlsFtpStream,
    /// The login directory as a session path. Everything the caller names is resolved under it.
    root: String,
    listing: Listing,
    secure: bool,
}

impl FtpSession {
    /// Connects, secures the control connection if asked, logs in, and learns the session root.
    ///
    /// With `secure`, the order is deliberate: TCP, the greeting, `AUTH TLS`, the TLS handshake,
    /// then — and only then — `USER`/`PASS`. A server that refuses `AUTH TLS` or presents a
    /// certificate that does not verify ends the attempt with the credentials still unsent.
    pub async fn connect(options: FtpConnectionOptions) -> Result<Self, ShellError> {
        refuse_control_characters(&options)?;

        let endpoint = format!("{}:{}", options.host, options.port);
        let tcp = TcpStream::connect(endpoint.as_str())
            .await
            .map_err(|error| connect_error("connecting", error))?;
        let mut stream = AsyncNativeTlsFtpStream::connect_with_stream(tcp)
            .await
            .map_err(|error| error::failure("greeting", &error, OperationOutcome::NotSent))?;

        if options.secure {
            // `TlsConnector::new()` is native-tls's default configuration: the platform verifier
            // and hostname checking are both on, and this crate never builds one with either
            // disabled. A server this machine cannot verify is a failure here, not a prompt.
            stream = stream
                .into_secure(AsyncNativeTlsConnector::from(TlsConnector::new()), &options.host)
                .await
                .map_err(|error| error::failure("FTPS negotiation", &error, OperationOutcome::NotSent))?;
        }

        stream
            .login(options.username.as_str(), options.password.expose())
            .await
            .map_err(|error| error::failure("login", &error, OperationOutcome::Sent))?;

        // Binary, not the protocol's ASCII default: the server must not translate the bytes of a
        // download or an upload. A server that refuses the type would corrupt both.
        stream
            .transfer_type(FileType::Binary)
            .await
            .map_err(|error| error::failure("selecting binary transfer", &error, OperationOutcome::Sent))?;

        let root = match stream.pwd().await {
            Ok(directory) => path::normalize(&directory).unwrap_or_else(|_| "/".to_owned()),
            // A server that will not say where it landed is not a reason to refuse the session;
            // treating the server's own absolute root as the session root is the usual behaviour
            // of such servers.
            Err(_) => "/".to_owned(),
        };

        let listing = match stream.feat().await {
            Ok(features) => {
                let machine = features
                    .keys()
                    .any(|feature| feature.eq_ignore_ascii_case("MLSD") || feature.eq_ignore_ascii_case("MLST"));
                if machine { Listing::Machine } else { Listing::Human }
            }
            // Not every server implements FEAT, and a refusal says nothing about MLSD. Try it and
            // let the verb itself decide.
            Err(_) => Listing::Unknown,
        };

        Ok(Self {
            stream,
            root,
            listing,
            secure: options.secure,
        })
    }

    /// Whether this session's control connection is protected by TLS.
    pub fn is_secure(&self) -> bool {
        self.secure
    }

    /// Entries of one directory.
    ///
    /// `path` is resolved against the session root. The returned entry paths are session paths, so
    /// they can be passed straight back to the other methods.
    pub async fn list(&mut self, path: &str) -> Result<Vec<FtpEntry>, ShellError> {
        let session_path = self.session_path(path)?;
        let absolute = self.absolute(&session_path);

        if self.listing != Listing::Human {
            match self.stream.mlsd(Some(absolute.as_str())).await {
                Ok(lines) => {
                    return entry::from_mlsd(&session_path, &lines).map_err(|_| error::unreadable_listing("MLSD"));
                }
                Err(error) if command_not_understood(&error) => {
                    // The server does not have the verb. Remember that, so the next listing does
                    // not pay for the same refusal twice.
                    self.listing = Listing::Human;
                }
                Err(error) => return Err(error::failure("listing", &error, OperationOutcome::Unknown)),
            }
        }

        let lines = self
            .stream
            .list(Some(absolute.as_str()))
            .await
            .map_err(|error| error::failure("listing", &error, OperationOutcome::Unknown))?;
        entry::from_list(&session_path, &lines).map_err(|_| error::unreadable_listing("LIST"))
    }

    /// Streams a remote file into `out`, returning how many bytes arrived.
    ///
    /// Nothing is buffered beyond the copy's own chunk. The server's completion reply is read
    /// before this returns, so success means the server also considered the transfer complete.
    pub async fn download<W>(&mut self, path: &str, out: &mut W) -> Result<u64, ShellError>
    where
        W: AsyncWrite + Unpin + ?Sized,
    {
        let session_path = self.session_path(path)?;
        let absolute = self.absolute(&session_path);
        let mut transfer = self
            .stream
            .retr_as_stream(absolute)
            .await
            .map_err(|error| error::failure("download", &error, OperationOutcome::Unknown))?;

        let copied = tokio::io::copy(&mut transfer, out)
            .await
            .map_err(|error| interrupted("download", &session_path, &error))?;
        transfer
            .finish()
            .await
            .map_err(|error| error::failure("download", &error, OperationOutcome::Unknown))?;
        Ok(copied)
    }

    /// Streams `input` to a remote file, returning how many bytes were sent.
    ///
    /// The bytes are sent unchanged (binary mode was selected at login). The transfer is only
    /// complete once the server's reply has been read, which happens before this returns.
    pub async fn upload<R>(&mut self, path: &str, input: &mut R) -> Result<u64, ShellError>
    where
        R: AsyncRead + Unpin + ?Sized,
    {
        let session_path = self.session_path(path)?;
        let absolute = self.absolute(&session_path);
        let mut transfer = self
            .stream
            .put_with_stream(absolute)
            .await
            .map_err(|error| error::failure("upload", &error, OperationOutcome::Unknown))?;

        let copied = tokio::io::copy(input, &mut transfer)
            .await
            .map_err(|error| interrupted("upload", &session_path, &error))?;
        transfer
            .finish()
            .await
            .map_err(|error| error::failure("upload", &error, OperationOutcome::Unknown))?;
        Ok(copied)
    }

    /// Creates one directory. Missing parents are not created: FTP has no recursive `MKD`, and
    /// doing it here would hide which level failed.
    pub async fn mkdir(&mut self, path: &str) -> Result<(), ShellError> {
        let absolute = self.absolute(&self.session_path(path)?);
        self.stream
            .mkdir(absolute)
            .await
            .map_err(|error| error::failure("creating a directory", &error, OperationOutcome::Unknown))
    }

    /// Removes one file.
    pub async fn remove_file(&mut self, path: &str) -> Result<(), ShellError> {
        let absolute = self.absolute(&self.session_path(path)?);
        self.stream
            .rm(absolute)
            .await
            .map_err(|error| error::failure("removing a file", &error, OperationOutcome::Unknown))
    }

    /// Removes one empty directory. A non-empty one is refused by the server, and that refusal is
    /// passed on: silently recursing is how a mistyped path deletes a tree.
    pub async fn remove_dir(&mut self, path: &str) -> Result<(), ShellError> {
        let absolute = self.absolute(&self.session_path(path)?);
        self.stream
            .rmdir(absolute)
            .await
            .map_err(|error| error::failure("removing a directory", &error, OperationOutcome::Unknown))
    }

    /// Renames or moves one entry. Both paths are resolved against the session root, so a move
    /// cannot cross out of it.
    pub async fn rename(&mut self, from: &str, to: &str) -> Result<(), ShellError> {
        let from = self.absolute(&self.session_path(from)?);
        let to = self.absolute(&self.session_path(to)?);
        self.stream
            .rename(from, to)
            .await
            .map_err(|error| error::failure("renaming", &error, OperationOutcome::Unknown))
    }

    /// Facts about one entry, or `None` when the server says it is not there.
    ///
    /// `MLST` answers this in one round trip. On a server without it, the parent directory is
    /// listed instead — one extra round trip, but a listing is the only other way that works for
    /// both files and directories. `exists` is this with the facts dropped.
    pub async fn stat(&mut self, path: &str) -> Result<Option<FtpEntry>, ShellError> {
        let session_path = self.session_path(path)?;
        let absolute = self.absolute(&session_path);

        match self.stream.mlst(Some(absolute.as_str())).await {
            Ok(line) => {
                return entry::from_mlst(&session_path, &line)
                    .map(Some)
                    .map_err(|_| error::unreadable_listing("MLST"));
            }
            Err(FtpError::UnexpectedResponse(response)) if response.status.code() == 550 => return Ok(None),
            Err(error) if command_not_understood(&error) => {}
            Err(error) => {
                return Err(error::failure(
                    "reading entry details",
                    &error,
                    OperationOutcome::Unknown,
                ));
            }
        }

        if session_path == "/" {
            return Ok(Some(FtpEntry {
                name: "/".to_owned(),
                path: "/".to_owned(),
                is_dir: true,
                size: 0,
                modified: None,
            }));
        }

        let parent = path::parent(&session_path);
        let name = path::leaf(&session_path).to_owned();
        let entries = self.list(&parent).await?;
        Ok(entries.into_iter().find(|entry| entry.name == name))
    }

    /// Whether a path exists. See [`FtpSession::stat`] for the cost on servers without `MLST`.
    pub async fn exists(&mut self, path: &str) -> Result<bool, ShellError> {
        Ok(self.stat(path).await?.is_some())
    }

    /// Sends `QUIT`. Dropping a session closes the socket without it; this is for a clean goodbye.
    pub async fn quit(mut self) -> Result<(), ShellError> {
        self.stream
            .quit()
            .await
            .map_err(|error| error::failure("quitting", &error, OperationOutcome::Unknown))
    }

    fn session_path(&self, path: &str) -> Result<String, ShellError> {
        path::normalize(path).map_err(error::invalid_path)
    }

    /// The path to put on the wire: the session path below the directory the login landed in.
    fn absolute(&self, session_path: &str) -> String {
        match (self.root.as_str(), session_path) {
            ("/", path) => path.to_owned(),
            (root, "/") => root.to_owned(),
            (root, path) => format!("{root}{path}"),
        }
    }
}

/// A server that does not have a verb usually says 500; the other three are the neighbours that
/// mean the same thing on servers that are strict about which one they pick.
fn command_not_understood(error: &FtpError) -> bool {
    matches!(
        error,
        FtpError::UnexpectedResponse(response)
            if matches!(response.status.code(), 500 | 501 | 502 | 504)
    )
}

fn connect_error(action: &str, error: std::io::Error) -> ShellError {
    ShellError::ftp_unreachable(format!("FTP {action} failed: connection error ({error})"))
        .with_outcome(OperationOutcome::NotSent)
}

fn interrupted(verb: &str, session_path: &str, error: &std::io::Error) -> ShellError {
    ShellError::ftp_operation_failed(format!("FTP {verb} of {session_path} was interrupted: {error}"))
}

/// Refuses credentials that would split a command line before anything is sent.
fn refuse_control_characters(options: &FtpConnectionOptions) -> Result<(), ShellError> {
    for (field, value) in [("host", options.host.as_str()), ("username", options.username.as_str())] {
        if path::carries_control_character(value) {
            return Err(
                ShellError::ftp_invalid_path(format!("FTP {field} contains a line break or NUL"))
                    .with_outcome(OperationOutcome::NotSent),
            );
        }
    }
    if path::carries_control_character(options.password.expose()) {
        return Err(
            ShellError::ftp_invalid_path("FTP password contains a line break or NUL")
                .with_outcome(OperationOutcome::NotSent),
        );
    }
    Ok(())
}
