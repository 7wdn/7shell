//! Pod logs and a pod terminal through a tunnel.
//!
//! A tunnel is a pinned CONNECT proxy on the loopback: the client asks it for the server the
//! kubeconfig named, and it carries that one server and nothing else. These tests put a fake
//! cluster behind the dialer and point the kubeconfig at `127.0.0.1:1`, where nothing listens — so
//! a stream that went around the tunnel would produce nothing at all, and a stream that went
//! through it has to have spoken to the fake cluster's own certificate.

mod common;

use std::io::{Read as _, Write as _};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use sevenshell_error::{ErrorCode, ShellError};
use sevenshell_k8s::{ApiDialer, BoxedApiStream, DialFuture, K8sManager, Kubeconfig, TunneledCluster};
use sevenshell_types::{K8sExecRequest, K8sLogRequest};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::timeout;

use common::pem;

/// The one subprotocol these tests expect the upgrade to ask for.
const PROTOCOL: &str = "v4.channel.k8s.io";

/// A dialer standing in for the SSH channel, recording what it was asked to carry.
struct LocalDialer {
    /// Where a carried channel ends up, or `None` to fail the way an unreachable far side does.
    far: Option<SocketAddr>,
    asked: Mutex<Vec<(String, u16)>>,
}

impl LocalDialer {
    fn new(far: Option<SocketAddr>) -> Arc<Self> {
        Arc::new(Self {
            far,
            asked: Mutex::new(Vec::new()),
        })
    }

    fn asked(&self) -> Vec<(String, u16)> {
        self.asked.lock().expect("the asked lock").clone()
    }
}

impl ApiDialer for LocalDialer {
    fn dial(&self, host: String, port: u16) -> DialFuture {
        self.asked.lock().expect("the asked lock").push((host, port));
        let far = self.far;
        Box::pin(async move {
            let Some(address) = far else {
                return Err(ShellError::k8s_unavailable(
                    "the far side of the test tunnel is unreachable",
                ));
            };
            let stream = TcpStream::connect(address)
                .await
                .map_err(|error| ShellError::k8s_unavailable(format!("the test tunnel could not connect: {error}")))?;
            let stream: BoxedApiStream = Box::new(stream);
            Ok(stream)
        })
    }
}

/// A kubeconfig for one cluster, trusting the first test authority.
fn document(server: &str) -> String {
    use base64::Engine as _;
    let authority = base64::engine::general_purpose::STANDARD.encode(pem("ca1.pem"));
    format!(
        r#"apiVersion: v1
kind: Config
current-context: test
clusters:
  - name: test-cluster
    cluster:
      server: {server}
      certificate-authority-data: {authority}
contexts:
  - name: test
    context:
      cluster: test-cluster
      user: test-user
users:
  - name: test-user
    user:
      token: not-a-real-token
"#
    )
}

/// A manager connected through `dialer`, with the cluster's address being port 1 on the loopback.
async fn tunneled(dialer: Arc<LocalDialer>) -> (K8sManager, String) {
    // Nothing listens on port 1, so every byte of these tests has to have come through the tunnel.
    let config = Kubeconfig::parse(document("https://127.0.0.1:1").as_bytes()).expect("a kubeconfig");
    let prepared = TunneledCluster::prepare(&config, None).expect("a tunneled cluster");
    let manager = K8sManager::new();
    let shared: Arc<dyn ApiDialer> = dialer;
    let connection = manager.connect_via(prepared, shared).await.expect("connect");
    (manager, connection)
}

fn log_request() -> K8sLogRequest {
    K8sLogRequest {
        namespace: "lab".to_owned(),
        pod: "web-0".to_owned(),
        container: "app".to_owned(),
        tail_lines: 10,
        timestamps: false,
        previous: false,
    }
}

fn exec_request(command: &str) -> K8sExecRequest {
    K8sExecRequest {
        namespace: "lab".to_owned(),
        pod: "web-0".to_owned(),
        container: "app".to_owned(),
        command: command.to_owned(),
    }
}

// -- a TLS log server ---------------------------------------------------------------

/// A TLS server that answers one log request with `lines`, chunked, and then ends the body.
struct LogServer {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
}

impl LogServer {
    fn requests(&self) -> Vec<String> {
        self.requests.lock().expect("the requests lock").clone()
    }
}

fn log_server(lines: Vec<String>) -> LogServer {
    let identity =
        native_tls::Identity::from_pkcs8(&pem("server1.pem"), &pem("server1.key")).expect("a server identity");
    let acceptor = native_tls::TlsAcceptor::new(identity).expect("a TLS acceptor");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("an address");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&requests);

    std::thread::spawn(move || {
        for accepted in listener.incoming() {
            let Ok(accepted) = accepted else { continue };
            let acceptor = acceptor.clone();
            let recorded = Arc::clone(&recorded);
            let lines = lines.clone();
            std::thread::spawn(move || {
                let Ok(mut stream) = acceptor.accept(accepted) else {
                    return;
                };
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    match stream.read(&mut byte) {
                        Ok(0) | Err(_) => return,
                        Ok(_) => request.push(byte[0]),
                    }
                }
                let head = String::from_utf8_lossy(&request).into_owned();
                let target = head
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or_default()
                    .to_owned();
                recorded.lock().expect("the requests lock").push(target);

                let mut answer =
                    String::from("HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ntransfer-encoding: chunked\r\n\r\n");
                for line in lines {
                    // The newline belongs to the log body, not to the chunk framing.
                    let body = format!("{line}\n");
                    answer.push_str(&format!("{:x}\r\n{body}\r\n", body.len()));
                }
                answer.push_str("0\r\n\r\n");
                let _ = stream.write_all(answer.as_bytes());
                let _ = stream.flush();
            });
        }
    });

    LogServer { address, requests }
}

#[tokio::test]
async fn a_followed_log_is_carried_through_the_tunnel() {
    let far = log_server(vec!["tick 1".to_owned(), "tick 2".to_owned()]);
    let dialer = LocalDialer::new(Some(far.address));
    let (manager, connection) = tunneled(Arc::clone(&dialer)).await;

    let (lines, mut line_events) = mpsc::unbounded_channel();
    let (end, mut end_events) = mpsc::unbounded_channel();
    let _stream = manager
        .logs_start(&connection, &log_request(), lines, end)
        .await
        .expect("a tunneled cluster follows a log");

    let mut seen = Vec::new();
    let _ = timeout(Duration::from_secs(10), async {
        while seen.len() < 2 {
            match line_events.recv().await {
                Some(line) => seen.push(line.line),
                // The channel closes when the stream ends, so a failure shows up as a short list
                // and the end event below says why.
                None => return,
            }
        }
    })
    .await;

    assert_eq!(seen, vec!["tick 1".to_owned(), "tick 2".to_owned()]);
    let end = timeout(Duration::from_secs(5), end_events.recv())
        .await
        .expect("the log stream ended")
        .expect("the stream reports its end once");
    assert!(end.error.is_none(), "the log ended with an error: {:?}", end.error);

    let requests = far.requests();
    assert_eq!(requests.len(), 1, "the log was requested {} times", requests.len());
    assert!(
        requests[0].starts_with("/api/v1/namespaces/lab/pods/web-0/log?"),
        "the log request named {}",
        requests[0]
    );
    assert!(
        requests[0].contains("follow=true"),
        "the request did not follow the log: {}",
        requests[0]
    );
    assert_eq!(
        dialer.asked(),
        vec![("127.0.0.1".to_owned(), 1)],
        "the log stream did not go through the tunnel"
    );

    manager.disconnect(&connection);
}

// -- a TLS WebSocket server ---------------------------------------------------------

/// What the fake terminal server saw, filled in as the upgrade and the frames arrive.
#[derive(Default)]
struct Seen {
    subprotocol: Option<String>,
    authorization: Option<String>,
    target: String,
    frames: Vec<Vec<u8>>,
}

struct TerminalServer {
    address: SocketAddr,
    seen: Arc<Mutex<Seen>>,
}

impl TerminalServer {
    fn seen(&self) -> Seen {
        let seen = self.seen.lock().expect("the seen lock");
        Seen {
            subprotocol: seen.subprotocol.clone(),
            authorization: seen.authorization.clone(),
            target: seen.target.clone(),
            frames: seen.frames.clone(),
        }
    }
}

/// A TLS socket wrapped as an async stream by doing its I/O on this thread.
///
/// `native-tls` has no async server side in this tree, and the far side of these tests has to do a
/// real handshake. The blocking calls are confined to the server's own runtime thread, which has
/// nothing else to run, so nothing in the test's runtime waits on them.
struct BlockingTls(native_tls::TlsStream<std::net::TcpStream>);

impl AsyncRead for BlockingTls {
    fn poll_read(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut().0.read(buffer.initialize_unfilled()) {
            Ok(0) => Poll::Ready(Ok(())),
            Ok(read) => {
                buffer.advance(read);
                Poll::Ready(Ok(()))
            }
            Err(error) => Poll::Ready(Err(error)),
        }
    }
}

impl AsyncWrite for BlockingTls {
    fn poll_write(self: Pin<&mut Self>, _context: &mut Context<'_>, bytes: &[u8]) -> Poll<std::io::Result<usize>> {
        Poll::Ready(self.get_mut().0.write(bytes))
    }

    fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(self.get_mut().0.flush())
    }

    fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// A WebSocket-over-TLS server that answers one upgrade and echoes stdin back as stdout.
fn terminal_server() -> TerminalServer {
    use futures_util::SinkExt as _;
    use futures_util::StreamExt as _;
    use tokio_tungstenite::accept_hdr_async;
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
    use tokio_tungstenite::tungstenite::http::HeaderValue;

    let identity =
        native_tls::Identity::from_pkcs8(&pem("server1.pem"), &pem("server1.key")).expect("a server identity");
    let acceptor = native_tls::TlsAcceptor::new(identity).expect("a TLS acceptor");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("an address");
    let seen = Arc::new(Mutex::new(Seen::default()));
    let recorded = Arc::clone(&seen);

    std::thread::spawn(move || {
        for accepted in listener.incoming() {
            let Ok(accepted) = accepted else { continue };
            let Ok(tls) = acceptor.accept(accepted) else {
                return;
            };
            let recorded = Arc::clone(&recorded);
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime for the fake terminal");
            runtime.block_on(async move {
                let callback = {
                    let recorded = Arc::clone(&recorded);
                    // The error type is tungstenite's own `ErrorResponse`; its size is the
                    // library's, and nothing here ever raises one.
                    #[allow(clippy::result_large_err)]
                    move |request: &Request, mut response: Response| -> Result<Response, ErrorResponse> {
                        {
                            let mut seen = recorded.lock().expect("the seen lock");
                            seen.subprotocol = request
                                .headers()
                                .get("sec-websocket-protocol")
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned);
                            seen.authorization = request
                                .headers()
                                .get("authorization")
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned);
                            seen.target = request.uri().to_string();
                        }
                        let answered = HeaderValue::from_str(PROTOCOL).expect("an answer header");
                        response.headers_mut().insert("sec-websocket-protocol", answered);
                        Ok(response)
                    }
                };
                let Ok(mut socket) = accept_hdr_async(BlockingTls(tls), callback).await else {
                    return;
                };
                while let Some(Ok(message)) = socket.next().await {
                    match message {
                        Message::Binary(bytes) => {
                            let channel = bytes.first().copied();
                            recorded.lock().expect("the seen lock").frames.push(bytes.to_vec());
                            // Channel 0 is the container's stdin; a shell echoes it on stdout.
                            if channel == Some(0) && bytes.len() > 1 {
                                let mut answer = vec![1u8];
                                answer.extend_from_slice(&bytes[1..]);
                                if socket.send(Message::binary(answer)).await.is_err() {
                                    return;
                                }
                            }
                        }
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
            });
        }
    });

    TerminalServer { address, seen }
}

#[tokio::test]
async fn a_pod_terminal_is_upgraded_over_the_tunnel_and_speaks_the_channel_protocol() {
    let far = terminal_server();
    let dialer = LocalDialer::new(Some(far.address));
    let (manager, connection) = tunneled(Arc::clone(&dialer)).await;

    let session = manager
        .exec_open(&connection, &exec_request("echo tunnel-ok"), 100, 30)
        .await
        .expect("a tunneled cluster opens a terminal");
    session.write(b"echo tunnel-ok\n").expect("a write");
    let mut output = session.take_output().expect("an output receiver");

    let mut seen = String::new();
    let _ = timeout(Duration::from_secs(10), async {
        while let Some(bytes) = output.recv().await {
            seen.push_str(&String::from_utf8_lossy(&bytes));
            if seen.contains("echo tunnel-ok") {
                return;
            }
        }
    })
    .await;
    assert!(seen.contains("echo tunnel-ok"), "the terminal answered with {seen:?}");

    let seen = far.seen();
    assert_eq!(
        seen.subprotocol.as_deref(),
        Some(PROTOCOL),
        "the upgrade did not ask for the channel protocol through the tunnel"
    );
    assert_eq!(
        seen.authorization.as_deref(),
        Some("Bearer not-a-real-token"),
        "the upgrade did not carry the cluster credential through the tunnel"
    );
    assert!(
        seen.target.starts_with("/api/v1/namespaces/lab/pods/web-0/exec?"),
        "the terminal was opened at {}",
        seen.target
    );
    assert_eq!(
        dialer.asked(),
        vec![("127.0.0.1".to_owned(), 1)],
        "the terminal did not go through the tunnel"
    );

    session.close();
    manager.disconnect(&connection);
}

#[tokio::test]
async fn a_tunnel_that_cannot_reach_the_cluster_fails_the_terminal_before_the_handshake() {
    // The far side is unreachable, so the bridge answers the CONNECT with a 502 instead of a
    // carried connection. A terminal that treated any answer as carried would try a TLS handshake
    // against an error page and report the wrong thing.
    let dialer = LocalDialer::new(None);
    let (manager, connection) = tunneled(Arc::clone(&dialer)).await;

    let error = match manager
        .exec_open(&connection, &exec_request("echo never"), 80, 24)
        .await
    {
        Ok(session) => {
            session.close();
            panic!("a terminal through a bridge that cannot reach the cluster is not a terminal");
        }
        Err(error) => error,
    };

    assert_eq!(error.code(), ErrorCode::K8sUnavailable);
    assert!(
        error.detail().unwrap_or_default().contains("502"),
        "the failure did not carry the bridge's own answer: {error}"
    );
    assert_eq!(
        dialer.asked(),
        vec![("127.0.0.1".to_owned(), 1)],
        "the bridge was not asked for the server the kubeconfig named"
    );

    manager.disconnect(&connection);
}
