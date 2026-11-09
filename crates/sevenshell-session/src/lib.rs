//! Live sessions: the registry of terminals that are open right now.
//!
//! The registry owns the connection handles and the task that forwards their output; the host
//! owns the receiver and decides what to do with the bytes (the desktop shell turns them into
//! window events). This crate keeps no credentials: whatever a session needed to authenticate was
//! handed to `sevenshell-ssh` and dropped there.
//!
//! A handle may name either an SSH channel or one of the other transports (Telnet, serial, a local
//! shell). Both produce the same two events, and only the SSH kind can open further channels —
//! SFTP, the monitor and port forwarding ask for that and are told, in one sentence, why a serial
//! port cannot answer.

mod recording;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use sevenshell_error::ShellError;
use sevenshell_ssh::{OpenRequest, SshSession};
use sevenshell_term::StreamSession;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;

pub use recording::{Recording, RecordingSpec};

/// The events a terminal produces.
///
/// The same shape an SSH channel and every other transport produce, so the host matches once.
#[derive(Debug)]
pub enum TerminalEvent {
    /// Bytes the terminal produced, in the order the device sent them.
    Data(Vec<u8>),
    /// The last event on the terminal. `reason` explains an end nobody asked for.
    Closed { reason: Option<String> },
}

/// One event from one session, tagged with where it came from.
#[derive(Debug)]
pub struct SessionEvent {
    pub handle: String,
    pub event: TerminalEvent,
}

/// What is behind a handle.
enum LiveSession {
    Ssh(Arc<SshSession>),
    Stream(Arc<StreamSession>),
}

/// A terminal that is open, and the task reading it.
struct OpenSession {
    session_id: String,
    session: Arc<LiveSession>,
    forwarder: JoinHandle<()>,
}

/// Every open terminal, addressed by a handle the host hands back on every call.
///
/// A handle is never reused: it comes from a counter rather than from a slot, so a stale handle
/// fails loudly instead of reaching a different machine than the caller meant.
pub struct SessionRegistry {
    sessions: Mutex<HashMap<String, OpenSession>>,
    events: mpsc::Sender<SessionEvent>,
    next_handle: AtomicU64,
}

impl SessionRegistry {
    /// Starts a registry and returns it with the receiver its output arrives on. One host drains it.
    pub fn start() -> (Arc<Self>, mpsc::Receiver<SessionEvent>) {
        let (events, receiver) = mpsc::channel(512);
        let registry = Arc::new(Self {
            sessions: Mutex::new(HashMap::new()),
            events,
            next_handle: AtomicU64::new(1),
        });
        (registry, receiver)
    }

    /// Opens a terminal for a session whose host key has been verified and whose credentials are ready.
    ///
    /// Returns the handle. If the connection fails, no handle is registered and the error is the
    /// one the transport produced.
    /// Opens a terminal, recording it when the caller asked for that.
    ///
    /// A recording that cannot be opened stops the session from opening at all, which is the one
    /// moment it can be said out loud: a file created once the terminal is already on screen would
    /// leave somebody believing they were recording.
    pub async fn open(
        self: &Arc<Self>,
        session_id: &str,
        request: OpenRequest,
        recording: Option<RecordingSpec>,
    ) -> Result<String, ShellError> {
        let handle = format!("t{}", self.next_handle.fetch_add(1, Ordering::Relaxed));

        let recording = match &recording {
            Some(spec) => Some(Recording::create(spec).await?),
            None => None,
        };

        let (sender, receiver) = mpsc::channel(256);
        let session = SshSession::open(request, sender).await?;

        let forwarder = self.forward_ssh(&handle, receiver, recording);
        self.sessions.lock().await.insert(
            handle.clone(),
            OpenSession {
                session_id: session_id.to_owned(),
                session: Arc::new(LiveSession::Ssh(Arc::new(session))),
                forwarder,
            },
        );

        Ok(handle)
    }

    /// Registers a terminal whose transport is not SSH, recording it when the caller asked for that.
    ///
    /// The transport is already open: its dial (or process start) happened before this call, so a
    /// failure here can only be the recording's.
    pub async fn open_stream(
        self: &Arc<Self>,
        session_id: &str,
        session: StreamSession,
        receiver: mpsc::Receiver<sevenshell_term::TerminalEvent>,
        recording: Option<RecordingSpec>,
    ) -> Result<String, ShellError> {
        let handle = format!("t{}", self.next_handle.fetch_add(1, Ordering::Relaxed));

        let recording = match &recording {
            Some(spec) => Some(Recording::create(spec).await?),
            None => None,
        };

        let forwarder = self.forward_stream(&handle, receiver, recording);
        self.sessions.lock().await.insert(
            handle.clone(),
            OpenSession {
                session_id: session_id.to_owned(),
                session: Arc::new(LiveSession::Stream(Arc::new(session))),
                forwarder,
            },
        );

        Ok(handle)
    }

    /// Sends keystrokes or pasted text to a terminal.
    pub async fn write(&self, handle: &str, data: &[u8]) -> Result<(), ShellError> {
        match self.session(handle).await?.as_ref() {
            LiveSession::Ssh(session) => session.write(data).await,
            LiveSession::Stream(session) => session.write(data).await,
        }
    }

    /// Tells the remote side how big the terminal is now.
    pub async fn resize(&self, handle: &str, columns: u32, rows: u32) -> Result<(), ShellError> {
        match self.session(handle).await?.as_ref() {
            LiveSession::Ssh(session) => session.resize(columns, rows).await,
            LiveSession::Stream(session) => session.resize(columns, rows).await,
        }
    }

    /// Closes one terminal and forgets it.
    pub async fn close(&self, handle: &str) -> Result<(), ShellError> {
        let Some(open) = self.sessions.lock().await.remove(handle) else {
            // Closing something that is already gone is what a second click means.
            return Ok(());
        };

        let result = match open.session.as_ref() {
            LiveSession::Ssh(session) => session.close().await,
            LiveSession::Stream(session) => session.close().await,
        };
        open.forwarder.abort();
        result
    }

    /// Closes every terminal, used when the interface goes away.
    pub async fn close_all(&self) {
        let handles: Vec<String> = self.sessions.lock().await.keys().cloned().collect();
        for handle in handles {
            let _ = self.close(&handle).await;
        }
    }

    /// The terminals that are open.
    pub async fn open_sessions(&self) -> Vec<OpenTerminal> {
        let sessions = self.sessions.lock().await;
        let mut open: Vec<OpenTerminal> = sessions
            .iter()
            .map(|(handle, session)| OpenTerminal {
                handle: handle.clone(),
                session_id: session.session_id.clone(),
            })
            .collect();
        open.sort_by(|left, right| left.handle.cmp(&right.handle));
        open
    }

    /// True when no terminal is open.
    pub async fn is_empty(&self) -> bool {
        self.sessions.lock().await.is_empty()
    }

    /// The connection behind a handle, for a caller that needs to open channels on it.
    ///
    /// Only SSH has channels. A caller that reached a local shell or a serial port this way is told
    /// which of the two things it has, rather than being handed a connection that would fail later.
    pub async fn session_of(&self, handle: &str) -> Result<Arc<SshSession>, ShellError> {
        match self.session(handle).await?.as_ref() {
            LiveSession::Ssh(session) => Ok(Arc::clone(session)),
            LiveSession::Stream(_) => Err(ShellError::app_unsupported(
                "this terminal is not an SSH session, so it has no channels to open",
            )),
        }
    }

    async fn session(&self, handle: &str) -> Result<Arc<LiveSession>, ShellError> {
        self.sessions
            .lock()
            .await
            .get(handle)
            .map(|open| Arc::clone(&open.session))
            // Not an internal error: a handle whose terminal has ended is a thing that happens to
            // everybody who leaves a tab open, and the catalog has a code that says so — which is
            // also what lets the interface translate it instead of printing an English sentence.
            .ok_or_else(|| ShellError::session_not_found(handle))
    }

    /// Forwards one SSH terminal's output to the host, and forgets the session when it ends.
    fn forward_ssh(
        self: &Arc<Self>,
        handle: &str,
        mut receiver: mpsc::Receiver<sevenshell_ssh::ChannelEvent>,
        recording: Option<Recording>,
    ) -> JoinHandle<()> {
        let registry = Arc::clone(self);
        let events = self.events.clone();
        let forwarded = handle.to_owned();
        let mut recording = recording;

        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                let (terminal_event, ended) = match event {
                    sevenshell_ssh::ChannelEvent::Data(bytes) => (TerminalEvent::Data(bytes), false),
                    sevenshell_ssh::ChannelEvent::Closed { reason } => (TerminalEvent::Closed { reason }, true),
                };
                let keep_going = publish(&forwarded, terminal_event, &mut recording, &events).await;
                if !keep_going || ended {
                    break;
                }
            }

            finish(&registry, &forwarded, recording).await;
        })
    }

    /// Forwards one non-SSH terminal's output to the host, and forgets the session when it ends.
    fn forward_stream(
        self: &Arc<Self>,
        handle: &str,
        mut receiver: mpsc::Receiver<sevenshell_term::TerminalEvent>,
        recording: Option<Recording>,
    ) -> JoinHandle<()> {
        let registry = Arc::clone(self);
        let events = self.events.clone();
        let forwarded = handle.to_owned();
        let mut recording = recording;

        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                let (terminal_event, ended) = match event {
                    sevenshell_term::TerminalEvent::Data(bytes) => (TerminalEvent::Data(bytes), false),
                    sevenshell_term::TerminalEvent::Closed { reason } => (TerminalEvent::Closed { reason }, true),
                };
                let keep_going = publish(&forwarded, terminal_event, &mut recording, &events).await;
                if !keep_going || ended {
                    break;
                }
            }

            finish(&registry, &forwarded, recording).await;
        })
    }
}

/// Records and forwards one event. Returns whether the host is still listening.
async fn publish(
    handle: &str,
    event: TerminalEvent,
    recording: &mut Option<Recording>,
    events: &mpsc::Sender<SessionEvent>,
) -> bool {
    // Recorded as it arrives and before it is forwarded: what the person saw and what the file
    // holds have to be the same bytes, and waiting on the interface to keep up would make the
    // recording a report about the interface instead.
    if let (Some(recording), TerminalEvent::Data(bytes)) = (recording.as_mut(), &event) {
        recording.output(bytes).await;
    }
    // Backpressure is the point: if the interface cannot keep up, the remote reads pause rather
    // than the bytes piling up in this process.
    events
        .send(SessionEvent {
            handle: handle.to_owned(),
            event,
        })
        .await
        .is_ok()
}

async fn finish(registry: &Arc<SessionRegistry>, handle: &str, recording: Option<Recording>) {
    if let Some(recording) = recording {
        recording.finish().await;
    }
    // The session ended on its own, so it is dropped here rather than waiting for a close call
    // that will never come.
    if let Some(open) = registry.sessions.lock().await.remove(handle) {
        open.forwarder.abort();
    }
}

/// A terminal that is open, as the interface sees it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenTerminal {
    pub handle: String,
    pub session_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_stale_handle_is_refused() {
        let (registry, _events) = SessionRegistry::start();

        let error = registry.write("t404", b"ls\n").await.expect_err("no such terminal");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SessionNotFound);
        assert_eq!(error.diagnostics().get("sessionId").map(String::as_str), Some("t404"));
    }

    #[tokio::test]
    async fn closing_a_terminal_that_is_already_gone_is_not_an_error() {
        let (registry, _events) = SessionRegistry::start();

        assert!(registry.close("t404").await.is_ok());
        assert!(registry.is_empty().await);
        assert!(registry.open_sessions().await.is_empty());
    }
}
