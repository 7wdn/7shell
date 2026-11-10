//! The wire layer: how a frame is spelled, escaped and checked.
//!
//! Two details here are easy to get wrong and expensive to discover late, so they live in one
//! place with tests that pin the bytes rather than the arithmetic:
//!
//! * A 32-bit position fills all four information bytes from the low end (`0x0c5a` travels as
//!   `5a 0c 00 00`), while flags live in the *last* information byte.
//! * The two checksums do not share a byte order. The 16-bit CRC travels high byte first, the
//!   32-bit one low byte first.
//!
//! Reading is deliberately tolerant at the outer edge and strict in the middle: junk between
//! frames (the echo of the `rz` command, a shell prompt) is skipped, but a header that starts
//! correctly and then fails its own checksum, ends in the middle, or names a frame type that does
//! not exist is an error the caller can report.

use std::collections::VecDeque;
use std::io::ErrorKind;

use sevenshell_error::ShellError;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::crc::{crc16, crc16_update, crc16_wire_bytes, crc32, crc32_update, crc32_wire_bytes};

/// Padding that precedes every frame; the escape character that begins every header.
pub const ZPAD: u8 = 0x2a;
pub const ZDLE: u8 = 0x18;

/// The three ways a header can be spelled.
pub const ZHEX: u8 = b'B';
pub const ZBIN: u8 = b'A';
pub const ZBIN32: u8 = b'C';

/// Two rubout values some senders use instead of the plain escape of 0x7f/0xff.
const ZRUB0: u8 = 0x6c;
const ZRUB1: u8 = 0x6d;

/// Bytes that must never appear raw inside an escaped run.
///
/// The high-bit twins are included because a transport that strips the eighth bit would otherwise
/// turn them into XON/XOFF/ZDLE; escaping both halves makes the stream survive that.
const ESCAPED_BYTES: [u8; 8] = [0x10, 0x11, 0x13, 0x18, 0x90, 0x91, 0x93, 0x98];

/// A frame type from the protocol's catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    RqInit = 0x00,
    RInit = 0x01,
    SInit = 0x02,
    Ack = 0x03,
    File = 0x04,
    Skip = 0x05,
    Nak = 0x06,
    Abort = 0x07,
    Fin = 0x08,
    RPos = 0x09,
    Data = 0x0a,
    Eof = 0x0b,
    FError = 0x0c,
    Crc = 0x0d,
    Challenge = 0x0e,
    Compl = 0x0f,
    Can = 0x10,
    FreeCnt = 0x11,
    Command = 0x12,
    StdErr = 0x13,
}

impl FrameType {
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x00 => Some(Self::RqInit),
            0x01 => Some(Self::RInit),
            0x02 => Some(Self::SInit),
            0x03 => Some(Self::Ack),
            0x04 => Some(Self::File),
            0x05 => Some(Self::Skip),
            0x06 => Some(Self::Nak),
            0x07 => Some(Self::Abort),
            0x08 => Some(Self::Fin),
            0x09 => Some(Self::RPos),
            0x0a => Some(Self::Data),
            0x0b => Some(Self::Eof),
            0x0c => Some(Self::FError),
            0x0d => Some(Self::Crc),
            0x0e => Some(Self::Challenge),
            0x0f => Some(Self::Compl),
            0x10 => Some(Self::Can),
            0x11 => Some(Self::FreeCnt),
            0x12 => Some(Self::Command),
            0x13 => Some(Self::StdErr),
            _ => None,
        }
    }

    /// The protocol's own name, for error details a person can look up.
    pub const fn name(self) -> &'static str {
        match self {
            Self::RqInit => "ZRQINIT",
            Self::RInit => "ZRINIT",
            Self::SInit => "ZSINIT",
            Self::Ack => "ZACK",
            Self::File => "ZFILE",
            Self::Skip => "ZSKIP",
            Self::Nak => "ZNAK",
            Self::Abort => "ZABORT",
            Self::Fin => "ZFIN",
            Self::RPos => "ZRPOS",
            Self::Data => "ZDATA",
            Self::Eof => "ZEOF",
            Self::FError => "ZFERR",
            Self::Crc => "ZCRC",
            Self::Challenge => "ZCHALLENGE",
            Self::Compl => "ZCOMPL",
            Self::Can => "ZCAN",
            Self::FreeCnt => "ZFREECNT",
            Self::Command => "ZCOMMAND",
            Self::StdErr => "ZSTDERR",
        }
    }
}

/// Capability bits the receiver puts in the last information byte of its ZRINIT.
pub mod capabilities {
    pub const CANFDX: u8 = 0x01;
    pub const CANOVIO: u8 = 0x02;
    pub const CANFC32: u8 = 0x20;
    pub const ESCCTL: u8 = 0x40;
    pub const ESC8: u8 = 0x80;
}

/// The conversion options a sender can put in the last information byte of a ZFILE.
pub mod file_option {
    pub const ZCBIN: u8 = 1;
    pub const ZCNL: u8 = 2;
    pub const ZCRECOV: u8 = 3;
}

/// How a header (and, by extension, the data subpackets after it) is spelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderCoding {
    Hex,
    Bin16,
    Bin32,
}

impl HeaderCoding {
    pub const fn checksum_bytes(self) -> usize {
        match self {
            Self::Bin32 => 4,
            Self::Hex | Self::Bin16 => 2,
        }
    }

    const fn marker(self) -> u8 {
        match self {
            Self::Hex => ZHEX,
            Self::Bin16 => ZBIN,
            Self::Bin32 => ZBIN32,
        }
    }
}

/// The marker that closes a data subpacket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SubpacketEnd {
    CrcE = 0x68,
    CrcG = 0x69,
    CrcQ = 0x6a,
    CrcW = 0x6b,
}

impl SubpacketEnd {
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x68 => Some(Self::CrcE),
            0x69 => Some(Self::CrcG),
            0x6a => Some(Self::CrcQ),
            0x6b => Some(Self::CrcW),
            _ => None,
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Whether a header (the next ZDATA of the batch, or the ZEOF) follows directly.
    pub const fn ends_frame(self) -> bool {
        matches!(self, Self::CrcE | Self::CrcW)
    }

    /// Whether the receiver owes the sender a ZACK for this subpacket.
    pub const fn expects_ack(self) -> bool {
        matches!(self, Self::CrcQ | Self::CrcW)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::CrcE => "ZCRCE",
            Self::CrcG => "ZCRCG",
            Self::CrcQ => "ZCRCQ",
            Self::CrcW => "ZCRCW",
        }
    }
}

/// One frame header: what it is, plus the four information bytes that mean different things for
/// different frame types.
///
/// The bytes are kept in wire order. A position is read little-endian from all four; flags live in
/// the last byte. Keeping one representation and converting at the edges is what stops the two
/// interpretations from drifting apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    frame: FrameType,
    info: [u8; 4],
}

impl Header {
    pub const fn new(frame: FrameType, info: [u8; 4]) -> Self {
        Self { frame, info }
    }

    /// A header whose information bytes are a byte offset into the file.
    pub const fn at(frame: FrameType, position: u32) -> Self {
        Self::new(frame, position.to_le_bytes())
    }

    pub const fn frame(self) -> FrameType {
        self.frame
    }

    pub const fn info(self) -> [u8; 4] {
        self.info
    }

    /// The offset a position-bearing frame carries (ZRPOS, ZDATA, ZACK, ZEOF).
    pub const fn position(self) -> u32 {
        u32::from_le_bytes(self.info)
    }

    /// The last information byte, where ZRINIT's capability bits and ZFILE's conversion option go.
    pub const fn flags(self) -> u8 {
        self.info[3]
    }

    pub const fn with_flags(mut self, flags: u8) -> Self {
        self.info[3] = flags;
        self
    }

    /// The receiver's input buffer size: zero means it can take an uninterrupted stream.
    pub const fn buffer_size(self) -> u16 {
        u16::from_le_bytes([self.info[0], self.info[1]])
    }

    pub const fn with_buffer_size(mut self, size: u16) -> Self {
        let bytes = size.to_le_bytes();
        self.info[0] = bytes[0];
        self.info[1] = bytes[1];
        self
    }

    /// An information byte by its protocol name: 0 is the last byte (ZF0), 3 the first (ZF3).
    pub const fn flag_byte(self, index: usize) -> u8 {
        self.info[3 - index]
    }

    /// Spells the header out. Receivers answer in hex; senders use binary in either CRC width.
    pub fn encoded(self, coding: HeaderCoding) -> Vec<u8> {
        let payload = self.payload();
        match coding {
            HeaderCoding::Hex => {
                let mut out = Vec::with_capacity(24);
                out.extend_from_slice(&[ZPAD, ZPAD, ZDLE, ZHEX]);
                for byte in payload {
                    push_hex(&mut out, byte);
                }
                for byte in crc16_wire_bytes(crc16(&payload)) {
                    push_hex(&mut out, byte);
                }
                // A CR LF footer is the most common spelling and every receiver tolerates it.
                out.extend_from_slice(b"\r\n");
                out
            }
            HeaderCoding::Bin16 | HeaderCoding::Bin32 => {
                let mut out = Vec::with_capacity(16);
                out.extend_from_slice(&[ZPAD, ZDLE, coding.marker()]);
                for byte in payload {
                    push_escaped(&mut out, byte);
                }
                let checksum: Vec<u8> = match coding {
                    HeaderCoding::Bin32 => crc32_wire_bytes(crc32(&payload)).to_vec(),
                    _ => crc16_wire_bytes(crc16(&payload)).to_vec(),
                };
                for byte in checksum {
                    push_escaped(&mut out, byte);
                }
                out
            }
        }
    }

    const fn payload(self) -> [u8; 5] {
        [
            self.frame.as_u8(),
            self.info[0],
            self.info[1],
            self.info[2],
            self.info[3],
        ]
    }
}

/// Escapes a run of bytes for the wire.
pub fn escape(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for &byte in data {
        push_escaped(&mut out, byte);
    }
    out
}

/// Reverses [`escape`]. A dangling escape character is a truncation, not an empty byte.
pub fn unescape(data: &[u8]) -> Result<Vec<u8>, ShellError> {
    let mut out = Vec::with_capacity(data.len());
    let mut index = 0;
    while index < data.len() {
        let byte = data[index];
        index += 1;
        if byte != ZDLE {
            out.push(byte);
            continue;
        }
        let Some(&escaped) = data.get(index) else {
            return Err(Fault::Truncated.into());
        };
        index += 1;
        match escaped {
            ZRUB0 => out.push(0x7f),
            ZRUB1 => out.push(0xff),
            other => out.push(other ^ 0x40),
        }
    }
    Ok(out)
}

/// Spells out one data subpacket: the payload, the frame end marker, and the checksum — which
/// covers the payload and the end marker but not the escape character in front of the marker.
pub fn encode_subpacket(data: &[u8], end: SubpacketEnd, coding: HeaderCoding) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 8);
    for &byte in data {
        push_escaped(&mut out, byte);
    }
    out.push(ZDLE);
    out.push(end.as_u8());
    let ender = [end.as_u8()];
    match coding {
        HeaderCoding::Bin32 => {
            let crc = crc32_update(crc32_update(0xffff_ffff, data), &ender) ^ 0xffff_ffff;
            for byte in crc32_wire_bytes(crc) {
                push_escaped(&mut out, byte);
            }
        }
        HeaderCoding::Hex | HeaderCoding::Bin16 => {
            let crc = crc16_update(crc16(data), &ender);
            for byte in crc16_wire_bytes(crc) {
                push_escaped(&mut out, byte);
            }
        }
    }
    out
}

fn push_escaped(out: &mut Vec<u8>, byte: u8) {
    if ESCAPED_BYTES.contains(&byte) {
        out.push(ZDLE);
        out.push(byte ^ 0x40);
    } else {
        out.push(byte);
    }
}

fn push_hex(out: &mut Vec<u8>, byte: u8) {
    // Hex headers allow only the lower-case digits; some receivers refuse upper case outright.
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    out.push(DIGITS[usize::from(byte >> 4)]);
    out.push(DIGITS[usize::from(byte & 0x0f)]);
}

fn decode_hex_pair(high: u8, low: u8) -> Result<u8, Fault> {
    fn digit(character: u8) -> Option<u8> {
        match character {
            b'0'..=b'9' => Some(character - b'0'),
            b'a'..=b'f' => Some(character - b'a' + 10),
            b'A'..=b'F' => Some(character - b'A' + 10),
            _ => None,
        }
    }
    let high = digit(high).ok_or_else(|| Fault::Malformed(format!("0x{high:02x} is not a hex digit")))?;
    let low = digit(low).ok_or_else(|| Fault::Malformed(format!("0x{low:02x} is not a hex digit")))?;
    Ok((high << 4) | low)
}

/// What went wrong below the state machines.
///
/// The distinction matters: a corrupt data subpacket can be answered with "resume from here",
/// while a broken stream or a malformed header ends the session. Collapsing them into one error
/// would make the receiver give up on an ordinary bit of line noise.
#[derive(Debug)]
pub(crate) enum Fault {
    /// The stream ended inside a frame.
    Truncated,
    /// The underlying reader or writer failed.
    Input(String),
    /// Bytes that claim to be a header are not one.
    Malformed(String),
    /// A data subpacket's checksum did not match; the end marker was still readable.
    Corrupt(String),
}

impl From<Fault> for ShellError {
    fn from(fault: Fault) -> Self {
        match fault {
            Fault::Input(detail) => ShellError::app_internal(detail),
            Fault::Truncated => ShellError::terminal_invalid("the byte stream ended in the middle of a ZMODEM frame"),
            Fault::Malformed(detail) | Fault::Corrupt(detail) => ShellError::terminal_invalid(detail),
        }
    }
}

impl std::fmt::Display for Fault {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => formatter.write_str("the byte stream ended in the middle of a ZMODEM frame"),
            Self::Input(detail) | Self::Malformed(detail) | Self::Corrupt(detail) => formatter.write_str(detail),
        }
    }
}

/// How much junk may precede a header before the reader decides this is not a ZMODEM stream.
const MAX_HEADER_SCAN: usize = 64 * 1024;

/// A data subpacket larger than this is not an honest 1024-byte block; the reader is desynchronised.
const MAX_SUBPACKET_BYTES: usize = 1024 * 1024;

/// Reads frames off an asynchronous byte stream.
pub(crate) struct FrameReader<R> {
    inner: R,
    buffer: VecDeque<u8>,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    pub(crate) fn new(inner: R) -> Self {
        Self {
            inner,
            buffer: VecDeque::new(),
        }
    }

    /// Reads the next header, skipping whatever noise precedes it.
    ///
    /// Returns `Ok(None)` only when the stream ended cleanly before another header began. A header
    /// that starts and then fails (checksum, width, frame type, truncation) is reported as an
    /// error once the scan budget runs out or the stream closes, so a caller can say what was
    /// wrong instead of "nothing arrived".
    pub(crate) async fn read_header(&mut self) -> Result<Option<(Header, HeaderCoding)>, Fault> {
        let mut scanned = 0usize;
        let mut last_malformed: Option<Fault> = None;
        loop {
            let Some(mut byte) = self.read_byte().await? else {
                return match last_malformed {
                    Some(fault) => Err(fault),
                    None => Ok(None),
                };
            };
            while byte != ZPAD {
                scanned += 1;
                if scanned > MAX_HEADER_SCAN {
                    return Err(last_malformed.unwrap_or_else(|| {
                        Fault::Malformed(format!(
                            "no ZMODEM frame header within {MAX_HEADER_SCAN} bytes of the stream"
                        ))
                    }));
                }
                let Some(next) = self.read_byte().await? else {
                    return match last_malformed {
                        Some(fault) => Err(fault),
                        None => Ok(None),
                    };
                };
                byte = next;
            }

            let mut next = self.read_byte().await?;
            while next == Some(ZPAD) {
                next = self.read_byte().await?;
            }
            let Some(next) = next else {
                return Err(Fault::Truncated);
            };
            if next != ZDLE {
                continue;
            }
            let Some(marker) = self.read_byte().await? else {
                return Err(Fault::Truncated);
            };
            let coding = match marker {
                ZHEX => HeaderCoding::Hex,
                ZBIN => HeaderCoding::Bin16,
                ZBIN32 => HeaderCoding::Bin32,
                _ => continue,
            };
            match self.read_header_payload(coding).await {
                Ok(header) => return Ok(Some((header, coding))),
                Err(Fault::Malformed(detail)) => {
                    last_malformed = Some(Fault::Malformed(detail));
                }
                Err(fault) => return Err(fault),
            }
        }
    }

    /// Reads one data subpacket. The checksum width follows the header that opened the frame.
    pub(crate) async fn read_subpacket(&mut self, coding: HeaderCoding) -> Result<(Vec<u8>, SubpacketEnd), Fault> {
        if coding == HeaderCoding::Hex {
            return Err(Fault::Malformed(
                "a data subpacket after a hex header is ASCII encoded, which this reader does not decode".to_owned(),
            ));
        }
        let mut data = Vec::new();
        let end = loop {
            let Some(byte) = self.read_byte().await? else {
                return Err(Fault::Truncated);
            };
            if byte != ZDLE {
                data.push(byte);
            } else {
                let Some(escaped) = self.read_byte().await? else {
                    return Err(Fault::Truncated);
                };
                if let Some(end) = SubpacketEnd::from_u8(escaped) {
                    break end;
                }
                match escaped {
                    ZRUB0 => data.push(0x7f),
                    ZRUB1 => data.push(0xff),
                    other => data.push(other ^ 0x40),
                }
            }
            if data.len() > MAX_SUBPACKET_BYTES {
                return Err(Fault::Malformed(format!(
                    "a data subpacket grew past {MAX_SUBPACKET_BYTES} bytes without an end marker"
                )));
            }
        };

        let mut checksum = [0u8; 4];
        for slot in checksum.iter_mut().take(coding.checksum_bytes()) {
            *slot = self.read_escaped_byte().await?.ok_or(Fault::Truncated)?;
        }
        let ender = [end.as_u8()];
        let (expected, found) = match coding {
            HeaderCoding::Bin32 => (
                u32::from_le_bytes(checksum),
                crc32_update(crc32_update(0xffff_ffff, &data), &ender) ^ 0xffff_ffff,
            ),
            HeaderCoding::Hex | HeaderCoding::Bin16 => (
                u32::from(u16::from_be_bytes([checksum[0], checksum[1]])),
                u32::from(crc16_update(crc16(&data), &ender)),
            ),
        };
        if expected != found {
            return Err(Fault::Corrupt(format!(
                "a data subpacket ({}) failed its CRC: the packet says 0x{expected:08x}, the bytes check out to 0x{found:08x}",
                end.name()
            )));
        }
        Ok((data, end))
    }

    async fn read_header_payload(&mut self, coding: HeaderCoding) -> Result<Header, Fault> {
        let mut payload = [0u8; 5];
        let mut checksum = [0u8; 4];
        match coding {
            HeaderCoding::Hex => {
                // Five payload bytes and one 16-bit checksum, two hex digits each. Reading more
                // would eat the CR LF footer and the first bytes of whatever comes next.
                let mut characters = [0u8; 14];
                for slot in &mut characters {
                    *slot = self.read_byte().await?.ok_or(Fault::Truncated)?;
                }
                for (index, slot) in payload.iter_mut().enumerate() {
                    *slot = decode_hex_pair(characters[index * 2], characters[index * 2 + 1])?;
                }
                for (index, slot) in checksum.iter_mut().take(2).enumerate() {
                    let offset = 10 + index * 2;
                    *slot = decode_hex_pair(characters[offset], characters[offset + 1])?;
                }
                self.skip_hex_footer().await?;
            }
            HeaderCoding::Bin16 | HeaderCoding::Bin32 => {
                for slot in &mut payload {
                    *slot = self.read_escaped_byte().await?.ok_or(Fault::Truncated)?;
                }
                for slot in checksum.iter_mut().take(coding.checksum_bytes()) {
                    *slot = self.read_escaped_byte().await?.ok_or(Fault::Truncated)?;
                }
            }
        }

        let frame = FrameType::from_u8(payload[0])
            .ok_or_else(|| Fault::Malformed(format!("0x{:02x} is not a ZMODEM frame type", payload[0])))?;
        let (expected, found) = match coding {
            HeaderCoding::Bin32 => (u32::from_le_bytes(checksum), crc32(&payload)),
            HeaderCoding::Hex | HeaderCoding::Bin16 => (
                u32::from(u16::from_be_bytes([checksum[0], checksum[1]])),
                u32::from(crc16(&payload)),
            ),
        };
        if expected != found {
            return Err(Fault::Malformed(format!(
                "the {} header CRC did not match: the header says 0x{expected:08x}, the bytes check out to 0x{found:08x}",
                frame.name()
            )));
        }
        let mut info = [0u8; 4];
        info.copy_from_slice(&payload[1..]);
        Ok(Header::new(frame, info))
    }

    /// A hex header may be followed by CR, LF (with or without the high bit) and an XON. Any other
    /// byte belongs to whatever comes next and is put back.
    ///
    /// At most two line-ending bytes are consumed. A third read would block on a live terminal
    /// stream that simply has no more bytes yet — the XON some senders add is skipped by the next
    /// header scan like any other noise.
    async fn skip_hex_footer(&mut self) -> Result<(), Fault> {
        for _ in 0..2 {
            let Some(byte) = self.read_byte().await? else {
                break;
            };
            match byte {
                0x0d | 0x0a | 0x8a => {}
                other => {
                    self.push_back(other);
                    break;
                }
            }
        }
        Ok(())
    }

    async fn read_escaped_byte(&mut self) -> Result<Option<u8>, Fault> {
        let Some(byte) = self.read_byte().await? else {
            return Ok(None);
        };
        if byte != ZDLE {
            return Ok(Some(byte));
        }
        let Some(escaped) = self.read_byte().await? else {
            return Err(Fault::Truncated);
        };
        match escaped {
            ZRUB0 => Ok(Some(0x7f)),
            ZRUB1 => Ok(Some(0xff)),
            0x68..=0x6b => Err(Fault::Malformed(
                "a data subpacket end marker appeared inside a frame header".to_owned(),
            )),
            other => Ok(Some(other ^ 0x40)),
        }
    }

    async fn read_byte(&mut self) -> Result<Option<u8>, Fault> {
        if let Some(byte) = self.buffer.pop_front() {
            return Ok(Some(byte));
        }
        let mut chunk = [0u8; 4096];
        loop {
            match self.inner.read(&mut chunk).await {
                Ok(0) => return Ok(None),
                Ok(read) => {
                    self.buffer.extend(&chunk[..read]);
                    return Ok(self.buffer.pop_front());
                }
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => {
                    return Err(Fault::Input(format!("the ZMODEM stream could not be read: {error}")));
                }
            }
        }
    }

    fn push_back(&mut self, byte: u8) {
        self.buffer.push_front(byte);
    }

    /// Hands back bytes that were read ahead but that no frame consumed, and empties the buffer.
    ///
    /// When a session ends, whatever followed its last frame — usually the shell prompt — is still
    /// here. Dropping it with the reader would make the prompt disappear from the screen.
    pub(crate) fn take_buffered(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buffer).into_iter().collect()
    }
}

/// Writes frames onto an asynchronous byte stream.
pub(crate) struct FrameWriter<W> {
    inner: W,
}

impl<W: AsyncWrite + Unpin> FrameWriter<W> {
    pub(crate) fn new(inner: W) -> Self {
        Self { inner }
    }

    pub(crate) async fn write_header(&mut self, header: Header, coding: HeaderCoding) -> Result<(), Fault> {
        self.write_all(&header.encoded(coding)).await
    }

    pub(crate) async fn write_subpacket(
        &mut self,
        data: &[u8],
        end: SubpacketEnd,
        coding: HeaderCoding,
    ) -> Result<(), Fault> {
        self.write_all(&encode_subpacket(data, end, coding)).await
    }

    pub(crate) async fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), Fault> {
        self.write_all(bytes).await
    }

    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), Fault> {
        // Flush after every frame: on a pty or a channel the far side is waiting for exactly this
        // frame, and a buffered write that waits for a full buffer would deadlock both ends.
        self.inner
            .write_all(bytes)
            .await
            .map_err(|error| Fault::Input(format!("the ZMODEM stream could not be written: {error}")))?;
        self.inner
            .flush()
            .await
            .map_err(|error| Fault::Input(format!("the ZMODEM stream could not be flushed: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn next_header<R: AsyncRead + Unpin>(reader: &mut FrameReader<R>) -> (Header, HeaderCoding) {
        reader
            .read_header()
            .await
            .expect("the header should read back")
            .expect("the stream should still hold a header")
    }

    #[test]
    fn escaping_and_unescaping_a_run_of_every_byte_round_trips() {
        let all: Vec<u8> = (0..=255u8).collect();
        let escaped = escape(&all);
        assert_eq!(unescape(&escaped).expect("well-formed escapes"), all);
    }

    #[test]
    fn the_eight_control_bytes_are_the_only_ones_that_grow() {
        for byte in 0..=255u8 {
            let escaped = escape(&[byte]);
            let must_escape = [0x10u8, 0x11, 0x13, 0x18, 0x90, 0x91, 0x93, 0x98].contains(&byte);
            assert_eq!(
                escaped.len() == 2,
                must_escape,
                "byte 0x{byte:02x} was escaped: {must_escape}"
            );
            if must_escape {
                assert_eq!(escaped[0], ZDLE);
                assert_eq!(escaped[1], byte ^ 0x40);
            }
        }
    }

    #[test]
    fn a_dangling_escape_character_is_reported_as_truncation() {
        let error = unescape(&[b'a', ZDLE]).expect_err("a dangling escape must not decode");
        assert!(error.detail().unwrap_or_default().contains("middle"));
    }

    #[test]
    fn a_position_fills_the_information_bytes_from_the_low_end() {
        let header = Header::at(FrameType::Eof, 0x0c5a);
        assert_eq!(header.info(), [0x5a, 0x0c, 0x00, 0x00]);
        assert_eq!(header.position(), 0x0c5a);
    }

    #[test]
    fn flags_live_in_the_last_information_byte() {
        let header = Header::new(FrameType::RInit, [0, 0, 0, 0]).with_flags(0x23);
        assert_eq!(header.info(), [0, 0, 0, 0x23]);
        assert_eq!(header.flags(), 0x23);
        assert_eq!(header.flag_byte(0), 0x23);
        assert_eq!(header.flag_byte(3), 0x00);

        let sized = header.with_buffer_size(4096);
        assert_eq!(sized.buffer_size(), 4096);
        assert_eq!(sized.flag_byte(0), 0x23);
    }

    #[test]
    fn a_hex_header_looks_like_the_captured_streams() {
        let encoded = Header::new(FrameType::RInit, [0, 0, 0, 0x23]).encoded(HeaderCoding::Hex);
        assert_eq!(&encoded[..4], b"**\x18B");
        assert_eq!(&encoded[4..], b"0100000023be50\r\n");
    }

    #[tokio::test]
    async fn a_hex_header_round_trips_through_the_reader() {
        let header = Header::at(FrameType::RPos, 1_234_567).with_flags(0x5a);
        let encoded = header.encoded(HeaderCoding::Hex);
        let mut reader = FrameReader::new(encoded.as_slice());
        let (decoded, coding) = next_header(&mut reader).await;
        assert_eq!(coding, HeaderCoding::Hex);
        assert_eq!(decoded, header);
    }

    #[tokio::test]
    async fn a_bin16_header_round_trips_through_the_reader() {
        let header = Header::at(FrameType::Data, 0x0c5a);
        let encoded = header.encoded(HeaderCoding::Bin16);
        let mut reader = FrameReader::new(encoded.as_slice());
        let (decoded, coding) = next_header(&mut reader).await;
        assert_eq!(coding, HeaderCoding::Bin16);
        assert_eq!(decoded, header);
    }

    #[tokio::test]
    async fn a_bin32_header_round_trips_through_the_reader() {
        let header = Header::new(FrameType::File, [0, 0, 0, 1]);
        let encoded = header.encoded(HeaderCoding::Bin32);
        let mut reader = FrameReader::new(encoded.as_slice());
        let (decoded, coding) = next_header(&mut reader).await;
        assert_eq!(coding, HeaderCoding::Bin32);
        assert_eq!(decoded, header);
    }

    #[tokio::test]
    async fn binary_headers_never_carry_a_raw_control_byte() {
        for coding in [HeaderCoding::Bin16, HeaderCoding::Bin32] {
            let header = Header::new(FrameType::RPos, [0x10, 0x11, 0x13, 0x18]);
            let encoded = header.encoded(coding);
            // The three-byte prefix is unescaped by definition; everything after it must not
            // contain any of the eight control bytes except as a ZDLE escape pair.
            let body = &encoded[3..];
            let mut index = 0;
            while index < body.len() {
                if body[index] == ZDLE {
                    index += 2;
                    continue;
                }
                assert!(
                    !ESCAPED_BYTES.contains(&body[index]),
                    "{coding:?} let 0x{:02x} through raw",
                    body[index]
                );
                index += 1;
            }
        }
    }

    #[tokio::test]
    async fn a_header_with_a_bad_checksum_is_refused() {
        let mut encoded = Header::at(FrameType::Data, 7).encoded(HeaderCoding::Bin16);
        let last = encoded.len() - 1;
        encoded[last] ^= 0xff;
        let mut reader = FrameReader::new(encoded.as_slice());
        let error = reader.read_header().await.expect_err("the CRC must be checked");
        assert!(
            error.to_string().contains("CRC"),
            "the error should name the checksum: {error}"
        );
    }

    #[tokio::test]
    async fn a_truncated_header_is_refused() {
        let encoded = Header::at(FrameType::Data, 1).encoded(HeaderCoding::Bin16);
        let mut reader = FrameReader::new(&encoded[..encoded.len() - 2]);
        let error = reader.read_header().await.expect_err("a cut header must not read");
        assert!(
            error.to_string().contains("middle"),
            "the error should say the frame was cut: {error}"
        );
    }

    #[tokio::test]
    async fn an_unknown_frame_type_is_refused() {
        let payload = [0x7fu8, 0, 0, 0, 0];
        let mut raw = vec![ZPAD, ZDLE, ZBIN];
        for byte in payload {
            push_escaped(&mut raw, byte);
        }
        for byte in crc16_wire_bytes(crc16(&payload)) {
            push_escaped(&mut raw, byte);
        }
        let mut reader = FrameReader::new(raw.as_slice());
        let error = reader.read_header().await.expect_err("0x7f is not a frame type");
        assert!(
            error.to_string().contains("frame type"),
            "the error should name the frame type: {error}"
        );
    }

    #[tokio::test]
    async fn junk_before_a_header_is_skipped() {
        let mut raw = b"rz\r\nwaiting to receive".to_vec();
        raw.extend_from_slice(&Header::new(FrameType::RqInit, [0; 4]).encoded(HeaderCoding::Hex));
        let mut reader = FrameReader::new(raw.as_slice());
        let (header, _) = next_header(&mut reader).await;
        assert_eq!(header.frame(), FrameType::RqInit);
    }

    #[tokio::test]
    async fn a_subpacket_round_trips_in_both_checksum_widths() {
        let data: Vec<u8> = (0..=255u8).chain([0x18, 0x10, 0x8d]).collect();
        for coding in [HeaderCoding::Bin16, HeaderCoding::Bin32] {
            let encoded = encode_subpacket(&data, SubpacketEnd::CrcW, coding);
            let mut reader = FrameReader::new(encoded.as_slice());
            let (decoded, end) = reader
                .read_subpacket(coding)
                .await
                .expect("a well-formed subpacket must read back");
            assert_eq!(decoded, data, "{coding:?} lost bytes");
            assert_eq!(end, SubpacketEnd::CrcW);
            assert!(
                reader.read_header().await.expect("clean end").is_none(),
                "the subpacket should be the whole stream"
            );
        }
    }

    #[tokio::test]
    async fn a_corrupt_subpacket_is_reported_with_its_checksum_and_end_marker() {
        for coding in [HeaderCoding::Bin16, HeaderCoding::Bin32] {
            let mut encoded = encode_subpacket(b"payload", SubpacketEnd::CrcE, coding);
            let last = encoded.len() - 1;
            encoded[last] ^= 0x01;
            let mut reader = FrameReader::new(encoded.as_slice());
            let fault = reader
                .read_subpacket(coding)
                .await
                .expect_err("the checksum must be checked");
            match fault {
                Fault::Corrupt(detail) => {
                    assert!(detail.contains("ZCRCE"), "the end marker should be named: {detail}");
                    assert!(detail.contains("CRC"), "unhelpful detail: {detail}");
                }
                other => panic!("expected a corruption, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn a_stream_that_ends_before_any_frame_is_a_clean_end() {
        let mut reader = FrameReader::new(b"rz\r\n".as_slice());
        assert!(reader.read_header().await.expect("no error").is_none());
    }

    #[test]
    fn subpacket_end_markers_classify_themselves() {
        assert!(SubpacketEnd::CrcE.ends_frame());
        assert!(!SubpacketEnd::CrcG.ends_frame());
        assert_eq!(SubpacketEnd::from_u8(0x6a), Some(SubpacketEnd::CrcQ));
        assert_eq!(SubpacketEnd::from_u8(0x6c), None);
        assert!(SubpacketEnd::CrcW.expects_ack());
        assert!(!SubpacketEnd::CrcG.expects_ack());
    }
}
