//! Terminals that are not SSH: Telnet consoles, raw TCP consoles, serial ports, and shells on this
//! machine.
//!
//! Each transport owns a task that reads its byte stream and answers commands (`write`, `resize`,
//! `close`) in the same shape, so the session registry above this crate does not care which one a
//! handle belongs to. What crosses back is bytes and one closing sentence — the same two things an
//! SSH channel produced.
//!
//! Two rules are deliberate here. A local shell starts with no credentials, no host key and no
//! network: it is a process on this machine, and pretending it has an identity would make the
//! interface promise something it cannot check. And a serial port is opened with exactly the line
//! parameters the session stored; an out-of-range value is refused rather than quietly replaced
//! with 8N1, because a silently wrong baud rate looks like a broken device.

mod local;
mod serial;
mod tcp;
mod telnet;

/// ZMODEM (`rz`/`sz`) framing and session state machines, for file transfer inside a terminal.
pub mod zmodem;

use sevenshell_error::ShellError;
use tokio::sync::mpsc;

pub use local::{LocalSession, LocalSpec, WslSpec, local_shells, wsl_distros};
pub use serial::{SerialSession, SerialSpec, serial_ports};
pub use sevenshell_types::{LocalShell, SerialPortInfo};
pub use tcp::{TcpSession, TcpSpec};
pub use telnet::{TelnetSession, TelnetSpec};

/// The size a terminal is told it has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub columns: u32,
    pub rows: u32,
}

impl Default for Size {
    fn default() -> Self {
        Self { columns: 80, rows: 24 }
    }
}

/// One event from a live terminal.
///
/// The same shape an SSH channel produces, by design: whoever forwards events to the interface
/// matches on one enumeration regardless of the transport behind the handle.
#[derive(Debug)]
pub enum TerminalEvent {
    Data(Vec<u8>),
    Closed { reason: Option<String> },
}

/// A terminal whose transport is not SSH.
pub enum StreamSession {
    Telnet(TelnetSession),
    Tcp(TcpSession),
    Serial(SerialSession),
    Local(LocalSession),
}

impl StreamSession {
    pub async fn write(&self, data: &[u8]) -> Result<(), ShellError> {
        match self {
            Self::Telnet(session) => session.write(data).await,
            Self::Tcp(session) => session.write(data).await,
            Self::Serial(session) => session.write(data).await,
            Self::Local(session) => session.write(data).await,
        }
    }

    pub async fn resize(&self, columns: u32, rows: u32) -> Result<(), ShellError> {
        match self {
            Self::Telnet(session) => session.resize(columns, rows).await,
            Self::Tcp(session) => session.resize(columns, rows).await,
            // A serial line has no notion of a window; resizing one is a no-op for the same
            // reason `stty` has nothing to set on a pipe.
            Self::Serial(session) => session.resize(columns, rows).await,
            Self::Local(session) => session.resize(columns, rows).await,
        }
    }

    pub async fn close(&self) -> Result<(), ShellError> {
        match self {
            Self::Telnet(session) => session.close().await,
            Self::Tcp(session) => session.close().await,
            Self::Serial(session) => session.close().await,
            Self::Local(session) => session.close().await,
        }
    }
}

/// Opens a Telnet console and returns the handle plus the stream its output arrives on.
pub async fn open_telnet(
    spec: TelnetSpec,
    size: Size,
) -> Result<(StreamSession, mpsc::Receiver<TerminalEvent>), ShellError> {
    let (session, events) = telnet::open(spec, size).await?;
    Ok((StreamSession::Telnet(session), events))
}

/// Opens a raw TCP console and returns the handle plus the stream its output arrives on.
pub async fn open_raw_tcp(spec: TcpSpec) -> Result<(StreamSession, mpsc::Receiver<TerminalEvent>), ShellError> {
    let (session, events) = tcp::open(spec).await?;
    Ok((StreamSession::Tcp(session), events))
}

/// Opens a serial port and returns the handle plus the stream its output arrives on.
pub async fn open_serial(spec: SerialSpec) -> Result<(StreamSession, mpsc::Receiver<TerminalEvent>), ShellError> {
    let (session, events) = serial::open(spec).await?;
    Ok((StreamSession::Serial(session), events))
}

/// Starts a shell on this machine and returns the handle plus the stream its output arrives on.
pub async fn open_local(
    spec: LocalSpec,
    size: Size,
) -> Result<(StreamSession, mpsc::Receiver<TerminalEvent>), ShellError> {
    let (session, events) = local::open(spec, size).await?;
    Ok((StreamSession::Local(session), events))
}

/// Starts a WSL distribution and returns the handle plus the stream its output arrives on.
pub async fn open_wsl(spec: WslSpec, size: Size) -> Result<(StreamSession, mpsc::Receiver<TerminalEvent>), ShellError> {
    let (session, events) = local::open_wsl(spec, size).await?;
    Ok((StreamSession::Local(session), events))
}
