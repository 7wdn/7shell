//! A WebDAV session through the core: open, browse, and move bytes both ways.
//!
//! The protocol is tested in `sevenshell-sync`; what this file proves is the wiring — that a
//! `webdav` session reaches the same file browser and the same transfer registry as an SSH one,
//! and that nothing on that path still assumes an SFTP channel.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sevenshell_core::Core;
use sevenshell_types::{
    AuthMethod, EntryKind, Protocol, Session, SessionAuth, SftpOutcome, TerminalSettings, TransferState,
    WebDavSettings, WslSettings,
};
use sevenshell_vault::{SessionSecret, Vault};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Debug, Clone)]
struct Received {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

struct Fake {
    base: String,
    seen: Arc<Mutex<Vec<Received>>>,
}

impl Fake {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
        let base = format!("http://{}", listener.local_addr().expect("an address"));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_for_task = Arc::clone(&seen);

        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let seen = Arc::clone(&seen_for_task);
                tokio::spawn(async move {
                    let _ = serve_one(stream, seen).await;
                });
            }
        });

        Self { base, seen }
    }

    fn requests(&self) -> Vec<Received> {
        self.seen.lock().expect("seen").clone()
    }
}

async fn serve_one(stream: TcpStream, seen: Arc<Mutex<Vec<Received>>>) -> std::io::Result<()> {
    let mut stream = stream;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];

    loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(head) = find_headers_end(&buffer) {
            let text = String::from_utf8_lossy(&buffer[..head]).into_owned();
            if buffer.len() >= head + 4 + content_length(&text) {
                break;
            }
        }
    }

    let head_end = find_headers_end(&buffer).unwrap_or(buffer.len());
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let body = String::from_utf8_lossy(&buffer[head_end + 4..]).into_owned();

    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let headers: HashMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();

    seen.lock().expect("seen").push(Received {
        method: method.clone(),
        path,
        headers,
        body,
    });

    let (status, answer) = answer(&method);
    let mut response = format!(
        "HTTP/1.1 {status} {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        reason(status),
        answer.len()
    );
    response.push_str(answer);
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await
}

fn answer(method: &str) -> (u16, &'static str) {
    match method {
        // Every collection has the same two entries: one file the download asks about, and one
        // directory the panel draws first.
        "PROPFIND" => (207, LISTING),
        "GET" => (200, "hello"),
        "PUT" | "MKCOL" | "MOVE" | "COPY" => (201, ""),
        "DELETE" => (204, ""),
        _ => (404, ""),
    }
}

const LISTING: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/dav/</D:href>
    <D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop>
      <D:status>HTTP/1.1 200 OK</D:status></D:propstat>
  </D:response>
  <D:response>
    <D:href>/dav/docs/</D:href>
    <D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop>
      <D:status>HTTP/1.1 200 OK</D:status></D:propstat>
  </D:response>
  <D:response>
    <D:href>/dav/notes.txt</D:href>
    <D:propstat><D:prop>
      <D:resourcetype/>
      <D:getcontentlength>5</D:getcontentlength>
      <D:getlastmodified>Mon, 09 Nov 2026 10:00:00 GMT</D:getlastmodified>
    </D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat>
  </D:response>
</D:multistatus>"#;

fn find_headers_end(buffer: &[u8]) -> Option<usize> {
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
        207 => "Multi-Status",
        404 => "Not Found",
        _ => "Something",
    }
}

fn webdav_session(url: String) -> Session {
    Session {
        id: "dav-1".to_owned(),
        name: "dav".to_owned(),
        group_id: None,
        protocol: Protocol::WebDav,
        host: String::new(),
        port: 443,
        user: "alice".to_owned(),
        auth: SessionAuth::with_password(AuthMethod::Password),
        terminal: TerminalSettings::default(),
        desktop: sevenshell_types::DesktopSettings::default(),
        telnet: sevenshell_types::TelnetSettings::default(),
        serial: sevenshell_types::SerialSettings::default(),
        local: sevenshell_types::LocalSettings::default(),
        wsl: WslSettings::default(),
        webdav: WebDavSettings { url },
        ftp: sevenshell_types::FtpSettings::default(),
        jump: Vec::new(),
        forwards: Vec::new(),
        updated_at: 1_700_000_000,
    }
}

async fn settle(core: &Core, id: &str) -> TransferState {
    for _ in 0..200 {
        if let Some(info) = core.transfers().await.into_iter().find(|info| info.id == id)
            && !matches!(info.state, TransferState::Running | TransferState::Paused)
        {
            return info.state;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the transfer did not settle");
}

#[tokio::test]
async fn a_webdav_session_browses_and_moves_bytes_both_ways() {
    let server = Fake::start().await;
    let directory = tempfile::tempdir().expect("temp dir");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault.create("master password").expect("a vault");
    vault
        .save_session(
            webdav_session(format!("{}/dav/", server.base)),
            Some(SessionSecret::Password("hunter2".to_owned())),
        )
        .expect("a session");
    let (core, _events) = Core::new(vault);

    let outcome = core.open_files("dav-1").await.expect("opened");
    let SftpOutcome::Opened { handle, listing, .. } = outcome else {
        panic!("a WebDAV session should open a browser");
    };
    assert_eq!(listing.path, "/");
    assert_eq!(listing.entries.len(), 2);
    assert_eq!(listing.entries[0].name, "docs", "folders come first");
    assert_eq!(listing.entries[0].kind, EntryKind::Directory);
    assert_eq!(listing.entries[1].name, "notes.txt");
    assert_eq!(listing.entries[1].size, 5);
    assert!(listing.entries[1].modified_at.is_some(), "the date column has a time");
    assert!(listing.entries[1].mode.is_none(), "and no invented permission bits");

    let opened = server.requests();
    assert_eq!(opened[0].method, "PROPFIND");
    assert_eq!(
        opened[0].headers.get("authorization").map(String::as_str),
        Some("Basic YWxpY2U6aHVudGVyMg=="),
        "the session's stored password reached the server"
    );

    let target = directory.path().join("notes.txt");
    let download = core
        .start_download(&handle, "/notes.txt", target.clone())
        .await
        .expect("a download");
    assert_eq!(settle(&core, &download).await, TransferState::Done);
    assert_eq!(std::fs::read_to_string(&target).expect("read"), "hello");

    let source = directory.path().join("up.txt");
    std::fs::write(&source, b"world").expect("write");
    let upload = core.start_upload(&handle, source, "/").await.expect("an upload");
    assert_eq!(settle(&core, &upload).await, TransferState::Done);

    let requests = server.requests();
    let put = requests
        .iter()
        .find(|request| request.method == "PUT")
        .expect("the upload reached the server");
    assert_eq!(put.path, "/dav/.up.txt.7shell-part", "the target is written last");
    assert_eq!(put.body, "world");
    assert_eq!(put.headers.get("content-length").map(String::as_str), Some("5"));

    let moved = requests
        .iter()
        .find(|request| request.method == "MOVE")
        .expect("the part file was moved into place");
    assert_eq!(moved.path, "/dav/.up.txt.7shell-part");
    assert_eq!(
        moved.headers.get("destination"),
        Some(&format!("{}/dav/up.txt", server.base))
    );
}
