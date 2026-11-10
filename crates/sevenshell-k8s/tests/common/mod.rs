//! A fake API server, and the pieces the integration tests build a kubeconfig from.
//!
//! Hand-written HTTP on purpose: the client's request line, headers and body are what these tests
//! are about, and a real server library would answer questions nobody asked. Two shapes are needed
//! — a plain socket for the request and watch tests, and a TLS one for the certificate tests — and
//! both speak just enough HTTP/1.1 for the client to be honest about what it sends.

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// One request as the fake server saw it.
#[derive(Debug, Clone)]
pub struct Received {
    pub method: String,
    /// The path exactly as it arrived, query included.
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Received {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_ascii_lowercase()).map(String::as_str)
    }
}

/// What the fake server answers one request with.
pub enum Answer {
    /// A complete answer, and the connection closes after it.
    Once { status: u16, body: String },
    /// A stream of one JSON event per line, chunked.
    Stream {
        status: u16,
        lines: Vec<String>,
        then: Ending,
    },
    /// The promised body is never finished, which is what a connection that breaks mid-answer looks
    /// like to the client.
    Truncated { body: String, promised: usize },
    /// Chunked filler, for the response-size cap.
    Endless { chunk: Vec<u8>, limit: usize },
    /// An error answer whose body is a JSON `Status`, as a real API server sends one.
    Status {
        status: u16,
        reason: &'static str,
        message: &'static str,
        code: u16,
    },
}

/// How a stream ends.
pub enum Ending {
    /// The terminating chunk, then close: the server ended the stream on its own terms.
    Close,
    /// Keep the connection open until the client goes away.
    Hold,
    /// Drop the connection without the terminating chunk: a stream that broke, which the client
    /// has to tell apart from one that ended.
    Abort,
}

/// What the fake server answers, given each request and the number of the connection it came on.
type Script = dyn Fn(&Received, usize) -> Answer + Send + Sync;

/// A server on 127.0.0.1 that answers with a script.
pub struct Fake {
    pub base: String,
    requests: Arc<Mutex<Vec<Received>>>,
}

impl Fake {
    pub fn requests(&self) -> Vec<Received> {
        self.requests.lock().expect("requests").clone()
    }

    pub fn last(&self) -> Received {
        self.requests().pop().expect("at least one request")
    }

    pub fn count(&self) -> usize {
        self.requests.lock().expect("requests").len()
    }

    /// Waits until the server has seen `count` requests, which a spawned task makes necessary.
    pub async fn wait_for(&self, count: usize) {
        for _ in 0..200 {
            if self.count() >= count {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("the fake server saw {} requests, not {count}", self.count());
    }
}

/// Starts a fake server. `script` is given each request and the 1-based number of the connection it
/// arrived on, so a test can make the first connection fail and the second one work.
pub async fn fake<F>(script: F) -> Fake
where
    F: Fn(&Received, usize) -> Answer + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
    let base = format!("http://{}", listener.local_addr().expect("an address"));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let script = Arc::new(script);
    let connections = Arc::new(AtomicUsize::new(0));

    let for_task = Arc::clone(&requests);
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let index = connections.fetch_add(1, Ordering::SeqCst) + 1;
            let requests = Arc::clone(&for_task);
            let script = Arc::clone(&script);
            tokio::spawn(async move {
                let _ = serve(stream, index, script, requests).await;
            });
        }
    });

    Fake { base, requests }
}

async fn serve<S>(
    mut stream: S,
    index: usize,
    script: Arc<Script>,
    requests: Arc<Mutex<Vec<Received>>>,
) -> std::io::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let request = read_request(&mut stream).await?;
    let answer = script(&request, index);
    requests.lock().expect("requests").push(request);

    match answer {
        Answer::Once { status, body } => {
            let head = format!(
                "HTTP/1.1 {status} {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                reason(status),
                body.len()
            );
            stream.write_all(head.as_bytes()).await?;
            stream.write_all(body.as_bytes()).await?;
            stream.flush().await?;
        }
        Answer::Status {
            status,
            reason: name,
            message,
            code,
        } => {
            let body = format!(
                r#"{{"kind":"Status","apiVersion":"v1","status":"Failure","message":"{message}","reason":"{name}","code":{code}}}"#
            );
            let head = format!(
                "HTTP/1.1 {status} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                reason(status),
                body.len()
            );
            stream.write_all(head.as_bytes()).await?;
            stream.write_all(body.as_bytes()).await?;
            stream.flush().await?;
        }
        Answer::Stream { status, lines, then } => {
            let head = format!(
                "HTTP/1.1 {status} {}\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\n\r\n",
                reason(status)
            );
            stream.write_all(head.as_bytes()).await?;
            for line in lines {
                // The newline is part of the body, not of the chunk framing: a watch answer is
                // newline-delimited JSON, and the client splits it on those.
                write_chunk(&mut stream, format!("{line}\n").as_bytes()).await?;
                // A real watch is a stream over time, not one buffer: the client has to be able to
                // read events as they arrive.
                stream.flush().await?;
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            match then {
                Ending::Close => {
                    stream.write_all(b"0\r\n\r\n").await?;
                    stream.flush().await?;
                }
                Ending::Hold => {
                    // Until the client hangs up. Reading is how this side learns that it has.
                    let mut sink = [0u8; 64];
                    loop {
                        match stream.read(&mut sink).await {
                            Ok(0) | Err(_) => return Ok(()),
                            Ok(_) => {}
                        }
                    }
                }
                // Dropping the stream here closes the connection with the chunked body unfinished,
                // which the client reads as a break rather than as an end.
                Ending::Abort => {}
            }
        }
        Answer::Truncated { body, promised } => {
            let head =
                format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {promised}\r\n\r\n");
            stream.write_all(head.as_bytes()).await?;
            stream.write_all(body.as_bytes()).await?;
            stream.flush().await?;
            // Closing with the body unfinished is a broken answer, which is what a connection that
            // drops mid-stream looks like from the other end.
        }
        Answer::Endless { chunk, limit } => {
            let head = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\n\r\n";
            stream.write_all(head.as_bytes()).await?;
            let mut written = 0;
            while written < limit {
                if write_chunk(&mut stream, &chunk).await.is_err() {
                    // The client stopped reading, which is the point of the cap.
                    return Ok(());
                }
                written += chunk.len();
            }
            stream.write_all(b"0\r\n\r\n").await?;
            stream.flush().await?;
        }
    }
    Ok(())
}

async fn write_chunk<S>(stream: &mut S, data: &[u8]) -> std::io::Result<()>
where
    S: tokio::io::AsyncWrite + Unpin,
{
    stream.write_all(format!("{:x}\r\n", data.len()).as_bytes()).await?;
    stream.write_all(data).await?;
    stream.write_all(b"\r\n").await
}

async fn read_request<S>(stream: &mut S) -> std::io::Result<Received>
where
    S: tokio::io::AsyncRead + Unpin,
{
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(head) = header_end(&buffer) {
            let text = String::from_utf8_lossy(&buffer[..head]).into_owned();
            if buffer.len() >= head + 4 + content_length(&text) {
                break;
            }
        }
    }

    let head_end = header_end(&buffer).unwrap_or(buffer.len());
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let body = buffer.get(head_end + 4..).unwrap_or_default().to_vec();

    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();

    Ok(Received {
        method,
        path,
        headers,
        body,
    })
}

fn header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length(head: &str) -> usize {
    head.lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse().ok())
        .unwrap_or(0)
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        410 => "Gone",
        500 => "Internal Server Error",
        _ => "Something",
    }
}

// -- TLS -----------------------------------------------------------------------------

/// A TLS server that answers every request the same way, on its own thread.
///
/// Blocking rather than async because `native-tls`'s server side is blocking, and this crate has no
/// async wrapper for it: a thread is the honest way to have a real handshake in a test.
pub struct TlsFake {
    pub base: String,
    pub connections: Arc<AtomicUsize>,
}

pub fn tls_server(server_pem: &[u8], server_key: &[u8]) -> TlsFake {
    let identity = native_tls::Identity::from_pkcs8(server_pem, server_key).expect("a server identity");
    let acceptor = native_tls::TlsAcceptor::new(identity).expect("a TLS acceptor");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let port = listener.local_addr().expect("an address").port();
    let connections = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&connections);

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            counted.fetch_add(1, Ordering::SeqCst);
            let acceptor = acceptor.clone();
            std::thread::spawn(move || {
                let Ok(mut stream) = acceptor.accept(stream) else {
                    return;
                };
                let mut buffer = [0u8; 4096];
                // One request per connection is all these tests send.
                let _ = stream.read(&mut buffer);
                let body = "{}";
                let answer = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(answer.as_bytes());
                let _ = stream.flush();
            });
        }
    });

    TlsFake {
        base: format!("https://127.0.0.1:{port}"),
        connections,
    }
}

/// One of the certificates committed beside these tests.
pub fn pem(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("testdata")
        .join("tls")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{} could not be read: {error}", path.display()))
}

// -- kubeconfigs ---------------------------------------------------------------------

/// A one-context kubeconfig pointing at `server`, with a bearer token.
pub fn document(server: &str, token: &str) -> String {
    format!(
        r#"apiVersion: v1
kind: Config
current-context: test
clusters:
  - name: test-cluster
    cluster:
      server: {server}
contexts:
  - name: test
    context:
      cluster: test-cluster
      user: test-user
      namespace: default
users:
  - name: test-user
    user:
      token: {token}
"#
    )
}

/// A directory of our own under the system temporary directory, removed when dropped.
pub struct Scratch(std::path::PathBuf);

impl Scratch {
    pub fn new() -> Self {
        let path = std::env::temp_dir().join(format!("7shell-k8s-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("a scratch directory");
        Self(path)
    }

    pub fn path(&self) -> &std::path::Path {
        &self.0
    }

    /// Writes a file and answers the path it was written to.
    pub fn write(&self, name: &str, contents: &[u8]) -> std::path::PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, contents).expect("a scratch file");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Default for Scratch {
    fn default() -> Self {
        Self::new()
    }
}
