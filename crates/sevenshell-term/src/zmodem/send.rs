//! The sending half of the session: feed `rz` on the far side from a seekable byte source.
//!
//! The source is `AsyncRead + AsyncSeek` rather than a path, because resuming needs to jump back
//! and this layer must not know anything about local files. The size announced in ZFILE is the
//! authority: exactly that many bytes are sent, and a source that ends early is an error rather
//! than a short file the far side would silently accept.
//!
//! What is not here: timers. Every wait blocks until the peer sends the header it owes. A caller
//! that wants "give up after N seconds" wraps the whole session in its own timeout, which is the
//! only place that can also stop the underlying stream.

use std::io::{ErrorKind, SeekFrom};

use sevenshell_error::ShellError;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite};

use super::file::FileInfo;
use super::frame::{
    FrameReader, FrameType, FrameWriter, Header, HeaderCoding, SubpacketEnd, capabilities, file_option,
};
use super::{offset_of, peer_gave_up};

/// The block size every sender in the wild understands.
const CHUNK_BYTES: u64 = 1024;

/// How many times one file may be restarted before the session is declared broken.
const MAX_RESTARTS: usize = 10;

/// What the receiver said it can do, from its ZRINIT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiverCapabilities {
    flags: u8,
    buffer_size: u16,
}

impl ReceiverCapabilities {
    /// The receiver can check 32-bit CRCs, so data frames use them.
    pub fn accepts_crc32(self) -> bool {
        self.flags & capabilities::CANFC32 != 0
    }

    /// The receiver can take data while it writes to disk.
    pub fn overlaps_io(self) -> bool {
        self.flags & capabilities::CANOVIO != 0
    }

    pub fn full_duplex(self) -> bool {
        self.flags & capabilities::CANFDX != 0
    }

    /// Zero means the receiver can take an uninterrupted stream.
    pub fn buffer_size(self) -> u16 {
        self.buffer_size
    }
}

/// Sends files to a peer running `rz`.
pub struct Sender<R, W> {
    reader: FrameReader<R>,
    writer: FrameWriter<W>,
    coding: HeaderCoding,
    initiated: bool,
    finished: bool,
}

enum Outcome {
    Complete,
    Restart(u64),
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> Sender<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader: FrameReader::new(reader),
            writer: FrameWriter::new(writer),
            coding: HeaderCoding::Bin16,
            initiated: false,
            finished: false,
        }
    }

    /// Sends ZRQINIT and waits for the receiver's ZRINIT.
    pub async fn initiate(&mut self) -> Result<ReceiverCapabilities, ShellError> {
        self.writer
            .write_header(Header::new(FrameType::RqInit, [0; 4]), HeaderCoding::Hex)
            .await
            .map_err(ShellError::from)?;
        loop {
            let Some((header, coding)) = self.reader.read_header().await.map_err(ShellError::from)? else {
                return Err(ShellError::terminal_invalid(
                    "the byte stream ended before the receiver answered ZRQINIT",
                ));
            };
            match header.frame() {
                FrameType::RInit => {
                    self.initiated = true;
                    // The receiver advertises 32-bit CRC support in the last information byte.
                    self.coding = if header.flags() & capabilities::CANFC32 != 0 {
                        HeaderCoding::Bin32
                    } else {
                        HeaderCoding::Bin16
                    };
                    return Ok(ReceiverCapabilities {
                        flags: header.flags(),
                        buffer_size: header.buffer_size(),
                    });
                }
                FrameType::SInit => {
                    let _ = self.reader.read_subpacket(coding).await.map_err(ShellError::from)?;
                    self.writer
                        .write_header(Header::new(FrameType::Ack, [0; 4]), HeaderCoding::Hex)
                        .await
                        .map_err(ShellError::from)?;
                }
                FrameType::Abort | FrameType::Can | FrameType::FError => return Err(peer_gave_up(header)),
                // A stray ZACK or a repeated ZRQINIT does not change what this side is waiting for.
                _ => {}
            }
        }
    }

    /// Sends one file.
    ///
    /// Returns the number of bytes the receiver confirmed. Zero means the receiver answered ZSKIP
    /// and nothing was transferred; that is a normal outcome, not an error.
    pub async fn send_file<S>(&mut self, file: &FileInfo, source: &mut S) -> Result<u64, ShellError>
    where
        S: AsyncRead + AsyncSeek + Unpin,
    {
        if file.size > u64::from(u32::MAX) {
            return Err(ShellError::terminal_invalid(
                "the file is larger than the four gigabytes a ZMODEM offset can address",
            ));
        }
        // Validated before anything is written, so a name this side would refuse never reaches the
        // far side at all (and its refusal happens before the peer is told a file is coming).
        let info = file.encoded()?;
        if !self.initiated {
            self.initiate().await?;
        }

        self.writer
            .write_header(
                Header::new(FrameType::File, [0, 0, 0, file_option::ZCBIN]),
                HeaderCoding::Bin16,
            )
            .await
            .map_err(ShellError::from)?;
        self.writer
            .write_subpacket(&info, SubpacketEnd::CrcW, HeaderCoding::Bin16)
            .await
            .map_err(ShellError::from)?;

        let Some(mut offset) = self.await_start(file.size).await? else {
            return Ok(0);
        };
        let mut restarts = 0usize;
        loop {
            let end = self.send_frame(source, file.size, offset).await?;
            self.writer
                .write_header(Header::at(FrameType::Eof, offset_of(end)?), self.coding)
                .await
                .map_err(ShellError::from)?;
            match self.await_outcome(offset).await? {
                Outcome::Complete => return Ok(file.size),
                Outcome::Restart(position) => {
                    restarts += 1;
                    if restarts > MAX_RESTARTS {
                        return Err(ShellError::terminal_invalid(format!(
                            "the receiver restarted the file {restarts} times; giving up"
                        )));
                    }
                    offset = position;
                }
            }
        }
    }

    /// Sends ZFIN and agrees to the end of the session.
    pub async fn finish(&mut self) -> Result<(), ShellError> {
        if !self.initiated || self.finished {
            return Ok(());
        }
        self.writer
            .write_header(Header::new(FrameType::Fin, [0; 4]), HeaderCoding::Hex)
            .await
            .map_err(ShellError::from)?;
        loop {
            let Some((header, _)) = self.reader.read_header().await.map_err(ShellError::from)? else {
                // Some receivers close the stream the moment the last file is acknowledged; the
                // goodbye is a courtesy and its absence loses nothing.
                self.finished = true;
                return Ok(());
            };
            match header.frame() {
                FrameType::Fin => {
                    self.finished = true;
                    // The protocol's closing courtesy, and the only write whose failure is
                    // expected: the far side has already said goodbye.
                    let _ = self.writer.write_bytes(b"OO").await;
                    return Ok(());
                }
                FrameType::Abort | FrameType::Can => return Err(peer_gave_up(header)),
                _ => {}
            }
        }
    }

    /// Waits for the position to start at. `None` means the receiver skipped the file.
    async fn await_start(&mut self, size: u64) -> Result<Option<u64>, ShellError> {
        loop {
            let Some((header, _)) = self.reader.read_header().await.map_err(ShellError::from)? else {
                return Err(ShellError::terminal_invalid(
                    "the byte stream ended before the receiver said where to start",
                ));
            };
            match header.frame() {
                FrameType::RPos => {
                    let position = u64::from(header.position());
                    if position > size {
                        return Err(ShellError::terminal_invalid(format!(
                            "the receiver asked to start at byte {position} of a {size}-byte file"
                        )));
                    }
                    return Ok(Some(position));
                }
                FrameType::Skip => return Ok(None),
                FrameType::Abort | FrameType::Can | FrameType::FError => return Err(peer_gave_up(header)),
                _ => {}
            }
        }
    }

    /// Waits for the receiver's verdict on the frame just sent.
    async fn await_outcome(&mut self, frame_start: u64) -> Result<Outcome, ShellError> {
        loop {
            let Some((header, _)) = self.reader.read_header().await.map_err(ShellError::from)? else {
                return Err(ShellError::terminal_invalid(
                    "the byte stream ended before the receiver confirmed the file",
                ));
            };
            match header.frame() {
                FrameType::RInit => return Ok(Outcome::Complete),
                FrameType::RPos => return Ok(Outcome::Restart(u64::from(header.position()))),
                // A NAK says "the last thing was corrupt" without naming a position. Restarting at
                // the beginning of the frame is safe: the receiver's checksum decides what it
                // already has, and it will ask for a later offset if it kept more.
                FrameType::Nak => return Ok(Outcome::Restart(frame_start)),
                FrameType::Abort | FrameType::Can | FrameType::FError => return Err(peer_gave_up(header)),
                _ => {}
            }
        }
    }

    /// Sends the ZDATA header and every subpacket of one frame, starting at `offset`.
    async fn send_frame<S>(&mut self, source: &mut S, size: u64, offset: u64) -> Result<u64, ShellError>
    where
        S: AsyncRead + AsyncSeek + Unpin,
    {
        self.writer
            .write_header(Header::at(FrameType::Data, offset_of(offset)?), self.coding)
            .await
            .map_err(ShellError::from)?;
        source.seek(SeekFrom::Start(offset)).await.map_err(|error| {
            ShellError::app_internal(format!("the file could not be sought to byte {offset}: {error}"))
        })?;

        let mut position = offset;
        if position == size {
            // An empty file still ends its frame with a (zero-length) subpacket; a bare header
            // with nothing after it would leave the receiver waiting for data that never comes.
            self.writer
                .write_subpacket(&[], SubpacketEnd::CrcE, self.coding)
                .await
                .map_err(ShellError::from)?;
            return Ok(position);
        }

        let mut buffer = vec![0u8; CHUNK_BYTES as usize];
        while position < size {
            let want = usize::try_from((size - position).min(CHUNK_BYTES)).expect("a 1 KiB block fits a usize");
            fill(source, &mut buffer[..want]).await?;
            position += want as u64;
            let end = if position == size {
                SubpacketEnd::CrcE
            } else {
                SubpacketEnd::CrcG
            };
            self.writer
                .write_subpacket(&buffer[..want], end, self.coding)
                .await
                .map_err(ShellError::from)?;
        }
        Ok(position)
    }

    /// Bytes that were read from the stream but belong to whatever followed the session — whatever
    /// `rz` printed on its way out. Handed back instead of dropped with the sender.
    pub(crate) fn take_buffered(&mut self) -> Vec<u8> {
        self.reader.take_buffered()
    }
}

/// Fills `buffer` completely; a source that ends first is shorter than its own header claimed.
async fn fill<S>(source: &mut S, buffer: &mut [u8]) -> Result<(), ShellError>
where
    S: AsyncRead + Unpin,
{
    let mut filled = 0;
    while filled < buffer.len() {
        match source.read(&mut buffer[filled..]).await {
            Ok(0) => {
                return Err(ShellError::terminal_invalid(
                    "the file ended before the length announced in its ZFILE header",
                ));
            }
            Ok(read) => filled += read,
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => {
                return Err(ShellError::app_internal(format!(
                    "the file could not be read while sending: {error}"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    fn rinit(flags: u8) -> Vec<u8> {
        Header::new(FrameType::RInit, [0; 4])
            .with_flags(flags)
            .encoded(HeaderCoding::Hex)
    }

    fn zpos(position: u32) -> Vec<u8> {
        Header::at(FrameType::RPos, position).encoded(HeaderCoding::Hex)
    }

    fn zfin() -> Vec<u8> {
        Header::new(FrameType::Fin, [0; 4]).encoded(HeaderCoding::Hex)
    }

    async fn run(file: &FileInfo, data: &[u8], script: &[u8]) -> (u64, Vec<u8>) {
        let (mut peer, side) = tokio::io::duplex(1024 * 1024);
        peer.write_all(script).await.expect("the script fits");
        let (reader, writer) = tokio::io::split(side);
        let mut sender = Sender::new(reader, writer);
        let mut source = Cursor::new(data.to_vec());
        let sent = sender.send_file(file, &mut source).await.expect("the file should send");
        sender.finish().await.expect("the session should end cleanly");
        drop(sender);
        let mut output = Vec::new();
        peer.read_to_end(&mut output).await.expect("output is readable");
        (sent, output)
    }

    fn info(name: &str, size: u64) -> FileInfo {
        FileInfo {
            name: name.to_owned(),
            size,
            modified: Some(1_760_000_000),
            mode: Some(0o100644),
        }
    }

    #[tokio::test]
    async fn a_file_is_announced_sent_and_ended_in_the_order_the_protocol_asks_for() {
        let payload = b"hello, zmodem";
        let file = info("greeting.txt", payload.len() as u64);
        let mut script = rinit(0x23);
        script.extend(zpos(0));
        script.extend(rinit(0x23));
        script.extend(zfin());

        let (sent, output) = run(&file, payload, &script).await;
        assert_eq!(sent, payload.len() as u64);

        let mut reader = FrameReader::new(output.as_slice());
        let (hello, _) = reader.read_header().await.expect("parse").expect("a header");
        assert_eq!(hello.frame(), FrameType::RqInit);

        let (announce, coding) = reader.read_header().await.expect("parse").expect("a header");
        assert_eq!(announce.frame(), FrameType::File);
        assert_eq!(coding, HeaderCoding::Bin16);
        assert_eq!(announce.flags(), file_option::ZCBIN);
        let (announced, end) = reader.read_subpacket(coding).await.expect("the info packet");
        assert_eq!(announced, file.encoded().expect("a safe name"));
        assert_eq!(end, SubpacketEnd::CrcW);
        assert_eq!(FileInfo::parse(&announced).expect("parses back"), file);

        let (data_header, data_coding) = reader.read_header().await.expect("parse").expect("a header");
        assert_eq!(data_header.frame(), FrameType::Data);
        assert_eq!(data_header.position(), 0);
        // The receiver advertised 32-bit checksums, so the data frame must use them.
        assert_eq!(data_coding, HeaderCoding::Bin32);

        let mut received = Vec::new();
        loop {
            let (chunk, end) = reader.read_subpacket(data_coding).await.expect("a data packet");
            received.extend_from_slice(&chunk);
            if end.ends_frame() {
                assert_eq!(end, SubpacketEnd::CrcE);
                break;
            }
            assert_eq!(end, SubpacketEnd::CrcG);
        }
        assert_eq!(received, payload, "the payload must survive escaping");

        let (eof, _) = reader.read_header().await.expect("parse").expect("a header");
        assert_eq!(eof.frame(), FrameType::Eof);
        assert_eq!(eof.position(), payload.len() as u32);

        let (last, _) = reader.read_header().await.expect("parse").expect("a header");
        assert_eq!(last.frame(), FrameType::Fin);
        assert!(output.ends_with(b"OO"), "the session should close with OO");
    }

    #[tokio::test]
    async fn a_receiver_without_32_bit_crc_support_gets_16_bit_data_frames() {
        let payload = b"two bytes";
        let file = info("x.bin", payload.len() as u64);
        let mut script = rinit(0x03);
        script.extend(zpos(0));
        script.extend(rinit(0x03));
        script.extend(zfin());

        let (_, output) = run(&file, payload, &script).await;
        let mut reader = FrameReader::new(output.as_slice());
        let _ = reader.read_header().await.expect("parse");
        let _ = reader.read_header().await.expect("parse");
        let _ = reader.read_subpacket(HeaderCoding::Bin16).await.expect("info");
        let (header, coding) = reader.read_header().await.expect("parse").expect("a header");
        assert_eq!(header.frame(), FrameType::Data);
        assert_eq!(coding, HeaderCoding::Bin16);
    }

    #[tokio::test]
    async fn a_receiver_that_skips_the_file_gets_no_data_at_all() {
        let payload = b"never sent";
        let file = info("refused.txt", payload.len() as u64);
        let mut script = rinit(0x23);
        script.extend(Header::new(FrameType::Skip, [0; 4]).encoded(HeaderCoding::Hex));
        script.extend(zfin());

        let (sent, output) = run(&file, payload, &script).await;
        assert_eq!(sent, 0, "a skipped file transfers nothing");

        let mut reader = FrameReader::new(output.as_slice());
        let mut frames = Vec::new();
        while let Some((header, _)) = reader.read_header().await.expect("parse") {
            frames.push(header.frame());
        }
        assert_eq!(
            frames,
            vec![FrameType::RqInit, FrameType::File, FrameType::Fin],
            "no ZDATA may follow a skipped file"
        );
    }

    #[tokio::test]
    async fn a_restart_request_resends_from_the_requested_offset() {
        let payload = b"0123456789";
        let file = info("resume.bin", payload.len() as u64);
        let mut script = rinit(0x23);
        script.extend(zpos(0));
        script.extend(zpos(4));
        script.extend(rinit(0x23));
        script.extend(zfin());

        let (sent, output) = run(&file, payload, &script).await;
        assert_eq!(sent, payload.len() as u64);

        let mut reader = FrameReader::new(output.as_slice());
        let _ = reader.read_header().await.expect("parse");
        let _ = reader.read_header().await.expect("parse");
        let _ = reader.read_subpacket(HeaderCoding::Bin16).await.expect("info");

        let mut starts = Vec::new();
        let mut frames = Vec::new();
        loop {
            let Some((header, coding)) = reader.read_header().await.expect("parse") else {
                break;
            };
            if header.frame() == FrameType::Data {
                starts.push(header.position());
                let mut chunk = Vec::new();
                loop {
                    let (part, end) = reader.read_subpacket(coding).await.expect("data");
                    chunk.extend_from_slice(&part);
                    if end.ends_frame() {
                        break;
                    }
                }
                frames.push(chunk);
            }
        }
        assert_eq!(starts, vec![0, 4], "the second frame must start at the requested byte");
        assert_eq!(frames[0], payload);
        assert_eq!(frames[1], &payload[4..]);
    }

    #[tokio::test]
    async fn a_file_too_large_for_a_32_bit_offset_is_refused_before_anything_is_written() {
        let (mut peer, side) = tokio::io::duplex(1024);
        let (reader, writer) = tokio::io::split(side);
        let mut sender = Sender::new(reader, writer);
        let mut source = Cursor::new(vec![0u8; 16]);
        let file = info("huge.img", u64::from(u32::MAX) + 1);
        let error = sender.send_file(&file, &mut source).await.expect_err("must be refused");
        assert!(error.to_string().contains("four gigabytes"), "unhelpful error: {error}");
        drop(sender);
        let mut output = Vec::new();
        peer.read_to_end(&mut output).await.expect("readable");
        assert!(output.is_empty(), "nothing should have reached the wire");
    }

    #[tokio::test]
    async fn child_frames_report_what_the_receiver_offered() {
        let (mut peer, side) = tokio::io::duplex(1024 * 1024);
        peer.write_all(&rinit(0x23)).await.expect("the script fits");
        let (reader, writer) = tokio::io::split(side);
        let mut sender = Sender::new(reader, writer);
        let capabilities = sender.initiate().await.expect("the receiver answered");
        assert!(capabilities.accepts_crc32());
        assert!(capabilities.overlaps_io());
        assert!(capabilities.full_duplex());
        assert_eq!(capabilities.buffer_size(), 0);
    }
}
