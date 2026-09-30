//! Live sessions: the registry of terminals that are open right now.
//!
//! The registry owns the connection handles and the task that forwards their output; the host
//! owns the receiver and decides what to do with the bytes (the desktop shell turns them into
//! window events). This crate keeps no credentials: whatever a session needed to authenticate was
//! handed to `sevenshell-ssh` and dropped there.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use sevenshell_error::ShellError;
use sevenshell_ssh::{ChannelEvent, OpenRequest, SshSession};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;

/// The events a terminal produces, re-exported so callers do not need the transport crate.
pub use sevenshell_ssh::ChannelEvent as TerminalEvent;

/// One event from one session, tagged with where it came from.
#[derive(Debug)]
pub struct SessionEvent {
    pub handle: String,
    pub event: ChannelEvent,
}

/// A terminal that is open, and the task reading it.
struct OpenSession {
    session_id: String,
    session: Arc<SshSession>,
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
    pub async fn open(self: &Arc<Self>, session_id: &str, request: OpenRequest) -> Result<String, ShellError> {
        let handle = format!("t{}", self.next_handle.fetch_add(1, Ordering::Relaxed));
        let (sender, receiver) = mpsc::channel(256);
        let session = SshSession::open(request, sender).await?;

        let forwarder = self.forward(&handle, receiver);
        self.sessions.lock().await.insert(
            handle.clone(),
            OpenSession {
                session_id: session_id.to_owned(),
                session: Arc::new(session),
                forwarder,
            },
        );

        Ok(handle)
    }

    /// Sends keystrokes or pasted text to a terminal.
    pub async fn write(&self, handle: &str, data: &[u8]) -> Result<(), ShellError> {
        let session = self.session(handle).await?;
        session.write(data).await
    }

    /// Tells the remote side how big the terminal is now.
    pub async fn resize(&self, handle: &str, columns: u32, rows: u32) -> Result<(), ShellError> {
        let session = self.session(handle).await?;
        session.resize(columns, rows).await
    }

    /// Closes one terminal and forgets it.
    pub async fn close(&self, handle: &str) -> Result<(), ShellError> {
        let Some(open) = self.sessions.lock().await.remove(handle) else {
            // Closing something that is already gone is what a second click means.
            return Ok(());
        };

        let result = open.session.close().await;
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

    async fn session(&self, handle: &str) -> Result<Arc<SshSession>, ShellError> {
        self.sessions
            .lock()
            .await
            .get(handle)
            .map(|open| Arc::clone(&open.session))
            .ok_or_else(|| ShellError::app_internal(format!("terminal {handle} is no longer open")))
    }

    /// Forwards one terminal's output to the host, and forgets the session when it ends.
    fn forward(self: &Arc<Self>, handle: &str, mut receiver: mpsc::Receiver<ChannelEvent>) -> JoinHandle<()> {
        let registry = Arc::clone(self);
        let events = self.events.clone();
        let forwarded = handle.to_owned();

        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                let ended = matches!(event, ChannelEvent::Closed { .. });
                // Backpressure is the point: if the interface cannot keep up, the remote reads
                // pause rather than the bytes piling up in this process.
                if events
                    .send(SessionEvent {
                        handle: forwarded.clone(),
                        event,
                    })
                    .await
                    .is_err()
                {
                    break;
                }
                if ended {
                    break;
                }
            }

            // The session ended on its own, so it is dropped here rather than waiting for a close
            // call that will never come.
            if let Some(open) = registry.sessions.lock().await.remove(&forwarded) {
                open.forwarder.abort();
            }
        })
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

        assert_eq!(error.code(), sevenshell_error::ErrorCode::AppInternal);
    }

    #[tokio::test]
    async fn closing_a_terminal_that_is_already_gone_is_not_an_error() {
        let (registry, _events) = SessionRegistry::start();

        assert!(registry.close("t404").await.is_ok());
        assert!(registry.is_empty().await);
        assert!(registry.open_sessions().await.is_empty());
    }
}
