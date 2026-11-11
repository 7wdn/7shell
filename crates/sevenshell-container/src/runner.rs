//! Where a container command runs.
//!
//! Two places are possible: a process on this machine, and an SSH exec channel on a host that has
//! the runtime installed. The difference stops at this file. Above it, an operation is an argument
//! vector; below it, one variant spawns that vector and the other quotes each argument into a
//! command string, because an SSH exec channel takes a command line and not an argv.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use sevenshell_error::ShellError;
use sevenshell_ssh::{ExecOutput, SshSession};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::mpsc;

use crate::{local, remote};

/// How big one chunk of a streamed transfer is.
///
/// Large enough that the per-chunk cost disappears against a disk read or a channel send, small
/// enough that a slow reader never holds more than a few hundred kilobytes of somebody's archive.
const CHUNK_BYTES: usize = 64 * 1024;

/// How many chunks may be in flight before whoever produces them has to wait.
///
/// This is the whole memory bound of a streamed transfer: a producer that outruns the command waits
/// here rather than growing a buffer, which is what makes a 40 GiB tree and a 4 KiB file cost the
/// same.
const CHUNK_BACKLOG: usize = 8;

/// Bytes for a command's standard input, and where the failure to produce them is left.
///
/// The bytes are not held here; a producer — a file, a tar being built from a local tree — sends
/// them into a bounded queue, and the runner drains it into the child's pipe or the SSH channel.
/// That is why an archive never has to fit in memory, and why the interface keeps working while one
/// is being streamed.
pub struct InputStream {
    chunks: Option<mpsc::Receiver<Vec<u8>>>,
    failure: Arc<Mutex<Option<String>>>,
}

impl InputStream {
    /// Bytes read from anything that implements [`AsyncRead`], pushed as they arrive.
    pub fn from_reader<R>(reader: R) -> Self
    where
        R: AsyncRead + Send + Unpin + 'static,
    {
        let mut reader = Box::pin(reader);
        let (sender, chunks) = mpsc::channel(CHUNK_BACKLOG);
        let failure = Arc::new(Mutex::new(None));
        let recorded = Arc::clone(&failure);
        tokio::spawn(async move {
            let mut buffer = vec![0u8; CHUNK_BYTES];
            loop {
                match reader.read(&mut buffer).await {
                    Ok(0) => break,
                    Ok(count) => {
                        // A closed queue means the command has ended; there is nowhere left to put
                        // the rest of the source and nothing to report about it.
                        if sender.send(buffer[..count].to_vec()).await.is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        record(&recorded, error.to_string());
                        return;
                    }
                }
            }
        });
        Self {
            chunks: Some(chunks),
            failure,
        }
    }

    /// Bytes produced on a blocking thread: a tar built from a local tree, whose reads are file I/O
    /// rather than something a runtime should be asked to suspend.
    ///
    /// The producer is told when to stop by its queue closing, which is what happens when the
    /// command exits before reading everything. It then returns an ordinary error, and this does not
    /// record it: the command's own answer is already the reason.
    pub fn from_producer<F>(producer: F) -> Self
    where
        F: FnOnce(&mut ChunkSender) -> Result<(), ShellError> + Send + 'static,
    {
        let (sender, chunks) = mpsc::channel(CHUNK_BACKLOG);
        let failure = Arc::new(Mutex::new(None));
        let recorded = Arc::clone(&failure);
        std::thread::spawn(move || {
            let mut sink = ChunkSender::new(sender);
            let outcome = producer(&mut sink);
            let flushed = sink.finish();
            // A source that failed is the root cause of whatever the command is about to say: a
            // truncated archive and a read error look the same on the far side.
            if !sink.abandoned() {
                match (outcome, flushed) {
                    (Err(error), _) => record(&recorded, error.to_string()),
                    (Ok(()), Err(error)) => record(&recorded, error.to_string()),
                    (Ok(()), Ok(())) => {}
                }
            }
        });
        Self {
            chunks: Some(chunks),
            failure,
        }
    }

    /// Why the source stopped before its end, when it did.
    pub fn failure(&self) -> Option<String> {
        self.failure.lock().ok().and_then(|failure| failure.clone())
    }

    /// Takes the queue a command's standard input is drained from, once.
    ///
    /// Public because a transport outside this crate may be the thing feeding a command: a pod's
    /// `exec` takes the same chunks a local child or an SSH channel would, without this crate
    /// knowing which of the three it is.
    pub fn take_chunks(&mut self) -> mpsc::Receiver<Vec<u8>> {
        let Some(chunks) = self.chunks.take() else {
            // Already drained by an earlier run; an empty queue is a stream that ended.
            let (_sender, receiver) = mpsc::channel(1);
            return receiver;
        };
        chunks
    }
}

/// The writing end handed to a producer, so a tar can be written piece by piece.
///
/// Writes are buffered to [`CHUNK_BYTES`] because a tar header is 512 bytes and a chunked transfer
/// would otherwise pay for a channel send per header.
pub struct ChunkSender {
    sender: mpsc::Sender<Vec<u8>>,
    buffer: Vec<u8>,
    abandoned: bool,
}

impl ChunkSender {
    pub(crate) fn new(sender: mpsc::Sender<Vec<u8>>) -> Self {
        Self {
            sender,
            buffer: Vec::with_capacity(CHUNK_BYTES),
            abandoned: false,
        }
    }

    /// Whether the reader went away, which makes the producer's own error uninteresting.
    pub fn abandoned(&self) -> bool {
        self.abandoned
    }

    /// Sends whatever is buffered, for a producer that is finished.
    pub fn finish(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let chunk = std::mem::take(&mut self.buffer);
        self.send(chunk)
    }

    fn push(&mut self, data: &[u8]) -> io::Result<()> {
        self.buffer.extend_from_slice(data);
        while self.buffer.len() >= CHUNK_BYTES {
            let rest = self.buffer.split_off(CHUNK_BYTES);
            let chunk = std::mem::replace(&mut self.buffer, rest);
            self.send(chunk)?;
        }
        Ok(())
    }

    fn send(&mut self, chunk: Vec<u8>) -> io::Result<()> {
        // A blocking send is what applies backpressure: this thread runs beside the command, not
        // inside a runtime, so waiting here simply means the producer reads slower.
        match self.sender.blocking_send(chunk) {
            Ok(()) => Ok(()),
            Err(_) => {
                self.abandoned = true;
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "the command stopped reading its input",
                ))
            }
        }
    }
}

impl Write for ChunkSender {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.push(data)?;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.finish()
    }
}

fn record(slot: &Arc<Mutex<Option<String>>>, detail: String) {
    if let Ok(mut slot) = slot.lock() {
        *slot = Some(detail);
    }
}

/// What a streamed command left behind when its output ended.
#[derive(Default)]
pub(crate) struct StreamEnding {
    pub exit_code: Option<u32>,
    /// The reader's own failure, or the command's standard error.
    pub detail: String,
}

/// The ending of a stream this crate did not itself run, and the one handle its producer needs.
///
/// A protocol that is not a process — a pod's `exec` channel, in practice — knows when its command
/// finished and what said command printed, but the reader of [`ByteStream`] only asks after the
/// bytes stopped. Writing the ending here, before the last chunk is handed over, is what makes the
/// two answers arrive in the order the reader expects them.
#[derive(Clone)]
pub struct StreamState {
    ending: Arc<Mutex<StreamEnding>>,
}

impl StreamState {
    pub fn new() -> Self {
        Self {
            ending: Arc::new(Mutex::new(StreamEnding::default())),
        }
    }

    /// How the command ended: its status, and what it said when it did not end well.
    ///
    /// An exit status is a byte in a script and a number here; a command killed by a signal has no
    /// status this can carry, so the reason goes in `detail` and the status stays absent.
    pub fn finish(&self, exit_code: Option<u32>, detail: impl Into<String>) {
        if let Ok(mut ending) = self.ending.lock() {
            ending.exit_code = exit_code;
            ending.detail = detail.into();
        }
    }
}

impl Default for StreamState {
    fn default() -> Self {
        Self::new()
    }
}

/// One command's raw standard output, arriving in chunks.
///
/// A download is a tar stream, so it cannot be collected as text and cannot be held whole: the
/// caller reads a chunk, writes it somewhere, and asks for the next. [`failure`](Self::failure) is
/// the ending, and it is only meaningful once [`next`](Self::next) has answered `None`.
pub struct ByteStream {
    source: ByteSource,
}

enum ByteSource {
    Local {
        chunks: mpsc::Receiver<Vec<u8>>,
        ending: Arc<Mutex<StreamEnding>>,
    },
    Remote(ExecOutput),
}

impl ByteStream {
    pub(crate) fn local(chunks: mpsc::Receiver<Vec<u8>>, ending: Arc<Mutex<StreamEnding>>) -> Self {
        Self {
            source: ByteSource::Local { chunks, ending },
        }
    }

    pub(crate) fn remote(output: ExecOutput) -> Self {
        Self {
            source: ByteSource::Remote(output),
        }
    }

    /// A stream whose bytes come from a queue somebody outside this crate fills, ending when that
    /// queue closes.
    ///
    /// This is the seam a transport that is not a process or an SSH channel needs: the bytes and
    /// the ending are produced by whoever owns the connection, and [`StreamState`] is where the
    /// command's own verdict is left before the last chunk is handed over — which is what keeps a
    /// reader from asking for the ending before it exists.
    pub fn from_receiver(chunks: mpsc::Receiver<Vec<u8>>, state: &StreamState) -> Self {
        Self::local(chunks, Arc::clone(&state.ending))
    }

    /// A stream that answers with these bytes and then ends, for the tests that need one.
    #[cfg(test)]
    pub(crate) fn from_chunks(chunks: Vec<Vec<u8>>, detail: Option<String>) -> Self {
        let (sender, receiver) = mpsc::channel(chunks.len().max(1));
        for chunk in chunks {
            // The queue is exactly as deep as the chunks, so no send can wait.
            let _ = sender.try_send(chunk);
        }
        drop(sender);
        let ending = StreamEnding {
            exit_code: Some(0),
            detail: detail.unwrap_or_default(),
        };
        Self::local(receiver, Arc::new(Mutex::new(ending)))
    }

    /// The next chunk of the command's output, or nothing when the command ended.
    pub async fn next(&mut self) -> Option<Vec<u8>> {
        match &mut self.source {
            ByteSource::Local { chunks, .. } => chunks.recv().await,
            ByteSource::Remote(output) => output.next().await,
        }
    }

    /// Why the command ended badly, or nothing when it ended well.
    ///
    /// Only the local runner has read the status by the time this is asked; over SSH the reader has
    /// it as soon as the channel closed, which is what ended the output.
    pub fn failure(&self) -> Option<String> {
        match &self.source {
            ByteSource::Local { ending, .. } => {
                let ending = ending.lock().ok()?;
                if !ending.detail.is_empty() {
                    return Some(ending.detail.clone());
                }
                match ending.exit_code {
                    Some(0) => None,
                    Some(code) => Some(format!("the command exited with status {code}")),
                    None => Some("the command ended without reporting an exit status".to_owned()),
                }
            }
            ByteSource::Remote(output) => {
                let stderr = output.stderr();
                let stderr = stderr.trim();
                match output.exit_code() {
                    Some(0) => None,
                    Some(code) => Some(if stderr.is_empty() {
                        format!("the command exited with status {code}")
                    } else {
                        stderr.to_owned()
                    }),
                    None => Some(if stderr.is_empty() {
                        "the command ended without reporting an exit status".to_owned()
                    } else {
                        stderr.to_owned()
                    }),
                }
            }
        }
    }
}

/// A command that keeps producing lines after it was started: logs, stats, a pull or a push.
pub struct LineStream {
    pub(crate) lines: mpsc::Receiver<String>,
    stop: Arc<dyn Fn() + Send + Sync>,
}

impl LineStream {
    /// A stream whose producer is already running. Crate-internal: the two runners are the only
    /// things that know how to attach a process to it.
    pub(crate) fn new(lines: mpsc::Receiver<String>, stop: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            lines,
            stop: Arc::new(stop),
        }
    }

    /// The next line, or nothing when the command has finished.
    pub async fn next(&mut self) -> Option<String> {
        self.lines.recv().await
    }

    /// A handle that stops the command without ending the stream that is being read here.
    ///
    /// Two things need it: a stop button that arrives on another task, and a registry that has to
    /// let go of a stream it cannot borrow any more.
    pub fn stopper(&self) -> Arc<dyn Fn() + Send + Sync> {
        Arc::clone(&self.stop)
    }

    /// Stops the command and everything reading it. Calling it twice is allowed.
    pub fn stop(&self) {
        (self.stop)();
    }
}

/// A terminal inside a container: bytes in both directions, a window size, and a close.
///
/// The shape is deliberately the same as every other terminal this application carries, so the
/// session registry can forward it without learning where the bytes come from.
pub struct PtySession {
    pub(crate) output: mpsc::Receiver<Vec<u8>>,
    pub(crate) control: mpsc::Sender<PtyControl>,
}

// The local runner drives one directly; the remote runner is being written against the same shape.
#[allow(dead_code)]
pub(crate) enum PtyControl {
    Write(Vec<u8>),
    Resize { columns: u32, rows: u32 },
    Close,
}

impl PtySession {
    /// Takes the output half, once, leaving a closed channel behind.
    ///
    /// The terminal registry owns the one reader there is; a second reader would be a second
    /// interpretation of the same bytes. The placeholder is closed, so an accidental second call
    /// ends rather than waits forever.
    pub fn take_output(&mut self) -> mpsc::Receiver<Vec<u8>> {
        let (placeholder, closed) = mpsc::channel(1);
        drop(placeholder);
        std::mem::replace(&mut self.output, closed)
    }

    /// The next chunk of output, or nothing when the terminal has closed.
    pub async fn next_output(&mut self) -> Option<Vec<u8>> {
        self.output.recv().await
    }

    pub async fn write(&self, data: &[u8]) -> Result<(), ShellError> {
        self.control
            .send(PtyControl::Write(data.to_vec()))
            .await
            .map_err(|_| ShellError::app_unsupported("the container terminal has already closed"))
    }

    pub async fn resize(&self, columns: u32, rows: u32) -> Result<(), ShellError> {
        self.control
            .send(PtyControl::Resize { columns, rows })
            .await
            .map_err(|_| ShellError::app_unsupported("the container terminal has already closed"))
    }

    pub async fn close(&self) -> Result<(), ShellError> {
        let _ = self.control.send(PtyControl::Close).await;
        Ok(())
    }
}

/// Which side of the connection a container command runs on.
///
/// Cloned rather than shared: a second user of the same connection — the file browser an exec tab
/// opened — has to run its commands there too, and both variants are cheap to copy (a unit, and an
/// `Arc` of the session that is already open).
#[derive(Clone)]
pub enum Runner {
    /// A process on this machine. The runtime's binary is found on `PATH`.
    Local,
    /// An exec channel on an SSH connection that is already open and verified.
    Ssh(Arc<SshSession>),
}

impl Runner {
    /// Runs a command and returns everything it wrote to stdout.
    pub async fn run(&self, argv: &[String]) -> Result<String, ShellError> {
        match self {
            Self::Local => local::run(argv).await,
            Self::Ssh(session) => remote::run(session, argv).await,
        }
    }

    /// Runs a command with bytes on stdin, for `load`, `login` and packed uploads.
    pub async fn run_stdin(&self, argv: &[String], input: Vec<u8>) -> Result<String, ShellError> {
        match self {
            Self::Local => local::run_stdin(argv, input).await,
            Self::Ssh(session) => remote::run_stdin(session, argv, input).await,
        }
    }

    /// Runs a command whose standard input is still being produced: an archive on disk, a tree
    /// being packed.
    ///
    /// Nothing here holds the whole input. The producer fills a bounded queue, the runner drains it
    /// into the child's pipe or the SSH channel, and a producer that reads faster than the command
    /// waits. A source that stops early is reported as the reason, because a command fed half an
    /// archive can only complain about the archive.
    pub async fn run_stream_stdin(&self, argv: &[String], mut input: InputStream) -> Result<String, ShellError> {
        let outcome = match self {
            Self::Local => local::run_stream_stdin(argv, &mut input).await,
            Self::Ssh(session) => remote::run_stream_stdin(session, argv, &mut input).await,
        };
        if let Some(detail) = input.failure() {
            return Err(ShellError::container_command_failed(
                "the standard input",
                format!("the bytes could not be read: {detail}"),
            ));
        }
        outcome
    }

    /// Runs a command whose standard output is bytes rather than text.
    ///
    /// The chunks arrive as the command writes them, which is the only shape a tar download can
    /// have: the archive is neither text nor small, and collecting it would put the whole tree in
    /// memory before anything reached the disk.
    pub async fn stream_bytes(&self, argv: &[String]) -> Result<ByteStream, ShellError> {
        match self {
            Self::Local => local::stream_bytes(argv).await,
            Self::Ssh(session) => remote::stream_bytes(session, argv).await,
        }
    }

    /// Runs a command whose output keeps arriving after it starts.
    pub async fn stream(&self, argv: &[String]) -> Result<LineStream, ShellError> {
        match self {
            Self::Local => local::stream(argv).await,
            Self::Ssh(session) => remote::stream(session, argv).await,
        }
    }

    /// Runs a command attached to a pseudo terminal.
    pub async fn open_pty(&self, argv: &[String], columns: u32, rows: u32) -> Result<PtySession, ShellError> {
        match self {
            Self::Local => local::open_pty(argv, columns, rows).await,
            Self::Ssh(session) => remote::open_pty(session, argv, columns, rows).await,
        }
    }
}
