//! A raw TCP console: a socket and the terminal, nothing in between.
//!
//! Some devices speak a line protocol of their own on a TCP port and do not want Telnet's option
//! negotiation in the way. This transport dials the address and carries bytes verbatim in both
//! directions — no IAC parsing, no window report, no keyboard translation. What is typed is what is
//! sent, which is the whole contract.

use std::time::Duration;

use sevenshell_error::ShellError;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::TerminalEvent;

/// What to dial.
#[derive(Debug, Clone)]
pub struct TcpSpec {
    pub host: String,
    pub port: u16,
    pub timeout: Duration,
}

/// A raw TCP console that is open.
pub struct TcpSession {
    commands: mpsc::Sender<Command>,
}

enum Command {
    Input(Vec<u8>),
    Close,
}

impl TcpSession {
    pub async fn write(&self, data: &[u8]) -> Result<(), ShellError> {
        self.commands
            .send(Command::Input(data.to_vec()))
            .await
            .map_err(|_| ShellError::session_not_found("the TCP console is no longer open"))
    }

    /// There is no window protocol on a raw socket; the caller does not have to know that.
    pub async fn resize(&self, _columns: u32, _rows: u32) -> Result<(), ShellError> {
        Ok(())
    }

    pub async fn close(&self) -> Result<(), ShellError> {
        let _ = self.commands.send(Command::Close).await;
        Ok(())
    }
}

/// Dials the address and starts the pump.
pub async fn open(spec: TcpSpec) -> Result<(TcpSession, mpsc::Receiver<TerminalEvent>), ShellError> {
    let stream = match tokio::time::timeout(spec.timeout, TcpStream::connect((spec.host.as_str(), spec.port))).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => {
            return Err(ShellError::terminal_unreachable(
                &spec.host,
                spec.port,
                error.to_string(),
            ));
        }
        Err(_) => {
            return Err(ShellError::terminal_unreachable(
                &spec.host,
                spec.port,
                format!("nothing answered within {} seconds", spec.timeout.as_secs()),
            ));
        }
    };
    let _ = stream.set_nodelay(true);

    let (events, receiver) = mpsc::channel(256);
    let (commands, command_receiver) = mpsc::channel(128);
    tokio::spawn(pump(stream, command_receiver, events));
    Ok((TcpSession { commands }, receiver))
}

async fn pump(stream: TcpStream, mut commands: mpsc::Receiver<Command>, events: mpsc::Sender<TerminalEvent>) {
    let (mut reader, mut writer) = stream.into_split();
    let mut buffer = vec![0u8; 8192];
    let mut reason = None;

    loop {
        tokio::select! {
            read = reader.read(&mut buffer) => match read {
                Ok(0) => break,
                Ok(count) => {
                    if events
                        .send(TerminalEvent::Data(buffer[..count].to_vec()))
                        .await
                        .is_err()
                    {
                        // Nobody is listening any more; the socket is the only thing left to drop.
                        return;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    reason = Some(format!("the connection failed: {error}"));
                    break;
                }
            },
            command = commands.recv() => match command {
                Some(Command::Input(data)) => {
                    // Verbatim, by design: this is the transport that does not rewrite what a
                    // person typed.
                    if !data.is_empty() && writer.write_all(&data).await.is_err() {
                        reason = Some("the connection stopped accepting input".to_owned());
                        break;
                    }
                }
                Some(Command::Close) | None => break,
            },
        }
    }

    let _ = events.send(TerminalEvent::Closed { reason }).await;
}
