//! A fake provider: a listener on this machine that answers with strings a test wrote.
//!
//! It is deliberately raw rather than a server framework: what is being tested is the exact bytes a
//! provider puts on the wire and how this crate reads them back, and a real HTTP implementation would
//! add its own opinions about headers, chunking and closing that are not this crate's business.
//!
//! Every request is recorded before it is answered, which is how the tests assert what was sent: the
//! path, the headers (a key among them), and the body.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

/// What a fake provider answers with.
pub enum Body {
    /// One body, in one write, then the connection closes.
    Whole(String),
    /// A chunked body in several parts, each written separately: this is where an SSE event that
    /// straddles two chunks comes from.
    Chunked(Vec<String>),
    /// The first chunk, and then nothing: the connection stays open until the client drops it. Used
    /// to prove that dropping the stream closes the connection and that a quiet stream times out.
    Held(String),
}

pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Body,
}

impl Reply {
    /// A complete event-stream answer, written in parts.
    pub fn stream(parts: Vec<String>) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: Body::Chunked(parts),
        }
    }

    /// A complete event-stream answer written in one go.
    pub fn whole(text: &str) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: Body::Whole(text.to_owned()),
        }
    }

    /// A refusal, with a JSON body.
    pub fn status(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Body::Whole(body.to_owned()),
        }
    }

    /// An answer that begins and then never finishes.
    pub fn held(first: &str) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: Body::Held(first.to_owned()),
        }
    }

    /// One more response header, such as a provider's `Retry-After`.
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

/// One request, as it arrived.
#[derive(Clone, Debug)]
pub struct Recorded {
    pub head: String,
    pub body: Vec<u8>,
}

impl Recorded {
    pub fn method(&self) -> &str {
        self.head.split_whitespace().next().unwrap_or_default()
    }

    pub fn path(&self) -> &str {
        self.head.split_whitespace().nth(1).unwrap_or_default()
    }

    /// One header, matched without regard to case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            if key.trim().eq_ignore_ascii_case(name) {
                Some(value.trim())
            } else {
                None
            }
        })
    }

    pub fn body_json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).expect("the request body is JSON")
    }
}

/// A listening provider that answers each connection with the next reply in the list.
pub struct FakeProvider {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<Recorded>>>,
    closed: Arc<Notify>,
}

impl FakeProvider {
    pub async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a listener on this machine");
        let address = listener.local_addr().expect("an address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let closed = Arc::new(Notify::new());

        let queue = Arc::new(Mutex::new(replies.into_iter().collect::<VecDeque<_>>()));
        let recorded = Arc::clone(&requests);
        let dropped = Arc::clone(&closed);

        tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    return;
                };
                let reply = queue.lock().expect("the queue").pop_front();
                let recorded = Arc::clone(&recorded);
                let dropped = Arc::clone(&dropped);

                tokio::spawn(async move {
                    let Some(reply) = reply else {
                        // More connections than the test scripted: a retry that should not have
                        // happened shows up as this answer.
                        serve(
                            socket,
                            Reply::status(500, "{\"error\":{\"message\":\"unexpected request\"}}"),
                            recorded,
                            dropped,
                        )
                        .await;
                        return;
                    };
                    serve(socket, reply, recorded, dropped).await;
                });
            }
        });

        Self {
            address,
            requests,
            closed,
        }
    }

    /// The base address to configure a provider with, without a path.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().expect("the record").clone()
    }

    pub fn request_count(&self) -> usize {
        self.requests.lock().expect("the record").len()
    }

    /// Waits until the client closed the connection it was holding open.
    pub async fn wait_for_close(&self) {
        self.closed.notified().await;
    }
}

async fn serve(mut socket: TcpStream, reply: Reply, recorded: Arc<Mutex<Vec<Recorded>>>, dropped: Arc<Notify>) {
    let Some(request) = read_request(&mut socket).await else {
        return;
    };
    recorded.lock().expect("the record").push(request);

    let mut head = format!("HTTP/1.1 {} {}\r\n", reply.status, reason(reply.status));
    head.push_str("content-type: text/event-stream\r\n");
    for (name, value) in &reply.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }

    match reply.body {
        Body::Whole(text) => {
            head.push_str(&format!("content-length: {}\r\nconnection: close\r\n\r\n", text.len()));
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(text.as_bytes()).await;
            let _ = socket.flush().await;
            let _ = socket.shutdown().await;
        }
        Body::Chunked(parts) => {
            head.push_str("transfer-encoding: chunked\r\nconnection: close\r\n\r\n");
            let _ = socket.write_all(head.as_bytes()).await;
            for part in parts {
                let frame = format!("{:x}\r\n{}\r\n", part.len(), part);
                let _ = socket.write_all(frame.as_bytes()).await;
                let _ = socket.flush().await;
                // A small pause so each part is its own read on the client's side, which is what
                // makes a chunk boundary a real test rather than a hopeful one.
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            let _ = socket.write_all(b"0\r\n\r\n").await;
            let _ = socket.flush().await;
            let _ = socket.shutdown().await;
        }
        Body::Held(part) => {
            head.push_str("transfer-encoding: chunked\r\n\r\n");
            let _ = socket.write_all(head.as_bytes()).await;
            let frame = format!("{:x}\r\n{}\r\n", part.len(), part);
            let _ = socket.write_all(frame.as_bytes()).await;
            let _ = socket.flush().await;

            // The client has to drop the response for this read to see anything; nothing else is
            // ever sent.
            let mut sink = [0u8; 64];
            loop {
                match socket.read(&mut sink).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            dropped.notify_one();
        }
    }
}

async fn read_request(socket: &mut TcpStream) -> Option<Recorded> {
    let mut data: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 8192];

    loop {
        if let Some(end) = find(&data, b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&data[..end]).into_owned();
            let length = content_length(&head).unwrap_or(0);
            let body_start = end + 4;

            while data.len() < body_start + length {
                let read = socket.read(&mut buffer).await.ok()?;
                if read == 0 {
                    break;
                }
                data.extend_from_slice(&buffer[..read]);
            }

            let body_end = (body_start + length).min(data.len());
            return Some(Recorded {
                head,
                body: data[body_start..body_end].to_vec(),
            });
        }

        let read = socket.read(&mut buffer).await.ok()?;
        if read == 0 {
            return None;
        }
        data.extend_from_slice(&buffer[..read]);
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn content_length(head: &str) -> Option<usize> {
    head.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        if key.trim().eq_ignore_ascii_case("content-length") {
            value.trim().parse().ok()
        } else {
            None
        }
    })
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        529 => "Overloaded",
        _ => "Status",
    }
}
