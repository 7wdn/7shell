//! Talking to a WebDAV server: the verbs a backup needs, and the ones a file browser needs.
//!
//! A WebDAV server is, for this application, a place a file can be put and got. The backup client
//! stays deliberately narrow — `PROPFIND` to ask whether the address is one, `MKCOL` to make the
//! folders a path needs, and `PUT`/`GET` for the file itself. The file browser is the other half:
//! depth-one `PROPFIND` to list a folder, `DELETE`, `MOVE`, `COPY` and the same `PUT`/`GET` over
//! arbitrary paths. Both share one authenticated client, one redirect rule and one debug shape.
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
use std::time::{Duration, UNIX_EPOCH};

use bytes::Bytes;
use futures_util::stream;
use percent_encoding::percent_decode_str;
use quick_xml::Reader;
use quick_xml::events::Event;
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
    credentials: Credentials,
    remote_path: String,
}

/// The account a request authenticates as.
///
/// One implementation of "the password travels in one header and nowhere else", shared by the
/// backup client and the file browser: two copies of that rule would disagree eventually.
#[derive(Clone, Default)]
struct Credentials {
    username: Option<String>,
    password: Option<ServicePassword>,
}

impl Credentials {
    fn new(username: Option<String>, password: Option<ServicePassword>) -> Self {
        Self {
            username: username.filter(|name| !name.is_empty()),
            password: password.filter(|password| !password.is_empty()),
        }
    }

    fn apply(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.username {
            Some(username) => request.basic_auth(username, self.password.as_ref().map(ServicePassword::expose)),
            None => request,
        }
    }

    /// Sends one request, with the credentials it is entitled to.
    ///
    /// The error is built by the caller because the two clients name the same failure differently;
    /// what is shared is that no request ever leaves without the header, and no failure ever names
    /// the address or the password.
    async fn send(
        &self,
        request: reqwest::RequestBuilder,
        unreachable: fn() -> ShellError,
    ) -> Result<reqwest::Response, ShellError> {
        self.apply(request).send().await.map_err(|error| {
            if error.is_timeout() {
                unreachable().with_detail("the server did not answer in time")
            } else {
                unreachable().with_detail(format!("the request failed: {error}"))
            }
        })
    }
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
            .field("has_username", &self.credentials.username.is_some())
            .field("has_password", &self.credentials.password.is_some())
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

        let client = client_for(&url)?;

        Ok(Self {
            client,
            endpoint: url,
            credentials: Credentials::new(service.username.clone(), service.password.clone()),
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

    /// Where inside the endpoint the file goes, normalized.
    ///
    /// Safe to show: this is configuration rather than a secret — the address a person configured
    /// themselves, and the path they chose inside it.
    pub fn remote_path(&self) -> &str {
        &self.remote_path
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
    async fn send(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response, ShellError> {
        self.credentials.send(request, ShellError::sync_unreachable).await
    }

    /// The one place a status the server returned becomes a sentence.
    fn refused(&self, status: u16) -> ShellError {
        if status == 401 || status == 403 {
            return ShellError::sync_auth_failed().with_detail(format!("the server answered {status}"));
        }
        ShellError::sync_http_failed(status)
    }
}

/// One entry a WebDAV directory listing returned.
///
/// The names are not the SFTP names: WebDAV has no mode, owner or group, and a listing that
/// invented them would draw a permissions column full of zeroes. What the protocol does carry is
/// here, and the file browser maps it onto the columns it can fill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DavEntry {
    pub name: String,
    /// The path inside the configured endpoint, always beginning with `/`; `/` is the endpoint.
    pub path: String,
    pub is_dir: bool,
    /// Bytes for a file, `0` for a collection or when the server did not say.
    pub size: u64,
    /// The server's own spelling of the modification time, kept as it arrived.
    pub modified: Option<String>,
    /// The same time as seconds since the Unix epoch, when it was in the HTTP date format.
    ///
    /// Kept beside the raw string because a listing column sorts by a number and a person reads a
    /// date; a server that writes something else leaves this empty rather than inventing a time.
    pub modified_at: Option<u64>,
    pub content_type: Option<String>,
}

/// A WebDAV folder as a file browser needs it: one endpoint, and the verbs over what is under it.
///
/// The client above is about one file that travels between machines. This is about a tree somebody
/// browses: `PROPFIND` depth one for a directory, and the methods that change it. Both share the
/// same authenticated client, which is what keeps the password in one header and out of any error.
pub struct WebDavFiles {
    client: reqwest::Client,
    /// The client bodies travel through: no total-request timeout, because a large file is not a
    /// stalled one. Metadata keeps the short timeout it has in [`WebDav`].
    transfer: reqwest::Client,
    root: reqwest::Url,
    credentials: Credentials,
}

/// Hand-written for the same reason as [`WebDav`]'s: the address can be a share link, and a debug
/// line is a thing that ends up in a bug report.
impl std::fmt::Debug for WebDavFiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebDavFiles")
            .field("has_username", &self.credentials.username.is_some())
            .field("has_password", &self.credentials.password.is_some())
            .finish_non_exhaustive()
    }
}

/// What a directory listing asks a server for: the properties the panel can draw.
const PROPFIND_BODY: &str = concat!(
    r#"<?xml version="1.0" encoding="utf-8"?>"#,
    r#"<d:propfind xmlns:d="DAV:"><d:prop>"#,
    r#"<d:displayname/><d:resourcetype/><d:getcontentlength/>"#,
    r#"<d:getlastmodified/><d:getcontenttype/>"#,
    r#"</d:prop></d:propfind>"#
);

impl WebDavFiles {
    /// Points a file browser at a WebDAV folder.
    ///
    /// The address is the folder that becomes `/` for every path this client takes; a trailing
    /// slash is added because every join below assumes it.
    pub fn new(
        endpoint: &str,
        username: Option<String>,
        password: Option<ServicePassword>,
    ) -> Result<Self, ShellError> {
        let endpoint = endpoint.trim();
        if endpoint.is_empty() {
            return Err(ShellError::webdav_invalid_address("the address is empty"));
        }

        let mut root = reqwest::Url::parse(endpoint)
            .map_err(|error| ShellError::webdav_invalid_address(format!("the address: {error}")))?;
        if !matches!(root.scheme(), "http" | "https") {
            return Err(ShellError::webdav_invalid_address(format!(
                "the address starts with '{}'",
                root.scheme()
            )));
        }
        // The account belongs in the two fields the form has; a URL that carries one is a password
        // on its way into a setting, a log line or a bug report.
        if !root.username().is_empty() || root.password().is_some() {
            return Err(ShellError::webdav_invalid_address(
                "the address contains credentials; enter the account separately",
            ));
        }
        if !root.path().ends_with('/') {
            let path = format!("{}/", root.path());
            root.set_path(&path);
        }

        Ok(Self {
            client: client_for(&root)?,
            transfer: transfer_client_for(&root)?,
            root,
            credentials: Credentials::new(username, password),
        })
    }

    /// Reads one directory: the entries under `path`, not `path` itself.
    pub async fn list(&self, path: &str) -> Result<Vec<DavEntry>, ShellError> {
        let requested = normalize_dav_path(path);
        let url = self.url_for(&requested, true)?;
        let response = self
            .send(
                self.client
                    .request(dav_method("PROPFIND")?, url)
                    .header("depth", "1")
                    .header(CONTENT_TYPE, "application/xml; charset=utf-8")
                    .body(PROPFIND_BODY),
            )
            .await?;

        let status = response.status();
        // 207 is the only answer that carries a multistatus document. Accepting any success here
        // would turn a plain HTTP server's 200 page into an empty folder.
        if status != reqwest::StatusCode::MULTI_STATUS {
            return Err(self.refused(status.as_u16(), &requested));
        }

        let body = response
            .text()
            .await
            .map_err(|error| ShellError::webdav_malformed(format!("the listing could not be read: {error}")))?;
        self.parse_listing(&body, &requested)
    }

    /// Whether something exists at `path`.
    ///
    /// Asked by listing the parent rather than by a `PROPFIND` on the entry itself: a collection
    /// and a file want different trailing slashes, and a depth-one listing answers both without
    /// the browser having to guess which one it is about to ask about.
    pub async fn exists(&self, path: &str) -> Result<bool, ShellError> {
        Ok(self.stat(path).await?.is_some())
    }

    /// One entry, when it exists.
    pub async fn stat(&self, path: &str) -> Result<Option<DavEntry>, ShellError> {
        let normalized = normalize_dav_path(path);
        if normalized == "/" {
            return Ok(Some(DavEntry {
                name: "/".to_owned(),
                path: normalized,
                is_dir: true,
                size: 0,
                modified: None,
                modified_at: None,
                content_type: None,
            }));
        }

        let parent = normalized
            .rsplit_once('/')
            .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
            .unwrap_or("/");
        let entries = self.list(parent).await?;
        Ok(entries.into_iter().find(|entry| entry.path == normalized))
    }

    /// Makes one collection, and only one: the parent has to be there already.
    pub async fn create_dir(&self, path: &str) -> Result<(), ShellError> {
        let normalized = normalize_dav_path(path);
        if normalized == "/" {
            return Err(ShellError::webdav_exists(&normalized));
        }
        let url = self.url_for(&normalized, true)?;
        let response = self.send(self.client.request(dav_method("MKCOL")?, url)).await?;

        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        // 405 is the server saying the collection is already what was asked for.
        if status == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            return Err(ShellError::webdav_exists(&normalized));
        }
        Err(self.refused(status.as_u16(), &normalized))
    }

    /// Removes one entry. A collection goes with everything under it, which is the server's rule.
    pub async fn remove(&self, path: &str) -> Result<(), ShellError> {
        let normalized = normalize_dav_path(path);
        if normalized == "/" {
            return Err(ShellError::webdav_permission_denied(&normalized));
        }
        let url = self.url_for(&normalized, false)?;
        let response = self.send(self.client.delete(url)).await?;

        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        Err(self.refused(status.as_u16(), &normalized))
    }

    /// Moves one entry, and refuses rather than replacing what is already at the target.
    pub async fn rename(&self, from: &str, to: &str) -> Result<(), ShellError> {
        self.transfer_entry("MOVE", from, to).await
    }

    /// Copies one entry, and refuses rather than replacing what is already at the target.
    pub async fn copy(&self, from: &str, to: &str) -> Result<(), ShellError> {
        self.transfer_entry("COPY", from, to).await
    }

    async fn transfer_entry(&self, method: &str, from: &str, to: &str) -> Result<(), ShellError> {
        let source = normalize_dav_path(from);
        let target = normalize_dav_path(to);
        if source == "/" || target == "/" {
            return Err(ShellError::webdav_permission_denied(if source == "/" {
                &source
            } else {
                &target
            }));
        }
        if source == target {
            return Ok(());
        }

        let url = self.url_for(&source, false)?;
        let destination = self.url_for(&target, false)?;
        let mut request = self
            .client
            .request(dav_method(method)?, url)
            .header("destination", destination.as_str())
            // "F" is the protocol's way of saying "do not replace anything"; a target that is
            // taken comes back as 412 rather than as somebody else's file going away.
            .header("overwrite", "F");
        if method == "COPY" {
            request = request.header("depth", "infinity");
        }

        let response = self.send(request).await?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        Err(self.refused(status.as_u16(), &target))
    }

    /// Reads a file's bytes.
    pub async fn read(&self, path: &str) -> Result<Vec<u8>, ShellError> {
        let normalized = normalize_dav_path(path);
        let url = self.url_for(&normalized, false)?;
        let response = self.send(self.client.get(url)).await?;

        let status = response.status();
        if !status.is_success() {
            return Err(self.refused(status.as_u16(), &normalized));
        }
        response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(|error| ShellError::webdav_transfer_failed(&normalized, format!("the body: {error}")))
    }

    /// Writes a file, replacing what is there. The collection it lands in has to exist.
    pub async fn write(&self, path: &str, bytes: Vec<u8>) -> Result<(), ShellError> {
        let normalized = normalize_dav_path(path);
        if normalized == "/" {
            return Err(ShellError::webdav_invalid_address("a file needs a name"));
        }
        let url = self.url_for(&normalized, false)?;
        let response = self
            .send(
                self.client
                    .put(url)
                    .header(CONTENT_TYPE, "application/octet-stream")
                    .body(bytes),
            )
            .await?;

        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        Err(self.refused(status.as_u16(), &normalized))
    }

    /// Starts reading a file's body, asking for the part after `offset` when it is not zero.
    ///
    /// A server that ignores `Range` answers 200 with the whole file; [`DavDownload::start_offset`]
    /// says which of the two happened so the caller can truncate the part file instead of appending
    /// a second copy of the file to it.
    pub async fn open_download(&self, path: &str, offset: u64) -> Result<DavDownload, ShellError> {
        let normalized = normalize_dav_path(path);
        let url = self.url_for(&normalized, false)?;
        let mut request = self.transfer.get(url);
        if offset > 0 {
            request = request.header("range", format!("bytes={offset}-"));
        }

        let response = self.send(request).await?;
        let status = response.status();
        let start = if status == reqwest::StatusCode::PARTIAL_CONTENT {
            offset
        } else if status.is_success() {
            0
        } else {
            return Err(self.refused(status.as_u16(), &normalized));
        };

        Ok(DavDownload {
            response,
            path: normalized,
            start,
        })
    }

    /// Writes a file from a stream of chunks, without ever holding the whole file in memory.
    ///
    /// The receiver closing is the end of the body; a caller that stops sending early sends a short
    /// body, which is why the caller is the one that decides whether the file may be moved into
    /// place afterwards.
    pub async fn write_stream(
        &self,
        path: &str,
        chunks: tokio::sync::mpsc::Receiver<Vec<u8>>,
        content_length: Option<u64>,
    ) -> Result<(), ShellError> {
        let normalized = normalize_dav_path(path);
        if normalized == "/" {
            return Err(ShellError::webdav_invalid_address("a file needs a name"));
        }
        let url = self.url_for(&normalized, false)?;
        let body = reqwest::Body::wrap_stream(stream::unfold(chunks, |mut receiver| async move {
            receiver
                .recv()
                .await
                .map(|chunk| (Ok::<Bytes, std::io::Error>(Bytes::from(chunk)), receiver))
        }));

        // A known length is worth sending: some servers and every proxy between here and the server
        // treat a chunked PUT differently, and a body that says how large it is needs no chunk
        // framing at all.
        let mut request = self
            .transfer
            .put(url)
            .header(CONTENT_TYPE, "application/octet-stream")
            .body(body);
        if let Some(length) = content_length {
            request = request.header(reqwest::header::CONTENT_LENGTH, length);
        }

        let response = self.send(request).await?;

        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        Err(self.refused(status.as_u16(), &normalized))
    }

    /// Creates an empty file, and refuses to replace one that is already there.
    ///
    /// `If-None-Match: *` is the protocol's own "only if nothing is there", so the check and the
    /// write are one request rather than a question another writer can answer in between.
    pub async fn create_file(&self, path: &str) -> Result<(), ShellError> {
        let normalized = normalize_dav_path(path);
        if normalized == "/" {
            return Err(ShellError::webdav_invalid_address("a file needs a name"));
        }
        let url = self.url_for(&normalized, false)?;
        let response = self
            .send(
                self.client
                    .put(url)
                    .header(CONTENT_TYPE, "application/octet-stream")
                    .header("if-none-match", "*")
                    .body(Vec::new()),
            )
            .await?;

        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        if status == reqwest::StatusCode::PRECONDITION_FAILED {
            return Err(ShellError::webdav_exists(&normalized));
        }
        Err(self.refused(status.as_u16(), &normalized))
    }

    /// Turns a status the server returned into the sentence this build shows for it.
    fn refused(&self, status: u16, path: &str) -> ShellError {
        match status {
            401 => ShellError::webdav_auth_failed().with_detail(format!("the server answered {status}")),
            403 => ShellError::webdav_permission_denied(path),
            404 => ShellError::webdav_not_found(path),
            412 => ShellError::webdav_exists(path),
            other => ShellError::webdav_http_failed(other),
        }
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response, ShellError> {
        self.credentials.send(request, ShellError::webdav_unreachable).await
    }

    /// The address of one path inside the endpoint.
    ///
    /// A collection is addressed with its trailing slash: a server that redirects `/folder` to
    /// `/folder/` would otherwise answer a `PROPFIND` with a redirect, and the method is not one
    /// every server keeps across a redirect.
    fn url_for(&self, path: &str, directory: bool) -> Result<reqwest::Url, ShellError> {
        let normalized = normalize_dav_path(path);
        let mut url = self.root.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| ShellError::webdav_invalid_address("the address cannot carry a path"))?;
            segments.pop_if_empty();
            for part in normalized.split('/').filter(|part| !part.is_empty()) {
                segments.push(part);
            }
            if directory {
                segments.push("");
            }
        }
        Ok(url)
    }

    /// The path a `href` in a listing names, relative to the endpoint.
    ///
    /// `href` is percent-encoded and may be absolute or rooted. Anything outside the endpoint is
    /// refused rather than represented: a server that points a listing at somebody else's tree is
    /// not a listing this browser can safely follow.
    fn path_from_href(&self, href: &str) -> Option<String> {
        let url = reqwest::Url::parse(href).or_else(|_| self.root.join(href)).ok()?;
        let root = self.root.path().trim_end_matches('/');
        let path = url.path();
        if path != root && !path.starts_with(&format!("{root}/")) {
            return None;
        }

        let relative = &path[root.len()..];
        let parts: Vec<String> = relative
            .split('/')
            .filter(|part| !part.is_empty())
            .map(|part| percent_decode_str(part).decode_utf8_lossy().into_owned())
            .collect();
        Some(if parts.is_empty() {
            "/".to_owned()
        } else {
            format!("/{}", parts.join("/"))
        })
    }

    /// Reads a multistatus answer into the entries a panel can draw.
    ///
    /// Prefixes are the server's business — `<D:href>` and `<d:href>` are the same element — so
    /// elements are matched by local name. A `propstat` whose status is not 2xx is ignored: some
    /// servers answer a requested property with 404 in a block of its own, and that is not a
    /// listing failure.
    fn parse_listing(&self, xml: &str, requested: &str) -> Result<Vec<DavEntry>, ShellError> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut entries = Vec::new();
        let mut in_response = false;
        let mut href: Option<String> = None;
        let mut is_dir = false;
        let mut size = None;
        let mut modified = None;
        let mut content_type = None;
        let mut display_name = None;

        let mut in_propstat = false;
        let mut propstat_status: Option<u16> = None;
        let mut ps_is_dir = false;
        let mut ps_size = None;
        let mut ps_modified = None;
        let mut ps_content_type = None;
        let mut ps_display_name = None;

        let mut capture: Option<&'static str> = None;
        let mut text = String::new();

        loop {
            let event = reader.read_event().map_err(|error| {
                ShellError::webdav_malformed(format!("the listing is not well-formed XML: {error}"))
            })?;

            match event {
                Event::Start(element) => match element.local_name().as_ref() {
                    "response" => {
                        in_response = true;
                        href = None;
                        is_dir = false;
                        size = None;
                        modified = None;
                        content_type = None;
                        display_name = None;
                    }
                    "propstat" => {
                        in_propstat = true;
                        propstat_status = None;
                        ps_is_dir = false;
                        ps_size = None;
                        ps_modified = None;
                        ps_content_type = None;
                        ps_display_name = None;
                    }
                    "collection" => {
                        if in_propstat {
                            ps_is_dir = true;
                        } else {
                            is_dir = true;
                        }
                    }
                    "href" if in_response => {
                        capture = Some("href");
                        text.clear();
                    }
                    "displayname" if in_response => {
                        capture = Some("displayname");
                        text.clear();
                    }
                    "getcontentlength" if in_response => {
                        capture = Some("size");
                        text.clear();
                    }
                    "getlastmodified" if in_response => {
                        capture = Some("modified");
                        text.clear();
                    }
                    "getcontenttype" if in_response => {
                        capture = Some("contenttype");
                        text.clear();
                    }
                    "status" if in_propstat => {
                        capture = Some("status");
                        text.clear();
                    }
                    _ => {}
                },
                Event::Empty(element) => {
                    if element.local_name().as_ref() == "collection" {
                        if in_propstat {
                            ps_is_dir = true;
                        } else {
                            is_dir = true;
                        }
                    }
                }
                Event::End(element) => match element.local_name().as_ref() {
                    "href" => {
                        if capture == Some("href") {
                            href = Some(text.clone());
                            capture = None;
                        }
                    }
                    "displayname" => {
                        if capture == Some("displayname") {
                            ps_display_name = non_empty(&text);
                            capture = None;
                        }
                    }
                    "getcontentlength" => {
                        if capture == Some("size") {
                            ps_size = text.trim().parse().ok();
                            capture = None;
                        }
                    }
                    "getlastmodified" => {
                        if capture == Some("modified") {
                            ps_modified = non_empty(&text);
                            capture = None;
                        }
                    }
                    "getcontenttype" => {
                        if capture == Some("contenttype") {
                            ps_content_type = non_empty(&text);
                            capture = None;
                        }
                    }
                    "status" => {
                        if capture == Some("status") {
                            propstat_status = text.split_whitespace().nth(1).and_then(|code| code.parse().ok());
                            capture = None;
                        }
                    }
                    "propstat" => {
                        let accepted = propstat_status
                            .map(|status| (200..300).contains(&status))
                            .unwrap_or(true);
                        if accepted {
                            is_dir |= ps_is_dir;
                            size = size.or(ps_size);
                            modified = modified.or_else(|| ps_modified.clone());
                            content_type = content_type.or_else(|| ps_content_type.clone());
                            display_name = display_name.or_else(|| ps_display_name.clone());
                        }
                        in_propstat = false;
                    }
                    "response" => {
                        if let Some(raw) = href.take()
                            && let Some(path) = self.path_from_href(&raw)
                            && path != requested
                        {
                            let name = path
                                .rsplit('/')
                                .next()
                                .filter(|name| !name.is_empty())
                                .map(str::to_owned);
                            let name = name.or(display_name.take()).unwrap_or_else(|| path.clone());
                            let modified = modified.take();
                            let modified_at = modified
                                .as_deref()
                                .and_then(|raw| httpdate::parse_http_date(raw).ok())
                                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                                .map(|age| age.as_secs());
                            entries.push(DavEntry {
                                name,
                                path,
                                is_dir,
                                size: size.unwrap_or(0),
                                modified,
                                modified_at,
                                content_type: content_type.take(),
                            });
                        }
                        in_response = false;
                    }
                    _ => {}
                },
                Event::Text(value) => {
                    if capture.is_some() {
                        text.push_str(&value);
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }

        Ok(entries)
    }
}

/// A file body on its way to this machine, one chunk at a time.
pub struct DavDownload {
    response: reqwest::Response,
    path: String,
    start: u64,
}

impl DavDownload {
    /// Where the first chunk begins: the requested offset for a 206, or zero when the server sent
    /// the whole file because it ignored the range.
    pub fn start_offset(&self) -> u64 {
        self.start
    }

    /// The next chunk, or `None` at the end of the body.
    pub async fn next(&mut self) -> Result<Option<Vec<u8>>, ShellError> {
        // A body that stops sending is not a body that ended: without a wait here a dead
        // connection would leave the transfer bar sitting still with nothing to report.
        let next = tokio::time::timeout(Duration::from_secs(60), self.response.chunk()).await;
        match next {
            Ok(Ok(Some(bytes))) => Ok(Some(bytes.to_vec())),
            Ok(Ok(None)) => Ok(None),
            Ok(Err(error)) => Err(ShellError::webdav_transfer_failed(
                &self.path,
                format!("the body: {error}"),
            )),
            Err(_) => Err(ShellError::webdav_transfer_failed(
                &self.path,
                "the server stopped sending bytes",
            )),
        }
    }
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// A path inside a WebDAV endpoint, normalized.
///
/// `/` is the endpoint itself, `.` and empty segments disappear, and `..` pops. A `..` at the root
/// is dropped rather than allowed to climb above the configured folder — the endpoint is the
/// boundary this client was given, and a listing must not be a way to read outside it.
pub fn normalize_dav_path(path: &str) -> String {
    let separators = path.replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    for part in separators.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }

    if parts.is_empty() {
        "/".to_owned()
    } else {
        format!("/{}", parts.join("/"))
    }
}

/// The HTTP client every WebDAV request goes through.
fn client_for(url: &reqwest::Url) -> Result<reqwest::Client, ShellError> {
    let mut builder = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        // A redirect must not carry the address it came from.
        //
        // Two reasons, and neither is hypothetical. Several servers answer a read with a
        // redirect to their own object store, and a bucket sitting behind a referer allow-list
        // refuses a request that arrives with somebody else's address on it — a 403 on a file
        // the server has just handed over a signed link to. The other reason is the one that
        // makes it a rule rather than a workaround: that address is the private server somebody
        // is backing up to, and a third-party CDN has no business being told it.
        .referer(false)
        .user_agent(concat!("7Shell/", env!("CARGO_PKG_VERSION")));
    if is_private_host(url) {
        builder = builder.no_proxy();
    }

    builder
        .build()
        .map_err(|error| ShellError::app_internal(format!("an HTTP client could not be built: {error}")))
}

/// The same connection rules as [`client_for`], without a total-request deadline.
///
/// A metadata request that takes a minute is stuck. A file body that takes an hour is a file body;
/// the only deadlines that belong on it are a connect timeout and the per-chunk wait the reader
/// applies itself.
fn transfer_client_for(url: &reqwest::Url) -> Result<reqwest::Client, ShellError> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .referer(false)
        .user_agent(concat!("7Shell/", env!("CARGO_PKG_VERSION")));
    if is_private_host(url) {
        builder = builder.no_proxy();
    }

    builder
        .build()
        .map_err(|error| ShellError::app_internal(format!("an HTTP client could not be built: {error}")))
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
        assert!(
            !second[0].headers.contains_key("referer"),
            "nor was it told the address the request came from: a bucket behind a referer \
             allow-list refuses a request carrying somebody else's address, which is how a read \
             of a file the server just offered turns into a 403"
        );
    }

    /// A client pointed at a folder the fake server calls `/dav/`.
    fn files(base: &str) -> WebDavFiles {
        WebDavFiles::new(
            &format!("{base}/dav/"),
            Some("alice".to_owned()),
            Some(ServicePassword::new("hunter2".to_owned())),
        )
        .expect("a client")
    }

    fn multistatus(body: &str) -> Answer {
        Answer {
            status: 207,
            body: body.to_owned(),
            location: None,
        }
    }

    const LISTING: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/dav/photos/</D:href>
    <D:propstat>
      <D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/dav/photos/2024%20summer/</D:href>
    <D:propstat>
      <D:prop>
        <D:displayname>2024 summer</D:displayname>
        <D:resourcetype><D:collection/></D:resourcetype>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/dav/photos/%E4%B8%AD%E6%96%87.txt</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype/>
        <D:getcontentlength>42</D:getcontentlength>
        <D:getlastmodified>Mon, 09 Nov 2026 10:00:00 GMT</D:getlastmodified>
        <D:getcontenttype>text/plain</D:getcontenttype>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

    #[tokio::test]
    async fn a_listing_reads_collections_and_files_by_local_name() {
        let server = Fake::start(vec![("PROPFIND /dav/photos/", multistatus(LISTING))]).await;
        let dav = files(&server.base);

        let entries = dav.list("/photos").await.expect("listed");

        assert_eq!(entries.len(), 2, "the directory itself is not one of its entries");
        assert_eq!(entries[0].name, "2024 summer");
        assert_eq!(entries[0].path, "/photos/2024 summer");
        assert!(entries[0].is_dir);
        assert_eq!(entries[1].name, "中文.txt");
        assert_eq!(entries[1].path, "/photos/中文.txt");
        assert!(!entries[1].is_dir);
        assert_eq!(entries[1].size, 42);
        assert_eq!(entries[1].modified.as_deref(), Some("Mon, 09 Nov 2026 10:00:00 GMT"));
        assert_eq!(entries[1].content_type.as_deref(), Some("text/plain"));

        let request = &server.requests()[0];
        assert_eq!(request.method, "PROPFIND");
        assert_eq!(request.path, "/dav/photos/", "a collection is asked for with its slash");
        assert_eq!(request.headers.get("depth").map(String::as_str), Some("1"));
        assert!(
            request.body.contains("getcontentlength"),
            "the panel's columns are requested"
        );
        assert_eq!(
            request.headers.get("authorization").map(String::as_str),
            Some("Basic YWxpY2U6aHVudGVyMg=="),
            "the credentials travel in the one header"
        );
        assert!(!request.path.contains("hunter2"), "and not in the address");
    }

    #[tokio::test]
    async fn a_property_the_server_says_it_does_not_have_is_not_used() {
        // Some servers answer one requested property with a 404 block of its own and the rest in a
        // 200 block. Taking the first value seen would make a 999-byte file out of a 7-byte one.
        let answer = r#"<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/dav/odd.txt</D:href>
    <D:propstat>
      <D:prop><D:getcontentlength>999</D:getcontentlength></D:prop>
      <D:status>HTTP/1.1 404 Not Found</D:status>
    </D:propstat>
    <D:propstat>
      <D:prop><D:getcontentlength>7</D:getcontentlength></D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#;
        let server = Fake::start(vec![("PROPFIND /dav/", multistatus(answer))]).await;

        let entries = files(&server.base).list("/").await.expect("listed");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].size, 7, "the 200 block is the answer; the 404 block is not");
    }

    #[tokio::test]
    async fn a_download_can_start_after_a_prefix() {
        let server = Fake::start(vec![(
            "GET /dav/big.bin",
            Answer {
                status: 206,
                body: "tail".to_owned(),
                location: None,
            },
        )])
        .await;
        let dav = files(&server.base);

        let mut download = dav.open_download("/big.bin", 4).await.expect("opened");

        assert_eq!(download.start_offset(), 4, "the server honored the range");
        assert_eq!(download.next().await.expect("a chunk").as_deref(), Some(&b"tail"[..]));
        assert!(download.next().await.expect("the end").is_none());
        assert_eq!(
            server.requests()[0].headers.get("range").map(String::as_str),
            Some("bytes=4-")
        );
    }

    #[tokio::test]
    async fn a_server_that_ignores_the_range_sends_the_whole_file() {
        let server = Fake::start(vec![("GET /dav/big.bin", Answer::ok("whole"))]).await;
        let dav = files(&server.base);

        let download = dav.open_download("/big.bin", 4).await.expect("opened");

        assert_eq!(
            download.start_offset(),
            0,
            "a 200 is the whole file, so the caller has to start its part file again"
        );
    }

    #[tokio::test]
    async fn an_upload_body_is_the_chunks_the_caller_sends() {
        let server = Fake::start(vec![("PUT /dav/up.txt", Answer::status(201))]).await;
        let dav = files(&server.base);
        let (sender, receiver) = tokio::sync::mpsc::channel::<Vec<u8>>(2);

        let upload = tokio::spawn(async move { dav.write_stream("/up.txt", receiver, Some(11)).await });
        sender.send(b"hello".to_vec()).await.expect("sent");
        sender.send(b" world".to_vec()).await.expect("sent");
        drop(sender);
        upload.await.expect("joined").expect("written");

        let request = &server.requests()[0];
        assert_eq!(request.method, "PUT");
        assert_eq!(request.body, "hello world");
        assert_eq!(request.headers.get("content-length").map(String::as_str), Some("11"));
        assert_eq!(
            request.headers.get("content-type").map(String::as_str),
            Some("application/octet-stream")
        );
    }

    #[tokio::test]
    async fn a_listing_with_nothing_under_it_is_empty_rather_than_an_error() {
        let server = Fake::start(vec![(
            "PROPFIND /dav/",
            multistatus(r#"<D:multistatus xmlns:D="DAV:"/>"#),
        )])
        .await;

        assert_eq!(files(&server.base).list("/").await.expect("empty"), Vec::new());
    }

    #[tokio::test]
    async fn an_entry_outside_the_endpoint_is_not_part_of_its_listing() {
        let answer = r#"<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/elsewhere/secret.txt</D:href>
    <D:propstat><D:prop/><D:status>HTTP/1.1 200 OK</D:status></D:propstat>
  </D:response>
</D:multistatus>"#;
        let server = Fake::start(vec![("PROPFIND /dav/", multistatus(answer))]).await;

        assert_eq!(
            files(&server.base).list("/").await.expect("listed"),
            Vec::new(),
            "a listing must not become a way to read outside the address it was given"
        );
    }

    #[tokio::test]
    async fn names_with_spaces_and_non_ascii_are_encoded_in_the_address() {
        let server = Fake::start(Vec::new()).await;
        let dav = files(&server.base);

        let error = dav
            .write("/photos/2024 夏天#1.txt", b"x".to_vec())
            .await
            .expect_err("404");
        assert_eq!(error.code(), sevenshell_error::ErrorCode::WebDavNotFound);

        let request = &server.requests()[0];
        assert_eq!(request.method, "PUT");
        assert_eq!(request.path, "/dav/photos/2024%20%E5%A4%8F%E5%A4%A9%231.txt");
    }

    #[test]
    fn a_path_is_normalized_inside_the_endpoint_it_belongs_to() {
        assert_eq!(normalize_dav_path(""), "/");
        assert_eq!(normalize_dav_path("/"), "/");
        assert_eq!(normalize_dav_path("/a//b/./c"), "/a/b/c");
        assert_eq!(normalize_dav_path("a\\b"), "/a/b");
        assert_eq!(
            normalize_dav_path("/../../escape"),
            "/escape",
            "the endpoint is the boundary; .. is dropped rather than climbing above it"
        );
    }

    #[tokio::test]
    async fn creating_a_folder_is_one_mkcol() {
        let server = Fake::start(vec![("MKCOL /dav/new/", Answer::status(201))]).await;

        files(&server.base).create_dir("/new").await.expect("created");

        let request = &server.requests()[0];
        assert_eq!(request.method, "MKCOL");
        assert_eq!(request.path, "/dav/new/");
    }

    #[tokio::test]
    async fn a_folder_that_is_already_there_is_a_taken_name() {
        let server = Fake::start(vec![("MKCOL /dav/taken/", Answer::status(405))]).await;

        let error = files(&server.base)
            .create_dir("/taken")
            .await
            .expect_err("already there");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::WebDavExists);
    }

    #[tokio::test]
    async fn deleting_sends_delete_to_the_entry() {
        let server = Fake::start(vec![("DELETE /dav/old.txt", Answer::status(204))]).await;

        files(&server.base).remove("/old.txt").await.expect("removed");

        assert_eq!(server.requests()[0].method, "DELETE");
        assert_eq!(server.requests()[0].path, "/dav/old.txt");
    }

    #[tokio::test]
    async fn renaming_sends_move_with_an_absolute_destination_and_no_overwrite() {
        let server = Fake::start(vec![("MOVE /dav/old.txt", Answer::status(201))]).await;

        files(&server.base)
            .rename("/old.txt", "/new name.txt")
            .await
            .expect("moved");

        let request = &server.requests()[0];
        let destination = format!("{}/dav/new%20name.txt", server.base);
        assert_eq!(request.method, "MOVE");
        assert_eq!(
            request.headers.get("destination").map(String::as_str),
            Some(destination.as_str()),
            "the protocol wants a full address, with the name encoded"
        );
        assert_eq!(request.headers.get("overwrite").map(String::as_str), Some("F"));
        assert!(
            !destination.contains("hunter2") && !destination.contains("alice"),
            "the destination names the file, not the account"
        );
    }

    #[tokio::test]
    async fn copying_sends_copy_with_an_infinite_depth() {
        let server = Fake::start(vec![("COPY /dav/tree", Answer::status(201))]).await;

        files(&server.base).copy("/tree", "/copy").await.expect("copied");

        let request = &server.requests()[0];
        assert_eq!(request.method, "COPY");
        assert_eq!(request.headers.get("depth").map(String::as_str), Some("infinity"));
        assert_eq!(request.headers.get("overwrite").map(String::as_str), Some("F"));
    }

    #[tokio::test]
    async fn a_move_onto_a_taken_name_is_a_taken_name() {
        let server = Fake::start(vec![("MOVE /dav/a", Answer::status(412))]).await;

        let error = files(&server.base).rename("/a", "/b").await.expect_err("taken");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::WebDavExists);
        assert_eq!(error.message_params().get("path").map(String::as_str), Some("/b"));
    }

    #[tokio::test]
    async fn a_file_comes_back_as_the_bytes_it_went_as() {
        let server = Fake::start(vec![("GET /dav/notes.txt", Answer::ok("hello"))]).await;

        assert_eq!(
            files(&server.base).read("/notes.txt").await.expect("read"),
            b"hello".to_vec()
        );
    }

    #[tokio::test]
    async fn writing_puts_the_bytes_and_says_what_they_are() {
        let server = Fake::start(vec![("PUT /dav/notes.txt", Answer::status(201))]).await;

        files(&server.base)
            .write("/notes.txt", b"hello".to_vec())
            .await
            .expect("written");

        let request = &server.requests()[0];
        assert_eq!(request.method, "PUT");
        assert_eq!(request.path, "/dav/notes.txt");
        assert_eq!(request.body, "hello");
        assert_eq!(
            request.headers.get("content-type").map(String::as_str),
            Some("application/octet-stream")
        );
    }

    #[tokio::test]
    async fn a_new_file_is_created_only_when_nothing_is_there() {
        let server = Fake::start(vec![("PUT /dav/new.txt", Answer::status(201))]).await;

        files(&server.base).create_file("/new.txt").await.expect("created");

        assert_eq!(
            server.requests()[0].headers.get("if-none-match").map(String::as_str),
            Some("*"),
            "the check and the write are one request"
        );
    }

    #[tokio::test]
    async fn a_new_file_that_is_already_there_is_a_taken_name() {
        let server = Fake::start(vec![("PUT /dav/taken.txt", Answer::status(412))]).await;

        let error = files(&server.base).create_file("/taken.txt").await.expect_err("taken");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::WebDavExists);
    }

    #[tokio::test]
    async fn existence_is_asked_by_listing_the_parent() {
        // A collection and a file want different trailing slashes, so the question is asked of the
        // parent instead of guessing which kind the path is about to turn out to be.
        let answer = r#"<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/dav/there</D:href>
    <D:propstat><D:prop><D:resourcetype/></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat>
  </D:response>
</D:multistatus>"#;
        let server = Fake::start(vec![("PROPFIND /dav/", multistatus(answer))]).await;
        let dav = files(&server.base);

        assert!(dav.exists("/there").await.expect("asked"));
        assert!(!dav.exists("/missing").await.expect("asked"));
        assert!(dav.exists("/").await.expect("the endpoint itself is there"));

        let request = &server.requests()[0];
        assert_eq!(request.path, "/dav/");
        assert_eq!(request.headers.get("depth").map(String::as_str), Some("1"));
    }

    #[tokio::test]
    async fn a_status_the_server_returns_is_named_rather_than_guessed() {
        let unauthorized = Fake::start(vec![("GET /dav/x", Answer::status(401))]).await;
        assert_eq!(
            files(&unauthorized.base).read("/x").await.expect_err("401").code(),
            sevenshell_error::ErrorCode::WebDavAuthFailed
        );

        let forbidden = Fake::start(vec![("GET /dav/x", Answer::status(403))]).await;
        let error = files(&forbidden.base).read("/x").await.expect_err("403");
        assert_eq!(error.code(), sevenshell_error::ErrorCode::WebDavPermissionDenied);
        assert_eq!(error.message_params().get("path").map(String::as_str), Some("/x"));

        let unwell = Fake::start(vec![("GET /dav/x", Answer::status(500))]).await;
        let error = files(&unwell.base).read("/x").await.expect_err("500");
        assert_eq!(error.code(), sevenshell_error::ErrorCode::WebDavHttpFailed);
        assert_eq!(error.message_params().get("status").map(String::as_str), Some("500"));
    }

    #[tokio::test]
    async fn a_plain_http_answer_is_not_an_empty_folder() {
        // A 200 page is what an ordinary web server answers; only 207 carries a multistatus
        // document, and the difference has to be an error rather than a folder that looks empty.
        let server = Fake::start(vec![("PROPFIND /dav/", Answer::ok("<html>not a DAV server</html>"))]).await;

        let error = files(&server.base).list("/").await.expect_err("not WebDAV");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::WebDavHttpFailed);
        assert_eq!(error.message_params().get("status").map(String::as_str), Some("200"));
    }

    #[tokio::test]
    async fn a_listing_that_is_not_xml_is_malformed_rather_than_empty() {
        let server = Fake::start(vec![("PROPFIND /dav/", multistatus("<D:multistatus"))]).await;

        let error = files(&server.base).list("/").await.expect_err("not XML");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::WebDavMalformed);
    }

    #[test]
    fn an_address_that_is_not_http_is_refused_before_anything_is_sent() {
        for endpoint in ["ftp://example.com/", "not an address"] {
            let error = WebDavFiles::new(endpoint, None, None).expect_err("not a WebDAV address");
            assert_eq!(error.code(), sevenshell_error::ErrorCode::WebDavInvalidAddress);
        }
    }

    #[test]
    fn an_address_that_carries_credentials_is_refused() {
        let error = WebDavFiles::new("https://alice:hunter2@example.com/dav/", None, None)
            .expect_err("credentials belong in the form");
        assert_eq!(error.code(), sevenshell_error::ErrorCode::WebDavInvalidAddress);
        assert!(!error.to_string().contains("hunter2"), "and the password is not echoed");
    }
}
