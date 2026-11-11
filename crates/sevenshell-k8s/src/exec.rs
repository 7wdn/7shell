//! A pod terminal: the exec WebSocket and the channel protocol it carries.
//!
//! The upgrade is ordinary HTTP with a WebSocket handshake, and the frames after it are not text:
//! the first byte of every binary frame is a channel number and the rest is that channel's bytes.
//! The numbers are the protocol's, not a choice this client makes — stdin 0, stdout 1, stderr 2, an
//! error 3 carrying an API `Status` document, and a resize 4 carrying `{"Width":…,"Height":…}`.
//!
//! A terminal is written to and read from by different tasks that must not be the same task: the
//! registry that owns terminals writes keystrokes and resizes from to front, and reads output in a
//! loop that lives as long as the session. So the socket never leaves the task that owns it: writing
//! means posting a frame to that task, and reading means receiving what it forwards.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::SinkExt as _;
use futures_util::StreamExt as _;
use sevenshell_error::{ShellError, truncate_chars};
use sevenshell_types::K8sExecRequest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::{AUTHORIZATION, HOST, SEC_WEBSOCKET_PROTOCOL};
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};

use crate::client::{Bridge, K8sClient, K8sTransport};

/// The channel numbers of `v4.channel.k8s.io`.
const CHANNEL_STDIN: u8 = 0;
const CHANNEL_STDOUT: u8 = 1;
const CHANNEL_STDERR: u8 = 2;
const CHANNEL_ERROR: u8 = 3;
const CHANNEL_RESIZE: u8 = 4;

/// The only subprotocol this client speaks. A server that answers with something else is speaking a
/// different protocol, and guessing which is how a terminal shows binary noise as text.
const PROTOCOL: &str = "v4.channel.k8s.io";

/// What "open a terminal" means when the request names no command: the container's own shell.
///
/// The probe runs inside the container with the very shell that is running it, so an image without
/// bash still gets a prompt instead of `exec: bash: not found`.
const SHELL_COMMAND: &str = "exec $(command -v bash || command -v ash || echo /bin/sh)";

/// Output chunks waiting for the reader. A full queue stops reading the socket, which pushes back on
/// the container instead of growing this process's memory with a `yes` flood.
const OUTPUT_QUEUE: usize = 64;

/// A status message and a transport error are sentences; this is where they stop.
const ERROR_CHARS: usize = 512;

/// How long a close handshake may take. The peer is a kubelet that may already be gone, and a
/// session that will not close because nobody answers is worse than a socket that is dropped.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

/// The socket, with and without TLS, as the upgrade returns it.
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A terminal inside one container.
///
/// Shared by reference: the registry keeps it behind an `Arc`, one task reads
/// [`ExecSession::take_output`], and anything holding the same `Arc` may write, resize or close it.
pub struct ExecSession {
    outbound: mpsc::UnboundedSender<Command>,
    /// The output halves, taken once by the task that reads them. Behind a lock because the
    /// receiver is `Send` but not `Sync`, and this type has to be shareable across threads.
    output: Mutex<Option<mpsc::Receiver<Vec<u8>>>>,
    control: Arc<ExecControl>,
    closed: AtomicBool,
}

impl ExecSession {
    /// Sends bytes to the container's stdin.
    pub fn write(&self, bytes: &[u8]) -> Result<(), ShellError> {
        if bytes.is_empty() {
            return Ok(());
        }
        let mut frame = Vec::with_capacity(bytes.len() + 1);
        frame.push(CHANNEL_STDIN);
        frame.extend_from_slice(bytes);
        self.send(Command::Frame(Message::binary(frame)))
    }

    /// Tells the container how big the terminal is.
    ///
    /// The protocol is uint16 in both directions; a value outside that is clamped rather than
    /// truncated, because a width of 0 or 65 536 wraps to something the container cannot draw.
    pub fn resize(&self, columns: u32, rows: u32) -> Result<(), ShellError> {
        self.send(Command::Frame(resize_frame(columns, rows)?))
    }

    /// Ends the session, and ends it once: a second call is not an error and sends nothing.
    pub fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = self.control.close.send_replace(true);
    }

    /// Takes the output half, once. The bytes belong to one reader; a second reader would be a
    /// second interpretation of the same terminal.
    pub fn take_output(&self) -> Option<mpsc::Receiver<Vec<u8>>> {
        let mut slot = self.output.lock().unwrap_or_else(|poison| poison.into_inner());
        slot.take()
    }

    /// Whether the session ended: the server closed it, the socket broke, or [`ExecSession::close`]
    /// was called.
    pub fn ended(&self) -> bool {
        self.control.ended()
    }

    /// Waits for the end and answers what it was. A container that exited normally has no error; a
    /// failed exec carries the message the API server put on the error channel.
    pub async fn wait_ended(&self) -> Option<ShellError> {
        self.control.wait_ended().await
    }

    fn send(&self, command: Command) -> Result<(), ShellError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(ShellError::k8s_unavailable("the pod exec session is closed"));
        }
        self.outbound
            .send(command)
            .map_err(|_| ShellError::k8s_unavailable("the pod exec session has ended"))
    }
}

/// What the session posts to the task that owns the socket.
///
/// The queue is unbounded because writing is not async: a keystroke cannot wait for room, and the
/// only producer is a person typing or pasting. The task drains it faster than anything that size.
enum Command {
    Frame(Message),
}

/// The end state, shared with every waiter.
struct ExecControl {
    close: watch::Sender<bool>,
    ended: watch::Sender<EndState>,
}

#[derive(Clone, Default)]
struct EndState {
    done: bool,
    error: Option<ShellError>,
}

impl ExecControl {
    fn new() -> Self {
        Self {
            close: watch::channel(false).0,
            ended: watch::channel(EndState::default()).0,
        }
    }

    fn ended(&self) -> bool {
        self.ended.borrow().done
    }

    fn end(&self, error: Option<ShellError>) {
        // The first answer wins: a close racing the server's close produces one end state.
        if self.ended.borrow().done {
            return;
        }
        let _ = self.ended.send_replace(EndState { done: true, error });
    }

    async fn wait_ended(&self) -> Option<ShellError> {
        let mut ended = self.ended.subscribe();
        loop {
            let state = ended.borrow_and_update().clone();
            if state.done {
                return state.error;
            }
            if ended.changed().await.is_err() {
                return None;
            }
        }
    }
}

/// Opens a terminal in one container of one pod.
///
/// Answers only once the upgrade is done, because "the pod is not running" is an answer the person
/// is waiting for and a handle that never produces output is not. `columns`/`rows` are the terminal's
/// size at the moment it opens; later sizes go through [`ExecSession::resize`].
pub async fn open_exec(
    client: &K8sClient,
    request: &K8sExecRequest,
    columns: u32,
    rows: u32,
) -> Result<ExecSession, ShellError> {
    let transport = client.transport();
    let url = exec_url(transport, request)?;
    let socket = upgrade(transport, url).await?;

    let (outbound, commands) = mpsc::unbounded_channel();
    let (output, outputs) = mpsc::channel(OUTPUT_QUEUE);
    let control = Arc::new(ExecControl::new());
    let running = Arc::clone(&control);
    let closing = control.close.subscribe();

    tokio::spawn(async move {
        let error = pump(socket, commands, output, closing, columns, rows).await;
        running.end(error);
    });

    Ok(ExecSession {
        outbound,
        output: Mutex::new(Some(outputs)),
        control,
        closed: AtomicBool::new(false),
    })
}

/// The upgrade itself: one subprotocol, one credential, one TLS story and one tunnel.
///
/// Shared by the terminal and by a command run to completion, because a file operation authorised
/// differently from a terminal would be a second, quieter answer about who may reach the pod.
async fn upgrade(transport: &K8sTransport, url: reqwest::Url) -> Result<Socket, ShellError> {
    let mut handshake = url
        .as_str()
        .into_client_request()
        .map_err(|error| ShellError::k8s_unavailable(format!("the exec address is not a WebSocket URL: {error}")))?;
    handshake
        .headers_mut()
        .insert(SEC_WEBSOCKET_PROTOCOL, HeaderValue::from_static(PROTOCOL));
    if let Some(authorization) = &transport.authorization {
        // A header value that cannot hold the authorization is a broken transport, not a terminal
        // problem; the value itself never appears in the message.
        let value = HeaderValue::from_str(authorization)
            .map_err(|_| ShellError::k8s_unavailable("the cluster credential cannot be sent as a header"))?;
        handshake.headers_mut().insert(AUTHORIZATION, value);
    }
    if !transport.server.is_empty() && url.host_str() != Some(transport.server.as_str()) {
        let value = HeaderValue::from_str(&transport.server)
            .map_err(|_| ShellError::k8s_unavailable("the server name cannot be sent as a header"))?;
        handshake.headers_mut().insert(HOST, value);
    }

    // The same connector the REST client uses: a client certificate has to authorise the exec
    // channel exactly as it authorises a list, or a cluster with certificate auth would list pods
    // and refuse every terminal.
    let connector = if url.scheme() == "wss" {
        Connector::NativeTls(transport.tls.clone())
    } else {
        Connector::Plain
    };
    let upgraded = match &transport.bridge {
        // A tunnel is asked for the cluster by name first; everything after that is the ordinary
        // upgrade over the socket it answered with, TLS included. So the name the certificate is
        // checked against is the one the kubeconfig gave, and the listener decides nothing beyond
        // the server it was opened for.
        Some(bridge) => {
            let tunnel = dial_bridge(bridge).await?;
            tokio_tungstenite::client_async_tls_with_config(handshake, tunnel, None, Some(connector)).await
        }
        None => tokio_tungstenite::connect_async_tls_with_config(handshake, None, true, Some(connector)).await,
    };
    let (socket, _response) = upgraded.map_err(|error| {
        ShellError::k8s_unavailable(format!(
            "the pod exec channel could not be opened: {}",
            truncate_chars(&error.to_string(), ERROR_CHARS)
        ))
    })?;
    // There is no subprotocol check here because the upgrade refuses one: tungstenite fails the
    // handshake unless the server echoes exactly the protocol this client offered, and a server
    // that answers nothing at all fails it too. A second check would be a second opinion about
    // something already decided.
    Ok(socket)
}

// --- A command run to completion ---------------------------------------------

/// How much standard input may wait for the socket. Bounded because the producer is a file stream:
/// an unbounded queue would hold a whole upload in memory.
const STDIN_QUEUE: usize = 8;

/// How large one frame of standard input is. The same size the file transport chunks at, so a frame
/// boundary and a read boundary agree and nothing is re-assembled on either side.
const STDIN_CHUNK_BYTES: usize = 64 * 1024;

/// How long the command's own ending may take to arrive once this side has done its part.
///
/// A cluster that leaves a non-terminal exec open after the process it ran is gone is the case this
/// is for: the status marker has already said the command is over, so waiting longer is waiting for
/// nothing.
const END_GRACE: Duration = Duration::from_secs(5);

/// How long the socket is kept after the API server sent the exec status.
///
/// The status is the last message of the protocol, but a frame carrying the marker may still be in
/// flight beside it; this is the window in which that frame is collected.
const STATUS_GRACE: Duration = Duration::from_secs(1);

/// How long the frames are still read after this side asked for the end of the input.
///
/// The protocol has no per-stream end for standard input, so closing the socket is what tells the
/// container there is no more — and the container's own answer is written after that, on a channel
/// the close has already made half-closed. This is the window in which it is still read.
const EOF_GRACE: Duration = Duration::from_secs(5);

/// How long the command is given to finish by itself once its input ended.
///
/// Short because closing early is harmless where it is not needed: a `tar` that stops at its own
/// terminator has already finished reading, and one that reads to the end of its input cannot finish
/// at all without the close that follows this window.
const INPUT_END_GRACE: Duration = Duration::from_millis(1000);

/// How much of a command's standard error is kept. The reason something failed is a line or two,
/// and the marker is the last thing printed, so a cap cannot hide it.
const MAX_STDERR_BYTES: usize = 64 * 1024;

/// The room a marker needs to be found when it was split across frames.
const MARKER_WINDOW: usize = 256;

/// What the script prints on standard error as its last act: its exit status between two record
/// separators.
///
/// The channel protocol carries no exit code, and on a download the standard output *is* a tar
/// archive, so the stream that never carries data is where the status goes. The separators are what
/// keeps a file's own stderr text from being read as one.
const MARKER_PREFIX: &str = "\u{1e}7shell-exit:";
const MARKER_SUFFIX: char = '\u{1e}';

/// The script with the status marker after it.
///
/// The status is captured into a variable first: `$?` is only the previous command's until
/// something else runs, and the `printf` about to run would otherwise be the previous command.
fn with_status_marker(script: &str) -> String {
    format!(
        "{script}\n__7shell_status=$?\nprintf '\\036%s\\036\\n' \"7shell-exit:$__7shell_status\" 1>&2\nexit \
         $__7shell_status"
    )
}

/// A command run to completion, with no terminal.
///
/// Two things differ from a terminal, and both come from the protocol: it carries no exit status, so
/// the script prints one, and it has no way to end standard input on its own — a command that reads
/// to the end of its input is ended by closing the channel, which is what [`RunSession::end_stdin`]
/// does after giving the command its chance to finish first.
pub struct RunSession {
    /// Standard input, taken once by the task that owns the socket; dropping it ends the input.
    stdin: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    /// The standard output, taken once by the one reader there is.
    stdout: Mutex<Option<mpsc::Receiver<Vec<u8>>>>,
    control: Arc<RunControl>,
    closed: AtomicBool,
}

impl RunSession {
    /// Sends bytes to the command's standard input, in frames it can absorb.
    pub async fn write(&self, bytes: &[u8]) -> Result<(), ShellError> {
        if bytes.is_empty() {
            return Ok(());
        }
        let sender = {
            let slot = self.stdin.lock().unwrap_or_else(|poison| poison.into_inner());
            slot.clone()
                .ok_or_else(|| ShellError::k8s_unavailable("the command's standard input has already ended"))?
        };
        for chunk in bytes.chunks(STDIN_CHUNK_BYTES) {
            // A bounded send is the backpressure: a file that is read faster than the pod reads
            // waits here rather than growing this process's memory by an archive.
            sender
                .send(chunk.to_vec())
                .await
                .map_err(|_| ShellError::k8s_unavailable("the pod exec session ended while its input was sent"))?;
        }
        Ok(())
    }

    /// Ends standard input.
    ///
    /// Not every reader stops at an archive's own terminator: the `tar` in a minimal image reads
    /// until its input ends, and this — followed by the close that gives it one — is what finishes
    /// such a command.
    pub fn end_stdin(&self) {
        let mut slot = self.stdin.lock().unwrap_or_else(|poison| poison.into_inner());
        slot.take();
    }

    /// The standard output, taken once.
    pub fn take_stdout(&self) -> Option<mpsc::Receiver<Vec<u8>>> {
        let mut slot = self.stdout.lock().unwrap_or_else(|poison| poison.into_inner());
        slot.take()
    }

    /// Everything the command wrote to standard error, without the status marker.
    pub fn stderr(&self) -> String {
        self.control.ending().stderr
    }

    /// The status the script reported, when it reported one.
    pub fn exit_status(&self) -> Option<i32> {
        self.control.ending().exit_status
    }

    /// Whether the end of this command's input had to be spelled by closing the socket.
    ///
    /// The channel protocol has no way to end one stream, so a command that reads to the end of its
    /// input is ended by the closing handshake — and whatever it says while it finishes is written
    /// to a channel that close has half-closed. A caller told `true` here knows the status it did
    /// not get was not lost in transit: there was no way to ask for it.
    pub fn input_ended_by_close(&self) -> bool {
        self.control.ending().input_closed
    }

    /// Waits for the end, and answers why it was not a normal one when it was not.
    pub async fn wait_ended(&self) -> Option<ShellError> {
        self.control.wait_ended().await
    }

    /// Ends the session now: what was collected stays, and the rest of the command's output is not
    /// read.
    pub fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = self.control.close.send_replace(true);
    }

    /// Runs the command to its end and answers everything it produced.
    ///
    /// `input` is written in bounded chunks and standard input is ended after it; the caller that
    /// wants the output as a stream instead takes it with [`RunSession::take_stdout`].
    pub async fn run_to_end(self, input: &[u8]) -> Result<RunOutput, ShellError> {
        self.write(input).await?;
        self.end_stdin();
        let stdout = {
            let mut slot = self.stdout.lock().unwrap_or_else(|poison| poison.into_inner());
            slot.take()
        };
        let mut collected = Vec::new();
        if let Some(mut stdout) = stdout {
            while let Some(chunk) = stdout.recv().await {
                collected.extend_from_slice(&chunk);
            }
        }
        let error = self.wait_ended().await;
        let output = RunOutput {
            stdout: collected,
            stderr: self.stderr(),
            exit_status: self.exit_status(),
            input_ended_by_close: self.input_ended_by_close(),
        };
        match error {
            Some(error) => Err(error),
            None => Ok(output),
        }
    }
}

/// What a command that ran to completion left behind.
#[derive(Debug)]
pub struct RunOutput {
    pub stdout: Vec<u8>,
    pub stderr: String,
    /// The script's status, when the cluster let it be read: absent means the channel ended before
    /// the marker arrived, not that the command failed.
    pub exit_status: Option<i32>,
    /// Whether this side had to close the channel to end the command's input, which is the one case
    /// where an absent status is expected rather than suspicious.
    pub input_ended_by_close: bool,
}

impl RunOutput {
    /// The reason this command failed, or `None` when it did not.
    ///
    /// A status is only evidence of success when it is zero. No status at all is not success — but
    /// it is also not a failure when this side ended the input by closing the channel: the protocol
    /// has nowhere to send the exit code after that, so the answer would be missing for every
    /// upload. What the command said on standard error before the end is then all the evidence
    /// there is, and it is reported as the reason.
    pub fn failure(&self) -> Option<String> {
        match self.exit_status {
            Some(0) => None,
            Some(code) if self.stderr.is_empty() => Some(format!("the command exited with status {code}")),
            Some(code) => Some(format!("{} (status {code})", self.stderr)),
            None if self.input_ended_by_close && self.stderr.is_empty() => None,
            None if !self.stderr.is_empty() => Some(self.stderr.clone()),
            None => Some("the command ended without reporting an exit status".to_owned()),
        }
    }
}

/// Opens a command to run to completion in one container of one pod, and answers once the upgrade
/// is done.
///
/// `stdin` decides whether the channel carries an input at all: asking for a stream a script will
/// never write to leaves the container's `tar` waiting for an end that this side never sends.
pub async fn open_run(client: &K8sClient, request: &K8sExecRequest, stdin: bool) -> Result<RunSession, ShellError> {
    if request.command.trim().is_empty() {
        return Err(ShellError::k8s_unavailable(
            "a command that runs to completion has to be one",
        ));
    }
    let transport = client.transport();
    let script = with_status_marker(&request.command);
    let url = run_url(transport, request, stdin, &script)?;
    let socket = upgrade(transport, url).await?;

    let (input, input_chunks) = if stdin {
        let (sender, chunks) = mpsc::channel(STDIN_QUEUE);
        (Some(sender), Some(chunks))
    } else {
        (None, None)
    };
    let (stdout, outputs) = mpsc::channel(OUTPUT_QUEUE);
    let control = Arc::new(RunControl::new());
    let running = Arc::clone(&control);
    let closing = control.close.subscribe();

    tokio::spawn(async move {
        let ending = run_pump(socket, input_chunks, stdout, closing).await;
        running.end(ending);
    });

    Ok(RunSession {
        stdin: Mutex::new(input),
        stdout: Mutex::new(Some(outputs)),
        control,
        closed: AtomicBool::new(false),
    })
}

/// The end of one run, shared with every waiter.
#[derive(Clone, Default)]
struct RunEnding {
    done: bool,
    exit_status: Option<i32>,
    stderr: String,
    error: Option<ShellError>,
    /// Whether the end of the input was spelled by closing the socket, which is what makes an
    /// absent status expected rather than a lost answer.
    input_closed: bool,
}

struct RunControl {
    close: watch::Sender<bool>,
    ended: watch::Sender<RunEnding>,
}

impl RunControl {
    fn new() -> Self {
        Self {
            close: watch::channel(false).0,
            ended: watch::channel(RunEnding::default()).0,
        }
    }

    fn ending(&self) -> RunEnding {
        self.ended.borrow().clone()
    }

    /// The first answer wins: a close racing the server's close produces one ending.
    fn end(&self, ending: RunEnding) {
        if self.ended.borrow().done {
            return;
        }
        let _ = self.ended.send_replace(ending);
    }

    async fn wait_ended(&self) -> Option<ShellError> {
        let mut ended = self.ended.subscribe();
        loop {
            let state = ended.borrow_and_update().clone();
            if state.done {
                return state.error;
            }
            if ended.changed().await.is_err() {
                return None;
            }
        }
    }
}

/// Standard error as it arrives, with the status marker read out of it.
///
/// The marker can be split across frames, so a tail window is kept beside the text that is reported:
/// the window is what the marker is searched in, and the text is what a person is shown.
#[derive(Default)]
struct StderrSink {
    text: Vec<u8>,
    tail: Vec<u8>,
    status: Option<i32>,
}

impl StderrSink {
    fn push(&mut self, bytes: &[u8]) {
        if self.text.len() < MAX_STDERR_BYTES {
            let room = MAX_STDERR_BYTES - self.text.len();
            self.text.extend_from_slice(&bytes[..bytes.len().min(room)]);
        }
        self.tail.extend_from_slice(bytes);
        if self.tail.len() > MARKER_WINDOW {
            self.tail.drain(..self.tail.len() - MARKER_WINDOW);
        }
        if self.status.is_none() {
            self.status = status_in(&self.tail);
        }
    }

    /// What the command said, without the marker: the marker is text this side wrote.
    fn message(&self) -> String {
        let text = String::from_utf8_lossy(&self.text);
        match text.find(MARKER_PREFIX) {
            Some(index) => text[..index].trim().to_owned(),
            None => text.trim().to_owned(),
        }
    }
}

/// The status a marker carries, when a whole one is in `bytes`.
fn status_in(bytes: &[u8]) -> Option<i32> {
    let text = std::str::from_utf8(bytes).ok()?;
    let start = text.find(MARKER_PREFIX)? + MARKER_PREFIX.len();
    let rest = &text[start..];
    let end = rest.find(MARKER_SUFFIX)?;
    rest[..end].trim().parse().ok()
}

/// The one task that owns a run's socket: it forwards the input, keeps the two outputs apart, and
/// ends when the command is over.
///
/// Output delivery comes first in the select, for the same reason a terminal's does: bytes already
/// read are handed over before more are read, which is what bounds this process's memory.
async fn run_pump(
    mut socket: Socket,
    mut input: Option<mpsc::Receiver<Vec<u8>>>,
    stdout: mpsc::Sender<Vec<u8>>,
    mut close: watch::Receiver<bool>,
) -> RunEnding {
    let mut stderr = StderrSink::default();
    let mut failure: Option<ShellError> = None;
    let mut pending: Option<Vec<u8>> = None;
    let mut stdin_done = input.is_none();
    // Whether this session was given an input at all. Only then is there an input to end, and only
    // then is closing the socket part of the command's own contract.
    let had_input = input.is_some();
    // Whether this side has already asked the container to see the end of its input. The channel
    // carries no way to end one stream, so the whole socket is closed for it — and that is done
    // once, after the command had its chance to finish on the archive's own terminator.
    let mut ended_input = false;
    // Whether the protocol itself said this exec is over. Once it has, a socket that breaks is
    // transport noise after the fact, not a command that failed.
    let mut status_seen = false;
    // The moment after which the socket is closed: set when this side has done its part and the
    // server has not closed the channel.
    let mut grace: Option<tokio::time::Instant> = None;

    loop {
        let reading = pending.is_none();
        let grace_running = grace.is_some();
        let grace_at = grace.unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(3600));
        let input_running = !stdin_done && reading;
        tokio::select! {
            biased;
            _ = close.changed() => {
                close_socket(&mut socket).await;
                break;
            }
            _ = tokio::time::sleep_until(grace_at), if grace_running => {
                if had_input && stdin_done && !ended_input {
                    // The command is still reading, so nothing but closing the channel will end its
                    // input. The close is announced rather than the socket dropped, because the
                    // frames that say how the command ended arrive after the container has seen
                    // the end of its input.
                    ended_input = true;
                    let _ = socket.send(Message::Close(None)).await;
                    grace = Some(tokio::time::Instant::now() + EOF_GRACE);
                    continue;
                }
                close_socket(&mut socket).await;
                break;
            }
            permit = stdout.reserve(), if !reading => {
                match permit {
                    Ok(permit) => permit.send(pending.take().expect("guarded by the branch condition")),
                    // Nobody is reading any more, so there is nothing to keep the session for.
                    Err(_) => break,
                }
            }
            chunk = next_input(&mut input), if input_running => {
                match chunk {
                    Some(bytes) => {
                        let mut frame = Vec::with_capacity(bytes.len() + 1);
                        frame.push(CHANNEL_STDIN);
                        frame.extend_from_slice(&bytes);
                        if let Err(error) = socket.send(Message::binary(frame)).await {
                            failure.get_or_insert_with(|| channel_error(error));
                            break;
                        }
                    }
                    None => {
                        stdin_done = true;
                        // The input is over. Some readers finish as soon as the archive's terminator
                        // arrived; the ones in minimal images read until their input ends, and that
                        // is what the deadline below is for.
                        grace = Some(tokio::time::Instant::now() + INPUT_END_GRACE);
                    }
                }
            }
            frame = socket.next(), if reading => {
                match frame {
                    Some(Ok(Message::Binary(bytes))) => match bytes.first().copied() {
                        Some(CHANNEL_STDOUT) if bytes.len() > 1 => pending = Some(bytes[1..].to_vec()),
                        Some(CHANNEL_STDERR) if bytes.len() > 1 => {
                            stderr.push(&bytes[1..]);
                            if stderr.status.is_some() {
                                // The command is over; the server closes a moment later, and a
                                // server that does not is what the deadline is for.
                                grace.get_or_insert(tokio::time::Instant::now() + END_GRACE);
                            }
                        }
                        Some(CHANNEL_ERROR) => {
                            // The status is the protocol's last message and it is not the exit
                            // code: it says whether the exec worked at all. A marker still in
                            // flight beside it is collected during the short grace.
                            status_seen = true;
                            failure = status_error(&bytes[1..]);
                            grace = Some(match grace {
                                Some(at) => at.min(tokio::time::Instant::now() + STATUS_GRACE),
                                None => tokio::time::Instant::now() + STATUS_GRACE,
                            });
                        }
                        _ => {}
                    },
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        // A command that already said how it ended is not failed again by the
                        // channel that carried the answer: a kubelet that resets the socket after
                        // the last frame is what this arm would otherwise report. The same holds
                        // after this side asked for the end of the input, because the closing
                        // handshake is exactly where the answer stops being readable.
                        if !status_seen && stderr.status.is_none() && !ended_input {
                            failure.get_or_insert_with(|| channel_error(error));
                        }
                        break;
                    }
                }
            }
        }
    }

    RunEnding {
        done: true,
        exit_status: stderr.status,
        stderr: stderr.message(),
        error: failure,
        input_closed: ended_input,
    }
}

/// The next chunk of standard input, or nothing once the sender is gone.
async fn next_input(input: &mut Option<mpsc::Receiver<Vec<u8>>>) -> Option<Vec<u8>> {
    match input {
        Some(chunks) => chunks.recv().await,
        None => None,
    }
}

/// How long the local end of a tunnel may take to answer a CONNECT.
const BRIDGE_ANSWER_TIMEOUT: Duration = Duration::from_secs(15);

/// The largest answer head the tunnel may send. A 200 is a status line and a blank line; anything
/// bigger is not this client's answer, and buffering it would be buffering somebody else's.
const MAX_BRIDGE_ANSWER: usize = 8 * 1024;

/// Asks the tunnel to carry the cluster, answering the socket to upgrade.
///
/// The CONNECT names the server the kubeconfig gave, which is the only one the listener carries: a
/// client that could name another would turn a saved SSH session into a proxy for whatever it
/// asked for. The answer has to be a 2xx, because a bridge that refused or could not reach the far
/// side must fail here rather than half-way into a TLS handshake.
async fn dial_bridge(bridge: &Bridge) -> Result<TcpStream, ShellError> {
    let authority = bridge.target.authority();
    let mut socket = tokio::time::timeout(BRIDGE_ANSWER_TIMEOUT, TcpStream::connect(bridge.endpoint))
        .await
        .map_err(|_| ShellError::k8s_unavailable("the local end of the tunnel did not answer in time"))?
        .map_err(|error| {
            ShellError::k8s_unavailable(format!("the local end of the tunnel could not be reached: {error}"))
        })?;
    socket
        .set_nodelay(true)
        .map_err(|error| ShellError::k8s_unavailable(format!("the tunnel could not be tuned: {error}")))?;

    let request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n");
    socket
        .write_all(request.as_bytes())
        .await
        .map_err(|error| ShellError::k8s_unavailable(format!("the tunnel could not be asked: {error}")))?;

    let answer = tokio::time::timeout(BRIDGE_ANSWER_TIMEOUT, read_bridge_answer(&mut socket))
        .await
        .map_err(|_| ShellError::k8s_unavailable("the local end of the tunnel did not answer in time"))??;

    match answer {
        BridgeAnswer::Carried => Ok(socket),
        BridgeAnswer::Refused(line) => Err(ShellError::k8s_unavailable(format!(
            "the tunnel refused to carry {authority}: {}",
            truncate_chars(&line, ERROR_CHARS)
        ))),
    }
}

/// What the tunnel said about a CONNECT request.
enum BridgeAnswer {
    Carried,
    /// The status line, as the tunnel wrote it.
    Refused(String),
}

/// Reads one HTTP answer head from the bridge, one byte at a time.
///
/// Byte-wise rather than in chunks because the connection is not an HTTP connection after the
/// answer: anything read beyond the blank line would be TLS the handshake never sees. A status line
/// of 2xx is the only thing that carries.
async fn read_bridge_answer(socket: &mut TcpStream) -> Result<BridgeAnswer, ShellError> {
    let mut head = Vec::with_capacity(128);
    let mut byte = [0u8; 1];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = socket.read(&mut byte).await.map_err(|error| {
            ShellError::k8s_unavailable(format!("the answer of the tunnel could not be read: {error}"))
        })?;
        if read == 0 {
            return Err(ShellError::k8s_unavailable(
                "the local end of the tunnel ended before it answered the CONNECT",
            ));
        }
        head.push(byte[0]);
        if head.len() > MAX_BRIDGE_ANSWER {
            return Err(ShellError::k8s_unavailable(
                "the local end of the tunnel answered more than a tunnel answer",
            ));
        }
    }

    let text = String::from_utf8_lossy(&head).into_owned();
    let line = text.lines().next().unwrap_or_default().trim().to_owned();
    let carried = line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .is_some_and(|code| (200..300).contains(&code));
    Ok(if carried {
        BridgeAnswer::Carried
    } else {
        BridgeAnswer::Refused(line)
    })
}

/// The one task that owns the socket: it forwards what was posted to it, and it reads.
///
/// Reads and writes share the task because the two halves cannot be separated from the socket
/// without splitting it, and a split socket has to be polled from both sides anyway. The select is
/// biased towards delivery first: output already read is handed over before more is read, which is
/// what bounds this process's memory while a terminal is not being watched.
async fn pump(
    mut socket: Socket,
    mut commands: mpsc::UnboundedReceiver<Command>,
    output: mpsc::Sender<Vec<u8>>,
    mut close: watch::Receiver<bool>,
    columns: u32,
    rows: u32,
) -> Option<ShellError> {
    if columns > 0 && rows > 0 {
        match resize_frame(columns, rows) {
            Ok(frame) => {
                if let Err(error) = socket.send(frame).await {
                    return Some(channel_error(error));
                }
            }
            Err(error) => return Some(error),
        }
    }

    let mut status: Option<ShellError> = None;
    // Output read from the socket but not yet delivered. Keeping it here rather than inside a
    // `send` future is what lets a close interrupt a delivery that is waiting for room: dropping
    // that future would drop the bytes.
    let mut pending: Option<Vec<u8>> = None;

    loop {
        tokio::select! {
            biased;
            _ = close.changed() => {
                close_socket(&mut socket).await;
                break;
            }
            permit = output.reserve(), if pending.is_some() => {
                match permit {
                    Ok(permit) => permit.send(pending.take().expect("guarded by the branch condition")),
                    // Nobody is reading any more, so there is nothing to keep the session open for.
                    Err(_) => break,
                }
            }
            command = commands.recv(), if pending.is_none() => {
                match command {
                    Some(Command::Frame(frame)) => {
                        if let Err(error) = socket.send(frame).await {
                            status.get_or_insert_with(|| channel_error(error));
                            break;
                        }
                    }
                    // Every sender dropped: the session itself is gone.
                    None => {
                        close_socket(&mut socket).await;
                        break;
                    }
                }
            }
            frame = socket.next(), if pending.is_none() => {
                match frame {
                    Some(Ok(Message::Binary(bytes))) => match bytes.first().copied() {
                        Some(CHANNEL_STDOUT | CHANNEL_STDERR) if bytes.len() > 1 => {
                            pending = Some(bytes[1..].to_vec());
                        }
                        Some(CHANNEL_ERROR) => {
                            // The status is the last thing an exec sends: the process is over, and
                            // `Success` means it exited zero. Ending here rather than waiting for
                            // the socket to close is what keeps a cluster that leaves the channel
                            // open from leaving a terminal nobody can close either.
                            status = status_error(&bytes[1..]);
                            close_socket(&mut socket).await;
                            break;
                        }
                        _ => {}
                    },
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        status.get_or_insert_with(|| channel_error(error));
                        break;
                    }
                }
            }
        }
    }

    status
}

/// Ends the socket politely, but not for longer than [`CLOSE_TIMEOUT`].
async fn close_socket(socket: &mut Socket) {
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, socket.close(None)).await;
}

/// A transport failure, as the envelope the interface shows.
fn channel_error(error: WsError) -> ShellError {
    ShellError::k8s_unavailable(format!(
        "the pod exec channel ended: {}",
        truncate_chars(&error.to_string(), ERROR_CHARS)
    ))
}

/// The `Status` document the API server sends on channel 3.
#[derive(serde::Deserialize)]
struct Status {
    #[serde(default)]
    status: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    details: StatusDetails,
}

#[derive(serde::Deserialize, Default)]
struct StatusDetails {
    #[serde(default)]
    causes: Vec<StatusCause>,
}

#[derive(serde::Deserialize)]
struct StatusCause {
    #[serde(default)]
    message: String,
}

/// Channel 3 as an error, or `None` when there is nothing to report.
///
/// `Success` is the normal way an exec ends: the process exited zero, so the terminal did not fail.
/// Anything that is not JSON is passed through as text, which is what a proxy in front of the API
/// server tends to send.
fn status_error(payload: &[u8]) -> Option<ShellError> {
    if payload.is_empty() {
        return None;
    }
    let parsed: Status = match serde_json::from_slice(payload) {
        Ok(parsed) => parsed,
        Err(_) => {
            let text = String::from_utf8_lossy(payload).trim().to_owned();
            if text.is_empty() || text == "Success" {
                return None;
            }
            return Some(ShellError::k8s_unavailable(truncate_chars(&text, ERROR_CHARS)));
        }
    };
    if parsed.status == "Success" {
        return None;
    }
    let message = if !parsed.message.is_empty() {
        parsed.message
    } else if let Some(cause) = parsed.details.causes.first().filter(|cause| !cause.message.is_empty()) {
        cause.message.clone()
    } else {
        String::from_utf8_lossy(payload).trim().to_owned()
    };
    if message.is_empty() {
        return None;
    }
    Some(ShellError::k8s_unavailable(truncate_chars(&message, ERROR_CHARS)))
}

/// The size message, with the field names the protocol spells in capitals.
#[derive(serde::Serialize)]
struct ResizeFrame {
    #[serde(rename = "Width")]
    width: u16,
    #[serde(rename = "Height")]
    height: u16,
}

fn resize_frame(columns: u32, rows: u32) -> Result<Message, ShellError> {
    let size = ResizeFrame {
        width: terminal_dimension(columns),
        height: terminal_dimension(rows),
    };
    let encoded = serde_json::to_vec(&size)
        .map_err(|error| ShellError::app_internal(format!("the terminal size could not be encoded: {error}")))?;
    let mut frame = Vec::with_capacity(encoded.len() + 1);
    frame.push(CHANNEL_RESIZE);
    frame.extend_from_slice(&encoded);
    Ok(Message::binary(frame))
}

/// A terminal dimension as the protocol carries it: one cell at the least, `u16::MAX` at the most.
fn terminal_dimension(value: u32) -> u16 {
    value.clamp(1, u32::from(u16::MAX)) as u16
}

/// `ws(s)://…/api/v1/namespaces/{namespace}/pods/{pod}/exec` with the query an interactive shell
/// needs.
fn exec_url(transport: &K8sTransport, request: &K8sExecRequest) -> Result<reqwest::Url, ShellError> {
    let command = request.command.as_str();
    let script = if command.trim().is_empty() {
        SHELL_COMMAND
    } else {
        command
    };
    command_url(transport, request, &command_query(true, true, script))
}

/// The same address for a command that is not a terminal: no pty, and standard input only when the
/// script is going to be fed one.
fn run_url(
    transport: &K8sTransport,
    request: &K8sExecRequest,
    stdin: bool,
    script: &str,
) -> Result<reqwest::Url, ShellError> {
    command_url(transport, request, &command_query(stdin, false, script))
}

/// What the upgrade asks the API server to do: which streams, whether there is a terminal, and the
/// argv the container runs.
fn command_query(stdin: bool, tty: bool, script: &str) -> Vec<(&'static str, &str)> {
    vec![
        ("stdin", if stdin { "true" } else { "false" }),
        ("stdout", "true"),
        ("stderr", "true"),
        // A terminal merges the two streams and translates line endings; a command whose output is
        // parsed, or is a tar archive, can have neither.
        ("tty", if tty { "true" } else { "false" }),
    ]
    .into_iter()
    .chain([("command", "/bin/sh"), ("command", "-c"), ("command", script)])
    .collect()
}

/// The address, with the container and the argv that `pairs` describes.
fn command_url(
    transport: &K8sTransport,
    request: &K8sExecRequest,
    pairs: &[(&'static str, &str)],
) -> Result<reqwest::Url, ShellError> {
    let mut url = reqwest::Url::parse(&transport.base)
        .map_err(|error| ShellError::k8s_unavailable(format!("the cluster address is not a URL: {error}")))?;
    let secure = matches!(url.scheme(), "https" | "wss");
    url.set_scheme(if secure { "wss" } else { "ws" })
        .map_err(|()| ShellError::k8s_unavailable("the cluster address cannot be upgraded to a WebSocket"))?;
    {
        // Same reasoning as the log path: names are escaped by the URL builder rather than pasted in.
        let mut segments = url
            .path_segments_mut()
            .map_err(|()| ShellError::k8s_unavailable("the cluster address cannot carry a path"))?;
        segments.pop_if_empty();
        segments
            .push("api")
            .push("v1")
            .push("namespaces")
            .push(&request.namespace)
            .push("pods")
            .push(&request.pod)
            .push("exec");
    }
    {
        let mut query = url.query_pairs_mut();
        // Empty means the pod's own annotation decides, which is what a person expects when they
        // did not choose a container.
        if !request.container.is_empty() {
            query.append_pair("container", &request.container);
        }
        for (key, value) in pairs {
            // Repeated keys are an argv the API server assembles; a single value is not one.
            query.append_pair(key, value);
        }
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    //! A real WebSocket server on 127.0.0.1 speaks the channel protocol back, frame by frame.
    //! A mock would prove this client agrees with a mock; tungstenite's server side proves it agrees
    //! with a peer that does its own handshake, which is the half that actually breaks.

    use std::time::Duration;

    use tokio::net::TcpListener;
    use tokio::sync::mpsc::Receiver;
    use tokio::time::timeout;
    use tokio_tungstenite::accept_hdr_async;
    use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};

    use super::*;
    use crate::client::K8sTransport;

    /// What the server saw, filled in as the connection progresses.
    #[derive(Default)]
    struct Seen {
        subprotocol: Option<String>,
        authorization: Option<String>,
        host: Option<String>,
        target: String,
        frames: Vec<Vec<u8>>,
    }

    /// A WebSocket server that answers one connection: greets with `outgoing`, then records what
    /// the client sends until it closes.
    struct FakeExecServer {
        base: String,
        seen: Arc<Mutex<Seen>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl FakeExecServer {
        async fn start(outgoing: Vec<Vec<u8>>) -> Self {
            Self::start_answering(outgoing, PROTOCOL.to_owned()).await
        }

        /// The same server, answering the upgrade with `answer` instead of the protocol it was
        /// asked for — the case a cluster behind a rewriting proxy produces.
        // The callback's error type is tungstenite's own `ErrorResponse`; the size is the
        // library's, and the alternative is translating an error this server never raises.
        #[allow(clippy::result_large_err)]
        async fn start_answering(outgoing: Vec<Vec<u8>>, answer: String) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
            let port = listener.local_addr().expect("an address").port();
            let seen = Arc::new(Mutex::new(Seen::default()));
            let recorded = Arc::clone(&seen);
            let task = tokio::spawn(async move {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let callback = {
                    let recorded = Arc::clone(&recorded);
                    move |request: &Request, mut response: Response| -> Result<Response, ErrorResponse> {
                        {
                            let mut seen = recorded.lock().expect("the seen lock");
                            seen.subprotocol = request
                                .headers()
                                .get(SEC_WEBSOCKET_PROTOCOL)
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned);
                            seen.authorization = request
                                .headers()
                                .get(AUTHORIZATION)
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned);
                            seen.host = request
                                .headers()
                                .get(HOST)
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned);
                            seen.target = request.uri().to_string();
                        }
                        let answered = HeaderValue::from_str(&answer).expect("an answer header");
                        response.headers_mut().insert(SEC_WEBSOCKET_PROTOCOL, answered);
                        Ok(response)
                    }
                };
                let Ok(mut socket) = accept_hdr_async(stream, callback).await else {
                    return;
                };
                for frame in outgoing {
                    if socket.send(Message::binary(frame)).await.is_err() {
                        return;
                    }
                }
                while let Some(Ok(message)) = socket.next().await {
                    match message {
                        Message::Binary(bytes) => {
                            recorded.lock().expect("the seen lock").frames.push(bytes.to_vec());
                        }
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
            });
            Self {
                base: format!("http://127.0.0.1:{port}"),
                seen,
                task,
            }
        }

        fn client(&self) -> K8sClient {
            self.client_with(0, String::new())
        }

        fn client_with(&self, authorization: u8, server: String) -> K8sClient {
            let authorization = match authorization {
                0 => Some("Bearer test-token".to_owned()),
                _ => None,
            };
            K8sClient {
                transport: K8sTransport {
                    base: self.base.clone(),
                    server,
                    tls: native_tls::TlsConnector::new().expect("a TLS connector"),
                    authorization,
                    bridge: None,
                },
                http: reqwest::Client::new(),
            }
        }

        fn seen(&self) -> Seen {
            let seen = self.seen.lock().expect("the seen lock");
            Seen {
                subprotocol: seen.subprotocol.clone(),
                authorization: seen.authorization.clone(),
                host: seen.host.clone(),
                target: seen.target.clone(),
                frames: seen.frames.clone(),
            }
        }

        async fn frames(&self, at_least: usize) -> Vec<Vec<u8>> {
            for _ in 0..400 {
                let frames = self.seen().frames;
                if frames.len() >= at_least {
                    return frames;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!(
                "the server received {} frames, fewer than the {at_least} expected",
                self.seen().frames.len()
            );
        }
    }

    impl Drop for FakeExecServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    /// One frame as the protocol writes it.
    fn frame(channel: u8, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(payload.len() + 1);
        bytes.push(channel);
        bytes.extend_from_slice(payload);
        bytes
    }

    fn request(command: &str) -> K8sExecRequest {
        K8sExecRequest {
            namespace: "team-a".to_owned(),
            pod: "web-0".to_owned(),
            container: "app".to_owned(),
            command: command.to_owned(),
        }
    }

    /// The query pairs of a request target, decoded, so a test can talk about what was asked for
    /// rather than about percent signs.
    fn query(target: &str) -> Vec<(String, String)> {
        let url = reqwest::Url::parse(&format!("ws://example.invalid{target}")).expect("a URL");
        url.query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect()
    }

    fn value_of(target: &str, key: &str) -> Vec<String> {
        query(target)
            .into_iter()
            .filter(|(name, _)| name == key)
            .map(|(_, value)| value)
            .collect()
    }

    async fn chunks(output: &mut Receiver<Vec<u8>>, wanted: usize) -> Vec<u8> {
        let mut collected = Vec::new();
        for index in 0..wanted {
            match timeout(Duration::from_secs(5), output.recv()).await {
                Ok(Some(bytes)) => collected.extend_from_slice(&bytes),
                other => panic!("expected {wanted} output chunks, got {index} and then {other:?}"),
            }
        }
        collected
    }

    #[test]
    fn an_exec_session_can_live_in_a_registry_behind_a_lock() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Arc<ExecSession>>();
    }

    #[tokio::test]
    async fn the_upgrade_asks_for_the_v4_channel_subprotocol_and_carries_the_cluster_credential() {
        let server = FakeExecServer::start(Vec::new()).await;
        let session = open_exec(&server.client(), &request("ls -l"), 120, 40)
            .await
            .expect("an exec session");
        let seen = server.seen();
        assert_eq!(
            seen.subprotocol.as_deref(),
            Some(PROTOCOL),
            "the upgrade did not ask for the channel protocol"
        );
        assert_eq!(
            seen.authorization.as_deref(),
            Some("Bearer test-token"),
            "the upgrade did not carry the cluster credential"
        );
        assert_eq!(
            seen.target.split('?').next(),
            Some("/api/v1/namespaces/team-a/pods/web-0/exec"),
            "the exec path was {}",
            seen.target
        );
        for expected in [
            ("container", vec!["app".to_owned()]),
            ("stdin", vec!["true".to_owned()]),
            ("stdout", vec!["true".to_owned()]),
            ("stderr", vec!["true".to_owned()]),
            ("tty", vec!["true".to_owned()]),
            (
                "command",
                vec!["/bin/sh".to_owned(), "-c".to_owned(), "ls -l".to_owned()],
            ),
        ] {
            assert_eq!(
                value_of(&seen.target, expected.0),
                expected.1,
                "the {expected:?} parameter was not the argv the API expects"
            );
        }
        session.close();
    }

    #[tokio::test]
    async fn a_request_without_a_command_asks_for_the_containers_own_shell() {
        let server = FakeExecServer::start(Vec::new()).await;
        let session = open_exec(&server.client(), &request(""), 80, 24)
            .await
            .expect("an exec session");
        let target = server.seen().target;
        let command = value_of(&target, "command");
        assert_eq!(command.len(), 3, "the command was not an argv: {command:?}");
        assert_eq!(command[0], "/bin/sh");
        assert_eq!(command[1], "-c");
        assert!(
            command[2].contains("command -v bash"),
            "an empty command must probe the container's shell, not run an empty string: {command:?}"
        );
        session.close();
    }

    #[tokio::test]
    async fn the_terminal_size_travels_on_channel_four_with_the_protocol_field_names() {
        let server = FakeExecServer::start(Vec::new()).await;
        let session = open_exec(&server.client(), &request(""), 120, 40)
            .await
            .expect("an exec session");
        session.resize(200, 60).expect("a resize");
        let frames = server.frames(2).await;
        let first = String::from_utf8_lossy(&frames[0][1..]).into_owned();
        assert_eq!(frames[0][0], CHANNEL_RESIZE, "the initial size was not on channel 4");
        assert!(
            first.contains("\"Width\":120") && first.contains("\"Height\":40"),
            "the initial size was {first}"
        );
        let second = String::from_utf8_lossy(&frames[1][1..]).into_owned();
        assert_eq!(frames[1][0], CHANNEL_RESIZE, "a later resize was not on channel 4");
        assert!(
            second.contains("\"Width\":200") && second.contains("\"Height\":60"),
            "the resize was {second}"
        );
        session.close();
    }

    #[tokio::test]
    async fn typing_goes_out_on_channel_zero() {
        let server = FakeExecServer::start(Vec::new()).await;
        let session = open_exec(&server.client(), &request(""), 80, 24)
            .await
            .expect("an exec session");
        session.write(b"ls -l\r").expect("a write");
        // The initial size is a frame of its own, so the input is not necessarily first.
        let frames = server.frames(2).await;
        let typed = frames
            .iter()
            .find(|frame| frame.first() == Some(&CHANNEL_STDIN))
            .unwrap_or_else(|| panic!("no frame arrived on channel 0: {frames:?}"));
        assert_eq!(&typed[1..], b"ls -l\r", "input was altered on the way out");
        session.close();
    }

    #[tokio::test]
    async fn output_from_channels_one_and_two_arrives_as_one_stream() {
        let server = FakeExecServer::start(vec![
            frame(CHANNEL_STDOUT, b"out-1"),
            frame(CHANNEL_STDERR, b"err-1"),
            frame(CHANNEL_STDOUT, b"out-2"),
        ])
        .await;
        let session = open_exec(&server.client(), &request(""), 80, 24)
            .await
            .expect("an exec session");
        let mut output = session.take_output().expect("an output receiver");
        assert_eq!(chunks(&mut output, 3).await, b"out-1err-1out-2");
        session.close();
    }

    #[tokio::test]
    async fn an_error_status_on_channel_three_becomes_the_reason_the_session_ended() {
        let status = r#"{"status":"Failure","message":"container is not running","reason":"BadRequest"}"#;
        let server = FakeExecServer::start(vec![
            frame(CHANNEL_STDOUT, b"before\n"),
            frame(CHANNEL_ERROR, status.as_bytes()),
        ])
        .await;
        let session = open_exec(&server.client(), &request(""), 80, 24)
            .await
            .expect("an exec session");
        let mut output = session.take_output().expect("an output receiver");
        assert_eq!(chunks(&mut output, 1).await, b"before\n");
        // No close here on purpose: the status alone has to end the session, because a cluster may
        // leave the exec channel open after the process it was running is gone.
        let error = timeout(Duration::from_secs(2), session.wait_ended())
            .await
            .expect("the status did not end the session")
            .expect("a failure status is an error");
        assert!(
            error.to_string().contains("container is not running"),
            "the status message was lost: {error}"
        );
    }

    #[tokio::test]
    async fn a_success_status_is_a_normal_exit_not_an_error() {
        let status = r#"{"status":"Success"}"#;
        let server = FakeExecServer::start(vec![
            frame(CHANNEL_STDOUT, b"done\n"),
            frame(CHANNEL_ERROR, status.as_bytes()),
        ])
        .await;
        let session = open_exec(&server.client(), &request(""), 80, 24)
            .await
            .expect("an exec session");
        let mut output = session.take_output().expect("an output receiver");
        assert_eq!(chunks(&mut output, 1).await, b"done\n");
        assert_eq!(
            timeout(Duration::from_secs(2), session.wait_ended())
                .await
                .expect("the success status did not end the session"),
            None,
            "a zero exit was reported as a failure"
        );
    }

    #[tokio::test]
    async fn closing_twice_ends_the_session_once_and_stops_the_reader() {
        let server = FakeExecServer::start(Vec::new()).await;
        let session = Arc::new(
            open_exec(&server.client(), &request(""), 80, 24)
                .await
                .expect("an exec session"),
        );
        let mut output = session.take_output().expect("an output receiver");
        session.close();
        session.close();
        let ended = timeout(Duration::from_secs(2), session.wait_ended()).await;
        assert_eq!(ended.expect("closing took too long"), None);
        assert_eq!(
            timeout(Duration::from_secs(2), output.recv())
                .await
                .expect("the reader ended"),
            None,
            "the output stream stayed open after the session closed"
        );
        assert!(session.write(b"late").is_err(), "a closed session accepted keystrokes");
    }

    #[tokio::test]
    async fn the_output_half_can_be_taken_once() {
        let server = FakeExecServer::start(Vec::new()).await;
        let session = open_exec(&server.client(), &request(""), 80, 24)
            .await
            .expect("an exec session");
        assert!(session.take_output().is_some(), "the first reader got nothing");
        assert!(
            session.take_output().is_none(),
            "a second reader was handed the same terminal"
        );
        session.close();
    }

    #[tokio::test]
    async fn the_host_header_carries_the_kubeconfig_server_name_when_the_address_is_an_ip() {
        let server = FakeExecServer::start(Vec::new()).await;
        let client = server.client_with(0, "api.cluster.example".to_owned());
        let session = open_exec(&client, &request(""), 80, 24).await.expect("an exec session");
        assert_eq!(
            server.seen().host.as_deref(),
            Some("api.cluster.example"),
            "the Host header did not carry the name the kubeconfig gave"
        );
        session.close();
    }

    #[tokio::test]
    async fn a_server_that_answers_with_another_channel_protocol_is_not_a_session() {
        let server = FakeExecServer::start_answering(Vec::new(), "v5.channel.k8s.io".to_owned()).await;
        let opened = open_exec(&server.client(), &request(""), 80, 24).await;
        match opened {
            Ok(session) => {
                session.close();
                panic!("a server speaking another protocol was accepted as a terminal");
            }
            Err(error) => assert_eq!(error.code(), sevenshell_error::ErrorCode::K8sUnavailable),
        }
    }

    #[tokio::test]
    async fn a_server_without_a_credential_opens_a_session_without_one() {
        let server = FakeExecServer::start(Vec::new()).await;
        let client = server.client_with(1, String::new());
        let session = open_exec(&client, &request(""), 80, 24)
            .await
            .expect("a client certificate needs no bearer header");
        assert_eq!(
            server.seen().authorization,
            None,
            "a bearer header was sent with no token"
        );
        session.close();
    }

    // --- A command run to completion -----------------------------------------

    /// A client that reaches `base` with a bearer token, for the servers below.
    fn client_at(base: &str) -> K8sClient {
        K8sClient {
            transport: K8sTransport {
                base: base.to_owned(),
                server: String::new(),
                tls: native_tls::TlsConnector::new().expect("a TLS connector"),
                authorization: Some("Bearer test-token".to_owned()),
                bridge: None,
            },
            http: reqwest::Client::new(),
        }
    }

    /// A WebSocket server for one run connection.
    ///
    /// It answers with the frames it was made with once the client's input has reached
    /// `expect_input` bytes — the way a `tar` answers once it has read a whole archive — and then
    /// either closes, or stays open the way a cluster that does not close a finished exec does.
    struct FakeRunServer {
        base: String,
        seen: Arc<Mutex<Seen>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl FakeRunServer {
        // The callback's error type is tungstenite's own `ErrorResponse`; the size is the library's
        // and the alternative is translating an error this server never raises.
        #[allow(clippy::result_large_err)]
        async fn start(answer: Vec<Vec<u8>>, expect_input: usize, keep_open: bool) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
            let port = listener.local_addr().expect("an address").port();
            let seen = Arc::new(Mutex::new(Seen::default()));
            let recorded = Arc::clone(&seen);
            let task = tokio::spawn(async move {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let callback = {
                    let recorded = Arc::clone(&recorded);
                    move |request: &Request, mut response: Response| -> Result<Response, ErrorResponse> {
                        {
                            let mut seen = recorded.lock().expect("the seen lock");
                            seen.subprotocol = request
                                .headers()
                                .get(SEC_WEBSOCKET_PROTOCOL)
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned);
                            seen.target = request.uri().to_string();
                        }
                        // The upgrade refuses a server that answers no subprotocol, exactly as a
                        // rewriting proxy would be refused.
                        let answered = HeaderValue::from_str(PROTOCOL).expect("the protocol header");
                        response.headers_mut().insert(SEC_WEBSOCKET_PROTOCOL, answered);
                        Ok(response)
                    }
                };
                let Ok(mut socket) = accept_hdr_async(stream, callback).await else {
                    return;
                };
                let mut received = 0usize;
                while received < expect_input {
                    match socket.next().await {
                        Some(Ok(Message::Binary(bytes))) => {
                            let input = bytes.first() == Some(&CHANNEL_STDIN);
                            recorded.lock().expect("the seen lock").frames.push(bytes.to_vec());
                            if input {
                                received += bytes.len() - 1;
                            }
                        }
                        Some(Ok(_)) => {}
                        _ => return,
                    }
                }
                for frame in answer {
                    if socket.send(Message::binary(frame)).await.is_err() {
                        return;
                    }
                }
                if keep_open {
                    // Nothing more arrives, and the client is expected to end this by itself.
                    std::future::pending::<()>().await;
                }
                // A close handshake, which is how a cluster that is done with the exec ends it.
                let _ = socket.close(None).await;
            });
            Self {
                base: format!("http://127.0.0.1:{port}"),
                seen,
                task,
            }
        }

        fn client(&self) -> K8sClient {
            client_at(&self.base)
        }

        fn seen(&self) -> Seen {
            let seen = self.seen.lock().expect("the seen lock");
            Seen {
                subprotocol: seen.subprotocol.clone(),
                authorization: seen.authorization.clone(),
                host: seen.host.clone(),
                target: seen.target.clone(),
                frames: seen.frames.clone(),
            }
        }

        async fn frames(&self, at_least: usize) -> Vec<Vec<u8>> {
            for _ in 0..400 {
                let frames = self.seen().frames;
                if frames.len() >= at_least {
                    return frames;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!(
                "the server received {} frames, fewer than the {at_least} expected",
                self.seen().frames.len()
            );
        }
    }

    impl Drop for FakeRunServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    /// The frames a command that worked would produce: an answer, its marker, and the protocol's
    /// own status.
    fn successful_run(stdout: &[u8], stderr_text: &[u8], status: i32) -> Vec<Vec<u8>> {
        let marker = format!("\u{1e}7shell-exit:{status}\u{1e}\n");
        let mut stderr = stderr_text.to_vec();
        stderr.extend_from_slice(marker.as_bytes());
        vec![
            frame(CHANNEL_STDOUT, stdout),
            frame(CHANNEL_STDERR, &stderr),
            frame(CHANNEL_ERROR, br#"{"status":"Success"}"#),
        ]
    }

    #[tokio::test]
    async fn a_command_run_to_completion_asks_for_no_terminal_and_keeps_the_streams_apart() {
        let server = FakeRunServer::start(successful_run(b"out-1", b"", 0), 0, false).await;
        let session = open_run(&server.client(), &request("ls -l"), false)
            .await
            .expect("a run session");
        let output = session.run_to_end(&[]).await.expect("the command ran");
        let seen = server.seen();

        assert_eq!(
            value_of(&seen.target, "tty"),
            vec!["false".to_owned()],
            "a command whose output is parsed cannot have a terminal"
        );
        assert_eq!(
            value_of(&seen.target, "stdin"),
            vec!["false".to_owned()],
            "a script with no input must not ask for a stream the container would wait on"
        );
        assert_eq!(value_of(&seen.target, "stdout"), vec!["true".to_owned()]);
        assert_eq!(value_of(&seen.target, "stderr"), vec!["true".to_owned()]);

        let command = value_of(&seen.target, "command");
        assert_eq!(command[..2], ["/bin/sh".to_owned(), "-c".to_owned()]);
        assert!(
            command[2].starts_with("ls -l"),
            "the script runs as it was written: {}",
            command[2]
        );
        assert!(
            command[2].contains("7shell-exit:"),
            "the status marker is added by this side: {}",
            command[2]
        );

        assert_eq!(output.stdout, b"out-1");
        assert_eq!(output.stderr, "", "the marker is not part of what the command said");
        assert_eq!(output.exit_status, Some(0));
        assert_eq!(output.failure(), None);
    }

    #[tokio::test]
    async fn a_nonzero_status_is_the_reason_the_command_failed() {
        let server = FakeRunServer::start(successful_run(b"", b"tar: not found\n", 127), 0, false).await;
        let session = open_run(&server.client(), &request("tar cf -"), false)
            .await
            .expect("a run session");
        let output = session.run_to_end(&[]).await.expect("the exec itself worked");

        assert_eq!(output.exit_status, Some(127), "the marker carried the status");
        assert_eq!(output.failure().as_deref(), Some("tar: not found (status 127)"));
    }

    #[tokio::test]
    async fn standard_input_travels_on_channel_zero_and_is_ended_by_the_caller() {
        let server = FakeRunServer::start(successful_run(b"done", b"", 0), 5, false).await;
        let session = open_run(&server.client(), &request("tar xf -"), true)
            .await
            .expect("a run session");
        assert_eq!(
            value_of(&server.seen().target, "stdin"),
            vec!["true".to_owned()],
            "an upload needs the input stream"
        );

        let output = session.run_to_end(b"hello").await.expect("the upload ran");
        assert_eq!(output.stdout, b"done");

        let frames = server.frames(1).await;
        let typed: Vec<u8> = frames
            .iter()
            .filter(|frame| frame.first() == Some(&CHANNEL_STDIN))
            .flat_map(|frame| frame[1..].to_vec())
            .collect();
        assert_eq!(typed, b"hello", "the input was altered on the way out");
    }

    #[tokio::test]
    async fn a_cluster_that_leaves_the_channel_open_is_ended_by_the_status_marker() {
        // No exec status and no close: the marker is the only thing that says the command is over.
        let marker = "\u{1e}7shell-exit:0\u{1e}\n";
        let server = FakeRunServer::start(
            vec![
                frame(CHANNEL_STDOUT, b"whole"),
                frame(CHANNEL_STDERR, marker.as_bytes()),
            ],
            0,
            true,
        )
        .await;
        let session = open_run(&server.client(), &request("tar cf -"), false)
            .await
            .expect("a run session");
        let output = timeout(Duration::from_secs(20), session.run_to_end(&[]))
            .await
            .expect("a channel that never closes must not keep the run alive")
            .expect("the command ran");

        assert_eq!(output.stdout, b"whole");
        assert_eq!(output.exit_status, Some(0), "the marker is what ended it");
    }

    #[tokio::test]
    async fn an_exec_the_server_refused_is_the_reason_the_run_failed() {
        let status = r#"{"status":"Failure","message":"container is not running"}"#;
        let server = FakeRunServer::start(vec![frame(CHANNEL_ERROR, status.as_bytes())], 0, false).await;
        let session = open_run(&server.client(), &request("ls -l"), false)
            .await
            .expect("the upgrade itself worked");
        let error = timeout(Duration::from_secs(10), session.run_to_end(&[]))
            .await
            .expect("the refused exec did not end the run")
            .expect_err("a refused exec is not a successful command");

        assert!(
            error.to_string().contains("container is not running"),
            "the status message was lost: {error}"
        );
    }

    #[test]
    fn the_status_marker_is_the_last_thing_the_script_prints_and_keeps_its_own_status() {
        assert_eq!(
            with_status_marker("ls -la '/'"),
            "ls -la '/'\n__7shell_status=$?\nprintf '\\036%s\\036\\n' \"7shell-exit:$__7shell_status\" 1>&2\nexit \
             $__7shell_status",
            "the marker must not become part of the script it follows"
        );
    }

    #[test]
    fn a_marker_split_across_two_frames_is_still_read() {
        let mut sink = StderrSink::default();
        sink.push("tar: warning\n\u{1e}7shell-".as_bytes());
        sink.push("exit:2\u{1e}\n".as_bytes());
        assert_eq!(sink.status, Some(2));
        assert_eq!(sink.message(), "tar: warning", "the marker is not the command's text");
    }

    #[test]
    fn text_that_looks_like_a_marker_without_the_separators_is_left_alone() {
        let mut sink = StderrSink::default();
        sink.push(b"7shell-exit:0\n");
        assert_eq!(sink.status, None, "a status needs the protocol's separators");
        assert_eq!(sink.message(), "7shell-exit:0");
    }

    #[test]
    fn a_command_that_reported_no_status_is_not_reported_as_a_success() {
        let output = RunOutput {
            stdout: Vec::new(),
            stderr: String::new(),
            exit_status: None,
            input_ended_by_close: false,
        };
        assert_eq!(
            output.failure().as_deref(),
            Some("the command ended without reporting an exit status")
        );
    }

    #[test]
    fn ending_the_input_by_closing_is_not_a_failure_by_itself_and_keeps_what_was_said() {
        // The upload path: the protocol cannot carry an exit code after the closing handshake.
        let quiet = RunOutput {
            stdout: Vec::new(),
            stderr: String::new(),
            exit_status: None,
            input_ended_by_close: true,
        };
        assert_eq!(quiet.failure(), None, "a quiet upload is not a failure");

        let complaining = RunOutput {
            stdout: Vec::new(),
            stderr: "tar: can't create file: Permission denied".to_owned(),
            exit_status: None,
            input_ended_by_close: true,
        };
        assert_eq!(
            complaining.failure().as_deref(),
            Some("tar: can't create file: Permission denied"),
            "the command's own words are the only ending an upload has"
        );
    }
}
