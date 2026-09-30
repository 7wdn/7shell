//! The connection: verify the key, authenticate, carry the terminal.
//!
//! The two rules this module exists to keep are both about order. The key from the exchange is
//! held against the accepted one inside the same connection that will authenticate, so there
//! is no version of "ask about a key, then connect anyway" that a caller could reach by
//! accident. And nothing is authenticated before that comparison has passed: the credential is
//! only touched after the connection is already known to be talking to the right machine.

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, Config, DisconnectReason, Handle, KeyboardInteractiveAuthResponse, Msg};
use russh::keys::{PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{Channel, ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Disconnect, SshId};
use sevenshell_error::ShellError;

use crate::Purpose;
use crate::route::{BoxedStream, Route};
use sevenshell_types::PRODUCT_NAME;
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

use crate::credential::load_private_key;
use crate::hostkey::{ExpectedKey, plain_key};
use crate::{ChannelEvent, Credential, OpenRequest, Target, TerminalSize};

/// How long `close` waits for the reader to publish its last event before it stops waiting.
///
/// The socket is already gone by then; the grace period is about letting the caller see the
/// terminal's final bytes, not about the connection.
const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// The longest a server's own words are kept in a `Closed` reason. An error is not a place for
/// a data dump.
const MAX_REASON_CHARS: usize = 200;

/// The handshake's error channel.
///
/// russh needs one error type of its own choosing, and this is it: either an envelope that is
/// already the right answer, or the protocol library's own error, which is classified once the
/// phase it came from is known.
#[derive(Debug)]
pub(crate) enum Failure {
    Envelope(ShellError),
    Transport(russh::Error),
}

impl From<russh::Error> for Failure {
    fn from(error: russh::Error) -> Self {
        Self::Transport(error)
    }
}

/// An open connection with a terminal on it.
///
/// Holding this holds the connection: the session task ends when the last handle to it is
/// dropped, so a caller that loses interest in a session should still call [`SshSession::close`]
/// — the server is told, the terminal ends, and the reader task stops.
pub struct SshSession {
    /// Keeps the connection's task running, and is the only way to send a disconnect.
    handle: Handle<KeyCheck>,
    /// Everything the terminal sends. Writing does not need the read half or a lock.
    ///
    /// Absent on a carrier: a connection opened only to carry another one has no terminal, and
    /// pretending otherwise would mean a `write` that goes nowhere.
    channel: Option<ChannelWriteHalf<Msg>>,
    /// The task forwarding the terminal's output. `close` waits for it to finish.
    reader: AsyncMutex<Option<JoinHandle<()>>>,
    /// Set by `close`, so the reader can tell a deliberate end from a lost connection.
    closing: Arc<AtomicBool>,
    host: String,
    port: u16,
    /// The sessions this one runs over, kept alive for exactly as long as it lives.
    ///
    /// A connection opened over a channel dies with the connection carrying it, so dropping these
    /// would quietly kill this session. They are held rather than read.
    pub(crate) upstream: Vec<Arc<SshSession>>,
}

impl SshSession {
    /// Verifies the host key, authenticates, and opens a shell channel with a pty.
    ///
    /// Output arrives on `events`; the channel closes when the sender is dropped or the server
    /// closes it. Exactly one [`ChannelEvent::Closed`] is sent, last, so a caller that waits on
    /// the receiver always learns that the session ended.
    pub async fn open(request: OpenRequest, events: Sender<ChannelEvent>) -> Result<Self, ShellError> {
        let host = request.target.host.clone();
        let port = request.target.port;
        let timeout = request.timeout;

        // One budget for the whole open, not one per step: a server that accepts the socket and
        // then stalls in the key exchange has to hit the same limit as one that never answers.
        match tokio::time::timeout(timeout, open_connection(request, events)).await {
            Ok(result) => result,
            Err(_) => Err(ShellError::ssh_timeout(&host, port, whole_seconds(timeout))),
        }
    }

    /// Bytes for the terminal.
    ///
    /// What one call is given is sent in order; arranging the order *between* calls is the
    /// caller's, because a keystroke and a paste are one call each in the order the person
    /// made them and nothing here can know that order better.
    pub async fn write(&self, data: &[u8]) -> Result<(), ShellError> {
        if data.is_empty() {
            return Ok(());
        }
        self.terminal()?
            .data_bytes(data.to_vec())
            .await
            .map_err(|error| self.channel_failure("the terminal could not be written to", &error))
    }

    /// The terminal channel, or an explanation that this connection is carrying something else.
    fn terminal(&self) -> Result<&ChannelWriteHalf<Msg>, ShellError> {
        self.channel.as_ref().ok_or_else(|| {
            ShellError::app_internal(format!(
                "{}:{} was opened as a carrier, not a terminal",
                self.host, self.port
            ))
        })
    }

    /// Tells the server the terminal changed size, so the programs inside it redraw.
    pub async fn resize(&self, columns: u32, rows: u32) -> Result<(), ShellError> {
        // The pixel fields stay zero: they are for graphics terminals, and a character
        // terminal has no pixels to report.
        self.terminal()?
            .window_change(columns, rows, 0, 0)
            .await
            .map_err(|error| self.channel_failure("the terminal could not be resized", &error))
    }

    /// Ends the terminal and the connection.
    ///
    /// Every step runs even when an earlier one failed: the point is that nothing is left open,
    /// and the first failure is reported because a failure here means the connection was
    /// already unusable, not that something is still running.
    pub async fn close(&self) -> Result<(), ShellError> {
        self.closing.store(true, Ordering::Relaxed);

        let mut outcome = Ok(());
        if let Some(channel) = &self.channel {
            keep_first(
                &mut outcome,
                channel
                    .eof()
                    .await
                    .map_err(|error| self.channel_failure("the terminal could not be ended", &error)),
            );
            keep_first(
                &mut outcome,
                channel
                    .close()
                    .await
                    .map_err(|error| self.channel_failure("the channel could not be closed", &error)),
            );
        }
        keep_first(
            &mut outcome,
            self.handle
                .disconnect(Disconnect::ByApplication, "", "en")
                .await
                .map_err(|error| self.channel_failure("the connection could not be closed", &error)),
        );

        if let Some(reader) = self.reader.lock().await.take() {
            let abort = reader.abort_handle();
            if tokio::time::timeout(CLOSE_GRACE, reader).await.is_err() {
                // The connection is already down, so a reader still waiting on it is waiting
                // for something that will not come.
                abort.abort();
            }
        }
        outcome
    }

    /// Opens a subsystem channel — `sftp`, in practice — and hands it over as a byte stream.
    ///
    /// A subsystem is a program the server runs for the life of the channel, so it goes on a
    /// channel of its own: a file browser and a terminal can share one connection without either
    /// seeing the other's traffic.
    pub async fn open_subsystem_stream(&self, name: &str) -> Result<BoxedStream, ShellError> {
        let channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|error| self.channel_failure(&format!("a channel for {name} could not be opened"), &error))?;

        // A reply is asked for so a server without the subsystem says so, instead of the caller
        // waiting for output that will never come.
        channel
            .request_subsystem(true, name.to_owned())
            .await
            .map_err(|error| self.channel_failure(&format!("the server refused the {name} subsystem"), &error))?;

        Ok(Box::new(channel.into_stream()))
    }

    /// Opens a channel to a host as it is seen from the far side of this connection.
    ///
    /// This is the primitive a jump host and a forward are both built from, so it lives here
    /// rather than in the module that uses it, next to the handle it needs.
    pub(crate) async fn open_direct_tcpip_channel(&self, host: &str, port: u16) -> Result<Channel<Msg>, ShellError> {
        self.handle
            .channel_open_direct_tcpip(host.to_owned(), u32::from(port), "127.0.0.1".to_owned(), 0)
            .await
            .map_err(|error| self.channel_failure(&format!("a channel to {host}:{port} could not be opened"), &error))
    }

    /// The failure shape for everything that happens on an established session.
    fn channel_failure(&self, what: &str, error: &russh::Error) -> ShellError {
        ShellError::ssh_channel_failed(&self.host, self.port, format!("{what}: {error}"))
    }
}

/// Opens the connection and its terminal, in the order the security model requires.
async fn open_connection(request: OpenRequest, events: Sender<ChannelEvent>) -> Result<SshSession, ShellError> {
    let OpenRequest {
        target,
        credential,
        terminal_type,
        keepalive_seconds,
        timeout,
        expected_host_key,
        size,
        route,
        purpose,
    } = request;

    // The hops have to be held by the session this opens: it dies when they do.
    let upstream = route.upstream();

    // The one credential this build has no path for. Naming it here is the point: a caller
    // that was asked for an agent gets an answer, not a connection that fails for reasons of
    // its own.
    let credential = match credential {
        Credential::Agent => return Err(ShellError::app_unsupported("agent authentication")),
        other => other,
    };

    let ending: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let check = KeyCheck {
        expected: ExpectedKey::new(&expected_host_key.key_type, &expected_host_key.public_key),
        host: target.host.clone(),
        port: target.port,
        ending: Arc::clone(&ending),
    };

    // A route through a hop is the same handshake on a different stream: the channel behaves like
    // a socket, so nothing below this point knows the difference.
    let config = client_config(keepalive_seconds);
    let attempt = match &route {
        Route::Direct => client::connect(config, (target.host.as_str(), target.port), check).await,
        Route::Through { hop, host, port } => match hop.open_tunnel_stream(host, *port).await {
            Ok(stream) => client::connect_stream(config, stream, check).await,
            Err(error) => return Err(error),
        },
    };

    let mut handle = match attempt {
        Ok(handle) => handle,
        Err(failure) => return Err(failure_envelope(&target.host, target.port, timeout, failure)),
    };

    authenticate(&mut handle, &target, &credential).await?;
    // The secret has done its work. Nothing past this point is worth a copy of it.
    drop(credential);

    let closing = Arc::new(AtomicBool::new(false));
    let (channel, reader) = match purpose {
        Purpose::Carrier => (None, None),
        Purpose::Terminal => {
            let mut channel = handle
                .channel_open_session()
                .await
                .map_err(|error| channel_failure(&target, "a session channel could not be opened", &error))?;
            request_terminal(&mut channel, &target, &terminal_type, size, &events).await?;

            let (reading, writing) = channel.split();
            let reader = spawn_reader(reading, events, Arc::clone(&ending), Arc::clone(&closing));
            (Some(writing), Some(reader))
        }
    };

    Ok(SshSession {
        handle,
        channel,
        reader: AsyncMutex::new(reader),
        closing,
        host: target.host,
        port: target.port,
        upstream,
    })
}

/// The configuration one connection runs under.
///
/// Keep-alives are what stop a firewall in the path from quietly dropping an idle session, so
/// they go out on the interval the caller named; zero means the caller does not want the
/// connection held open and none are sent. The inactivity timeout stays off: a terminal that
/// has shown nothing for an hour is exactly what this is for.
pub(crate) fn client_config(keepalive_seconds: u16) -> Arc<Config> {
    Arc::new(Config {
        // The banner names the product, so an administrator reading `sshd` logs can see which
        // client connected.
        client_id: SshId::Standard(Cow::Owned(format!(
            "SSH-2.0-{PRODUCT_NAME}_{}",
            env!("CARGO_PKG_VERSION")
        ))),
        nodelay: true,
        inactivity_timeout: None,
        keepalive_interval: (keepalive_seconds > 0).then(|| Duration::from_secs(u64::from(keepalive_seconds))),
        ..Config::default()
    })
}

/// The budget in whole seconds, for the one message that reports it.
///
/// Rounded up and never zero: a caller who allowed a fraction of a second did not allow none,
/// and a message that says "after 0 seconds" reads like a bug.
pub(crate) fn whole_seconds(timeout: Duration) -> u64 {
    let seconds = timeout.as_secs();
    if timeout.subsec_nanos() > 0 {
        seconds.saturating_add(1)
    } else {
        seconds
    }
    .max(1)
}

/// Names why a connection could not be opened, in the vocabulary the caller reads.
pub(crate) fn failure_envelope(host: &str, port: u16, timeout: Duration, failure: Failure) -> ShellError {
    match failure {
        // Already an answer: the key did not match, or the server offered something this
        // build does not carry.
        Failure::Envelope(envelope) => envelope,
        Failure::Transport(error) => match error {
            // A name that does not resolve, a port nobody listens on, a network with no route:
            // the same sentence to the person reading it.
            russh::Error::IO(source) => ShellError::ssh_unreachable(host, port, source.to_string()),
            // The library has its own clocks for steps this crate does not time itself.
            russh::Error::ConnectionTimeout | russh::Error::KeepaliveTimeout | russh::Error::InactivityTimeout => {
                ShellError::ssh_timeout(host, port, whole_seconds(timeout))
            }
            // Only reachable if a key was refused without saying which one; the connection was
            // still refused over the key it presented, and nothing was sent.
            russh::Error::UnknownKey => ShellError::ssh_host_key_mismatch(host, port, "unknown"),
            // The host was reached but no session came of it — a closed door, a server that
            // does not speak SSH, or no algorithm both ends know. "Unreachable" is the code for
            // that, and it keeps the honest statement that no credential was offered.
            other => ShellError::ssh_unreachable(host, port, other.to_string()),
        },
    }
}

/// The failure shape for everything that happens on an established session.
fn channel_failure(target: &Target, what: &str, error: &russh::Error) -> ShellError {
    ShellError::ssh_channel_failed(&target.host, target.port, format!("{what}: {error}"))
}

/// Keeps the first failure of a teardown and carries on with the rest of it.
fn keep_first(outcome: &mut Result<(), ShellError>, step: Result<(), ShellError>) {
    if let (true, Err(error)) = (outcome.is_ok(), step) {
        *outcome = Err(error);
    }
}

/// Authenticates with the credential the caller supplied.
async fn authenticate(
    handle: &mut Handle<KeyCheck>,
    target: &Target,
    credential: &Credential,
) -> Result<(), ShellError> {
    if let Some(password) = credential.password() {
        return password_authentication(handle, target, password).await;
    }
    if let Some((contents, passphrase)) = credential.key() {
        return key_authentication(handle, target, contents, passphrase).await;
    }
    Err(ShellError::app_unsupported("agent authentication"))
}

/// Authenticates with a password, and answers a keyboard-interactive challenge with it.
///
/// Many servers do not accept a password method at all: they reject it and immediately ask a
/// question instead, which is the same password being requested by another name. OpenSSH's own
/// client follows the same path.
async fn password_authentication(
    handle: &mut Handle<KeyCheck>,
    target: &Target,
    password: &str,
) -> Result<(), ShellError> {
    // The library keeps its own copy for the exchange; the copy this crate owns — the
    // caller's credential — is the one it can promise to wipe.
    let result = handle
        .authenticate_password(target.user.clone(), password.to_owned())
        .await
        .map_err(|error| auth_failed(target, "password", Some(error.to_string())))?;
    if result.success() {
        return Ok(());
    }

    match keyboard_interactive(handle, target, password).await? {
        Challenge::Answered => Ok(()),
        // The server never asked anything, so the refusal to report is the password's.
        Challenge::NotOffered => Err(auth_failed(target, "password", rejection(&result))),
    }
}

/// Authenticates with a private key, choosing the signature hash the server will take.
async fn key_authentication(
    handle: &mut Handle<KeyCheck>,
    target: &Target,
    contents: &str,
    passphrase: Option<&str>,
) -> Result<(), ShellError> {
    // Reading the key can fail before anything is sent, and that failure belongs to the
    // caller: a wrong passphrase is not a rejected authentication, and saying so would claim
    // something left the machine that never did.
    let key = Arc::new(load_private_key(contents, passphrase)?);

    // Only RSA keys carry a choice of signature hash, and only the server knows which of the
    // choices it accepts. A key of any other kind ignores it.
    let hash = if key.algorithm().is_rsa() {
        handle
            .best_supported_rsa_hash()
            .await
            .map_err(|error| auth_failed(target, "publickey", Some(error.to_string())))?
            .flatten()
    } else {
        None
    };

    let result = handle
        .authenticate_publickey(target.user.clone(), PrivateKeyWithHashAlg::new(key, hash))
        .await
        .map_err(|error| auth_failed(target, "publickey", Some(error.to_string())))?;
    if result.success() {
        return Ok(());
    }
    Err(auth_failed(target, "publickey", rejection(&result)))
}

/// What came of asking for a keyboard-interactive challenge.
enum Challenge {
    /// The server accepted the answers.
    Answered,
    /// The server refused the method without asking anything, so there is nothing to report
    /// against it.
    NotOffered,
}

/// Answers a keyboard-interactive challenge with the password.
async fn keyboard_interactive(
    handle: &mut Handle<KeyCheck>,
    target: &Target,
    password: &str,
) -> Result<Challenge, ShellError> {
    let mut response = handle
        .authenticate_keyboard_interactive_start(target.user.clone(), None::<String>)
        .await
        .map_err(|error| auth_failed(target, "keyboard-interactive", Some(error.to_string())))?;

    let mut asked = false;
    loop {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(Challenge::Answered),
            KeyboardInteractiveAuthResponse::Failure { .. } if !asked => return Ok(Challenge::NotOffered),
            KeyboardInteractiveAuthResponse::Failure { .. } => {
                return Err(auth_failed(
                    target,
                    "keyboard-interactive",
                    Some("the server rejected the answers".to_owned()),
                ));
            }
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                let answers: Vec<String> = prompts
                    .iter()
                    .map(|prompt| {
                        // A prompt the server wants echoed back is usually a second factor or a
                        // one-time code. The account password is not an answer to those, and
                        // handing it over because the server asked would be the attack working.
                        if prompt.echo {
                            String::new()
                        } else {
                            password.to_owned()
                        }
                    })
                    .collect();
                asked = true;
                response = handle
                    .authenticate_keyboard_interactive_respond(answers)
                    .await
                    .map_err(|error| auth_failed(target, "keyboard-interactive", Some(error.to_string())))?;
            }
        }
    }
}

/// Authentication did not complete.
///
/// The outcome is `sent`, and that is the truthful one: a credential that reached the
/// authentication exchange has left this machine whether or not the server answered. "Did my
/// password leave?" is the first question after a failure, and the answer must not depend on
/// reading this code.
fn auth_failed(target: &Target, method: &str, reason: Option<String>) -> ShellError {
    let error = ShellError::ssh_auth_failed(&target.host, target.port, method);
    match reason {
        Some(reason) => error.with_diagnostic("reason", reason),
        None => error,
    }
}

/// What a rejected authentication says about the methods the server would still take.
///
/// It is the difference between "the server did not like these credentials" and "the server
/// wanted a key instead", and the second is something a person can act on.
fn rejection(result: &russh::client::AuthResult) -> Option<String> {
    match result {
        russh::client::AuthResult::Failure {
            remaining_methods,
            partial_success,
        } => {
            // A server that took the credential and asked for another one wants a second
            // factor, and the one secret the caller had has already been spent.
            if *partial_success {
                return Some("the server accepted the credential and asked for another".to_owned());
            }
            let methods: Vec<&str> = remaining_methods.iter().map(Into::into).collect();
            if methods.is_empty() {
                Some("the server rejected these credentials".to_owned())
            } else {
                Some(format!(
                    "the server rejected these credentials and would accept {}",
                    methods.join(", ")
                ))
            }
        }
        russh::client::AuthResult::Success => None,
    }
}

/// Opens the shell: a pty of the caller's size first, then the shell itself.
///
/// Both requests want a reply. Without one, a server that refuses a pty or has no shell for
/// this account leaves the caller with a channel that produces nothing and never says why; with
/// one, the refusal arrives as a channel failure while there is still a caller to tell.
async fn request_terminal(
    channel: &mut Channel<Msg>,
    target: &Target,
    terminal_type: &str,
    size: TerminalSize,
    events: &Sender<ChannelEvent>,
) -> Result<(), ShellError> {
    channel
        .request_pty(true, terminal_type, size.columns, size.rows, 0, 0, &[])
        .await
        .map_err(|error| channel_failure(target, "the pseudo-terminal could not be requested", &error))?;
    await_reply(channel, target, events, "the pseudo-terminal").await?;

    channel
        .request_shell(true)
        .await
        .map_err(|error| channel_failure(target, "the shell could not be started", &error))?;
    await_reply(channel, target, events, "the shell").await
}

/// Waits for the server's answer to a request that asked for one.
///
/// Anything the terminal produces while waiting is forwarded rather than dropped, so a server
/// that speaks before it answers does not lose its words.
async fn await_reply(
    channel: &mut Channel<Msg>,
    target: &Target,
    events: &Sender<ChannelEvent>,
    what: &str,
) -> Result<(), ShellError> {
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Success) => return Ok(()),
            Some(ChannelMsg::Failure) => {
                return Err(ShellError::ssh_channel_failed(
                    &target.host,
                    target.port,
                    format!("the server refused {what}"),
                ));
            }
            Some(ChannelMsg::Eof | ChannelMsg::Close) | None => {
                return Err(ShellError::ssh_channel_failed(
                    &target.host,
                    target.port,
                    format!("the channel ended before the server answered for {what}"),
                ));
            }
            Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                if events.send(ChannelEvent::Data(data.to_vec())).await.is_err() {
                    return Err(ShellError::ssh_channel_failed(
                        &target.host,
                        target.port,
                        format!("nobody is listening for {what}"),
                    ));
                }
            }
            // A window adjustment, an exit status, another request's answer: not this answer.
            Some(_) => {}
        }
    }
}

/// Forwards everything the terminal produces until the channel ends.
///
/// The sender is the caller's signal that the session is over: it is dropped after the one
/// `Closed` event, so waiting on the receiver always terminates.
fn spawn_reader(
    mut reading: ChannelReadHalf,
    events: Sender<ChannelEvent>,
    ending: Arc<Mutex<Option<String>>>,
    closing: Arc<AtomicBool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match reading.wait().await {
                // A terminal is one screen: output the server sends as extended data — a
                // stderr the pty did not merge — belongs in the same stream.
                Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                    if events.send(ChannelEvent::Data(data.to_vec())).await.is_err() {
                        // Nobody is listening any more. Stop reading rather than buffer a
                        // terminal nobody asked to see.
                        return;
                    }
                }
                Some(ChannelMsg::Close) => {
                    // The channel ended the way a channel ends. A deliberate close is not a
                    // reason to explain, and neither is one the server did not describe.
                    let asked_for = closing.load(Ordering::Relaxed);
                    let reason = recorded(&ending).filter(|_| !asked_for);
                    let _ = events.send(ChannelEvent::Closed { reason }).await;
                    return;
                }
                None => {
                    let reason = if closing.load(Ordering::Relaxed) {
                        // `close` was called: the caller already knows this end was coming.
                        None
                    } else {
                        // A connection that went away without closing its channel. Silence
                        // here would be the one outcome a caller cannot act on.
                        Some(
                            recorded(&ending)
                                .unwrap_or_else(|| "the connection ended without closing the channel".to_owned()),
                        )
                    };
                    let _ = events.send(ChannelEvent::Closed { reason }).await;
                    return;
                }
                Some(_) => {}
            }
        }
    })
}

/// The reason the handler recorded for the end of the connection, if it learned one.
fn recorded(ending: &Arc<Mutex<Option<String>>>) -> Option<String> {
    ending.lock().ok().and_then(|slot| slot.clone())
}

/// A server's own words, made safe to show.
///
/// A disconnect message is text from the far end. It must not be able to carry control
/// characters into a terminal that is drawing, and it is not a place for a data dump.
fn readable_reason(text: &str) -> Option<String> {
    let cleaned: String = text
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_REASON_CHARS)
        .collect();
    let trimmed = cleaned.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// Decides whether the connection may continue, and remembers why it ended.
struct KeyCheck {
    expected: ExpectedKey,
    host: String,
    port: u16,
    ending: Arc<Mutex<Option<String>>>,
}

impl client::Handler for KeyCheck {
    type Error = Failure;

    /// The decision that has to be right, taken before a single credential is offered.
    ///
    /// A key that is not the accepted one ends the connection here: the handshake is abandoned
    /// at the key exchange, nothing is authenticated, and the caller is told which key the
    /// server presented rather than what it presented earlier.
    async fn check_server_key(&mut self, server_key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let Some(key) = plain_key(server_key) else {
            return Err(Failure::Envelope(ShellError::app_unsupported("host certificates")));
        };
        if self.expected.accepts(key) {
            return Ok(true);
        }
        Err(Failure::Envelope(ShellError::ssh_host_key_mismatch(
            &self.host,
            self.port,
            key.algorithm().as_str(),
        )))
    }

    /// The session is over. Keeping the reason is what lets the reader task end the caller's
    /// stream with an explanation instead of a silence.
    async fn disconnected(&mut self, reason: DisconnectReason<Self::Error>) -> Result<(), Self::Error> {
        let text = match reason {
            DisconnectReason::ReceivedDisconnect(info) => info.message,
            DisconnectReason::Error(Failure::Envelope(envelope)) => envelope.to_string(),
            DisconnectReason::Error(Failure::Transport(error)) => error.to_string(),
        };
        if let Ok(mut slot) = self.ending.lock() {
            *slot = readable_reason(&text);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sevenshell_error::{ErrorCode, OperationOutcome};
    use sevenshell_types::PresentedHostKey;

    /// A request for a server that is not there, which is what the tests below need: they are
    /// about what happens before any server says anything.
    fn a_request(host: &str, port: u16, credential: Credential) -> OpenRequest {
        OpenRequest {
            target: Target {
                host: host.to_owned(),
                port,
                user: "person".to_owned(),
            },
            credential,
            terminal_type: "xterm-256color".to_owned(),
            keepalive_seconds: 0,
            timeout: Duration::from_secs(5),
            expected_host_key: PresentedHostKey {
                host: host.to_owned(),
                port,
                key_type: "ssh-ed25519".to_owned(),
                public_key: "AAAA".to_owned(),
                fingerprint: "SHA256:AAAA".to_owned(),
            },
            size: TerminalSize::default(),
            route: Route::Direct,
            purpose: Purpose::Terminal,
        }
    }

    #[test]
    fn a_budget_in_whole_seconds_is_rounded_up_and_never_zero() {
        assert_eq!(whole_seconds(Duration::from_secs(30)), 30);
        assert_eq!(whole_seconds(Duration::from_millis(1500)), 2);
        assert_eq!(whole_seconds(Duration::from_millis(700)), 1);
        assert_eq!(whole_seconds(Duration::ZERO), 1);
    }

    #[test]
    fn a_session_can_be_shared_between_tasks() {
        // The registry that owns sessions holds them behind an `Arc` and hands them to
        // whichever task is carrying a keystroke or a resize. This does not compile if that
        // stops being true, which is the point: the failure should be here, not in the crate
        // above.
        fn assert_send_and_sync<T: Send + Sync>() {}
        assert_send_and_sync::<SshSession>();
    }

    #[test]
    fn a_named_host_key_is_a_host_key_failure_and_nothing_was_sent() {
        let failure = Failure::Envelope(ShellError::ssh_host_key_mismatch("host.example", 22, "ssh-ed25519"));
        let error = failure_envelope("host.example", 22, Duration::from_secs(10), failure);
        assert_eq!(error.code(), ErrorCode::SshHostKeyMismatch);
        assert_eq!(error.outcome(), OperationOutcome::NotSent);
    }

    #[test]
    fn a_refused_socket_is_unreachable() {
        let failure = Failure::Transport(russh::Error::IO(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "connection refused",
        )));
        let error = failure_envelope("127.0.0.1", 9, Duration::from_secs(10), failure);
        assert_eq!(error.code(), ErrorCode::SshUnreachable);
        assert_eq!(error.outcome(), OperationOutcome::NotSent);
        assert!(error.detail().unwrap_or_default().contains("connection refused"));
    }

    #[test]
    fn a_library_timeout_is_reported_with_the_budget_in_it() {
        let failure = Failure::Transport(russh::Error::ConnectionTimeout);
        let error = failure_envelope("host.example", 22, Duration::from_secs(7), failure);
        assert_eq!(error.code(), ErrorCode::SshTimeout);
        assert_eq!(error.message_params().get("seconds").map(String::as_str), Some("7"));
    }

    #[test]
    fn an_auth_failure_says_that_something_was_sent() {
        let error = auth_failed(
            &Target {
                host: "host.example".to_owned(),
                port: 22,
                user: "person".to_owned(),
            },
            "password",
            Some("the server rejected these credentials".to_owned()),
        );
        assert_eq!(error.code(), ErrorCode::SshAuthFailed);
        assert_eq!(error.outcome(), OperationOutcome::Sent);
        assert_eq!(error.diagnostics().get("method").map(String::as_str), Some("password"));
    }

    #[test]
    fn a_keepalive_interval_of_zero_turns_keepalives_off() {
        assert!(client_config(0).keepalive_interval.is_none());
        assert_eq!(client_config(45).keepalive_interval, Some(Duration::from_secs(45)));
    }

    #[test]
    fn the_client_banner_names_the_product() {
        let config = client_config(0);
        let SshId::Standard(banner) = &config.client_id else {
            panic!("the client id should be a standard banner");
        };
        assert!(banner.starts_with("SSH-2.0-7Shell_"));
        assert!(!banner.contains(' '));
    }

    #[test]
    fn a_server_message_is_made_safe_to_show() {
        assert_eq!(readable_reason("bye"), Some("bye".to_owned()));
        assert_eq!(readable_reason("\u{1b}[2J\u{1b}[Hbye"), Some("[2J[Hbye".to_owned()));
        assert_eq!(readable_reason("   "), None);
        assert_eq!(readable_reason(""), None);
        let long = "x".repeat(MAX_REASON_CHARS + 50);
        assert_eq!(readable_reason(&long).map(|text| text.len()), Some(MAX_REASON_CHARS));
    }

    #[tokio::test]
    async fn an_agent_credential_is_answered_rather_than_attempted() {
        let (events, _output) = tokio::sync::mpsc::channel(4);
        match SshSession::open(a_request("127.0.0.1", 9, Credential::Agent), events).await {
            Ok(_) => panic!("no agent path exists in this build"),
            Err(error) => {
                assert_eq!(error.code(), ErrorCode::AppUnsupported);
                assert_eq!(error.outcome(), OperationOutcome::NotSent);
            }
        }
    }

    #[tokio::test]
    async fn a_port_with_nothing_listening_never_sees_the_password() {
        // Bind a port, learn its number, then let it go: nothing is listening by the time the
        // connection arrives.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a free port");
        let port = listener.local_addr().expect("the bound address").port();
        drop(listener);

        let (events, _output) = tokio::sync::mpsc::channel(4);
        let request = a_request("127.0.0.1", port, Credential::Password("hunter2".to_owned()));
        match SshSession::open(request, events).await {
            Ok(_) => panic!("nothing is listening"),
            Err(error) => {
                assert_eq!(error.code(), ErrorCode::SshUnreachable);
                assert_eq!(error.outcome(), OperationOutcome::NotSent);
                // A failure that quotes the credential would put it in a bug report.
                assert!(!error.to_string().contains("hunter2"));
            }
        }
    }
}

/// The tests that need a real server. They are ignored by default, so a normal `cargo test`
/// run needs nothing but a machine, and they take everything they touch from the environment:
///
/// ```text
/// SEVENSHELL_TEST_HOST  SEVENSHELL_TEST_PORT  SEVENSHELL_TEST_USER  SEVENSHELL_TEST_PASSWORD
/// ```
///
/// They are written for a password account. Run them against a server you are willing to open
/// a shell on with `cargo test -p sevenshell-ssh -- --ignored`; without the variables they say
/// so and do nothing, rather than failing on a missing name.
#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::probe_host_key;
    use sevenshell_error::{ErrorCode, OperationOutcome};
    use sevenshell_types::PresentedHostKey;
    use tokio::sync::mpsc;

    /// The server the environment points at.
    struct Lab {
        host: String,
        port: u16,
        user: String,
        password: String,
    }

    fn lab() -> Option<Lab> {
        let host = std::env::var("SEVENSHELL_TEST_HOST").ok()?;
        let port = std::env::var("SEVENSHELL_TEST_PORT").ok()?.parse().ok()?;
        let user = std::env::var("SEVENSHELL_TEST_USER").ok()?;
        let password = std::env::var("SEVENSHELL_TEST_PASSWORD").ok()?;
        Some(Lab {
            host,
            port,
            user,
            password,
        })
    }

    /// Announced once, so a run that does nothing says why.
    fn no_lab() {
        eprintln!(
            "no lab: set SEVENSHELL_TEST_HOST, SEVENSHELL_TEST_PORT, SEVENSHELL_TEST_USER and \
             SEVENSHELL_TEST_PASSWORD to run this test"
        );
    }

    fn request(lab: &Lab, expected_host_key: PresentedHostKey, credential: Credential) -> OpenRequest {
        OpenRequest {
            target: Target {
                host: lab.host.clone(),
                port: lab.port,
                user: lab.user.clone(),
            },
            credential,
            terminal_type: "xterm-256color".to_owned(),
            keepalive_seconds: 15,
            timeout: Duration::from_secs(20),
            expected_host_key,
            size: TerminalSize::default(),
            route: Route::Direct,
            purpose: Purpose::Terminal,
        }
    }

    /// Everything the terminal says until it says the needle, or the wait runs out.
    async fn output_until(events: &mut mpsc::Receiver<ChannelEvent>, needle: &str) -> String {
        let mut seen = String::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(left, events.recv()).await {
                Ok(Some(ChannelEvent::Data(data))) => {
                    seen.push_str(&String::from_utf8_lossy(&data));
                    if seen.contains(needle) {
                        return seen;
                    }
                }
                Ok(Some(ChannelEvent::Closed { reason })) => panic!("the session closed early: {reason:?}"),
                Ok(None) => panic!("the session ended without producing {needle}: {seen}"),
                Err(_) => panic!("the terminal never produced {needle}: {seen}"),
            }
        }
    }

    #[tokio::test]
    #[ignore = "needs a live server; see this module's comment"]
    async fn a_live_server_is_verified_and_then_hands_over_a_shell() {
        let Some(lab) = lab() else {
            no_lab();
            return;
        };

        let key = probe_host_key(&lab.host, lab.port, Duration::from_secs(10))
            .await
            .expect("the server's key");
        assert!(key.fingerprint.starts_with("SHA256:"));
        assert!(!key.key_type.is_empty());
        assert!(!key.public_key.is_empty());

        let (events, mut output) = mpsc::channel(64);
        let session = SshSession::open(request(&lab, key, Credential::Password(lab.password.clone())), events)
            .await
            .expect("an authenticated session");

        let marker = "sevenshell-live-test";
        session
            .write(format!("echo {marker}\n").as_bytes())
            .await
            .expect("a write to the terminal");
        assert!(output_until(&mut output, marker).await.contains(marker));

        session.resize(100, 40).await.expect("a resize");
        session.close().await.expect("a clean close");

        // A closed session always ends with the one event a caller can wait for.
        let mut closed = false;
        while let Some(event) = output.recv().await {
            if let ChannelEvent::Closed { .. } = event {
                closed = true;
            }
        }
        assert!(closed, "the caller was never told the channel closed");
    }

    #[tokio::test]
    #[ignore = "needs a live server; see this module's comment"]
    async fn a_live_server_with_a_key_that_changed_is_refused_before_authentication() {
        let Some(lab) = lab() else {
            no_lab();
            return;
        };

        let key = probe_host_key(&lab.host, lab.port, Duration::from_secs(10))
            .await
            .expect("the server's key");

        // The same key type with different material: what a machine that swapped its key
        // looks like to the connection that remembers the old one.
        let mut changed = key.clone();
        changed.public_key = match key.public_key.strip_prefix('A') {
            Some(rest) => format!("B{rest}"),
            None => format!("A{}", key.public_key),
        };

        let (events, _output) = mpsc::channel(4);
        match SshSession::open(
            request(&lab, changed, Credential::Password(lab.password.clone())),
            events,
        )
        .await
        {
            Ok(_) => panic!("a changed key must never be authenticated against"),
            Err(error) => {
                assert_eq!(error.code(), ErrorCode::SshHostKeyMismatch);
                // The password was never offered, and the envelope has to say so.
                assert_eq!(error.outcome(), OperationOutcome::NotSent);
            }
        }
    }
}
