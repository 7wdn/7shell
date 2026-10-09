//! Talking to a WebDAV server: the three verbs a backup needs, and nothing else.
//!
//! A WebDAV server is, for this application, a place a file can be put and got. No properties, no
//! locking, no WebDAV beyond `PROPFIND` to ask whether the address is one, `MKCOL` to make the
//! folders a path needs, and `PUT`/`GET` to move the file itself.
//!
//! Five rules hold the transport together, and each of them is a bug that somebody has already had:
//!
//! * **The password travels in one header and nowhere else.** Never in the URL (where it would end
//!   up in a log on either side), never in a body, never in an error.
//! * **A redirect must not become a way to hand credentials to somebody else.** The client follows
//!   redirects but drops the authorization header when the host changes — the one rule that keeps
//!   `Warning: 301` from being a credential leak. Pinned by a test against a second server.
//! * **A private address is not sent through a proxy.** A server on this machine or in this
//!   building is reachable without one, and a company proxy that cannot resolve `127.0.0.1` turns a
//!   working setup into a mystery.
//! * **Nothing hangs forever.** A request that never answers is reported as unreachable rather than
//!   leaving a page saying "working…" until the window is closed.
//! * **Every refusal has one of a few names.** Whether the address is wrong, the password is wrong
//!   or the server said no: they are different sentences to the person typing them in.

use std::net::IpAddr;
use std::time::Duration;

use reqwest::Method;
use reqwest::header::CONTENT_TYPE;
use sevenshell_error::ShellError;
use sevenshell_vault::ServicePassword;

/// How long one request may take before it is called unreachable.
///
/// Long enough for a slow server on a slow line to answer a small file, short enough that a person
/// watching a page is told something is wrong while they are still looking at it.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The name a backup is written under when nobody chose one.
pub const DEFAULT_REMOTE_PATH: &str = "7shell/sync/backup.json";

/// The file a backup is written to, and who to authenticate as.
///
/// The password is a [`ServicePassword`] rather than a `String` for the reason every secret in this
/// workspace is wrapped: this value goes into a header on its way to a server, and it must not be
/// one `{:?}` away from a log line while it does. The type itself lives in the vault crate, which
/// is where every secret type lives — one implementation of that discipline, not one per feature.
#[derive(Clone, Default)]
pub struct SyncService {
    pub endpoint: String,
    pub username: Option<String>,
    pub password: Option<ServicePassword>,
    /// Where the file goes inside the endpoint. Normalized on the way in.
    pub remote_path: String,
}

/// A WebDAV server this application can put a snapshot on.
pub struct WebDav {
    client: reqwest::Client,
    endpoint: reqwest::Url,
    username: Option<String>,
    password: Option<ServicePassword>,
    remote_path: String,
}

/// Hand-written because of what is *not* printed: the password, and the endpoint.
///
/// The address is skipped rather than printed because a URL's path is a token on some servers —
/// a share link is the address — and a `{:?}` that appears in a log line or a bug report must not
/// be the thing that leaks one. What is here is what is useful while debugging: where the file goes
/// and whether anybody was authenticated at all.
impl std::fmt::Debug for WebDav {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebDav")
            .field("remote_path", &self.remote_path)
            .field("has_username", &self.username.is_some())
            .field("has_password", &self.password.is_some())
            .finish_non_exhaustive()
    }
}

impl WebDav {
    /// Builds a client for one server, or says what is wrong with the address.
    pub fn new(service: &SyncService) -> Result<Self, ShellError> {
        let endpoint = service.endpoint.trim();
        if endpoint.is_empty() {
            return Err(ShellError::sync_not_configured());
        }

        let mut url = reqwest::Url::parse(endpoint)
            .map_err(|error| ShellError::sync_not_configured().with_detail(format!("the address: {error}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(
                ShellError::sync_not_configured().with_detail(format!("the address starts with '{}'", url.scheme()))
            );
        }
        // The endpoint is a folder, and every other part of this joins onto it: an address that
        // does not end in a slash loses its last segment to the join rather than keeping it.
        if !url.path().ends_with('/') {
            let path = format!("{}/", url.path());
            url.set_path(&path);
        }

        let mut builder = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("7Shell/", env!("CARGO_PKG_VERSION")));
        if is_private_host(&url) {
            builder = builder.no_proxy();
        }

        let client = builder
            .build()
            .map_err(|error| ShellError::app_internal(format!("an HTTP client could not be built: {error}")))?;

        Ok(Self {
            client,
            endpoint: url,
            username: service.username.clone().filter(|name| !name.is_empty()),
            password: service.password.clone().filter(|password| !password.is_empty()),
            remote_path: normalize_path(&service.remote_path),
        })
    }

    /// The name the file is written under, for a sentence somebody reads.
    pub fn file_name(&self) -> String {
        self.remote_path
            .rsplit('/')
            .next()
            .unwrap_or(&self.remote_path)
            .to_owned()
    }

    /// Asks the server whether this address is a WebDAV one.
    ///
    /// `PROPFIND` with depth zero is the question every WebDAV server answers, and the one a plain
    /// HTTP server refuses — which is the difference between "the address is right and this is not
    /// a WebDAV server" and "the address is right".
    pub async fn probe(&self) -> Result<(), ShellError> {
        let response = self
            .send(
                self.client
                    .request(dav_method("PROPFIND")?, self.endpoint.clone())
                    .header("depth", "0"),
            )
            .await?;

        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        Err(self.refused(status.as_u16()))
    }

    /// Reads the file, or `None` when the server does not have one yet.
    pub async fn read(&self) -> Result<Option<String>, ShellError> {
        let response = self.send(self.client.get(self.file_url()?)).await?;
        let status = response.status();

        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(self.refused(status.as_u16()));
        }

        response
            .text()
            .await
            .map(Some)
            .map_err(|error| ShellError::sync_unreachable().with_detail(format!("the answer: {error}")))
    }

    /// Writes the file, making the folders on the way if the server does not have them.
    ///
    /// `MKCOL` on a collection that is already there answers `405`, which is the server saying "it
    /// is already what you asked for" — the one status that is success here rather than a refusal.
    pub async fn write(&self, text: &str) -> Result<(), ShellError> {
        for collection in parent_collections(&self.remote_path) {
            let mut url = self.endpoint.clone();
            url.set_path(&format!("{}{}", self.endpoint.path(), collection));
            let response = self.send(self.client.request(dav_method("MKCOL")?, url)).await?;
            let status = response.status();

            if !status.is_success() && status != reqwest::StatusCode::METHOD_NOT_ALLOWED {
                return Err(self.refused(status.as_u16()));
            }
        }

        let response = self
            .send(
                self.client
                    .put(self.file_url()?)
                    .header(CONTENT_TYPE, "application/json")
                    .body(text.to_owned()),
            )
            .await?;
        let status = response.status();

        if !status.is_success() {
            return Err(self.refused(status.as_u16()));
        }
        Ok(())
    }

    /// The file's own address: the endpoint with the normalized path joined onto it.
    fn file_url(&self) -> Result<reqwest::Url, ShellError> {
        let mut url = self.endpoint.clone();
        url.set_path(&format!("{}{}", self.endpoint.path(), self.remote_path));
        Ok(url)
    }

    /// Sends one request, with the credentials it is entitled to.
    ///
    /// Nothing here builds an error out of the request: the address of a sync server is the one
    /// thing an error envelope must not carry, and a header is where the password lives.
    async fn send(&self, mut request: reqwest::RequestBuilder) -> Result<reqwest::Response, ShellError> {
        if let Some(username) = &self.username {
            request = request.basic_auth(username, self.password.as_ref().map(ServicePassword::expose));
        }

        request.send().await.map_err(|error| {
            if error.is_timeout() {
                ShellError::sync_unreachable().with_detail("the server did not answer in time")
            } else {
                ShellError::sync_unreachable().with_detail(format!("the request failed: {error}"))
            }
        })
    }

    /// The one place a status the server returned becomes a sentence.
    fn refused(&self, status: u16) -> ShellError {
        if status == 401 || status == 403 {
            return ShellError::sync_auth_failed().with_detail(format!("the server answered {status}"));
        }
        ShellError::sync_http_failed(status)
    }
}

/// The methods WebDAV needs that `reqwest::Method` has no constant for.
fn dav_method(name: &str) -> Result<Method, ShellError> {
    Method::from_bytes(name.as_bytes())
        .map_err(|error| ShellError::app_internal(format!("'{name}' is not a usable method: {error}")))
}

/// A path made safe to join onto an address.
///
/// Backslashes become slashes because a person pasting a path out of Windows writes them; empty
/// segments and `.` disappear and `..` pops, because a path that walks upwards out of the endpoint
/// is a path somebody did not mean. Anything empty lands on the default.
pub fn normalize_path(path: &str) -> String {
    let separators = path.replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    for part in separators.split('/') {
        match part {
            "" | "." => continue,
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }

    if parts.is_empty() {
        return DEFAULT_REMOTE_PATH.to_owned();
    }
    parts.join("/")
}

/// Every collection a path needs, outermost first.
///
/// `7shell/sync/backup.json` needs `7shell` and then `7shell/sync`; a file at the root needs none.
pub fn parent_collections(path: &str) -> Vec<String> {
    let parts: Vec<&str> = path.split('/').collect();
    (1..parts.len()).map(|depth| parts[..depth].join("/")).collect()
}

/// Whether an address is on this machine or in this building.
///
/// Those are reached directly rather than through a proxy: a company proxy that cannot resolve
/// `127.0.0.1` turns a working local setup into a mystery, and a lab server has no business being
/// forwarded anywhere.
fn is_private_host(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }

    // An IPv6 address comes back in brackets — `[::1]` — because that is how it is written in a
    // URL, and the brackets are not part of the address.
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    match bare.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => address.is_loopback() || address.is_private() || address.is_link_local(),
        Ok(IpAddr::V6(address)) => address.is_loopback() || address.is_unspecified(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    /// One request a fake server received, in the pieces the tests assert on.
    #[derive(Debug, Clone)]
    struct Received {
        method: String,
        path: String,
        headers: HashMap<String, String>,
        body: String,
    }

    /// What the server answers with, per method and path.
    #[derive(Clone)]
    struct Answer {
        status: u16,
        body: String,
        location: Option<String>,
    }

    impl Answer {
        fn ok(body: &str) -> Self {
            Self {
                status: 200,
                body: body.to_owned(),
                location: None,
            }
        }

        fn status(status: u16) -> Self {
            Self {
                status,
                body: String::new(),
                location: None,
            }
        }
    }

    /// A WebDAV server with no TLS, on a port the operating system picks.
    ///
    /// Plain HTTP on purpose: what these tests are about is the shape of the requests — which verb,
    /// which path, which headers — and a certificate would be a second thing to get right before
    /// any of that could be asserted.
    struct Fake {
        base: String,
        seen: Arc<Mutex<Vec<Received>>>,
    }

    impl Fake {
        async fn start(answers: Vec<(&str, Answer)>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
            let base = format!("http://{}", listener.local_addr().expect("an address"));
            let seen = Arc::new(Mutex::new(Vec::new()));
            let scripted: HashMap<String, Answer> = answers
                .into_iter()
                .map(|(key, answer)| (key.to_owned(), answer))
                .collect();
            let answers = Arc::new(Mutex::new(scripted));

            let seen_for_task = Arc::clone(&seen);
            let answers_for_task = Arc::clone(&answers);
            tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        return;
                    };
                    let seen = Arc::clone(&seen_for_task);
                    let answers = Arc::clone(&answers_for_task);
                    tokio::spawn(async move {
                        let _ = serve_one(stream, seen, answers).await;
                    });
                }
            });

            Self { base, seen }
        }

        fn requests(&self) -> Vec<Received> {
            self.seen.lock().expect("seen").clone()
        }
    }

    async fn serve_one(
        stream: TcpStream,
        seen: Arc<Mutex<Vec<Received>>>,
        answers: Arc<Mutex<HashMap<String, Answer>>>,
    ) -> std::io::Result<()> {
        let mut stream = stream;
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];

        // Headers first, then as much body as the headers promised.
        loop {
            let read = stream.read(&mut chunk).await?;
            if read == 0 {
                return Ok(());
            }
            buffer.extend_from_slice(&chunk[..read]);
            if let Some(head) = find_headers_end(&buffer) {
                let text = String::from_utf8_lossy(&buffer[..head]).into_owned();
                let length = content_length(&text);
                if buffer.len() >= head + 4 + length {
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
            path: path.clone(),
            headers: headers.clone(),
            body: body.to_string(),
        });

        let answer = answers
            .lock()
            .expect("answers")
            .get(&format!("{method} {path}"))
            .cloned()
            .unwrap_or_else(|| Answer::status(404));

        let mut response = format!(
            "HTTP/1.1 {} {}\r\ncontent-length: {}\r\nconnection: close\r\n",
            answer.status,
            reason(answer.status),
            answer.body.len()
        );
        if let Some(location) = &answer.location {
            response.push_str(&format!("location: {location}\r\n"));
        }
        response.push_str("\r\n");
        response.push_str(&answer.body);

        stream.write_all(response.as_bytes()).await?;
        stream.flush().await
    }

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
            207 => "Multi-Status",
            404 => "Not Found",
            405 => "Method Not Allowed",
            401 => "Unauthorized",
            403 => "Forbidden",
            500 => "Internal Server Error",
            302 => "Found",
            _ => "Something",
        }
    }

    fn service(base: &str) -> SyncService {
        SyncService {
            endpoint: format!("{base}/dav/"),
            username: Some("alice".to_owned()),
            password: Some(ServicePassword::new("hunter2".to_owned())),
            remote_path: "7shell/sync/backup.json".to_owned(),
        }
    }

    #[test]
    fn a_path_is_made_safe_to_join_onto_an_address() {
        assert_eq!(normalize_path("7shell/sync/backup.json"), "7shell/sync/backup.json");
        assert_eq!(normalize_path("7shell\\sync\\backup.json"), "7shell/sync/backup.json");
        assert_eq!(
            normalize_path("/leading//and/./dots/../here.json"),
            "leading/and/here.json"
        );
        assert_eq!(normalize_path("../../escape.json"), "escape.json");
        assert_eq!(normalize_path(""), DEFAULT_REMOTE_PATH);
        assert_eq!(
            normalize_path("   "),
            "   ",
            "a name is not trimmed into something else"
        );
    }

    #[test]
    fn the_folders_a_path_needs_are_outermost_first() {
        assert_eq!(
            parent_collections("7shell/sync/backup.json"),
            vec!["7shell".to_owned(), "7shell/sync".to_owned()]
        );
        assert!(
            parent_collections("backup.json").is_empty(),
            "a file at the root needs none"
        );
    }

    #[test]
    fn a_local_address_is_not_sent_through_a_proxy() {
        for address in [
            "http://127.0.0.1:8080/",
            "http://localhost:8080/",
            "http://192.168.1.10/dav/",
            "http://10.0.0.5/dav/",
            "http://[::1]:8080/",
        ] {
            assert!(
                is_private_host(&reqwest::Url::parse(address).expect("a URL")),
                "{address} is on this machine or in this building"
            );
        }

        for address in ["https://dav.example.com/", "https://203.0.113.9/"] {
            assert!(
                !is_private_host(&reqwest::Url::parse(address).expect("a URL")),
                "{address} is somewhere else"
            );
        }
    }

    #[test]
    fn an_address_that_is_not_one_is_refused_before_anything_is_sent() {
        for endpoint in ["", "   ", "ftp://example.com/", "not an address"] {
            let service = SyncService {
                endpoint: endpoint.to_owned(),
                ..service("http://127.0.0.1:1")
            };
            let error = WebDav::new(&service).expect_err("not an address");

            assert_eq!(error.code(), sevenshell_error::ErrorCode::SyncNotConfigured);
        }
    }

    #[tokio::test]
    async fn probing_a_server_asks_it_the_webdav_question() {
        let server = Fake::start(vec![("PROPFIND /dav/", Answer::status(207))]).await;
        let dav = WebDav::new(&service(&server.base)).expect("a client");

        dav.probe().await.expect("a WebDAV server answers 207");

        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "PROPFIND");
        assert_eq!(requests[0].path, "/dav/");
        assert_eq!(requests[0].headers.get("depth").map(String::as_str), Some("0"));
    }

    #[tokio::test]
    async fn the_password_travels_in_one_header_and_nowhere_else() {
        let server = Fake::start(vec![("PROPFIND /dav/", Answer::status(207))]).await;
        let dav = WebDav::new(&service(&server.base)).expect("a client");

        dav.probe().await.expect("answered");

        let request = &server.requests()[0];
        assert_eq!(
            request.headers.get("authorization").map(String::as_str),
            Some("Basic YWxpY2U6aHVudGVyMg=="),
            "alice and the password, in the header and only there"
        );
        assert!(!request.path.contains("hunter2"), "not in the address");
        assert!(!request.body.contains("hunter2"), "not in the body");
    }

    #[tokio::test]
    async fn writing_makes_the_folders_first_and_then_puts_the_file() {
        let server = Fake::start(vec![
            ("MKCOL /dav/7shell", Answer::status(201)),
            ("MKCOL /dav/7shell/sync", Answer::status(201)),
            ("PUT /dav/7shell/sync/backup.json", Answer::status(201)),
        ])
        .await;
        let dav = WebDav::new(&service(&server.base)).expect("a client");

        dav.write("{\"format\":\"sevenshell-sync\"}").await.expect("written");

        let requests = server.requests();
        let shape: Vec<(&str, &str)> = requests
            .iter()
            .map(|request| (request.method.as_str(), request.path.as_str()))
            .collect();
        assert_eq!(
            shape,
            vec![
                ("MKCOL", "/dav/7shell"),
                ("MKCOL", "/dav/7shell/sync"),
                ("PUT", "/dav/7shell/sync/backup.json"),
            ],
            "folders outermost first, and the file last"
        );
        assert_eq!(requests[2].body, "{\"format\":\"sevenshell-sync\"}");
        assert_eq!(
            requests[2].headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
    }

    #[tokio::test]
    async fn a_folder_that_is_already_there_is_not_a_refusal() {
        // 405 is the server saying "it is already what you asked for", and a second upload to the
        // same place is the ordinary case rather than an error.
        let server = Fake::start(vec![
            ("MKCOL /dav/7shell", Answer::status(405)),
            ("MKCOL /dav/7shell/sync", Answer::status(405)),
            ("PUT /dav/7shell/sync/backup.json", Answer::status(200)),
        ])
        .await;
        let dav = WebDav::new(&service(&server.base)).expect("a client");

        dav.write("{}").await.expect("still written");
    }

    #[tokio::test]
    async fn reading_a_file_that_is_not_there_is_an_answer_rather_than_a_failure() {
        let server = Fake::start(Vec::new()).await;
        let dav = WebDav::new(&service(&server.base)).expect("a client");

        assert_eq!(dav.read().await.expect("nothing there"), None);
    }

    #[tokio::test]
    async fn a_file_that_is_there_comes_back_as_it_went() {
        let server = Fake::start(vec![(
            "GET /dav/7shell/sync/backup.json",
            Answer::ok("{\"format\":\"sevenshell-sync\"}"),
        )])
        .await;
        let dav = WebDav::new(&service(&server.base)).expect("a client");

        assert_eq!(
            dav.read().await.expect("read").as_deref(),
            Some("{\"format\":\"sevenshell-sync\"}")
        );
    }

    #[tokio::test]
    async fn a_password_the_server_refuses_is_its_own_answer() {
        let server = Fake::start(vec![("PROPFIND /dav/", Answer::status(401))]).await;
        let dav = WebDav::new(&service(&server.base)).expect("a client");

        let error = dav.probe().await.expect_err("refused");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SyncAuthFailed);
    }

    #[tokio::test]
    async fn a_status_the_server_returns_travels_as_that_status() {
        let server = Fake::start(vec![("PROPFIND /dav/", Answer::status(500))]).await;
        let dav = WebDav::new(&service(&server.base)).expect("a client");

        let error = dav.probe().await.expect_err("the server is unwell");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SyncHttpFailed);
        assert_eq!(error.message_params().get("status").map(String::as_str), Some("500"));
    }

    #[tokio::test]
    async fn a_server_that_is_not_there_is_unreachable_rather_than_a_status() {
        // A port nothing is listening on: the address is well formed and the answer is that nobody
        // answered.
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
        let address = listener.local_addr().expect("an address");
        drop(listener);

        let dav = WebDav::new(&service(&format!("http://{address}"))).expect("a client");
        let error = dav.probe().await.expect_err("nobody there");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SyncUnreachable);
    }

    #[tokio::test]
    async fn a_redirect_to_another_host_takes_the_password_out_of_the_request() {
        // The rule that keeps `Warning: 301` from being a way to collect credentials: the second
        // server is a different machine, and it has no business seeing the first one's header.
        let elsewhere = Fake::start(vec![("PROPFIND /second/", Answer::status(207))]).await;
        let first = Fake::start(vec![(
            "PROPFIND /dav/",
            Answer {
                status: 302,
                body: String::new(),
                location: Some(format!("{}/second/", elsewhere.base)),
            },
        )])
        .await;

        let dav = WebDav::new(&service(&first.base)).expect("a client");
        dav.probe().await.expect("the redirect was followed");

        let second = elsewhere.requests();
        assert_eq!(second.len(), 1, "the redirect was followed");
        assert_eq!(second[0].path, "/second/");
        assert!(
            !second[0].headers.contains_key("authorization"),
            "and the credentials were left at the first one"
        );
    }
}
