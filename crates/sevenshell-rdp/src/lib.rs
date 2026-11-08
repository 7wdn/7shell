//! A Windows desktop over RDP, carried by this process.
//!
//! The order of operations is the security property, not an implementation detail. The server's
//! TLS certificate is read and compared with the fingerprint a person accepted **before** the
//! connector is allowed to send a credential. A changed certificate is a stop, not a prompt — the
//! caller decides whether to ask again, exactly as it does for a changed SSH host key.
//!
//! Nothing here knows about the vault, the session registry or the interface: plain values in,
//! plain values out. The interface's cursor is not composited here either; the session asks the
//! server to draw the pointer into the bitmap, which is one less state machine to get wrong.

mod engine;
mod trust;

use sevenshell_error::ShellError;
use sevenshell_types::PresentedCertificate;
use tokio::sync::mpsc;

/// Where to connect and who to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The name as the person typed it; resolving it is the network's business.
    pub host: String,
    pub port: u16,
    /// The account. A domain account is written `DOMAIN\user`, which is the spelling Windows
    /// itself uses and the one the reference dialog puts in this single field.
    pub user: String,
    /// The Windows domain, when it is not already part of `user`.
    pub domain: String,
}

/// A desktop's size in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopSize {
    pub width: u16,
    pub height: u16,
}

impl Default for DesktopSize {
    /// The size a remote desktop starts at when nothing has measured the window yet. The window
    /// asks for its own size immediately after the first frame.
    fn default() -> Self {
        Self {
            width: 1280,
            height: 800,
        }
    }
}

/// Everything an open needs.
///
/// Deliberately not `Debug`: the password is a field, and a derived implementation would put it
/// in a log the first time somebody formats the request.
pub struct OpenRequest {
    pub target: Target,
    /// Moved in and wiped when the connection attempt ends.
    pub password: zeroize::Zeroizing<String>,
    /// The fingerprint accepted for this endpoint. The connection is refused if the server
    /// presents anything else.
    pub expected_certificate: String,
    pub desktop: DesktopSize,
}

/// A rectangle of pixels that changed, in the desktop's coordinates.
#[derive(Debug)]
pub struct Frame {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    /// RGBA8, row-major, exactly `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

/// What an open desktop reports upward.
#[derive(Debug)]
pub enum DesktopEvent {
    /// The changed rectangles from one processing step, in draw order.
    ///
    /// Batched rather than one event per rectangle: a Windows frame is often dozens of small
    /// tiles, and one IPC message per tile is what made the desktop feel slow.
    Frames(Vec<Frame>),
    /// The far side changed the desktop size, including after a local resize request.
    Resized { width: u16, height: u16 },
    /// The session ended and nothing failed; the server closed it or the window asked to.
    Closed,
    /// The session ended because something went wrong. The envelope carries the code the
    /// interface localizes.
    Failed(ShellError),
}

/// Input and control, from the interface.
#[derive(Debug)]
pub enum DesktopCommand {
    Input(InputEvent),
    Resize {
        width: u16,
        height: u16,
    },
    /// Asks the server to repaint the whole desktop. Used when the canvas mounts: the first
    /// frames can arrive before there is anywhere to draw them.
    Refresh,
    Close,
}

#[derive(Debug)]
pub enum InputEvent {
    MouseMove {
        x: u16,
        y: u16,
    },
    MouseButton {
        button: MouseButton,
        pressed: bool,
    },
    Wheel {
        vertical: bool,
        /// In RDP's units: 120 per notch, negative for up/left.
        units: i16,
    },
    /// A PC scancode; `0xE0xx` is the extended form (`Scancode::from_u16` reads it that way).
    Key {
        scancode: u16,
        pressed: bool,
    },
    /// A character composed locally. This is how a Chinese input method reaches the far side:
    /// the composition finishes on this machine and the finished character goes as text.
    Unicode {
        character: char,
        pressed: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

/// A connected desktop.
///
/// Commands are accepted until the task that owns the connection ends. Closing is best-effort:
/// the task sends the disconnect it can and releases the socket either way.
pub struct DesktopSession {
    commands: mpsc::Sender<DesktopCommand>,
    width: u16,
    height: u16,
}

impl DesktopSession {
    /// Connects, verifies the certificate and starts the loop that carries the screen.
    ///
    /// The connection is established before this returns, so a bad password or a changed
    /// certificate is reported to the caller instead of arriving later as an event.
    pub async fn open(request: OpenRequest, events: mpsc::UnboundedSender<DesktopEvent>) -> Result<Self, ShellError> {
        let (framed, connection) = engine::connect(&request).await?;
        let (commands, receiver) = mpsc::channel(256);
        let host = request.target.host;
        let port = request.target.port;
        let width = connection.desktop_size.width;
        let height = connection.desktop_size.height;

        tokio::spawn(async move {
            engine::run(framed, connection, receiver, events, host, port).await;
        });

        Ok(Self {
            commands,
            width,
            height,
        })
    }

    /// The desktop size the server agreed to at connection time, which is not always the one
    /// that was asked for.
    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    /// A handle for input, resize and close.
    pub fn commands(&self) -> mpsc::Sender<DesktopCommand> {
        self.commands.clone()
    }

    /// Asks the task to disconnect. Best-effort: the socket is released when the task ends.
    pub async fn close(&self) {
        let _ = self.commands.send(DesktopCommand::Close).await;
    }
}

/// Reads the certificate the server presents, without sending anything that could authenticate.
///
/// This is the RDP counterpart of an SSH host-key probe: the X.224 negotiation and a TLS
/// handshake, then the socket is dropped. The answer is a value the caller can put in front of a
/// person.
pub async fn probe(target: &Target) -> Result<PresentedCertificate, ShellError> {
    trust::probe(target).await
}
