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
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::{AUTHORIZATION, HOST, SEC_WEBSOCKET_PROTOCOL};
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};

use crate::client::{K8sClient, K8sTransport};

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
    let (socket, _response) = tokio_tungstenite::connect_async_tls_with_config(handshake, None, true, Some(connector))
        .await
        .map_err(|error| {
            ShellError::k8s_unavailable(format!(
                "the pod exec channel could not be opened: {}",
                truncate_chars(&error.to_string(), ERROR_CHARS)
            ))
        })?;
    // There is no subprotocol check here because the upgrade refuses one: tungstenite fails the
    // handshake unless the server echoes exactly the protocol this client offered, and a server
    // that answers nothing at all fails it too. A second check would be a second opinion about
    // something already decided.

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
        query.append_pair("stdin", "true");
        query.append_pair("stdout", "true");
        query.append_pair("stderr", "true");
        // A terminal, not a command runner: a pty is what makes the shell interactive.
        query.append_pair("tty", "true");
        let command = request.command.as_str();
        let script = if command.trim().is_empty() {
            SHELL_COMMAND
        } else {
            command
        };
        // The API repeats the parameter to build an argv; a single value is not an argv.
        query.append_pair("command", "/bin/sh");
        query.append_pair("command", "-c");
        query.append_pair("command", script);
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
}
