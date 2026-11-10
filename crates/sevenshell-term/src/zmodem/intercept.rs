//! The terminal side of ZMODEM: recognising a session inside ordinary output, and running the
//! state machines without owning the byte stream.
//!
//! A terminal hands its output over as chunks pushed at this layer, while [`Receiver`] and
//! [`Sender`] pull from an `AsyncRead`. [`Detector`] is the adapter. It scans for the hex header
//! that begins a session, returns ordinary bytes as [`DetectorEvent::Output`] (they continue to the
//! screen), pushes everything else into an in-memory queue the state machine reads, and reports
//! whatever the machine wrote as [`DetectorEvent::Reply`] — bytes the caller writes into the
//! terminal's input.
//!
//! Two consequences are deliberate. Nothing here blocks: a session advances only when
//! [`Detector::feed`] is called, so a peer that goes silent leaves the detector waiting while the
//! caller stays free to arm its own deadline and call [`Detector::abort`], which tells the far side
//! to stop and puts the terminal back on the plain path. And no local path is opened: a received
//! file is a stream of [`DetectorEvent::Received`] events whose bytes the caller writes wherever a
//! person said, while a file to send arrives as an `AsyncRead + AsyncSeek` the caller already has.

use std::future::Future;
use std::mem;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use sevenshell_error::ShellError;
use tokio::io::{AsyncRead, AsyncSeek};

use super::bridge::{ByteQueue, QueueReader, ReplyBuffer, ReplyWriter};
use super::crc16;
use super::file::FileInfo;
use super::frame::{FrameType, ZDLE, ZHEX, ZPAD};
use super::receive::{ReceiveEvent, Receiver};
use super::send::Sender;

/// The four bytes every session begins with: two pads, the escape character, and the marker that
/// says the header is spelled in hex.
///
/// Binary headers exist, but the header that opens a session is hex in every sender this side has
/// to answer; a binary one cannot be told apart from a short run of binary output.
const SIGNATURE: [u8; 4] = [ZPAD, ZPAD, ZDLE, ZHEX];

/// The signature plus seven bytes spelled as two hex digits each: five header bytes and the 16-bit
/// checksum. Reaching this many bytes is what makes a guess checkable.
const HEADER_CHARACTERS: usize = 14;

/// Which side of the transfer the far side asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// The far side is a sender (`sz`): this side receives files.
    Receive,
    /// The far side is a receiver (`rz`): this side may send files.
    Send,
}

/// Why a session stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// The protocol's own goodbye: ZFIN was exchanged.
    Complete,
    /// The caller cancelled, or reset a session that had stopped making progress.
    Aborted,
}

/// What the interception layer extracted from one chunk of terminal output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectorEvent {
    /// Bytes that belong on the screen, unchanged.
    Output(Vec<u8>),
    /// Protocol bytes that must be written into the terminal's input, in order.
    Reply(Vec<u8>),
    /// A session just started; emitted once, before its first `Reply`.
    Started(Direction),
    /// The receiver's report as a file arrives: the offer, verified chunks, the end, or a refusal.
    Received(ReceiveEvent),
    /// The far side confirmed the file this side sent, with how many bytes it took.
    Sent { bytes: u64 },
    /// The session is over. Every byte fed from now on is `Output` again.
    Ended(EndReason),
}

/// Watches a terminal's output for ZMODEM and runs the matching state machine.
///
/// Feed it every chunk of terminal output and do what the events say: `Output` goes to the screen,
/// `Reply` goes into the terminal's input, `Received`/`Sent` are the file traffic, and `Ended`
/// marks the point where the stream is plain text again. Nothing else is needed — there is no task
/// to keep alive and no timer to service.
pub struct Detector {
    phase: Phase,
}

enum Phase {
    /// Ordinary terminal output; scanning for a header that starts a session.
    Streaming(Scanner),
    /// A `sz` on the far side is sending files to this side.
    Receiving(Receiving),
    /// A `rz` on the far side is waiting for a file, and the caller has not picked one yet.
    Waiting(Waiting),
    /// This side is sending files to an `rz` on the far side.
    Sending(Sending),
}

/// One step a session asks for: keep going, or stop and hand the rest of the stream back.
enum Flow {
    Continue,
    End { reason: EndReason, trailing: Vec<u8> },
}

impl Default for Detector {
    fn default() -> Self {
        Self::new()
    }
}

impl Detector {
    pub fn new() -> Self {
        Self {
            phase: Phase::Streaming(Scanner::new()),
        }
    }

    /// The direction of the running session, or `None` while output is ordinary terminal text.
    pub fn active(&self) -> Option<Direction> {
        match &self.phase {
            Phase::Streaming(_) => None,
            Phase::Receiving(_) => Some(Direction::Receive),
            Phase::Waiting(_) | Phase::Sending(_) => Some(Direction::Send),
        }
    }

    /// Takes one chunk of terminal output and reports what it contained.
    ///
    /// An error means the ZMODEM stream was broken (a header that fails its checksum, a peer that
    /// answered something the protocol does not allow). The detector has already returned to plain
    /// output by then, so the caller recovers by showing the error and carrying on; bytes in the
    /// broken session are not reported, because their meaning was lost with it.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<DetectorEvent>, ShellError> {
        let mut events = Vec::new();
        let mut started = None;
        match &mut self.phase {
            Phase::Streaming(scanner) => started = scanner.push(bytes, &mut events),
            Phase::Waiting(waiting) => waiting.absorb(),
            Phase::Receiving(session) => session.absorb(bytes),
            Phase::Sending(session) => session.absorb(bytes),
        }
        if let Some(start) = started {
            events.push(DetectorEvent::Started(start.direction));
            self.phase = match start.direction {
                Direction::Receive => Phase::Receiving(Receiving::new(start.bytes)),
                Direction::Send => Phase::Waiting(Waiting::new(start.bytes)),
            };
        }
        self.pump(&mut events)?;
        Ok(events)
    }

    /// Starts sending a file into the terminal's input.
    ///
    /// Works after a ZRINIT was detected — the far side is already parked in `rz` — and on its own,
    /// in which case the first bytes written are ZRQINIT, so a caller that starts `rz` right after
    /// is served too. The file is announced with the length in `file`, and a source shorter than
    /// that is an error rather than a silently truncated file.
    pub fn start_sending<S>(&mut self, file: FileInfo, source: S) -> Result<Vec<DetectorEvent>, ShellError>
    where
        S: AsyncRead + AsyncSeek + Unpin + Send + 'static,
    {
        let phase = mem::replace(&mut self.phase, Phase::Streaming(Scanner::new()));
        let start = match phase {
            Phase::Waiting(waiting) => Some(waiting.start),
            Phase::Streaming(_) => None,
            other => {
                self.phase = other;
                return Err(ShellError::terminal_invalid(
                    "this terminal is already carrying a ZMODEM session",
                ));
            }
        };

        let channel = Channel::new();
        if let Some(start) = start {
            // `rz` announced itself with this header. Handing it to the sender lets the greeting
            // and the file announcement go out at once instead of waiting for `rz` to repeat.
            channel.queue.push(&start);
        }
        let mut session = Sending::new(channel);
        session.begin_file(file, source);
        self.phase = Phase::Sending(session);

        let mut events = vec![DetectorEvent::Started(Direction::Send)];
        self.pump(&mut events)?;
        Ok(events)
    }

    /// Queues another file in the running send session.
    ///
    /// `rz` takes any number of files between its greeting and the goodbye; one session per file
    /// would restart it every time.
    pub fn send_next<S>(&mut self, file: FileInfo, source: S) -> Result<Vec<DetectorEvent>, ShellError>
    where
        S: AsyncRead + AsyncSeek + Unpin + Send + 'static,
    {
        match &mut self.phase {
            Phase::Sending(session) if session.driver.is_none() => session.begin_file(file, source),
            _ => {
                return Err(ShellError::terminal_invalid(
                    "no ZMODEM send session is waiting for another file",
                ));
            }
        }
        let mut events = Vec::new();
        self.pump(&mut events)?;
        Ok(events)
    }

    /// Tells the far side the batch is over and waits for it to agree.
    ///
    /// The goodbye can take a chunk of output to arrive; until `Ended` shows up in a later `feed`
    /// the session is still active and its bytes still belong to the protocol.
    pub fn finish_sending(&mut self) -> Result<Vec<DetectorEvent>, ShellError> {
        match &mut self.phase {
            Phase::Sending(session) if session.driver.is_none() => session.begin_finish()?,
            _ => {
                return Err(ShellError::terminal_invalid("no idle ZMODEM send session to finish"));
            }
        }
        let mut events = Vec::new();
        self.pump(&mut events)?;
        Ok(events)
    }

    /// Ends whatever session is running and puts the terminal back on the ordinary path.
    ///
    /// The returned bytes still have to be written into the terminal's input: the far side is
    /// usually parked in raw mode waiting for the protocol, and the person's shell prompt does not
    /// come back until it is told to stop. An idle detector returns nothing.
    pub fn abort(&mut self) -> Vec<DetectorEvent> {
        if matches!(self.phase, Phase::Streaming(_)) {
            return Vec::new();
        }
        self.phase = Phase::Streaming(Scanner::new());
        vec![
            DetectorEvent::Reply(cancel_bytes()),
            DetectorEvent::Ended(EndReason::Aborted),
        ]
    }

    /// Drives the active session as far as the bytes already fed allow.
    fn pump(&mut self, events: &mut Vec<DetectorEvent>) -> Result<(), ShellError> {
        loop {
            let flow = match &mut self.phase {
                Phase::Receiving(session) => session.drive(events),
                Phase::Sending(session) => session.drive(events),
                _ => return Ok(()),
            };
            match flow {
                Ok(None) => return Ok(()),
                Ok(Some(Flow::Continue)) => {}
                Ok(Some(Flow::End { reason, trailing })) => {
                    self.phase = Phase::Streaming(Scanner::new());
                    events.push(DetectorEvent::Ended(reason));
                    if !trailing.is_empty() {
                        events.push(DetectorEvent::Output(trailing));
                    }
                    return Ok(());
                }
                Err(error) => {
                    self.phase = Phase::Streaming(Scanner::new());
                    return Err(error);
                }
            }
        }
    }
}

/// The queue pair a session talks through.
struct Channel {
    queue: ByteQueue,
    replies: ReplyBuffer,
}

impl Channel {
    fn new() -> Self {
        Self {
            queue: ByteQueue::new(),
            replies: ReplyBuffer::new(),
        }
    }

    fn drain_replies(&self, events: &mut Vec<DetectorEvent>) {
        let bytes = self.replies.take();
        if !bytes.is_empty() {
            events.push(DetectorEvent::Reply(bytes));
        }
    }
}

/// What the scanner found: where the session starts and in which direction.
struct Start {
    direction: Direction,
    /// From the first pad of the header to the last byte fed, which may already contain frames and
    /// is handed to the session untouched.
    bytes: Vec<u8>,
}

/// Looks for a session header in ordinary output without ever showing half of one.
///
/// Only a suffix that could still grow into [`SIGNATURE`] is held back, so a prompt or a build log
/// passes through in the same `feed` that received it.
struct Scanner {
    held: Vec<u8>,
}

impl Scanner {
    fn new() -> Self {
        Self { held: Vec::new() }
    }

    fn push(&mut self, bytes: &[u8], events: &mut Vec<DetectorEvent>) -> Option<Start> {
        self.held.extend_from_slice(bytes);
        loop {
            let Some(index) = find(&self.held, &SIGNATURE) else {
                let keep = signature_suffix(&self.held);
                let visible = self.held.len() - keep;
                if visible > 0 {
                    events.push(DetectorEvent::Output(self.held[..visible].to_vec()));
                    self.held.drain(..visible);
                }
                return None;
            };
            if index > 0 {
                events.push(DetectorEvent::Output(self.held[..index].to_vec()));
                self.held.drain(..index);
            }
            if self.held.len() < SIGNATURE.len() + HEADER_CHARACTERS {
                // Not enough bytes to tell a header from a coincidence yet.
                return None;
            }
            let direction = start_header(&self.held).and_then(direction_of);
            if let Some(direction) = direction {
                return Some(Start {
                    direction,
                    bytes: mem::take(&mut self.held),
                });
            }
            // A header that does not begin a session — a bad checksum, a stray ZACK — is screen
            // text after all. One byte at a time keeps a later, real header findable.
            events.push(DetectorEvent::Output(vec![self.held[0]]));
            self.held.drain(..1);
        }
    }
}

/// A `sz` on the far side is sending files; the receiver answers it directly.
struct Receiving {
    channel: Channel,
    receiver: Option<Receiver<QueueReader, ReplyWriter>>,
    driver: Option<RecvDriver>,
}

type RecvDriver = Pin<
    Box<
        dyn Future<
                Output = (
                    Receiver<QueueReader, ReplyWriter>,
                    Result<Option<ReceiveEvent>, ShellError>,
                ),
            > + Send,
    >,
>;

impl Receiving {
    fn new(start: Vec<u8>) -> Self {
        let channel = Channel::new();
        channel.queue.push(&start);
        let receiver = Receiver::new(
            QueueReader::new(channel.queue.clone()),
            ReplyWriter::new(channel.replies.clone()),
        );
        Self {
            channel,
            receiver: Some(receiver),
            driver: None,
        }
    }

    fn absorb(&mut self, bytes: &[u8]) {
        self.channel.queue.push(bytes);
    }

    fn drive(&mut self, events: &mut Vec<DetectorEvent>) -> Result<Option<Flow>, ShellError> {
        if self.driver.is_none() {
            let Some(receiver) = self.receiver.take() else {
                return Ok(None);
            };
            // The future owns the receiver: `next` is not cancel-safe, so it must survive every
            // `Pending` instead of being rebuilt from a state that had half a subpacket in a local.
            self.driver = Some(Box::pin(async move {
                let mut receiver = receiver;
                let outcome = receiver.next().await;
                (receiver, outcome)
            }));
        }
        let driver = self.driver.as_mut().expect("the driver was just created");
        let mut context = Context::from_waker(Waker::noop());
        let polled = driver.as_mut().poll(&mut context);
        self.channel.drain_replies(events);
        let (receiver, outcome) = match polled {
            Poll::Pending => return Ok(None),
            Poll::Ready(done) => done,
        };
        self.driver = None;
        self.receiver = Some(receiver);
        match outcome {
            Ok(Some(event)) => {
                events.push(DetectorEvent::Received(event));
                Ok(Some(Flow::Continue))
            }
            Ok(None) => Ok(Some(Flow::End {
                reason: EndReason::Complete,
                trailing: self.trailing(),
            })),
            Err(error) => Err(error),
        }
    }

    /// Bytes read ahead that no frame consumed — the prompt that arrived with ZFIN, usually.
    fn trailing(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(receiver) = &mut self.receiver {
            out.extend(receiver.take_buffered());
        }
        out.extend(self.channel.queue.take());
        out
    }
}

/// A `rz` on the far side is waiting for a file. The caller decides whether one is sent.
struct Waiting {
    /// The ZRINIT that pulled the terminal into a session. Kept so the sender can answer it without
    /// waiting for `rz` to repeat itself, and so it never reaches the screen.
    start: Vec<u8>,
}

impl Waiting {
    fn new(start: Vec<u8>) -> Self {
        Self { start }
    }

    fn absorb(&mut self) {
        // `rz` repeats its ZRINIT every few seconds while it waits. Those repeats carry nothing
        // new — the sender's own ZRQINIT will be answered with a fresh one — and letting them
        // through would print protocol frames onto the screen.
    }
}

/// This side is sending to an `rz` on the far side.
struct Sending {
    channel: Channel,
    /// The sender while a future does not own it: between files, or before the first one.
    sender: Option<Sender<QueueReader, ReplyWriter>>,
    driver: Option<SendDriver>,
}

enum SendDriver {
    File(SendFileDriver),
    Finish(SendFinishDriver),
}

type SendFileDriver = Pin<Box<dyn Future<Output = (Sender<QueueReader, ReplyWriter>, Result<u64, ShellError>)> + Send>>;
type SendFinishDriver =
    Pin<Box<dyn Future<Output = (Sender<QueueReader, ReplyWriter>, Result<(), ShellError>)> + Send>>;

/// What a driver handed back when it finished.
enum Done {
    File(Sender<QueueReader, ReplyWriter>, Result<u64, ShellError>),
    Finish(Sender<QueueReader, ReplyWriter>, Result<(), ShellError>),
}

impl Sending {
    fn new(channel: Channel) -> Self {
        Self {
            channel,
            sender: None,
            driver: None,
        }
    }

    fn absorb(&mut self, bytes: &[u8]) {
        self.channel.queue.push(bytes);
    }

    fn begin_file<S>(&mut self, file: FileInfo, mut source: S)
    where
        S: AsyncRead + AsyncSeek + Unpin + Send + 'static,
    {
        let sender = match self.sender.take() {
            Some(sender) => sender,
            None => Sender::new(
                QueueReader::new(self.channel.queue.clone()),
                ReplyWriter::new(self.channel.replies.clone()),
            ),
        };
        self.driver = Some(SendDriver::File(Box::pin(async move {
            let mut sender = sender;
            let result = sender.send_file(&file, &mut source).await;
            (sender, result)
        })));
    }

    fn begin_finish(&mut self) -> Result<(), ShellError> {
        let Some(mut sender) = self.sender.take() else {
            return Err(ShellError::terminal_invalid(
                "nothing has been sent in this ZMODEM session, so there is nothing to finish",
            ));
        };
        self.driver = Some(SendDriver::Finish(Box::pin(async move {
            let result = sender.finish().await;
            (sender, result)
        })));
        Ok(())
    }

    fn drive(&mut self, events: &mut Vec<DetectorEvent>) -> Result<Option<Flow>, ShellError> {
        let Some(driver) = self.driver.as_mut() else {
            return Ok(None);
        };
        let mut context = Context::from_waker(Waker::noop());
        let done = match driver {
            SendDriver::File(future) => future
                .as_mut()
                .poll(&mut context)
                .map(|(sender, result)| Done::File(sender, result)),
            SendDriver::Finish(future) => future
                .as_mut()
                .poll(&mut context)
                .map(|(sender, result)| Done::Finish(sender, result)),
        };
        self.channel.drain_replies(events);
        let done = match done {
            Poll::Pending => return Ok(None),
            Poll::Ready(done) => done,
        };
        self.driver = None;
        match done {
            Done::File(sender, result) => {
                self.sender = Some(sender);
                let bytes = result?;
                events.push(DetectorEvent::Sent { bytes });
                Ok(Some(Flow::Continue))
            }
            Done::Finish(sender, result) => {
                self.sender = Some(sender);
                result?;
                Ok(Some(Flow::End {
                    reason: EndReason::Complete,
                    trailing: self.trailing(),
                }))
            }
        }
    }

    /// Bytes read ahead that no frame consumed — whatever `rz` printed on its way out.
    fn trailing(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(sender) = &mut self.sender {
            out.extend(sender.take_buffered());
        }
        out.extend(self.channel.queue.take());
        out
    }
}

/// The interrupt every ZMODEM peer honours: several CAN characters (the escape byte doubles as
/// CAN here) followed by backspaces, so the tail of a half-written frame is erased from the line.
fn cancel_bytes() -> Vec<u8> {
    let mut bytes = vec![ZDLE; 5];
    bytes.extend_from_slice(&[0x08; 5]);
    bytes
}

/// Reads the hex header the signature introduces and says which frame it is.
///
/// The reader in `frame` is async because it pulls from a stream; detection has the bytes in hand
/// already, so this spells the same layout out synchronously. `None` means the bytes are not a
/// header this side may act on — a broken checksum, an unknown frame type, or too short a run.
fn start_header(bytes: &[u8]) -> Option<FrameType> {
    let characters = bytes.get(SIGNATURE.len()..SIGNATURE.len() + HEADER_CHARACTERS)?;
    let mut decoded = [0u8; 7];
    for (index, slot) in decoded.iter_mut().enumerate() {
        *slot = hex_pair(characters[index * 2], characters[index * 2 + 1])?;
    }
    let payload = [decoded[0], decoded[1], decoded[2], decoded[3], decoded[4]];
    let checksum = u16::from_be_bytes([decoded[5], decoded[6]]);
    if crc16(&payload) != checksum {
        return None;
    }
    FrameType::from_u8(payload[0])
}

fn hex_pair(high: u8, low: u8) -> Option<u8> {
    Some(digit(high)? << 4 | digit(low)?)
}

fn digit(character: u8) -> Option<u8> {
    match character {
        b'0'..=b'9' => Some(character - b'0'),
        b'a'..=b'f' => Some(character - b'a' + 10),
        b'A'..=b'F' => Some(character - b'A' + 10),
        _ => None,
    }
}

/// Only the two frames that open a session are acted on: ZRQINIT from a sender and ZRINIT from a
/// receiver. A valid header of any other type is a stray from a session this side does not know.
fn direction_of(frame: FrameType) -> Option<Direction> {
    match frame {
        FrameType::RqInit => Some(Direction::Receive),
        FrameType::RInit => Some(Direction::Send),
        _ => None,
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

/// How many trailing bytes of `held` are a proper prefix of the signature — the run that has to
/// wait for the next chunk before it can be shown or acted on.
fn signature_suffix(held: &[u8]) -> usize {
    let longest = held.len().min(SIGNATURE.len() - 1);
    (1..=longest)
        .rev()
        .find(|&length| held[held.len() - length..] == SIGNATURE[..length])
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::zmodem::frame::{FrameReader, Header, HeaderCoding, SubpacketEnd, encode_subpacket, file_option};

    fn rqinit() -> Vec<u8> {
        Header::new(FrameType::RqInit, [0; 4]).encoded(HeaderCoding::Hex)
    }

    fn rinit() -> Vec<u8> {
        Header::new(FrameType::RInit, [0; 4])
            .with_flags(0x23)
            .encoded(HeaderCoding::Hex)
    }

    fn zfin() -> Vec<u8> {
        Header::new(FrameType::Fin, [0; 4]).encoded(HeaderCoding::Hex)
    }

    fn file_info(name: &str, size: u64) -> FileInfo {
        FileInfo {
            name: name.to_owned(),
            size,
            modified: Some(1_760_000_000),
            mode: Some(0o100644),
        }
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

    fn outputs(events: &[DetectorEvent]) -> Vec<u8> {
        events
            .iter()
            .filter_map(|event| match event {
                DetectorEvent::Output(bytes) => Some(bytes.clone()),
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn replies(events: &[DetectorEvent]) -> Vec<u8> {
        events
            .iter()
            .filter_map(|event| match event {
                DetectorEvent::Reply(bytes) => Some(bytes.clone()),
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn received_data(events: &[DetectorEvent]) -> Vec<u8> {
        events
            .iter()
            .filter_map(|event| match event {
                DetectorEvent::Received(ReceiveEvent::Data(bytes)) => Some(bytes.clone()),
                _ => None,
            })
            .flatten()
            .collect()
    }

    /// Parses frames this side wrote, returning their types and the payload of any data frames.
    async fn parse_wire(bytes: &[u8]) -> (Vec<FrameType>, Vec<u8>) {
        let mut reader = FrameReader::new(bytes);
        let mut frames = Vec::new();
        let mut payload = Vec::new();
        while let Some((header, coding)) = reader.read_header().await.expect("our own frames must parse") {
            frames.push(header.frame());
            if header.frame() == FrameType::File {
                let _ = reader.read_subpacket(coding).await.expect("the information packet");
            }
            if header.frame() == FrameType::Data {
                loop {
                    let (chunk, end) = reader.read_subpacket(coding).await.expect("a data packet");
                    payload.extend_from_slice(&chunk);
                    if end.ends_frame() {
                        break;
                    }
                }
            }
        }
        (frames, payload)
    }

    #[test]
    fn the_detector_can_move_to_the_task_that_forwards_terminal_output() {
        fn movable<T: Send>() {}
        // The forwarding task outlives whichever call created the detector, so the boxed futures
        // inside it must not pin it to the thread that fed the first chunk.
        movable::<Detector>();
    }

    #[test]
    fn ordinary_output_passes_through_unchanged() {
        let mut detector = Detector::new();
        let line = b"total 4\r\n-rw-r--r-- 1 lab lab 3 note.txt\r\n$ ";
        let events = detector.feed(line).expect("plain output is not an error");
        assert_eq!(
            events,
            vec![DetectorEvent::Output(line.to_vec())],
            "nothing may be held back when no header is in sight"
        );
        assert_eq!(detector.active(), None);
    }

    #[test]
    fn a_partial_signature_at_a_chunk_boundary_is_held_and_then_shown() {
        let mut detector = Detector::new();
        let first = detector.feed(b"build ok **").expect("no header yet");
        assert_eq!(
            outputs(&first),
            b"build ok ",
            "only the two pads may be held back, the rest belongs on screen at once: {first:?}"
        );
        let second = detector.feed(b"x\r\n").expect("still no header");
        assert_eq!(outputs(&second), b"**x\r\n", "the held pads have to come out in order");
        assert_eq!(detector.active(), None);
    }

    #[test]
    fn a_signature_that_is_not_a_header_is_shown_as_plain_text() {
        let mut broken = rqinit();
        // A checksum digit is the third byte from the end, in front of the CR LF footer.
        let index = broken.len() - 3;
        broken[index] = if broken[index] == b'0' { b'1' } else { b'0' };
        let mut detector = Detector::new();
        let events = detector.feed(&broken).expect("line noise is not an error");
        assert_eq!(
            outputs(&events),
            broken,
            "a header that fails its checksum is text: {events:?}"
        );
        assert_eq!(detector.active(), None);

        // A well-formed header of a frame that does not open a session is text as well.
        let ack = Header::new(FrameType::Ack, [0; 4]).encoded(HeaderCoding::Hex);
        let events = detector.feed(&ack).expect("a stray header is not an error");
        assert_eq!(outputs(&events), ack);
        assert_eq!(detector.active(), None);
    }

    #[tokio::test]
    async fn a_receive_session_starts_with_the_noise_that_preceded_it_still_on_screen() {
        let mut detector = Detector::new();
        let events = detector
            .feed(b"rz\r\n**\x18B")
            .expect("a half-arrived header is not an error");
        assert_eq!(
            outputs(&events),
            b"rz\r\n",
            "the shell echo belongs on screen: {events:?}"
        );

        // The rest arrives one byte at a time: a header may be split at any byte.
        let header = rqinit();
        for byte in &header[4..] {
            let events = detector.feed(&[*byte]).expect("a split header must not fail");
            assert!(
                outputs(&events).is_empty(),
                "no protocol byte may reach the screen: {events:?}"
            );
        }
        assert_eq!(detector.active(), Some(Direction::Receive));
    }

    #[tokio::test]
    async fn a_complete_file_arrives_and_the_prompt_that_came_with_zfin_is_shown_after_it() {
        let info = file_info("report.txt", 11);
        let mut script = b"rz\r\n".to_vec();
        script.extend(rqinit());
        script.extend(zfile(&info));
        script.extend(zdata(
            0,
            &[(b"hello ", SubpacketEnd::CrcG), (b"world", SubpacketEnd::CrcE)],
        ));
        script.extend(Header::at(FrameType::Eof, 11).encoded(HeaderCoding::Bin32));
        script.extend(zfin());
        // The prompt arrives in the same read as the goodbye; it must not be swallowed with it.
        script.extend_from_slice(b"$ ");

        let mut detector = Detector::new();
        let events = detector.feed(&script).expect("a well-formed script must not fail");

        assert_eq!(events[0], DetectorEvent::Output(b"rz\r\n".to_vec()));
        assert_eq!(events[1], DetectorEvent::Started(Direction::Receive));
        assert_eq!(
            received_data(&events),
            b"hello world",
            "every verified byte exactly once"
        );
        assert!(
            events.contains(&DetectorEvent::Received(ReceiveEvent::Offer(info.clone()))),
            "the offer must be reported: {events:?}"
        );
        assert!(
            events.contains(&DetectorEvent::Received(ReceiveEvent::Complete(info))),
            "the end of the file must be reported: {events:?}"
        );
        assert_eq!(
            events.last(),
            Some(&DetectorEvent::Output(b"$ ".to_vec())),
            "the shell prompt has to come back after the session: {events:?}"
        );
        let ended = events
            .iter()
            .position(|event| *event == DetectorEvent::Ended(EndReason::Complete))
            .expect("the session must end cleanly");
        assert!(ended + 1 < events.len(), "output must follow the end, not precede it");
        assert_eq!(detector.active(), None);

        let (frames, _) = parse_wire(&replies(&events)).await;
        assert_eq!(
            frames,
            vec![FrameType::RInit, FrameType::RPos, FrameType::RInit, FrameType::Fin],
            "the receiver should greet, ask for offset zero, say it is ready again, and agree to finish"
        );
    }

    #[tokio::test]
    async fn a_file_split_into_single_byte_feeds_still_completes() {
        let info = file_info("tiny.bin", 3);
        let mut script = rqinit();
        script.extend(zfile(&info));
        script.extend(zdata(0, &[(b"abc", SubpacketEnd::CrcE)]));
        script.extend(Header::at(FrameType::Eof, 3).encoded(HeaderCoding::Bin32));
        script.extend(zfin());

        let mut detector = Detector::new();
        let mut events = Vec::new();
        for byte in &script {
            events.extend(detector.feed(&[*byte]).expect("a split frame must not fail"));
        }
        assert_eq!(received_data(&events), b"abc");
        assert!(
            events.contains(&DetectorEvent::Ended(EndReason::Complete)),
            "the file must complete even when every byte arrives alone: {events:?}"
        );
        assert_eq!(detector.active(), None);
    }

    #[tokio::test]
    async fn a_stream_that_breaks_is_an_error_and_the_terminal_keeps_working() {
        let mut detector = Detector::new();
        let mut script = rqinit();
        // A receiver answering a receiver: both ends are waiting for a file.
        script.extend(rinit());
        let error = detector
            .feed(&script)
            .expect_err("a receiver answering a receiver must fail");
        assert!(
            error.to_string().contains("receiver"),
            "the error should say what went wrong: {error}"
        );
        assert_eq!(detector.active(), None, "the detector must be back on the plain path");

        let events = detector
            .feed(b"$ ")
            .expect("output after a broken session is plain again");
        assert_eq!(events, vec![DetectorEvent::Output(b"$ ".to_vec())]);
    }

    #[test]
    fn abort_tells_the_far_side_to_stop_and_the_terminal_resumes() {
        let mut detector = Detector::new();
        let events = detector
            .feed(&rqinit())
            .expect("a sender's greeting must be recognised");
        assert_eq!(events[0], DetectorEvent::Started(Direction::Receive));
        assert_eq!(detector.active(), Some(Direction::Receive));

        let events = detector.abort();
        assert!(
            matches!(events[0], DetectorEvent::Reply(_)),
            "the far side must be told to stop: {events:?}"
        );
        assert_eq!(events[1], DetectorEvent::Ended(EndReason::Aborted));
        let cancel = replies(&events);
        assert!(
            cancel.starts_with(&[ZDLE, ZDLE]),
            "the cancel sequence starts with CAN characters: {cancel:02x?}"
        );
        assert_eq!(detector.active(), None);

        let events = detector.feed(b"ok\r\n").expect("the terminal is plain again");
        assert_eq!(outputs(&events), b"ok\r\n");
        assert!(detector.abort().is_empty(), "an idle detector has nothing to abort");
    }

    #[tokio::test]
    async fn a_file_can_be_sent_after_the_far_side_asked_for_one() {
        let file = file_info("note.txt", 5);

        let mut detector = Detector::new();
        let detected = detector
            .feed(&rinit())
            .expect("a receiver's greeting must be recognised");
        assert!(detected.contains(&DetectorEvent::Started(Direction::Send)));
        assert!(
            outputs(&detected).is_empty(),
            "a ZRINIT must never reach the screen: {detected:?}"
        );
        let repeated = detector
            .feed(&rinit())
            .expect("a repeated greeting is still not output");
        assert!(repeated.is_empty(), "repeats carry nothing new: {repeated:?}");

        let events = detector
            .start_sending(file.clone(), Cursor::new(b"hello".to_vec()))
            .expect("the file should be announced");
        assert_eq!(events[0], DetectorEvent::Started(Direction::Send));
        let mut wire = replies(&events);
        let (frames, _) = parse_wire(&wire).await;
        assert_eq!(
            frames,
            vec![FrameType::RqInit, FrameType::File],
            "the greeting and the file announcement go out together"
        );

        let mut answer = Header::at(FrameType::RPos, 0).encoded(HeaderCoding::Hex);
        answer.extend(rinit());
        let events = detector.feed(&answer).expect("the receiver's answer must be accepted");
        assert!(
            events.contains(&DetectorEvent::Sent { bytes: 5 }),
            "the confirmed file must be reported: {events:?}"
        );
        wire.extend(replies(&events));

        let events = detector.finish_sending().expect("the goodbye should be written");
        wire.extend(replies(&events));
        let events = detector.feed(&zfin()).expect("the receiver's goodbye must be accepted");
        wire.extend(replies(&events));
        assert!(
            events.contains(&DetectorEvent::Ended(EndReason::Complete)),
            "the session must end when the receiver agrees: {events:?}"
        );
        assert_eq!(detector.active(), None);

        let (frames, payload) = parse_wire(&wire).await;
        assert_eq!(payload, b"hello", "the far side must receive exactly the file");
        assert_eq!(
            frames,
            vec![
                FrameType::RqInit,
                FrameType::File,
                FrameType::Data,
                FrameType::Eof,
                FrameType::Fin
            ],
            "one file is announced, sent and ended in protocol order"
        );
    }

    #[tokio::test]
    async fn a_send_can_start_before_the_far_side_prints_anything() {
        let mut detector = Detector::new();
        let events = detector
            .start_sending(file_info("early.txt", 2), Cursor::new(b"hi".to_vec()))
            .expect("a send may start without a detected greeting");
        assert_eq!(events[0], DetectorEvent::Started(Direction::Send));
        assert!(
            replies(&events).starts_with(&[ZPAD, ZPAD, ZDLE, ZHEX]),
            "the first frame is ZRQINIT, so a `rz` started right after is served: {events:?}"
        );

        let events = detector.abort();
        assert_eq!(events[1], DetectorEvent::Ended(EndReason::Aborted));
        assert_eq!(detector.active(), None);
        let events = detector.feed(b"$ ").expect("the terminal is plain again");
        assert_eq!(events, vec![DetectorEvent::Output(b"$ ".to_vec())]);
    }

    #[tokio::test]
    async fn a_second_file_joins_the_same_session_and_the_goodbye_ends_it() {
        let mut detector = Detector::new();
        detector
            .feed(&rinit())
            .expect("the receiver's greeting must be recognised");
        detector
            .start_sending(file_info("one.txt", 2), Cursor::new(b"aa".to_vec()))
            .expect("the first file should be announced");

        let mut answer = Header::at(FrameType::RPos, 0).encoded(HeaderCoding::Hex);
        answer.extend(rinit());
        detector.feed(&answer).expect("the first file should be confirmed");

        let events = detector
            .send_next(file_info("two.txt", 2), Cursor::new(b"bb".to_vec()))
            .expect("a second file may join the session");
        let (frames, _) = parse_wire(&replies(&events)).await;
        assert_eq!(
            frames,
            vec![FrameType::File],
            "the session is already open, so the second file is announced without another greeting"
        );

        let mut answer = Header::at(FrameType::RPos, 0).encoded(HeaderCoding::Hex);
        answer.extend(rinit());
        let events = detector.feed(&answer).expect("the second file should be confirmed");
        assert!(events.contains(&DetectorEvent::Sent { bytes: 2 }));

        let events = detector.finish_sending().expect("the goodbye should be written");
        let (frames, _) = parse_wire(&replies(&events)).await;
        assert_eq!(frames, vec![FrameType::Fin]);
    }
}
