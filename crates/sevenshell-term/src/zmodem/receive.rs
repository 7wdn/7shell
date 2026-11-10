//! The receiving half of the session: answer `sz` on the far side and hand the bytes upward.
//!
//! The receiver never decides where a file lands. It turns the wire into four events — a proposed
//! file, chunks of file data that have already passed their checksum, the end of a file, and (for
//! a refused name) a skip — and the caller writes the bytes somewhere. That keeps local paths out
//! of this crate, which is the same rule the rest of the terminal layer follows.
//!
//! Recovery is deliberately modest. A corrupt data subpacket is answered with ZRPOS at the last
//! byte this side actually handed out, and the reader then resynchronises on the sender's next
//! header by scanning. A sender that goes *backwards* (restarts below what was already delivered)
//! is an error: no sink can un-write bytes, so pretending to resume would corrupt the file
//! silently. Retransmission timeouts are the caller's business; this layer waits as long as the
//! stream stays open.

use sevenshell_error::ShellError;
use tokio::io::{AsyncRead, AsyncWrite};

use super::file::{FileInfo, proposed_name};
use super::frame::{Fault, FrameReader, FrameType, FrameWriter, Header, HeaderCoding, capabilities};
use super::{offset_of, peer_gave_up, unexpected};

/// What the receiver reports as it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiveEvent {
    /// The sender offered this file and the receiver told it to start at zero. Data events follow.
    Offer(FileInfo),
    /// Bytes that passed their subpacket checksum, in order. The caller appends them.
    Data(Vec<u8>),
    /// Every byte the sender announced arrived: this many bytes were delivered.
    Complete(FileInfo),
    /// The sender offered a file this side will not accept (an unsafe name, for example). The
    /// session continues so a batch keeps going.
    Skipped { name: String, reason: String },
}

/// Receives files from a peer running `sz`.
pub struct Receiver<R, W> {
    reader: FrameReader<R>,
    writer: FrameWriter<W>,
    started: bool,
    finished: bool,
    incoming: Option<Incoming>,
}

struct Incoming {
    info: FileInfo,
    received: u64,
    coding: HeaderCoding,
    in_frame: bool,
}

/// One step of the state machine.
enum Step {
    Event(ReceiveEvent),
    Continue,
    End,
    Complete { info: FileInfo, received: u64 },
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> Receiver<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader: FrameReader::new(reader),
            writer: FrameWriter::new(writer),
            started: false,
            finished: false,
            incoming: None,
        }
    }

    /// The next thing that happened, or `None` once the peer has ended the session.
    ///
    /// A file that the caller never got a `Complete` for is a file that did not arrive; a clean
    /// `None` before that means the peer hung up mid-batch.
    pub async fn next(&mut self) -> Result<Option<ReceiveEvent>, ShellError> {
        if self.finished {
            return Ok(None);
        }
        loop {
            match self.step().await? {
                Step::Event(event) => return Ok(Some(event)),
                Step::Continue => {}
                Step::End => {
                    self.finished = true;
                    return Ok(None);
                }
                Step::Complete { info, received } => {
                    self.incoming = None;
                    if info.size != received {
                        return Err(ShellError::terminal_invalid(format!(
                            "the transfer of \"{}\" ended after {received} bytes but its header announced {}",
                            info.name, info.size
                        )));
                    }
                    self.send_rinit().await?;
                    return Ok(Some(ReceiveEvent::Complete(info)));
                }
            }
        }
    }

    async fn step(&mut self) -> Result<Step, ShellError> {
        if self.incoming.is_some() {
            self.step_in_file().await
        } else {
            self.step_between_files().await
        }
    }

    async fn step_in_file(&mut self) -> Result<Step, ShellError> {
        let incoming = self.incoming.as_mut().expect("the caller checked");
        if incoming.in_frame {
            return match self.reader.read_subpacket(incoming.coding).await {
                Ok((data, end)) => {
                    incoming.in_frame = !end.ends_frame();
                    if end.expects_ack() {
                        let ack = Header::at(FrameType::Ack, offset_of(incoming.received)?);
                        self.writer
                            .write_header(ack, HeaderCoding::Hex)
                            .await
                            .map_err(ShellError::from)?;
                    }
                    if data.is_empty() {
                        Ok(Step::Continue)
                    } else {
                        incoming.received += data.len() as u64;
                        Ok(Step::Event(ReceiveEvent::Data(data)))
                    }
                }
                Err(Fault::Corrupt { .. }) => {
                    // The subpacket is gone, so ask for it again from the first byte this side has
                    // not handed out. Whether the sender stops mid-frame or finishes it, the next
                    // header it writes is what this reader scans for — anything in between is data
                    // that was already discarded.
                    let resume = Header::at(FrameType::RPos, offset_of(incoming.received)?);
                    self.writer
                        .write_header(resume, HeaderCoding::Hex)
                        .await
                        .map_err(ShellError::from)?;
                    incoming.in_frame = false;
                    Ok(Step::Continue)
                }
                Err(fault) => Err(ShellError::from(fault)),
            };
        }

        let Some((header, coding)) = self.reader.read_header().await.map_err(ShellError::from)? else {
            return Err(ShellError::terminal_invalid(
                "the byte stream ended in the middle of a file",
            ));
        };
        match header.frame() {
            FrameType::Data => {
                let position = u64::from(header.position());
                incoming.coding = coding;
                incoming.in_frame = true;
                if position == incoming.received {
                    Ok(Step::Continue)
                } else if position > incoming.received {
                    // The sender jumped past what this side still needs (usually because it had
                    // already queued a block before our ZRPOS reached it). Drop this frame and
                    // ask again from the first missing byte.
                    let resume = Header::at(FrameType::RPos, offset_of(incoming.received)?);
                    self.writer
                        .write_header(resume, HeaderCoding::Hex)
                        .await
                        .map_err(ShellError::from)?;
                    incoming.in_frame = false;
                    Ok(Step::Continue)
                } else {
                    Err(ShellError::terminal_invalid(format!(
                        "the sender restarted at byte {position} after {} bytes had already been delivered",
                        incoming.received
                    )))
                }
            }
            FrameType::Eof => {
                let position = u64::from(header.position());
                if position != incoming.received {
                    let resume = Header::at(FrameType::RPos, offset_of(incoming.received)?);
                    self.writer
                        .write_header(resume, HeaderCoding::Hex)
                        .await
                        .map_err(ShellError::from)?;
                    return Ok(Step::Continue);
                }
                Ok(Step::Complete {
                    info: incoming.info.clone(),
                    received: incoming.received,
                })
            }
            FrameType::RqInit => Err(ShellError::terminal_invalid(
                "the sender restarted the session in the middle of a file",
            )),
            FrameType::Fin => Err(ShellError::terminal_invalid(
                "the sender ended the session in the middle of a file",
            )),
            FrameType::Abort | FrameType::Can | FrameType::FError => Err(peer_gave_up(header)),
            FrameType::Ack | FrameType::Nak => Ok(Step::Continue),
            other => Err(unexpected(other, "while a file was in flight")),
        }
    }

    async fn step_between_files(&mut self) -> Result<Step, ShellError> {
        let Some((header, coding)) = self.reader.read_header().await.map_err(ShellError::from)? else {
            // A peer that closes the stream after the last file has effectively ended the session.
            return Ok(Step::End);
        };
        match header.frame() {
            FrameType::RqInit => {
                self.started = true;
                self.send_rinit().await?;
                Ok(Step::Continue)
            }
            FrameType::SInit => {
                // The optional send-initialisation frame carries an attention string this side has
                // no use for; it still has to be consumed so the next frame starts where it should.
                let (_, _) = self.reader.read_subpacket(coding).await.map_err(ShellError::from)?;
                self.writer
                    .write_header(Header::new(FrameType::Ack, [0; 4]), HeaderCoding::Hex)
                    .await
                    .map_err(ShellError::from)?;
                Ok(Step::Continue)
            }
            FrameType::File => {
                if !self.started {
                    return Err(ShellError::terminal_invalid(
                        "a file was offered before the ZMODEM session started",
                    ));
                }
                let (payload, _) = self.reader.read_subpacket(coding).await.map_err(ShellError::from)?;
                match FileInfo::parse(&payload) {
                    Ok(info) => {
                        self.writer
                            .write_header(Header::at(FrameType::RPos, 0), HeaderCoding::Hex)
                            .await
                            .map_err(ShellError::from)?;
                        self.incoming = Some(Incoming {
                            info: info.clone(),
                            received: 0,
                            coding: HeaderCoding::Bin16,
                            in_frame: false,
                        });
                        Ok(Step::Event(ReceiveEvent::Offer(info)))
                    }
                    Err(error) => {
                        // A name this side cannot use is a reason to skip one file, not to tear
                        // down a batch; tell the sender to skip it and keep reading.
                        self.writer
                            .write_header(Header::new(FrameType::Skip, [0; 4]), HeaderCoding::Hex)
                            .await
                            .map_err(ShellError::from)?;
                        Ok(Step::Event(ReceiveEvent::Skipped {
                            name: proposed_name(&payload),
                            reason: error
                                .detail()
                                .unwrap_or("the file information packet was not usable")
                                .to_owned(),
                        }))
                    }
                }
            }
            FrameType::Fin => {
                self.writer
                    .write_header(Header::new(FrameType::Fin, [0; 4]), HeaderCoding::Hex)
                    .await
                    .map_err(ShellError::from)?;
                Ok(Step::End)
            }
            FrameType::Abort | FrameType::Can | FrameType::FError => Err(peer_gave_up(header)),
            FrameType::Ack | FrameType::Nak => Ok(Step::Continue),
            FrameType::RInit => Err(ShellError::terminal_invalid(
                "the far side is a receiver too: both ends are waiting for a file",
            )),
            other => Err(unexpected(other, "between files")),
        }
    }

    async fn send_rinit(&mut self) -> Result<(), ShellError> {
        // Full duplex, writes while receiving, and 32-bit checksums: the three things this side
        // really does. A zero buffer size means the far side may stream without pausing.
        let flags = capabilities::CANFDX | capabilities::CANOVIO | capabilities::CANFC32;
        let header = Header::new(FrameType::RInit, [0; 4]).with_flags(flags);
        self.writer
            .write_header(header, HeaderCoding::Hex)
            .await
            .map_err(ShellError::from)
    }

    /// Bytes that were read from the stream but belong to whatever followed the session — the shell
    /// prompt that arrived with ZFIN, usually. Handed back instead of dropped with the receiver.
    pub(crate) fn take_buffered(&mut self) -> Vec<u8> {
        self.reader.take_buffered()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zmodem::frame::{SubpacketEnd, encode_subpacket, file_option};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn rqinit() -> Vec<u8> {
        Header::new(FrameType::RqInit, [0; 4]).encoded(HeaderCoding::Hex)
    }

    fn zfile(info: &FileInfo) -> Vec<u8> {
        let mut out = Header::new(FrameType::File, [0, 0, 0, file_option::ZCBIN]).encoded(HeaderCoding::Bin16);
        out.extend(encode_subpacket(
            &info.encoded().expect("a safe name"),
            SubpacketEnd::CrcW,
            HeaderCoding::Bin16,
        ));
        out
    }

    fn zdata(position: u32, chunks: &[(&[u8], SubpacketEnd)]) -> Vec<u8> {
        let mut out = Header::at(FrameType::Data, position).encoded(HeaderCoding::Bin32);
        for (data, end) in chunks {
            out.extend(encode_subpacket(data, *end, HeaderCoding::Bin32));
        }
        out
    }

    /// Runs the receiver against a pre-queued peer script and returns the events plus the headers
    /// this side wrote back.
    async fn run(script: &[u8]) -> (Vec<ReceiveEvent>, Vec<Header>) {
        let (mut peer, side) = tokio::io::duplex(1024 * 1024);
        peer.write_all(script).await.expect("the script fits");
        let (reader, writer) = tokio::io::split(side);
        let mut receiver = Receiver::new(reader, writer);
        let mut events = Vec::new();
        loop {
            let next = tokio::time::timeout(std::time::Duration::from_secs(2), receiver.next())
                .await
                .unwrap_or_else(|_| panic!("the receiver stopped making progress after {events:?}"));
            let Some(event) = next.expect("the session should not fail") else {
                break;
            };
            events.push(event);
        }
        drop(receiver);
        let mut responses = Vec::new();
        peer.read_to_end(&mut responses).await.expect("responses are readable");

        let mut reader = FrameReader::new(responses.as_slice());
        let mut headers = Vec::new();
        while let Some((header, _coding)) = reader.read_header().await.expect("our own frames must parse") {
            headers.push(header);
        }
        (events, headers)
    }

    #[tokio::test]
    async fn a_complete_file_walks_from_zrqinit_to_zfin() {
        let info = FileInfo {
            name: "report.txt".to_owned(),
            size: 11,
            modified: Some(1_760_000_000),
            mode: Some(0o100644),
        };
        let mut script = rqinit();
        script.extend(zfile(&info));
        script.extend(zdata(
            0,
            &[(b"hello ", SubpacketEnd::CrcG), (b"world", SubpacketEnd::CrcE)],
        ));
        script.extend(Header::at(FrameType::Eof, 11).encoded(HeaderCoding::Bin32));
        script.extend(Header::new(FrameType::Fin, [0; 4]).encoded(HeaderCoding::Hex));

        let (events, sent) = run(&script).await;
        assert_eq!(events.len(), 4, "offer, two chunks, complete: {events:?}");
        assert_eq!(events[0], ReceiveEvent::Offer(info.clone()));
        assert_eq!(events[1], ReceiveEvent::Data(b"hello ".to_vec()));
        assert_eq!(events[2], ReceiveEvent::Data(b"world".to_vec()));
        assert_eq!(events[3], ReceiveEvent::Complete(info));

        let frames: Vec<FrameType> = sent.iter().map(|header| header.frame()).collect();
        assert_eq!(
            frames,
            vec![FrameType::RInit, FrameType::RPos, FrameType::RInit, FrameType::Fin],
            "the receiver should greet, ask for offset zero, say it is ready again, and agree to finish"
        );
        assert_eq!(sent[1].position(), 0);
    }

    #[tokio::test]
    async fn a_receiver_that_is_asked_to_resend_answers_with_the_first_missing_byte() {
        let info = FileInfo {
            name: "data.bin".to_owned(),
            size: 5,
            modified: None,
            mode: None,
        };
        // The second subpacket is corrupted in place: the first three bytes are fine.
        let mut broken = encode_subpacket(b"XYZ", SubpacketEnd::CrcE, HeaderCoding::Bin32);
        let last = broken.len() - 1;
        broken[last] ^= 0xff;

        let mut script = rqinit();
        script.extend(zfile(&info));
        script.extend(zdata(0, &[(b"ABC", SubpacketEnd::CrcG)]));
        script.extend(broken);
        // The sender reacts to our ZRPOS for byte 3 and resends the missing bytes.
        script.extend(zdata(3, &[(b"DE", SubpacketEnd::CrcE)]));
        script.extend(Header::at(FrameType::Eof, 5).encoded(HeaderCoding::Bin32));
        script.extend(Header::new(FrameType::Fin, [0; 4]).encoded(HeaderCoding::Hex));

        let (events, sent) = run(&script).await;
        let delivered: Vec<u8> = events
            .iter()
            .filter_map(|event| match event {
                ReceiveEvent::Data(data) => Some(data.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(delivered, b"ABCDE", "each byte must be delivered exactly once");
        assert!(
            events.iter().any(|event| matches!(event, ReceiveEvent::Complete(_))),
            "the file should complete after the resend: {events:?}"
        );
        assert!(
            sent.iter()
                .any(|header| header.frame() == FrameType::RPos && header.position() == 3),
            "the receiver should ask for byte 3 again: {sent:?}"
        );
    }

    #[tokio::test]
    async fn an_unsafe_name_is_skipped_and_the_batch_keeps_going() {
        let unsafe_info = b"../escape\0".to_vec();
        let good = FileInfo {
            name: "ok.txt".to_owned(),
            size: 2,
            modified: None,
            mode: None,
        };
        let mut script = rqinit();
        script.extend(Header::new(FrameType::File, [0, 0, 0, file_option::ZCBIN]).encoded(HeaderCoding::Bin16));
        script.extend(encode_subpacket(&unsafe_info, SubpacketEnd::CrcW, HeaderCoding::Bin16));
        script.extend(zfile(&good));
        script.extend(zdata(0, &[(b"hi", SubpacketEnd::CrcE)]));
        script.extend(Header::at(FrameType::Eof, 2).encoded(HeaderCoding::Bin32));
        script.extend(Header::new(FrameType::Fin, [0; 4]).encoded(HeaderCoding::Hex));

        let (events, sent) = run(&script).await;
        match &events[0] {
            ReceiveEvent::Skipped { name, reason } => {
                assert!(
                    name.contains("escape"),
                    "the refusal should name what was offered: {name}"
                );
                assert!(reason.contains("path separator"), "unhelpful reason: {reason}");
            }
            other => panic!("expected a skip first, got {other:?}"),
        }
        assert!(
            events.iter().any(|event| matches!(event, ReceiveEvent::Complete(_))),
            "the second file should still arrive: {events:?}"
        );
        assert!(
            sent.iter().any(|header| header.frame() == FrameType::Skip),
            "the skipped name must be answered with ZSKIP: {sent:?}"
        );
    }

    #[tokio::test]
    async fn a_sender_that_restarts_below_the_delivered_offset_is_an_error() {
        let info = FileInfo {
            name: "x".to_owned(),
            size: 6,
            modified: None,
            mode: None,
        };
        let mut script = rqinit();
        script.extend(zfile(&info));
        script.extend(zdata(0, &[(b"abc", SubpacketEnd::CrcE)]));
        script.extend(zdata(0, &[(b"abc", SubpacketEnd::CrcE)]));

        let (mut peer, side) = tokio::io::duplex(1024 * 1024);
        peer.write_all(&script).await.expect("the script fits");
        let (reader, writer) = tokio::io::split(side);
        let mut receiver = Receiver::new(reader, writer);
        let mut error = None;
        loop {
            match receiver.next().await {
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(caught) => {
                    error = Some(caught);
                    break;
                }
            }
        }
        let error = error.expect("a restart below the delivered offset must fail");
        assert!(
            error.to_string().contains("restarted at byte 0"),
            "the error should name the offset: {error}"
        );
    }

    #[tokio::test]
    async fn a_file_that_ends_short_of_its_announced_length_is_an_error() {
        let info = FileInfo {
            name: "x".to_owned(),
            size: 10,
            modified: None,
            mode: None,
        };
        let mut script = rqinit();
        script.extend(zfile(&info));
        script.extend(zdata(0, &[(b"abc", SubpacketEnd::CrcE)]));
        script.extend(Header::at(FrameType::Eof, 3).encoded(HeaderCoding::Bin32));

        let (mut peer, side) = tokio::io::duplex(1024 * 1024);
        peer.write_all(&script).await.expect("the script fits");
        let (reader, writer) = tokio::io::split(side);
        let mut receiver = Receiver::new(reader, writer);
        let mut error = None;
        loop {
            match receiver.next().await {
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(caught) => {
                    error = Some(caught);
                    break;
                }
            }
        }
        let error = error.expect("a short file must not complete");
        assert!(
            error.to_string().contains("announced"),
            "the error should compare the lengths: {error}"
        );
    }

    #[tokio::test]
    async fn a_corrupt_frame_end_is_answered_with_zpos_at_the_delivered_offset() {
        // The corrupt subpacket is marked ZCRCG, so the sender was still inside the frame when the
        // receiver gave up on it and jumped to a fresh ZDATA header — the scanning path.
        let info = FileInfo {
            name: "x".to_owned(),
            size: 5,
            modified: None,
            mode: None,
        };
        let mut broken = encode_subpacket(b"cde", SubpacketEnd::CrcG, HeaderCoding::Bin32);
        let last = broken.len() - 1;
        broken[last] ^= 0xff;

        let mut script = rqinit();
        script.extend(zfile(&info));
        script.extend(zdata(0, &[(b"abc", SubpacketEnd::CrcG)]));
        script.extend(broken);
        script.extend(zdata(3, &[(b"de", SubpacketEnd::CrcE)]));
        script.extend(Header::at(FrameType::Eof, 5).encoded(HeaderCoding::Bin32));
        script.extend(Header::new(FrameType::Fin, [0; 4]).encoded(HeaderCoding::Hex));

        let (events, sent) = run(&script).await;
        assert!(
            sent.iter()
                .any(|header| header.frame() == FrameType::RPos && header.position() == 3),
            "the receiver should ask for byte 3 again: {sent:?}"
        );
        let delivered: Vec<u8> = events
            .iter()
            .filter_map(|event| match event {
                ReceiveEvent::Data(data) => Some(data.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(delivered, b"abcde");
    }
}
