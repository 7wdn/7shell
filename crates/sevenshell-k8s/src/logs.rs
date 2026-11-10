//! Pod logs as a followed stream.
//!
//! A followed log is one HTTP response that stays open for as long as the container writes, which
//! is not the shape the rest of this client uses: there is no request/response pair to return, only
//! a body that arrives in pieces. It lives in its own module because its ending rules are its own.
//!
//! Three things end a stream and only one of them is an error. A clean end of the body means the
//! container stopped writing, so the stream is done. An HTTP status means the request will not get
//! a better answer by repeating it, so it ends with the server's own words. Everything else — a
//! kubelet restart, an API server bounce, a network that went away — is the case a log viewer exists
//! for: the connection is rebuilt after a growing pause, because a host that keeps dropping us
//! mid-stream should be asked less and less often.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt as _;
use sevenshell_error::{ShellError, truncate_chars};
use sevenshell_types::K8sLogRequest;
use tokio::sync::{mpsc, watch};

use crate::client::{K8sClient, K8sTransport};

/// How long the first retry waits. The sequence that follows is 1, 2, 4, 8, 16, 30 seconds.
const RECONNECT_INITIAL: Duration = Duration::from_secs(1);
/// The point where the delay stops doubling: a viewer that is still wanted after half a minute is
/// still wanted, and asking every half minute forever is cheaper than asking every second.
const RECONNECT_MAX: Duration = Duration::from_secs(30);
/// An error response is a sentence, not a document. Anything longer is cut before it reaches the
/// envelope, so a proxy's HTML error page cannot become the payload of an event.
const ERROR_BODY_LIMIT: usize = 512;
/// How long a server may take to finish the body of an error response. The status is the answer;
/// the body is a bonus, and a bonus must not hold a stream open.
const ERROR_BODY_TIMEOUT: Duration = Duration::from_secs(5);
/// Lines waiting for whoever is reading them. A full queue stops the read, which is the right
/// backpressure for a stream nobody is draining.
const LINE_QUEUE: usize = 256;
/// A line longer than this is not a log line. A reader that treats it as an error turns one
/// pathological write into a reconnect loop that repeats the same write; handing the buffer over as
/// a line and carrying on keeps the rest of the log.
const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

/// The retry schedule, and the bounds that keep a broken server from holding a stream open, as a
/// value rather than as constants.
///
/// The tests shorten all of them: a suite that waits a real second between attempts, or five
/// seconds for a body that never arrives, is a suite nobody runs. Production callers use
/// [`LogOptions::default`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogOptions {
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub max_line_bytes: usize,
    /// How long the body of an error response may take before the status alone is the answer.
    pub error_body_timeout: Duration,
}

impl Default for LogOptions {
    fn default() -> Self {
        Self {
            initial_backoff: RECONNECT_INITIAL,
            max_backoff: RECONNECT_MAX,
            max_line_bytes: MAX_LINE_BYTES,
            error_body_timeout: ERROR_BODY_TIMEOUT,
        }
    }
}

/// A followed log, and the only handle on the request behind it.
///
/// Dropping it stops the request: a stream whose reader went away would otherwise keep a
/// connection open until the container stopped writing.
pub struct LogStream {
    lines: mpsc::Receiver<String>,
    control: Arc<Control>,
}

impl LogStream {
    /// The next line, or `None` once the stream has ended for good.
    ///
    /// Lines arrive without their newline; an empty line in the log is an empty string, not `None`.
    pub async fn next_line(&mut self) -> Option<String> {
        self.lines.recv().await
    }

    /// Ends the stream promptly and forgets about the request.
    ///
    /// Safe to call twice, and safe to call after the stream ended on its own.
    pub fn stop(&self) {
        self.control.stop();
    }

    /// Whether the stream ended. What it ended with is [`LogStream::wait_ended`].
    pub fn ended(&self) -> bool {
        self.control.ended()
    }

    /// Waits for the end and answers what it was, or `None` when the stream ended cleanly or was
    /// stopped. Repeated calls answer the same thing: ending is one decision, however many waiters
    /// ask about it.
    pub async fn wait_ended(&self) -> Option<ShellError> {
        self.control.wait_ended().await
    }

    /// The part a registry keeps: stopping and waiting, without the lines somebody else reads.
    pub fn handle(&self) -> LogHandle {
        LogHandle {
            control: Arc::clone(&self.control),
        }
    }
}

impl Drop for LogStream {
    fn drop(&mut self) {
        self.control.stop();
    }
}

/// A [`LogStream`] without its lines, for the map that has to find the stream again to stop it.
///
/// Separate from the stream because two things need the same stream at once: the task reading lines
/// owns the receiver, and the registry that closes streams on window close owns the control. The
/// receiver cannot be handed to both, and the registry must not have to be the reader to stop it.
#[derive(Clone)]
pub struct LogHandle {
    control: Arc<Control>,
}

impl LogHandle {
    pub fn stop(&self) {
        self.control.stop();
    }

    pub fn ended(&self) -> bool {
        self.control.ended()
    }

    pub async fn wait_ended(&self) -> Option<ShellError> {
        self.control.wait_ended().await
    }
}

/// Follows the log of one container.
///
/// The request is answered immediately with a handle: the first HTTP response may take as long as
/// the cluster does, and the interface asked for a stream, not for a wait. Must be called from
/// inside a Tokio runtime, which is where the follow task is spawned.
pub fn open_logs(client: &K8sClient, request: &K8sLogRequest) -> Result<LogStream, ShellError> {
    open_logs_with_options(client, request, LogOptions::default())
}

/// [`open_logs`] with the retry schedule and the line bound chosen by the caller.
pub fn open_logs_with_options(
    client: &K8sClient,
    request: &K8sLogRequest,
    options: LogOptions,
) -> Result<LogStream, ShellError> {
    let transport = client.transport();
    let url = log_url(transport, request)?;
    let http = log_client(transport)?;
    let headers = request_headers(transport, &url);
    let path = url.path().to_owned();

    let (lines, receiver) = mpsc::channel(LINE_QUEUE);
    let control = Arc::new(Control::new());
    let task_control = Arc::clone(&control);
    let stop = control.subscribe();

    tokio::spawn(async move {
        let error = follow(http, url, path, headers, options, lines, stop).await;
        task_control.end(error);
    });

    Ok(LogStream {
        lines: receiver,
        control,
    })
}

/// The state every waiter shares: whether somebody asked to stop, and how the stream ended.
struct Control {
    stop: watch::Sender<bool>,
    ended: watch::Sender<EndState>,
}

/// The terminal state of a stream. A `watch` value rather than a `Mutex<Option<…>>` because the
/// question "has it ended, and with what" is answered by any number of waiters, including ones that
/// ask after the answer is known.
#[derive(Clone, Default)]
struct EndState {
    done: bool,
    error: Option<ShellError>,
}

impl Control {
    fn new() -> Self {
        Self {
            stop: watch::channel(false).0,
            ended: watch::channel(EndState::default()).0,
        }
    }

    fn subscribe(&self) -> watch::Receiver<bool> {
        self.stop.subscribe()
    }

    fn stop(&self) {
        // `send_replace` rather than `send`: there may be no receiver left when the handle is
        // dropped after the stream ended, and stopping a finished stream is not a failure.
        let _ = self.stop.send_replace(true);
    }

    fn ended(&self) -> bool {
        self.ended.borrow().done
    }

    async fn wait_ended(&self) -> Option<ShellError> {
        let mut ended = self.ended.subscribe();
        loop {
            let state = ended.borrow_and_update().clone();
            if state.done {
                return state.error;
            }
            // A closed channel means the task that owns the stream is gone without a word; the
            // stream is over either way, and claiming it ended cleanly is the honest reading of
            // "there is no error to report".
            if ended.changed().await.is_err() {
                return None;
            }
        }
    }

    fn end(&self, error: Option<ShellError>) {
        // The first answer wins. A stop racing the end of the body must produce one end state, not
        // the last one to arrive.
        if self.ended.borrow().done {
            return;
        }
        let _ = self.ended.send_replace(EndState { done: true, error });
    }
}

/// Resolves when somebody asked to stop, or when the control channel went away.
///
/// The `borrow` check is what makes a stop that arrived before this call visible: a `watch` receiver
/// only wakes on the change it has not seen, and a stream opened after a stop would otherwise wait
/// forever for a notification that already happened. The loop re-arms on a wake that is not a stop,
/// so this future resolves for one reason only.
async fn stop_requested(stop: &mut watch::Receiver<bool>) {
    loop {
        if *stop.borrow() {
            return;
        }
        // A closed channel means the handle is gone, which is a stop in every way that matters.
        if stop.changed().await.is_err() {
            return;
        }
    }
}

/// How one attempt ended.
enum Attempt {
    /// The stream is over. The error is what should be reported, if anything should be.
    Ended(Option<ShellError>),
    /// Somebody stopped the stream; nothing is reported.
    Stopped,
    /// The response body broke off. Reconnecting may tell the rest of the story.
    Retry,
}

/// The retry loop: one response at a time, with a growing pause between attempts.
///
/// The delay is never reset after a good stretch. A server that drops the stream every few seconds
/// is not two failures and a success, it is a cluster that cannot hold a log open, and backing off
/// further each time is what keeps the client from being the reason.
async fn follow(
    http: reqwest::Client,
    url: reqwest::Url,
    path: String,
    headers: RequestHeaders,
    options: LogOptions,
    lines: mpsc::Sender<String>,
    mut stop: watch::Receiver<bool>,
) -> Option<ShellError> {
    let mut backoff = options.initial_backoff;
    loop {
        let attempt = one_response(&http, &url, &path, &headers, &options, &lines, &mut stop).await;
        match attempt {
            Attempt::Ended(error) => return error,
            Attempt::Stopped => return None,
            Attempt::Retry => {}
        }
        let wait = std::cmp::min(backoff, options.max_backoff);
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            // Either the person stopped the stream or the handle is gone; in both cases nobody is
            // left to reconnect for.
            _ = stop_requested(&mut stop) => return None,
        }
        backoff = backoff.saturating_mul(2);
    }
}

/// One request, from the first byte to the last.
async fn one_response(
    http: &reqwest::Client,
    url: &reqwest::Url,
    path: &str,
    headers: &RequestHeaders,
    options: &LogOptions,
    lines: &mpsc::Sender<String>,
    stop: &mut watch::Receiver<bool>,
) -> Attempt {
    if *stop.borrow() {
        return Attempt::Stopped;
    }

    // A wildcard, not `text/plain`: the API server's content negotiation for the log subresource
    // rejects the explicit text type on some distributions (k3s answers 406) while a wildcard gets
    // the plain-text body this reader expects.
    let mut request = http.get(url.clone()).header(reqwest::header::ACCEPT, "*/*");
    if let Some(authorization) = &headers.authorization {
        request = request.header(reqwest::header::AUTHORIZATION, authorization);
    }
    if let Some(host) = &headers.host {
        request = request.header(reqwest::header::HOST, host);
    }

    let answer = tokio::select! {
        biased;
        _ = stop_requested(stop) => return Attempt::Stopped,
        answer = request.send() => answer,
    };
    let response = match answer {
        Ok(response) => response,
        Err(error) => {
            // No status, no body: the connection (or the DNS lookup, or the TLS handshake) failed,
            // which is the transient case this loop exists for. The error text is the URL and the
            // transport's own reason; no header of ours is in it.
            log::debug!("the pod log request will be retried: {error}");
            return Attempt::Retry;
        }
    };

    let status = response.status().as_u16();
    if status >= 400 {
        let detail = bounded_body(response, ERROR_BODY_LIMIT, options.error_body_timeout).await;
        return Attempt::Ended(Some(ShellError::k8s_request_failed(
            status,
            path,
            truncate_chars(&detail, ERROR_BODY_LIMIT),
        )));
    }

    let mut body = response.bytes_stream();
    let mut buffer = LineBuffer::new(options.max_line_bytes);
    loop {
        let chunk = tokio::select! {
            biased;
            _ = stop_requested(stop) => return Attempt::Stopped,
            chunk = body.next() => chunk,
        };
        match chunk {
            Some(Ok(bytes)) => {
                for line in buffer.push(&bytes) {
                    if send_line(lines, line, stop).await.is_err() {
                        return Attempt::Stopped;
                    }
                }
            }
            Some(Err(error)) => {
                // A body that stops mid-transfer is the reconnect case: the container may still be
                // running, and the API server that owned the response is gone.
                log::debug!("the pod log stream broke off and will be retried: {error}");
                return Attempt::Retry;
            }
            None => {
                // The whole body arrived. A pod that wrote without a trailing newline still said
                // something, so the last partial line is a line.
                if let Some(line) = buffer.finish() {
                    let _ = lines.send(line).await;
                }
                return Attempt::Ended(None);
            }
        }
    }
}

/// Hands one line over, or gives up if the stream is being stopped while the queue is full.
async fn send_line(lines: &mpsc::Sender<String>, line: String, stop: &mut watch::Receiver<bool>) -> Result<(), ()> {
    tokio::select! {
        biased;
        _ = stop_requested(stop) => Err(()),
        sent = lines.send(line) => sent.map_err(|_| ()),
    }
}

/// At most `limit` bytes of an error body, as text, and never longer than the caller allows.
async fn bounded_body(response: reqwest::Response, limit: usize, patience: Duration) -> String {
    let mut body = response.bytes_stream();
    let mut taken: Vec<u8> = Vec::new();
    let read = async {
        while let Some(Ok(chunk)) = body.next().await {
            taken.extend_from_slice(&chunk);
            if taken.len() >= limit {
                break;
            }
        }
    };
    if tokio::time::timeout(patience, read).await.is_err() {
        // Whatever arrived is still worth showing; an unfinished sentence beats no sentence.
        return format!(
            "{} (the error body did not finish arriving)",
            String::from_utf8_lossy(&taken).trim()
        );
    }
    taken.truncate(limit);
    String::from_utf8_lossy(&taken).trim().to_owned()
}

/// The headers a log request carries besides its URL.
///
/// Split from the transport because the follow task outlives the borrow of the client.
#[derive(Clone, Default)]
struct RequestHeaders {
    authorization: Option<String>,
    /// Set only when the kubeconfig's server name differs from the address the URL points at.
    host: Option<String>,
}

fn request_headers(transport: &K8sTransport, url: &reqwest::Url) -> RequestHeaders {
    RequestHeaders {
        authorization: transport.authorization.clone(),
        host: server_host(transport, url),
    }
}

/// The name the server expects in `Host`, when it is not the name in the URL.
fn server_host(transport: &K8sTransport, url: &reqwest::Url) -> Option<String> {
    if transport.server.is_empty() || url.host_str() == Some(transport.server.as_str()) {
        return None;
    }
    Some(transport.server.clone())
}

/// A reqwest client with the cluster's own TLS story and no proxy in the middle.
///
/// The connector is the transport's, so a client certificate authorises a log stream exactly as it
/// authorises a list. The proxy is off because the kubeconfig names the server directly: a system
/// proxy that happens to be configured for the browser would send cluster traffic somewhere the
/// kubeconfig never mentioned.
fn log_client(transport: &K8sTransport) -> Result<reqwest::Client, ShellError> {
    reqwest::Client::builder()
        .use_preconfigured_tls(transport.tls.clone())
        .no_proxy()
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| ShellError::k8s_unavailable(format!("the log client could not be built: {error}")))
}

/// `/api/v1/namespaces/{namespace}/pods/{pod}/log` with the query a follow needs.
fn log_url(transport: &K8sTransport, request: &K8sLogRequest) -> Result<reqwest::Url, ShellError> {
    let mut url = reqwest::Url::parse(&transport.base)
        .map_err(|error| ShellError::k8s_unavailable(format!("the cluster address is not a URL: {error}")))?;
    {
        // Segments rather than a format string: a namespace is a DNS label today, but nothing in
        // this client should decide that a name can be pasted into a URL unescaped.
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
            .push("log");
    }
    {
        let mut query = url.query_pairs_mut();
        if !request.container.is_empty() {
            query.append_pair("container", &request.container);
        }
        // `previous` is a one-off look at the container's last run, so following it would wait for
        // lines that can never come.
        query.append_pair("follow", if request.previous { "false" } else { "true" });
        query.append_pair("previous", if request.previous { "true" } else { "false" });
        query.append_pair("timestamps", if request.timestamps { "true" } else { "false" });
        // Zero is this client's "not asked". Sending `tailLines=0` would ask the server for zero
        // lines, which is the opposite of showing the log.
        if request.tail_lines > 0 {
            query.append_pair("tailLines", &request.tail_lines.to_string());
        }
    }
    Ok(url)
}

/// Splits a byte stream into lines without assuming a chunk boundary is a line boundary.
struct LineBuffer {
    pending: Vec<u8>,
    max_line: usize,
}

impl LineBuffer {
    fn new(max_line: usize) -> Self {
        Self {
            pending: Vec::new(),
            max_line,
        }
    }

    fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(chunk);
        let mut lines = Vec::new();
        let mut consumed = 0;
        while let Some(offset) = self.pending[consumed..].iter().position(|byte| *byte == b'\n') {
            let end = consumed + offset;
            lines.push(decode(&self.pending[consumed..end]));
            consumed = end + 1;
        }
        if consumed > 0 {
            self.pending.drain(..consumed);
        }
        if self.pending.len() >= self.max_line {
            let overflow = std::mem::take(&mut self.pending);
            lines.push(decode(&overflow));
        }
        lines
    }

    fn finish(&mut self) -> Option<String> {
        if self.pending.is_empty() {
            return None;
        }
        let rest = std::mem::take(&mut self.pending);
        Some(decode(&rest))
    }
}

/// One line as text. `lossy` because half of a multi-byte character can be the last thing a
/// container writes; showing a replacement character beats dropping the line.
fn decode(bytes: &[u8]) -> String {
    let trimmed = if bytes.ends_with(b"\r") {
        &bytes[..bytes.len() - 1]
    } else {
        bytes
    };
    String::from_utf8_lossy(trimmed).into_owned()
}

#[cfg(test)]
mod tests {
    //! The fake server here is a few lines of HTTP over a socket, on purpose: pulling in an HTTP
    //! server to test a client would make the test agree with a framework instead of with the wire.
    //!
    //! The client is built here rather than through `K8sClient::connect` because the connection
    //! layer is another module's: what these tests are about is the log stream, and a client aimed
    //! at 127.0.0.1 is the smallest thing that can carry one.

    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::time::timeout;

    use super::*;
    use crate::client::K8sTransport;

    /// What one connection answers with.
    #[derive(Clone)]
    enum Behaviour {
        /// Chunks written as chunked-encoding pieces, a terminating chunk, then the connection
        /// stays open. This is a clean end of the body.
        Lines(Vec<String>),
        /// The same, but the connection is closed without a terminating chunk: the body ends
        /// mid-transfer, which is what a kubelet restart looks like to the client.
        LinesThenDrop(Vec<String>),
        /// The chunks, then silence. Used for "stopping is prompt".
        LinesThenHang(Vec<String>),
        Status(u16, String),
        /// The status line and no body, ever. What a proxy that gave up mid-answer looks like.
        Stalled(u16),
    }

    /// The behaviours the tests write most often, in the shape they write them.
    fn chunks(chunks: &[&str]) -> Vec<String> {
        chunks.iter().map(|chunk| (*chunk).to_owned()).collect()
    }

    /// A server that answers one behaviour per connection, in order, and remembers the requests.
    struct FakeLogServer {
        base: String,
        hits: Arc<Mutex<Vec<String>>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl FakeLogServer {
        async fn start(behaviours: Vec<Behaviour>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
            let port = listener.local_addr().expect("an address").port();
            let hits = Arc::new(Mutex::new(Vec::new()));
            let recorded = Arc::clone(&hits);
            let script = Arc::new(Mutex::new(VecDeque::from(behaviours)));
            let task = tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        return;
                    };
                    let next = script.lock().expect("the script lock").pop_front();
                    // A test that reconnects more often than its script expected gets an error
                    // instead of a silent hang: the extra attempt is itself the finding.
                    let behaviour =
                        next.unwrap_or_else(|| Behaviour::Status(500, "the fake server ran out of answers".to_owned()));
                    let recorded = Arc::clone(&recorded);
                    tokio::spawn(async move {
                        serve(stream, behaviour, recorded).await;
                    });
                }
            });
            Self {
                base: format!("http://127.0.0.1:{port}"),
                hits,
                task,
            }
        }

        fn client(&self) -> K8sClient {
            K8sClient {
                transport: K8sTransport {
                    base: self.base.clone(),
                    server: String::new(),
                    tls: native_tls::TlsConnector::new().expect("a TLS connector"),
                    authorization: Some("Bearer test-token".to_owned()),
                },
                http: reqwest::Client::new(),
            }
        }

        fn hits(&self) -> Vec<String> {
            self.hits.lock().expect("the hits lock").clone()
        }
    }

    impl Drop for FakeLogServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn serve(mut stream: TcpStream, behaviour: Behaviour, hits: Arc<Mutex<Vec<String>>>) {
        let mut request = Vec::new();
        let mut byte = [0u8; 1];
        // The request is small and text; reading to the blank line is enough to see the target.
        loop {
            match stream.read(&mut byte).await {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
            }
        }
        let head = String::from_utf8_lossy(&request).into_owned();
        let target = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or_default()
            .to_owned();
        hits.lock().expect("the hits lock").push(target);

        match behaviour {
            Behaviour::Lines(chunks) => {
                write_chunks(&mut stream, &chunks, true).await;
                // Held open: a clean body end is a terminating chunk, not a close.
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
            Behaviour::LinesThenDrop(chunks) => {
                write_chunks(&mut stream, &chunks, false).await;
                // Dropping the socket without the terminating chunk is the mid-transfer end the
                // reconnect test needs.
                drop(stream);
            }
            Behaviour::LinesThenHang(chunks) => {
                write_chunks(&mut stream, &chunks, false).await;
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
            Behaviour::Status(status, body) => {
                let head = format!(
                    "HTTP/1.1 {status} Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes()).await;
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
            Behaviour::Stalled(status) => {
                let head = format!("HTTP/1.1 {status} Error\r\nContent-Length: 64\r\n\r\n");
                let _ = stream.write_all(head.as_bytes()).await;
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        }
    }

    /// One chunked-encoded body, optionally finished with the terminating chunk.
    async fn write_chunks(stream: &mut TcpStream, chunks: &[String], terminate: bool) {
        let mut head =
            String::from("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\n\r\n");
        for chunk in chunks {
            head.push_str(&format!("{:x}\r\n{chunk}\r\n", chunk.len()));
        }
        if terminate {
            head.push_str("0\r\n\r\n");
        }
        let _ = stream.write_all(head.as_bytes()).await;
    }

    fn request() -> K8sLogRequest {
        K8sLogRequest {
            namespace: "team-a".to_owned(),
            pod: "web-0".to_owned(),
            container: "app".to_owned(),
            tail_lines: 50,
            timestamps: true,
            previous: false,
        }
    }

    fn options() -> LogOptions {
        LogOptions {
            initial_backoff: Duration::from_millis(20),
            max_backoff: Duration::from_millis(40),
            max_line_bytes: 1024,
            error_body_timeout: Duration::from_millis(200),
        }
    }

    async fn lines(stream: &mut LogStream, wanted: usize) -> Vec<String> {
        let mut collected = Vec::new();
        for _ in 0..wanted {
            match timeout(Duration::from_secs(5), stream.next_line()).await {
                Ok(Some(line)) => collected.push(line),
                other => panic!("expected {wanted} lines, got {collected:?} and then {other:?}"),
            }
        }
        collected
    }

    #[tokio::test]
    async fn a_follow_request_names_the_namespace_pod_container_previous_and_tail() {
        let server = FakeLogServer::start(vec![Behaviour::Lines(chunks(&["one\n"]))]).await;
        let mut stream = open_logs_with_options(&server.client(), &request(), options()).expect("a stream");
        assert_eq!(lines(&mut stream, 1).await, vec!["one".to_owned()]);
        let target = &server.hits()[0];
        assert!(
            target.starts_with("/api/v1/namespaces/team-a/pods/web-0/log?"),
            "the log path was {target}"
        );
        for expected in [
            "container=app",
            "follow=true",
            "previous=false",
            "timestamps=true",
            "tailLines=50",
        ] {
            assert!(target.contains(expected), "{expected} was missing from {target}");
        }
    }

    #[tokio::test]
    async fn a_previous_request_does_not_follow() {
        let server = FakeLogServer::start(vec![Behaviour::Lines(chunks(&["gone\n"]))]).await;
        let mut previous = request();
        previous.previous = true;
        previous.tail_lines = 0;
        let mut stream = open_logs_with_options(&server.client(), &previous, options()).expect("a stream");
        assert_eq!(lines(&mut stream, 1).await, vec!["gone".to_owned()]);
        let target = &server.hits()[0];
        assert!(
            target.contains("follow=false"),
            "the request followed a finished log: {target}"
        );
        assert!(
            target.contains("previous=true"),
            "the request did not ask for the last run: {target}"
        );
        assert!(
            !target.contains("tailLines"),
            "zero tail lines must mean 'not asked', not 'show nothing': {target}"
        );
    }

    #[tokio::test]
    async fn a_name_that_could_end_a_path_is_escaped() {
        let server = FakeLogServer::start(vec![Behaviour::Lines(chunks(&["x\n"]))]).await;
        let mut awkward = request();
        awkward.pod = "web 0".to_owned();
        awkward.container = "app sidecar".to_owned();
        let mut stream = open_logs_with_options(&server.client(), &awkward, options()).expect("a stream");
        assert_eq!(lines(&mut stream, 1).await, vec!["x".to_owned()]);
        let target = &server.hits()[0];
        assert!(
            target.contains("/pods/web%200/log"),
            "the pod name was not escaped: {target}"
        );
        assert!(
            target.contains("container=app+sidecar"),
            "the container was not escaped: {target}"
        );
    }

    #[tokio::test]
    async fn a_line_split_across_chunks_is_one_line_and_crlf_is_stripped() {
        let server = FakeLogServer::start(vec![Behaviour::Lines(chunks(&["par", "tial\r\n", "next\n"]))]).await;
        let mut stream = open_logs_with_options(&server.client(), &request(), options()).expect("a stream");
        assert_eq!(
            lines(&mut stream, 2).await,
            vec!["partial".to_owned(), "next".to_owned()]
        );
    }

    #[tokio::test]
    async fn a_body_that_breaks_off_is_reconnected_and_the_stream_continues() {
        let server = FakeLogServer::start(vec![
            Behaviour::LinesThenDrop(chunks(&["before\n"])),
            Behaviour::Lines(chunks(&["after\n"])),
        ])
        .await;
        let mut stream = open_logs_with_options(&server.client(), &request(), options()).expect("a stream");
        assert_eq!(
            lines(&mut stream, 2).await,
            vec!["before".to_owned(), "after".to_owned()]
        );
        let hits = server.hits();
        assert_eq!(hits.len(), 2, "the stream did not reconnect: {hits:?}");
        assert_eq!(
            hits[0].split('?').next(),
            hits[1].split('?').next(),
            "the retry asked a different path"
        );
    }

    #[tokio::test]
    async fn a_clean_end_ends_the_stream_and_says_nothing_went_wrong() {
        let server = FakeLogServer::start(vec![Behaviour::Lines(chunks(&["last\n"]))]).await;
        let mut stream = open_logs_with_options(&server.client(), &request(), options()).expect("a stream");
        assert_eq!(lines(&mut stream, 1).await, vec!["last".to_owned()]);
        assert_eq!(stream.next_line().await, None);
        assert_eq!(stream.wait_ended().await, None);
        assert!(stream.ended());
        assert_eq!(server.hits().len(), 1, "a clean end was retried");
        // Answering again must not invent a second end.
        assert_eq!(stream.wait_ended().await, None);
    }

    #[tokio::test]
    async fn an_error_status_ends_the_stream_with_the_servers_words_and_is_not_retried() {
        let server = FakeLogServer::start(vec![Behaviour::Status(404, "pods \"web-0\" not found".to_owned())]).await;
        let mut stream = open_logs_with_options(&server.client(), &request(), options()).expect("a stream");
        assert_eq!(stream.next_line().await, None);
        let error = stream.wait_ended().await.expect("a 404 is an error");
        assert_eq!(error.code(), sevenshell_error::ErrorCode::K8sRequestFailed);
        assert_eq!(
            error.message_params().get("status").map(String::as_str),
            Some("404"),
            "the envelope lost the status: {error}"
        );
        assert!(
            error.to_string().contains("not found"),
            "the envelope lost the server's message: {error}"
        );
        assert_eq!(server.hits().len(), 1, "a 404 was retried");
    }

    #[tokio::test]
    async fn an_error_body_that_never_finishes_does_not_hold_the_stream_open() {
        let server = FakeLogServer::start(vec![Behaviour::Stalled(500)]).await;
        let stream = open_logs_with_options(&server.client(), &request(), options()).expect("a stream");
        let error = timeout(Duration::from_secs(3), stream.wait_ended())
            .await
            .expect("the stalled error body held the stream open")
            .expect("a 500 is an error");
        assert_eq!(error.code(), sevenshell_error::ErrorCode::K8sRequestFailed);
        assert_eq!(
            error.message_params().get("status").map(String::as_str),
            Some("500"),
            "the status was lost when the body was: {error}"
        );
    }

    #[tokio::test]
    async fn stopping_is_prompt_even_when_the_server_has_gone_quiet() {
        let server = FakeLogServer::start(vec![Behaviour::LinesThenHang(chunks(&["quiet\n"]))]).await;
        let mut stream = open_logs_with_options(&server.client(), &request(), options()).expect("a stream");
        assert_eq!(lines(&mut stream, 1).await, vec!["quiet".to_owned()]);
        stream.stop();
        let ended = timeout(Duration::from_secs(2), stream.wait_ended()).await;
        assert_eq!(ended.expect("stopping took too long"), None);
        assert!(stream.ended());
    }

    #[tokio::test]
    async fn stopping_during_the_backoff_does_not_open_another_connection() {
        let server = FakeLogServer::start(vec![Behaviour::LinesThenDrop(Vec::new())]).await;
        let slow = LogOptions {
            initial_backoff: Duration::from_secs(30),
            max_backoff: Duration::from_secs(30),
            max_line_bytes: 1024,
            error_body_timeout: Duration::from_millis(200),
        };
        let stream = open_logs_with_options(&server.client(), &request(), slow).expect("a stream");
        // The body ended at once, so the loop is now waiting a long time before the retry.
        tokio::time::sleep(Duration::from_millis(100)).await;
        stream.stop();
        let ended = timeout(Duration::from_secs(2), stream.wait_ended()).await;
        assert_eq!(ended.expect("stopping took too long"), None);
        assert_eq!(
            server.hits().len(),
            1,
            "the stop waited for the retry instead of ending the stream"
        );
    }

    #[tokio::test]
    async fn a_line_that_never_ends_is_bounded_and_the_stream_keeps_going() {
        let long = "x".repeat(64);
        let server = FakeLogServer::start(vec![Behaviour::Lines(vec![long, "next\n".to_owned()])]).await;
        let options = LogOptions {
            initial_backoff: Duration::from_millis(20),
            max_backoff: Duration::from_millis(40),
            max_line_bytes: 8,
            error_body_timeout: Duration::from_millis(200),
        };
        let mut stream = open_logs_with_options(&server.client(), &request(), options).expect("a stream");
        let first = lines(&mut stream, 2).await;
        assert_eq!(first[0].len(), 64, "the overflow was dropped instead of handed over");
        assert_eq!(first[1], "next");
    }

    #[tokio::test]
    async fn a_half_line_at_a_clean_end_is_still_a_line() {
        let server = FakeLogServer::start(vec![Behaviour::Lines(chunks(&["no newline"]))]).await;
        let mut stream = open_logs_with_options(&server.client(), &request(), options()).expect("a stream");
        assert_eq!(lines(&mut stream, 1).await, vec!["no newline".to_owned()]);
        assert_eq!(stream.next_line().await, None);
    }

    #[test]
    fn the_handle_a_registry_keeps_is_shareable_across_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<LogHandle>();
    }

    #[tokio::test]
    async fn dropping_the_stream_stops_the_request_it_opened() {
        let server = FakeLogServer::start(vec![Behaviour::LinesThenHang(chunks(&["quiet\n"]))]).await;
        let mut stream = open_logs_with_options(&server.client(), &request(), options()).expect("a stream");
        assert_eq!(lines(&mut stream, 1).await, vec!["quiet".to_owned()]);
        let handle = stream.handle();
        drop(stream);
        let ended = timeout(Duration::from_secs(2), handle.wait_ended()).await;
        assert_eq!(
            ended.expect("dropping the stream left the request open"),
            None,
            "a dropped stream reported an error instead of a stop"
        );
    }
}
