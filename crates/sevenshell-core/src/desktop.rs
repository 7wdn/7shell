//! Remote desktops: the trust decision before one opens, and the handles afterwards.
//!
//! RDP has no SSH host key. The TLS certificate a server presents is recorded in the same trust
//! table under the `x509` kind, keyed by host and port like any other entry; a new certificate is a
//! question and a changed one is a stop. The order matters: the certificate is read before any
//! credential is offered, which is why the probe and the open are two separate steps.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use sevenshell_error::ShellError;
use sevenshell_rdp::{DesktopCommand, DesktopEvent, DesktopSession, InputEvent, MouseButton, OpenRequest, Target};
use sevenshell_types::{
    AuthMethod, DesktopInput, DesktopMouseButton, DesktopOutcome, KnownHostEntry, OpenDesktop, PresentedCertificate,
    Protocol, Session,
};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use zeroize::Zeroizing;

use crate::sessions::{Core, now_seconds};

/// The `key_type` an RDP certificate is stored under, so the table can hold both kinds.
const CERTIFICATE_KEY_TYPE: &str = "x509";

/// One event from an open desktop, tagged with the handle it belongs to.
#[derive(Debug)]
pub enum DesktopSessionEvent {
    Frame {
        handle: String,
        /// The session the frame belongs to, so a frame that arrives before the handle is wired
        /// to its channel can still find the channel that was registered for the attempt.
        session_id: String,
        /// One IPC batch: `count` as a little-endian `u32`, then per frame `x, y, width, height`
        /// as `u16` and RGBA8. Encoded here so the host does not have to know the RDP crate's types.
        bytes: Vec<u8>,
    },
    Resized {
        handle: String,
        width: u16,
        height: u16,
    },
    Closed {
        handle: String,
    },
    Failed {
        handle: String,
        error: ShellError,
    },
}

/// One IPC message holds a whole batch: `count` as a little-endian `u32`, then for each frame
/// `x, y, width, height` as `u16` and its RGBA8 bytes. Batching is what keeps a Windows frame of
/// dozens of tiles from becoming dozens of IPC messages.
fn encode_frames(frames: &[sevenshell_rdp::Frame]) -> Vec<u8> {
    let capacity = 4 + frames.iter().map(|frame| 8 + frame.rgba.len()).sum::<usize>();
    let mut bytes = Vec::with_capacity(capacity);
    let count = u32::try_from(frames.len()).expect("a frame batch fits in u32");
    bytes.extend_from_slice(&count.to_le_bytes());
    for frame in frames {
        bytes.extend_from_slice(&frame.x.to_le_bytes());
        bytes.extend_from_slice(&frame.y.to_le_bytes());
        bytes.extend_from_slice(&frame.width.to_le_bytes());
        bytes.extend_from_slice(&frame.height.to_le_bytes());
        bytes.extend_from_slice(&frame.rgba);
    }
    bytes
}

struct OpenDesktopSession {
    session_id: String,
    width: u16,
    height: u16,
    session: Arc<DesktopSession>,
    forwarder: JoinHandle<()>,
}

/// Every open desktop, addressed by a handle that is never reused.
pub(crate) struct Desktops {
    sessions: Mutex<HashMap<String, OpenDesktopSession>>,
    events: mpsc::UnboundedSender<DesktopSessionEvent>,
    next_handle: AtomicU64,
}

impl Desktops {
    pub(crate) fn new(events: mpsc::UnboundedSender<DesktopSessionEvent>) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            events,
            next_handle: AtomicU64::new(1),
        }
    }

    async fn open(
        &self,
        session_id: &str,
        target: Target,
        password: Zeroizing<String>,
        expected_certificate: String,
    ) -> Result<DesktopOutcome, ShellError> {
        let handle = format!("d{}", self.next_handle.fetch_add(1, Ordering::Relaxed));
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let session = DesktopSession::open(
            OpenRequest {
                target,
                password,
                expected_certificate,
                desktop: Default::default(),
            },
            sender,
        )
        .await?;
        let (width, height) = session.size();
        let session = Arc::new(session);

        let events = self.events.clone();
        let handle_for_task = handle.clone();
        let session_for_task = session_id.to_owned();
        let forwarder = tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                let event = match event {
                    DesktopEvent::Frames(frames) => DesktopSessionEvent::Frame {
                        handle: handle_for_task.clone(),
                        session_id: session_for_task.clone(),
                        bytes: encode_frames(&frames),
                    },
                    DesktopEvent::Resized { width, height } => DesktopSessionEvent::Resized {
                        handle: handle_for_task.clone(),
                        width,
                        height,
                    },
                    DesktopEvent::Closed => DesktopSessionEvent::Closed {
                        handle: handle_for_task.clone(),
                    },
                    DesktopEvent::Failed(error) => DesktopSessionEvent::Failed {
                        handle: handle_for_task.clone(),
                        error,
                    },
                };
                if events.send(event).is_err() {
                    break;
                }
            }
        });

        self.sessions.lock().await.insert(
            handle.clone(),
            OpenDesktopSession {
                session_id: session_id.to_owned(),
                width,
                height,
                session,
                forwarder,
            },
        );

        Ok(DesktopOutcome::Opened {
            handle,
            session_id: session_id.to_owned(),
            width,
            height,
        })
    }

    async fn session(&self, handle: &str) -> Result<Arc<DesktopSession>, ShellError> {
        self.sessions
            .lock()
            .await
            .get(handle)
            .map(|open| Arc::clone(&open.session))
            .ok_or_else(|| ShellError::app_internal("the remote desktop is no longer open"))
    }

    async fn close(&self, handle: &str) {
        let Some(open) = self.sessions.lock().await.remove(handle) else {
            return;
        };
        open.session.close().await;
        open.forwarder.abort();
    }

    pub(crate) async fn close_all(&self) {
        let handles: Vec<String> = self.sessions.lock().await.keys().cloned().collect();
        for handle in handles {
            self.close(&handle).await;
        }
    }

    async fn open_sessions(&self) -> Vec<OpenDesktop> {
        let sessions = self.sessions.lock().await;
        let mut open: Vec<OpenDesktop> = sessions
            .iter()
            .map(|(handle, session)| OpenDesktop {
                handle: handle.clone(),
                session_id: session.session_id.clone(),
                width: session.width,
                height: session.height,
            })
            .collect();
        open.sort_by(|left, right| left.handle.cmp(&right.handle));
        open
    }
}

impl Core {
    /// Opens a remote desktop, or reports which certificate decision has to be made first.
    ///
    /// The probe sends nothing but a TLS handshake: no credential is offered until a fingerprint
    /// has been compared with one a person accepted.
    pub async fn open_desktop(&self, session_id: &str) -> Result<DesktopOutcome, ShellError> {
        let session = self.session(session_id)?;
        if session.protocol != Protocol::Rdp {
            return Err(ShellError::app_unsupported("this session is not a remote desktop"));
        }

        let password = match session.auth.method {
            AuthMethod::Password | AuthMethod::KeyboardInteractive => {
                match self.vault.secret_for_session(&session.id)? {
                    Some(secret) => Zeroizing::new(secret.into_value()),
                    // A Windows account may genuinely have a blank password — mstsc and the
                    // reference client both connect that way. An empty field therefore means
                    // "send an empty password", not "refuse to connect". If the server refuses it,
                    // the refusal comes back as an authentication failure, which is the truth.
                    None => Zeroizing::new(String::new()),
                }
            }
            _ => {
                return Err(ShellError::app_unsupported(
                    "remote desktops authenticate with a password in this build",
                ));
            }
        };

        let target = target_of(&session);
        let presented = sevenshell_rdp::probe(&target).await?;
        let trusted = self
            .vault
            .find_known_host(&session.host, session.port)?
            .filter(|entry| entry.key_type == CERTIFICATE_KEY_TYPE);

        match trusted {
            None => Ok(DesktopOutcome::CertificateUnknown { certificate: presented }),
            Some(trusted) if trusted.fingerprint != presented.fingerprint => Ok(DesktopOutcome::CertificateMismatch {
                certificate: presented,
                trusted,
            }),
            Some(_) => {
                self.desktops
                    .open(session_id, target, password, presented.fingerprint)
                    .await
            }
        }
    }

    /// Records the certificate a person has just accepted, replacing any earlier one for the
    /// endpoint. The next open re-reads the certificate and compares again.
    pub fn trust_desktop_certificate(&self, certificate: &PresentedCertificate) -> Result<(), ShellError> {
        self.vault.trust_host(KnownHostEntry {
            host: certificate.host.clone(),
            port: certificate.port,
            key_type: CERTIFICATE_KEY_TYPE.to_owned(),
            public_key: certificate.certificate.clone(),
            fingerprint: certificate.fingerprint.clone(),
            added_at: now_seconds(),
        })
    }

    pub async fn desktop_input(&self, handle: &str, input: DesktopInput) -> Result<(), ShellError> {
        let session = self.desktops.session(handle).await?;
        let commands = session.commands();
        match input {
            DesktopInput::Text { text } => {
                for character in text.chars() {
                    let _ = commands
                        .send(DesktopCommand::Input(InputEvent::Unicode {
                            character,
                            pressed: true,
                        }))
                        .await;
                    let _ = commands
                        .send(DesktopCommand::Input(InputEvent::Unicode {
                            character,
                            pressed: false,
                        }))
                        .await;
                }
            }
            other => {
                if let Some(event) = input_event(other) {
                    let _ = commands.send(DesktopCommand::Input(event)).await;
                }
            }
        }
        Ok(())
    }

    pub async fn desktop_resize(&self, handle: &str, width: u16, height: u16) -> Result<(), ShellError> {
        let session = self.desktops.session(handle).await?;
        let _ = session.commands().send(DesktopCommand::Resize { width, height }).await;
        Ok(())
    }

    /// Asks the far side to repaint everything, for a canvas that has just mounted.
    pub async fn desktop_refresh(&self, handle: &str) -> Result<(), ShellError> {
        let session = self.desktops.session(handle).await?;
        let _ = session.commands().send(DesktopCommand::Refresh).await;
        Ok(())
    }

    pub async fn desktop_close(&self, handle: &str) -> Result<(), ShellError> {
        self.desktops.close(handle).await;
        Ok(())
    }

    pub async fn open_desktops(&self) -> Vec<OpenDesktop> {
        self.desktops.open_sessions().await
    }

    /// The desktop event stream, taken once by whichever host drains it.
    pub fn take_desktop_events(&self) -> Option<mpsc::UnboundedReceiver<DesktopSessionEvent>> {
        self.desktop_events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

fn target_of(session: &Session) -> Target {
    let (domain, user) = match session.user.split_once('\\') {
        Some((domain, user)) => (domain.to_owned(), user.to_owned()),
        None => (String::new(), session.user.clone()),
    };

    Target {
        host: session.host.clone(),
        port: session.port,
        user,
        domain,
    }
}

fn input_event(input: DesktopInput) -> Option<InputEvent> {
    Some(match input {
        DesktopInput::PointerMove { x, y } => InputEvent::MouseMove { x, y },
        DesktopInput::PointerButton { button, pressed } => InputEvent::MouseButton {
            button: match button {
                DesktopMouseButton::Left => MouseButton::Left,
                DesktopMouseButton::Right => MouseButton::Right,
                DesktopMouseButton::Middle => MouseButton::Middle,
                DesktopMouseButton::Back => MouseButton::Back,
                DesktopMouseButton::Forward => MouseButton::Forward,
            },
            pressed,
        },
        DesktopInput::Wheel { vertical, units } => InputEvent::Wheel { vertical, units },
        DesktopInput::Key { scancode, pressed } => InputEvent::Key { scancode, pressed },
        // Text is expanded into Unicode key events before this is reached.
        DesktopInput::Text { .. } => return None,
    })
}
