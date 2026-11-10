//! ZMODEM (`rz`/`sz`) framing and the two session state machines.
//!
//! This module is the protocol only. It reads and writes bytes through `AsyncRead`/`AsyncWrite`
//! and knows nothing about terminals, SSH channels or local paths: a caller hands it the stream
//! the shell is using, picks up the events a receiver produces, and writes those bytes wherever a
//! person said they should go. Terminal output interception, the file dialogs and the decision of
//! where a download lands belong above this layer.
//!
//! Shape of the API:
//!
//! * [`Receiver`](crate::zmodem::Receiver) answers a peer running `sz` and yields
//!   [`ReceiveEvent`](crate::zmodem::ReceiveEvent)s: a proposed file, chunks whose checksums passed,
//!   the end of a file, and skips for names that are not usable.
//! * [`Sender`](crate::zmodem::Sender) feeds a peer running `rz` from an `AsyncRead + AsyncSeek`
//!   source, so a resume can jump back without this crate touching the filesystem.
//! * [`Detector`](crate::zmodem::Detector) is the terminal-stream side: it recognises a session
//!   inside ordinary output, routes protocol bytes to the machine, reports replies to write into the
//!   terminal's input, and says when the stream is plain text again.
//! * [`Header`](crate::zmodem::Header), [`FrameType`](crate::zmodem::FrameType),
//!   [`SubpacketEnd`](crate::zmodem::SubpacketEnd),
//!   [`escape`](crate::zmodem::escape)/[`unescape`](crate::zmodem::unescape),
//!   [`encode_subpacket`](crate::zmodem::encode_subpacket), [`crc16`](crate::zmodem::crc16) and
//!   [`crc32`](crate::zmodem::crc32) are the frame layer, exposed because a future transport may
//!   want to place frames itself — and because the tests pin their bytes.
//!
//! The links above are spelled out because this module has an outer doc comment on its declaration
//! in `lib.rs`, and rustdoc resolves the inner block in that scope rather than in this file.
//!
//! Deliberate boundaries, so nobody has to guess:
//!
//! * No timers and no automatic retransmission on silence. Waiting is as long as the stream stays
//!   open; a caller that wants a deadline wraps the session in its own timeout. A corrupt block
//!   *is* answered with ZRPOS and the peer may resend, but a peer that stays silent is not poked.
//!   The detector never blocks on its own, so the deadline lives with the caller that also owns the
//!   timer — and [`Detector::abort`](crate::zmodem::Detector::abort) is the explicit way out.
//! * No ASCII-encoded data subpackets (the 7-bit fallback that can follow a hex header) and no
//!   RLE-compressed frames. A real `sz` uses binary headers for both.
//! * A sender that restarts below what was already delivered is refused instead of silently
//!   producing a corrupt file: an `AsyncWrite` sink cannot un-write.
//! * **No live `rz`/`sz` test here.** The lab image has no `lrzsz`; these tests drive the state
//!   machines against scripted peers. A real terminal session is the manual acceptance step for
//!   that part.

mod bridge;
mod crc;
mod file;
mod frame;
mod intercept;
mod receive;
mod send;

use sevenshell_error::ShellError;

pub use crc::{crc16, crc32};
pub use file::FileInfo;
pub use frame::{
    FrameType, Header, HeaderCoding, SubpacketEnd, ZBIN, ZBIN32, ZDLE, ZHEX, ZPAD, capabilities, encode_subpacket,
    escape, file_option, unescape,
};
pub use intercept::{Detector, DetectorEvent, Direction, EndReason};
pub use receive::{ReceiveEvent, Receiver};
pub use send::{ReceiverCapabilities, Sender};

/// Turns a byte count into the 32-bit offset the protocol carries.
pub(crate) fn offset_of(received: u64) -> Result<u32, ShellError> {
    u32::try_from(received).map_err(|_| {
        ShellError::terminal_invalid("the file is larger than the four gigabytes a ZMODEM offset can address")
    })
}

/// A frame that the protocol does not allow at this point in the session.
pub(crate) fn unexpected(frame: FrameType, when: &str) -> ShellError {
    ShellError::terminal_invalid(format!(
        "{} arrived {when}, where the protocol expects something else",
        frame.name()
    ))
}

/// The peer said it is not continuing (ZABORT, ZCAN or ZFERR).
pub(crate) fn peer_gave_up(header: Header) -> ShellError {
    ShellError::terminal_invalid(format!(
        "the far side ended the transfer with {}",
        header.frame().name()
    ))
}
