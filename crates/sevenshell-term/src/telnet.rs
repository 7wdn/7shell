//! A Telnet console, with just enough of the protocol to be useful.
//!
//! The negotiation is the classic byte machine: answer `WILL ECHO` and `WILL SGA` with `DO`, answer
//! a `DO NAWS` with `WILL NAWS` and report the window size, decline everything else, and never let
//! an IAC sequence that straddles two reads corrupt the payload. That is all a console server needs
//! to draw a login prompt; subnegotiation we do not use is swallowed rather than guessed at.
//!
//! The keyboard is translated on the way out because old consoles disagree about two keys: Enter is
//! CRLF, CR or LF, and Backspace is DEL or Ctrl+H. Both are stored per session.

use std::time::Duration;

use sevenshell_error::ShellError;
use sevenshell_types::{BackspaceMode, EnterMode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::{Size, TerminalEvent};

/// What to dial and how the keyboard should be translated.
#[derive(Debug, Clone)]
pub struct TelnetSpec {
    pub host: String,
    pub port: u16,
    pub enter: EnterMode,
    pub backspace: BackspaceMode,
    pub timeout: Duration,
}

/// A Telnet console that is open.
pub struct TelnetSession {
    commands: mpsc::Sender<Command>,
}

enum Command {
    Input(Vec<u8>),
    Resize { columns: u16, rows: u16 },
    Close,
}

const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;

const OPT_ECHO: u8 = 1;
const OPT_SGA: u8 = 3;
const OPT_NAWS: u8 = 31;

impl TelnetSession {
    pub async fn write(&self, data: &[u8]) -> Result<(), ShellError> {
        self.commands
            .send(Command::Input(data.to_vec()))
            .await
            .map_err(|_| ShellError::session_not_found("the Telnet console is no longer open"))
    }

    pub async fn resize(&self, columns: u32, rows: u32) -> Result<(), ShellError> {
        self.commands
            .send(Command::Resize {
                columns: columns.clamp(1, u16::MAX as u32) as u16,
                rows: rows.clamp(1, u16::MAX as u32) as u16,
            })
            .await
            .map_err(|_| ShellError::session_not_found("the Telnet console is no longer open"))
    }

    pub async fn close(&self) -> Result<(), ShellError> {
        let _ = self.commands.send(Command::Close).await;
        Ok(())
    }
}

/// Dials the console and starts its pump.
pub async fn open(spec: TelnetSpec, size: Size) -> Result<(TelnetSession, mpsc::Receiver<TerminalEvent>), ShellError> {
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
    tokio::spawn(pump(stream, size, spec.enter, spec.backspace, command_receiver, events));
    Ok((TelnetSession { commands }, receiver))
}

async fn pump(
    stream: TcpStream,
    size: Size,
    enter: EnterMode,
    backspace: BackspaceMode,
    mut commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<TerminalEvent>,
) {
    let (mut reader, mut writer) = stream.into_split();
    let mut parser = TelnetParser::new();
    let mut buffer = vec![0u8; 8192];
    let mut reason = None;

    // The first window report is sent before the server asks for one. Most implementations ignore
    // a report they did not negotiate, and the ones that do ask get an answer before the first
    // full-screen program draws.
    let columns = size.columns.clamp(1, u16::MAX as u32) as u16;
    let rows = size.rows.clamp(1, u16::MAX as u32) as u16;
    if writer.write_all(&naws_frame(columns, rows)).await.is_err() {
        let _ = events
            .send(TerminalEvent::Closed {
                reason: Some("the console did not accept the opening window size".to_owned()),
            })
            .await;
        return;
    }

    loop {
        tokio::select! {
            read = reader.read(&mut buffer) => match read {
                Ok(0) => break,
                Ok(count) => {
                    let (payload, replies) = parser.feed(&buffer[..count]);
                    if !replies.is_empty() && writer.write_all(&replies).await.is_err() {
                        reason = Some("the console stopped accepting negotiation replies".to_owned());
                        break;
                    }
                    if payload.is_empty() {
                        continue;
                    }
                    if events.send(TerminalEvent::Data(payload)).await.is_err() {
                        // Nobody is listening any more; the session is gone in every way that
                        // matters, and holding the socket open would only leak it.
                        return;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    reason = Some(format!("the console read failed: {error}"));
                    break;
                }
            },
            command = commands.recv() => match command {
                Some(Command::Input(data)) => {
                    let wire = transform_input(&data, enter, backspace);
                    if !wire.is_empty() && writer.write_all(&wire).await.is_err() {
                        reason = Some("the console stopped accepting input".to_owned());
                        break;
                    }
                }
                Some(Command::Resize { columns, rows }) => {
                    if writer.write_all(&naws_frame(columns, rows)).await.is_err() {
                        reason = Some("the console stopped accepting window reports".to_owned());
                        break;
                    }
                }
                Some(Command::Close) | None => break,
            },
        }
    }

    let _ = events.send(TerminalEvent::Closed { reason }).await;
}

/// What the keyboard puts on the wire.
///
/// Backspace first, then Enter, because a pasted `\r\n` must stay one Enter: translating each byte
/// on its own would send two in CRLF mode and move two lines.
pub fn transform_input(data: &[u8], enter: EnterMode, backspace: BackspaceMode) -> Vec<u8> {
    let mut output = Vec::with_capacity(data.len() + 8);
    let mut previous_was_cr = false;
    for &byte in data {
        let byte = match (backspace, byte) {
            (BackspaceMode::CtrlH, 0x7f) => 0x08,
            (_, other) => other,
        };
        match byte {
            0x0d => {
                previous_was_cr = true;
                match enter {
                    EnterMode::Crlf => output.extend_from_slice(&[0x0d, 0x0a]),
                    EnterMode::Cr => output.push(0x0d),
                    EnterMode::Lf => output.push(0x0a),
                }
            }
            0x0a if previous_was_cr => previous_was_cr = false,
            other => {
                previous_was_cr = false;
                output.push(other);
            }
        }
    }
    output
}

/// `IAC SB NAWS <columns> <rows> IAC SE`, with a value byte of `0xFF` doubled.
pub fn naws_frame(columns: u16, rows: u16) -> Vec<u8> {
    let mut frame = vec![IAC, SB, OPT_NAWS];
    for half in [columns, rows] {
        for byte in half.to_be_bytes() {
            if byte == IAC {
                frame.push(IAC);
            }
            frame.push(byte);
        }
    }
    frame.extend_from_slice(&[IAC, SE]);
    frame
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ParseState {
    #[default]
    Ground,
    Iac,
    Negotiation {
        verb: u8,
    },
    Subnegotiation,
    SubnegotiationIac,
}

/// The IAC stripper and negotiation answerer.
///
/// The state lives between `feed` calls so a sequence split across TCP segments is still parsed as
/// one sequence; scanning each chunk on its own would leak the trailing IAC bytes to the terminal.
#[derive(Debug, Default)]
pub struct TelnetParser {
    state: ParseState,
}

impl TelnetParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Consumes one wire chunk and answers `(payload for the terminal, bytes for the server)`.
    pub fn feed(&mut self, chunk: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let mut payload = Vec::with_capacity(chunk.len());
        let mut replies = Vec::new();
        for &byte in chunk {
            match self.state {
                ParseState::Ground => {
                    if byte == IAC {
                        self.state = ParseState::Iac;
                    } else {
                        payload.push(byte);
                    }
                }
                ParseState::Iac => {
                    self.state = match byte {
                        IAC => {
                            payload.push(IAC);
                            ParseState::Ground
                        }
                        WILL | WONT | DO | DONT => ParseState::Negotiation { verb: byte },
                        SB => ParseState::Subnegotiation,
                        _ => ParseState::Ground,
                    }
                }
                ParseState::Negotiation { verb } => {
                    self.state = ParseState::Ground;
                    match (verb, byte) {
                        (WILL, OPT_ECHO) | (WILL, OPT_SGA) => replies.extend_from_slice(&[IAC, DO, byte]),
                        (WILL, _) => replies.extend_from_slice(&[IAC, DONT, byte]),
                        (DO, OPT_NAWS) => replies.extend_from_slice(&[IAC, WILL, byte]),
                        (DO, _) => replies.extend_from_slice(&[IAC, WONT, byte]),
                        _ => {}
                    }
                }
                ParseState::Subnegotiation => {
                    if byte == IAC {
                        self.state = ParseState::SubnegotiationIac;
                    }
                }
                ParseState::SubnegotiationIac => {
                    self.state = match byte {
                        SE => ParseState::Ground,
                        IAC => ParseState::Subnegotiation,
                        _ => ParseState::Ground,
                    };
                }
            }
        }
        (payload, replies)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(chunks: &[&[u8]]) -> (Vec<u8>, Vec<u8>) {
        let mut parser = TelnetParser::new();
        let (mut payload, mut replies) = (Vec::new(), Vec::new());
        for chunk in chunks {
            let (chunk_payload, chunk_replies) = parser.feed(chunk);
            payload.extend_from_slice(&chunk_payload);
            replies.extend_from_slice(&chunk_replies);
        }
        (payload, replies)
    }

    #[test]
    fn negotiation_answers_only_the_options_this_build_uses() {
        assert_eq!(run(&[&[IAC, WILL, OPT_ECHO]]).1, vec![IAC, DO, OPT_ECHO]);
        assert_eq!(run(&[&[IAC, WILL, OPT_SGA]]).1, vec![IAC, DO, OPT_SGA]);
        assert_eq!(run(&[&[IAC, WILL, 24]]).1, vec![IAC, DONT, 24]);
        assert_eq!(run(&[&[IAC, DO, OPT_NAWS]]).1, vec![IAC, WILL, OPT_NAWS]);
        assert_eq!(run(&[&[IAC, DO, OPT_ECHO]]).1, vec![IAC, WONT, OPT_ECHO]);
        for refusal in [vec![IAC, WONT, OPT_ECHO], vec![IAC, DONT, OPT_NAWS], vec![IAC, 241]] {
            let (payload, replies) = run(&[&refusal]);
            assert!(payload.is_empty(), "{refusal:?} is not payload");
            assert!(replies.is_empty(), "{refusal:?} needs no answer");
        }
    }

    #[test]
    fn an_escaped_iac_is_a_payload_byte_and_commands_are_not() {
        let (payload, replies) = run(&[&[b'a', IAC, IAC, 0xfe, b'b']]);
        assert_eq!(payload, vec![b'a', 0xff, 0xfe, b'b']);
        assert!(replies.is_empty());

        let (payload, _) = run(&[&[b'a', IAC, WILL, OPT_ECHO, b'b', IAC, 241, b'c']]);
        assert_eq!(payload, b"abc");
    }

    #[test]
    fn a_sequence_split_across_chunks_is_one_sequence() {
        let (payload, replies) = run(&[&[b'a', IAC], &[WILL], &[OPT_ECHO, b'b']]);
        assert_eq!(payload, b"ab");
        assert_eq!(replies, vec![IAC, DO, OPT_ECHO]);
    }

    #[test]
    fn subnegotiation_is_swallowed_even_when_it_carries_an_iac() {
        let (payload, replies) = run(&[&[b'a', IAC, SB, 24, b'x', b'y', IAC], &[SE, b'b']]);
        assert_eq!(payload, b"ab");
        assert!(replies.is_empty());
    }

    #[test]
    fn a_window_report_escapes_a_byte_that_looks_like_iac() {
        let frame = naws_frame(0x00ff, 0x0102);
        assert_eq!(frame, vec![IAC, SB, OPT_NAWS, 0x00, IAC, IAC, 0x01, 0x02, IAC, SE]);
    }

    #[test]
    fn enter_and_backspace_follow_the_stored_modes() {
        assert_eq!(
            transform_input(b"a\x7f\r", EnterMode::Crlf, BackspaceMode::Del),
            b"a\x7f\r\n"
        );
        assert_eq!(
            transform_input(b"a\x7f\r", EnterMode::Cr, BackspaceMode::CtrlH),
            b"a\x08\r"
        );
        assert_eq!(
            transform_input(b"\r\n", EnterMode::Lf, BackspaceMode::Del),
            b"\n",
            "a pasted CRLF is one Enter, not two"
        );
        assert_eq!(transform_input(b"x\ny", EnterMode::Crlf, BackspaceMode::Del), b"x\ny");
    }
}
