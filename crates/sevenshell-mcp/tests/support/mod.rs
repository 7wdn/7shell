//! What the integration tests need on the other side of the bridge.
//!
//! The desktop is a listener on loopback that speaks exactly the contract in
//! `sevenshell-api/src/endpoint.rs`: one `POST /tools/call`, a bearer token, a JSON body. It is
//! hand-written on `tokio::net` rather than built out of an HTTP framework because what the tests
//! have to assert is the bytes — the method line, the headers, the body — and a framework's typed
//! handler is precisely the layer that hides them. One route is not worth a dependency.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

use sevenshell_api::{ENDPOINT_FILE_NAME, EndpointFile, ToolError, ToolOutcome, ToolResult};

/// One request as it arrived, before anything interpreted it.
#[derive(Debug, Clone)]
pub struct Captured {
    pub method: String,
    pub path: String,
    /// Names lower-cased, values trimmed: HTTP header names are case-insensitive.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Captured {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// What the fake desktop answers next.
#[derive(Debug, Clone)]
pub struct Reply {
    status: u16,
    body: String,
}

impl Reply {
    /// A success the control endpoint would produce for a tool that ran.
    pub fn succeeded(value: Value) -> Self {
        Self::outcome(ToolOutcome::succeeded(ToolResult::new(value)))
    }

    /// The same, for a tool that refused.
    pub fn failed(error: ToolError) -> Self {
        Self::outcome(ToolOutcome::failed(error))
    }

    pub fn status(status: u16) -> Self {
        Self {
            status,
            body: String::new(),
        }
    }

    fn outcome(outcome: ToolOutcome) -> Self {
        Self {
            status: 200,
            body: serde_json::to_string(&outcome).expect("a tool outcome is JSON"),
        }
    }

    fn reason(&self) -> &'static str {
        match self.status {
            200 => "OK",
            401 => "Unauthorized",
            413 => "Payload Too Large",
            _ => "Error",
        }
    }
}

/// A desktop's loopback endpoint, answering every connection with one configured reply.
pub struct FakeDesktop {
    reply: Arc<Mutex<Reply>>,
    captured: Arc<Mutex<Vec<Captured>>>,
    port: u16,
}

impl FakeDesktop {
    pub async fn start(reply: Reply) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("a loopback port is available");
        let port = listener.local_addr().expect("a bound address").port();
        let reply = Arc::new(Mutex::new(reply));
        let captured = Arc::new(Mutex::new(Vec::<Captured>::new()));

        let accepting = (Arc::clone(&reply), Arc::clone(&captured));
        tokio::spawn(async move {
            let (reply, captured) = accepting;
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let reply = Arc::clone(&reply);
                let captured = Arc::clone(&captured);
                tokio::spawn(async move {
                    let _ = answer(stream, reply, captured).await;
                });
            }
        });

        Self { reply, captured, port }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Writes the endpoint file the desktop would have written, and returns its path.
    pub fn write_endpoint(&self, directory: &Path, token: &str) -> PathBuf {
        let path = directory.join(ENDPOINT_FILE_NAME);
        write_endpoint_file(&path, self.port, token);
        path
    }

    pub fn reply_with(&self, reply: Reply) {
        *self.reply.lock().expect("the reply lock") = reply;
    }

    pub fn captured(&self) -> Vec<Captured> {
        self.captured.lock().expect("the capture lock").clone()
    }
}

pub fn write_endpoint_file(path: &Path, port: u16, token: &str) {
    let endpoint = EndpointFile::new(port, token, std::process::id(), 1_700_000_000);
    let raw = serde_json::to_vec(&endpoint).expect("an endpoint file is JSON");
    std::fs::write(path, raw).expect("the endpoint file is written");
}

/// Reads one request, records it, and answers with whatever the desktop is configured to say.
async fn answer(
    stream: TcpStream,
    reply: Arc<Mutex<Reply>>,
    captured: Arc<Mutex<Vec<Captured>>>,
) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);

    let mut request_line = String::new();
    reader.read_line(&mut request_line).await?;
    let mut parts = request_line.trim_end().split(' ');
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();

    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            break;
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }

    let length: usize = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body).await?;

    captured.lock().expect("the capture lock").push(Captured {
        method,
        path,
        headers,
        body,
    });

    let (status, reason, body) = {
        let reply = reply.lock().expect("the reply lock");
        (reply.status, reply.reason(), reply.body.clone())
    };

    write
        .write_all(
            format!(
                "HTTP/1.1 {status} {reason}\r\n\
                 content-type: application/json\r\n\
                 content-length: {}\r\n\
                 connection: close\r\n\
                 \r\n",
                body.len()
            )
            .as_bytes(),
        )
        .await?;
    write.write_all(body.as_bytes()).await?;
    write.flush().await?;
    Ok(())
}
