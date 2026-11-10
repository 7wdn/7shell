//! Kubeconfig and the client built from it.
//!
//! One client per cluster connection, and everything a request needs comes from that one client:
//! one TLS story, one `Authorization` header, one base URL. A list, a watch and (in the other
//! module) an exec channel therefore cannot come to trust different things than each other.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use sevenshell_error::ShellError;
use sevenshell_types::{K8sResponse, K8sStreamEnd, K8sWatchEvent};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch as cancel_channel;

use crate::kubeconfig::{Cluster, User};
use crate::rest;
use crate::watch;

pub use crate::kubeconfig::{KubeContext, Kubeconfig};

/// How long a single API request may take.
///
/// Not zero: a hung API server would otherwise pin the caller for as long as the socket stays open,
/// and every list in the interface would wait behind it. Watches opt out of this per request — they
/// are supposed to stay open.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

/// How long the TCP (and TLS) connection to the API server may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// A connection to one cluster.
pub struct K8sClient {
    pub(crate) transport: K8sTransport,
    pub(crate) http: reqwest::Client,
}

/// Everything a request needs that is not the path or the body.
///
/// The REST client and the WebSocket upgrade share this: one TLS story, one authorization header
/// and one base URL, so an exec channel cannot accidentally trust a different server than a list.
#[allow(dead_code)]
#[derive(Clone)]
pub(crate) struct K8sTransport {
    /// The server URL exactly as the kubeconfig spells it, without a trailing slash.
    pub(crate) base: String,
    /// The host name the certificate is checked against and the `Host` header carries.
    pub(crate) server: String,
    pub(crate) tls: native_tls::TlsConnector,
    /// `Bearer …` when the kubeconfig uses a token, otherwise nothing (client certificates).
    pub(crate) authorization: Option<String>,
}

impl K8sClient {
    pub async fn connect(config: &Kubeconfig, context: Option<&str>) -> Result<Self, ShellError> {
        let selected = config.select(context)?;
        let server = rest::parse_server(&selected.cluster.server)?;
        let tls = connector(&selected.cluster, &selected.user)?;
        let http = http_client(&tls)?;

        Ok(Self {
            transport: K8sTransport {
                base: server.base,
                server: server.host,
                tls,
                authorization: selected.user.authorization(),
            },
            http,
        })
    }

    /// The parts a WebSocket upgrade needs, which are the same parts every request uses.
    #[allow(dead_code)]
    pub(crate) fn transport(&self) -> &K8sTransport {
        &self.transport
    }

    /// The address every path hangs off.
    pub(crate) fn base(&self) -> &str {
        &self.transport.base
    }

    /// A request with the headers that hold for every call to this cluster.
    ///
    /// Shared with the watch loop, so that a stream cannot be the one request that forgets the
    /// authorization header and reports it as an authentication failure.
    pub(crate) fn prepare(&self, method: &str, url: &str) -> Result<reqwest::RequestBuilder, ShellError> {
        let method = rest::method(method)?;
        let mut request = self
            .http
            .request(method, url)
            .header(reqwest::header::ACCEPT, "application/json");
        if let Some(authorization) = &self.transport.authorization {
            request = request.header(reqwest::header::AUTHORIZATION, authorization);
        }
        Ok(request)
    }

    /// One API request. A non-2xx answer is returned as a response, not an error: the caller decides
    /// what a 404 or a 409 means, and the status is part of the answer.
    pub(crate) async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        content_type: Option<&str>,
    ) -> Result<K8sResponse, ShellError> {
        let url = rest::url(&self.transport.base, path)?;
        let mut request = self.prepare(method, &url)?.timeout(REQUEST_TIMEOUT);
        if let Some(content_type) = content_type {
            request = request.header(reqwest::header::CONTENT_TYPE, content_type);
        }
        if let Some(body) = body {
            request = request.body(body.to_vec());
        }

        let response = request.send().await.map_err(|error| {
            // The token is in a header, never in the URL, so an error that quotes the address does
            // not quote the credential.
            ShellError::k8s_unavailable(format!("`{path}` could not be sent: {error}"))
        })?;
        let status = response.status().as_u16();
        let body = rest::read_capped(response, rest::MAX_RESPONSE_BYTES)
            .await
            .map_err(|error| match error {
                rest::BodyError::TooLarge => ShellError::k8s_request_failed(
                    status,
                    path,
                    format!(
                        "the response body exceeded {} MiB",
                        rest::MAX_RESPONSE_BYTES / (1024 * 1024)
                    ),
                ),
                rest::BodyError::Transport(detail) => {
                    ShellError::k8s_unavailable(format!("the response body could not be read: {detail}"))
                }
            })?;

        Ok(K8sResponse { status, body })
    }
}

/// The connector one cluster's certificates are checked with.
///
/// A kubeconfig that carries an authority is not saying "trust this as well" — it is saying "this is
/// the cluster". So the system store is switched off for that connection: a certificate the machine
/// happens to trust, signed by a public authority the cluster never used, is not that cluster, and
/// accepting it is how a proxy in the middle reads everything.
fn connector(cluster: &Cluster, user: &User) -> Result<native_tls::TlsConnector, ShellError> {
    let mut builder = native_tls::TlsConnector::builder();

    if cluster.insecure {
        // `insecure-skip-tls-verify` is the person saying they accept what the connection is; a CA
        // written beside it is then redundant rather than additive.
        builder.danger_accept_invalid_certs(true);
    } else if let Some(material) = &cluster.ca {
        let certificates = native_tls::Certificate::stack_from_pem(material).map_err(|error| {
            ShellError::k8s_kubeconfig_invalid(format!("the cluster certificate authority could not be read: {error}"))
        })?;
        if certificates.is_empty() {
            return Err(ShellError::k8s_kubeconfig_invalid(
                "the cluster certificate authority contains no certificate",
            ));
        }
        for certificate in certificates {
            builder.add_root_certificate(certificate);
        }
        builder.disable_built_in_roots(true);
    }

    if let Some(identity) = identity(user)? {
        builder.identity(identity);
    }

    builder
        .build()
        .map_err(|error| ShellError::k8s_unavailable(format!("the TLS connector could not be built: {error}")))
}

/// The client certificate pair, when the user has one.
fn identity(user: &User) -> Result<Option<native_tls::Identity>, ShellError> {
    let (Some(certificate), Some(key)) = (&user.cert, &user.key) else {
        // The kubeconfig reader refuses half a pair, so reaching this with one half means the two
        // halves were assembled somewhere else. Refusing here too keeps the rule in one place that
        // cannot be skipped.
        return match (&user.cert, &user.key) {
            (None, None) => Ok(None),
            _ => Err(ShellError::k8s_kubeconfig_invalid(
                "a client certificate is only usable together with its key",
            )),
        };
    };

    let key = normalized_key(key)?;

    match native_tls::Identity::from_pkcs8(certificate, &key) {
        Ok(identity) => Ok(Some(identity)),
        // Schannel's PKCS#8 path acquires an RSA provider, so an EC client certificate — what k3s
        // and many kubeadm clusters write — reaches this arm with an ASN.1 error. The PKCS#12 path
        // is the one Windows accepts for both key kinds.
        Err(primary) => pkcs12_identity(certificate, &key)
            .map(Some)
            .map_err(|_| pkcs8_error(primary)),
    }
}

fn pkcs8_error(error: native_tls::Error) -> ShellError {
    ShellError::k8s_kubeconfig_invalid(format!("the client certificate and key could not be read: {error}"))
}

const PFX_PASSPHRASE: &str = "7shell-kubeconfig";

/// The identity as a PKCS#12 archive, which is the shape Schannel accepts for EC client keys.
fn pkcs12_identity(certificate: &[u8], key: &[u8]) -> Result<native_tls::Identity, ShellError> {
    let certificates = pem_blocks(certificate, "CERTIFICATE")?;
    let key = pem_blocks(key, "PRIVATE KEY")?;
    let leaf = certificates
        .first()
        .ok_or_else(|| ShellError::k8s_kubeconfig_invalid("the client certificate is not a PEM document"))?;
    let authorities: Vec<&[u8]> = certificates.iter().skip(1).map(Vec::as_slice).collect();

    let pfx = p12::PFX::new_with_cas(leaf, &key[0], &authorities, PFX_PASSPHRASE, "7shell")
        .ok_or_else(|| ShellError::k8s_kubeconfig_invalid("the client identity could not be packed"))?;
    native_tls::Identity::from_pkcs12(&pfx.to_der(), PFX_PASSPHRASE)
        .map_err(|error| ShellError::k8s_kubeconfig_invalid(format!("the client identity could not be read: {error}")))
}

/// Every PEM block with this label, decoded to DER.
fn pem_blocks(document: &[u8], label: &str) -> Result<Vec<Vec<u8>>, ShellError> {
    use base64::Engine as _;

    let text = std::str::from_utf8(document)
        .map_err(|_| ShellError::k8s_kubeconfig_invalid("a certificate or key is not a PEM document"))?;
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let mut rest = text;
    let mut blocks = Vec::new();
    while let Some(start) = rest.find(&begin) {
        let after = &rest[start + begin.len()..];
        let Some(stop) = after.find(&end) else {
            break;
        };
        let body: String = after[..stop]
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        let der = base64::engine::general_purpose::STANDARD
            .decode(body)
            .map_err(|error| {
                ShellError::k8s_kubeconfig_invalid(format!("a PEM block could not be decoded: {error}"))
            })?;
        blocks.push(der);
        rest = &after[stop + end.len()..];
    }

    if blocks.is_empty() {
        return Err(ShellError::k8s_kubeconfig_invalid(format!(
            "the document has no {label} block"
        )));
    }
    Ok(blocks)
}

/// The client key in the one PEM shape `native-tls` reads.
///
/// Kubeconfigs are written by tools that chose different key encodings: kubectl and kubeadm often
/// write PKCS#1 RSA, k3s writes SEC1 EC, and newer tools write PKCS#8. Refusing two of the three
/// would refuse most real clusters, so the older two are re-encoded here — the alternative is
/// telling somebody to run `openssl` before their kubeconfig works.
fn normalized_key(key: &[u8]) -> Result<Vec<u8>, ShellError> {
    use rsa::pkcs1::DecodeRsaPrivateKey as _;
    use rsa::pkcs8::{EncodePrivateKey as _, LineEnding};

    let text = std::str::from_utf8(key)
        .map_err(|_| ShellError::k8s_kubeconfig_invalid("the client key is not a PEM document"))?;

    if text.contains("-----BEGIN PRIVATE KEY-----") {
        return Ok(key.to_vec());
    }

    let converted = if text.contains("-----BEGIN EC PRIVATE KEY-----") {
        p256::SecretKey::from_sec1_pem(text)
            .map_err(|error| {
                ShellError::k8s_kubeconfig_invalid(format!("the EC client key could not be read: {error}"))
            })?
            .to_pkcs8_pem(LineEnding::LF)
    } else if text.contains("-----BEGIN RSA PRIVATE KEY-----") {
        rsa::RsaPrivateKey::from_pkcs1_pem(text)
            .map_err(|error| {
                ShellError::k8s_kubeconfig_invalid(format!("the RSA client key could not be read: {error}"))
            })?
            .to_pkcs8_pem(LineEnding::LF)
    } else {
        return Err(ShellError::k8s_kubeconfig_invalid(
            "the client key is not PKCS#8, SEC1 EC or PKCS#1 RSA",
        ));
    };

    converted
        .map(|pem| pem.as_bytes().to_vec())
        .map_err(|error| ShellError::k8s_kubeconfig_invalid(format!("the client key could not be converted: {error}")))
}

fn http_client(tls: &native_tls::TlsConnector) -> Result<reqwest::Client, ShellError> {
    // No total timeout on the client: a watch is supposed to stay open, and a client-wide deadline
    // would cut it off after five minutes. Ordinary requests apply their own deadline instead.
    reqwest::Client::builder()
        .tls_backend_preconfigured(tls.clone())
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(concat!("7Shell/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| ShellError::k8s_unavailable(format!("the HTTP client could not be built: {error}")))
}

/// The cluster connections a window holds open, and the streams they are feeding.
pub struct K8sManager {
    connections: Mutex<HashMap<String, Arc<K8sClient>>>,
    streams: Arc<Streams>,
}

impl K8sManager {
    pub fn new() -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
            streams: Arc::new(Streams::default()),
        }
    }

    /// Opens one connection and answers the opaque id the interface keeps.
    pub async fn connect(&self, config: &Kubeconfig, context: Option<&str>) -> Result<String, ShellError> {
        // Built before the id exists, so a connection that cannot be built leaves nothing behind.
        let client = Arc::new(K8sClient::connect(config, context).await?);
        let id = uuid::Uuid::new_v4().to_string();
        self.connections().insert(id.clone(), client);
        Ok(id)
    }

    /// Drops a connection and everything it was streaming. Answers whether it was open.
    pub fn disconnect(&self, connection_id: &str) -> bool {
        let client = self.connections().remove(connection_id);
        // Streams are stopped even when the connection was already gone: the two maps are written
        // separately, and a watch whose connection was dropped is still a watch holding a socket.
        self.streams.stop_connection(connection_id);
        client.is_some()
    }

    pub async fn request(
        &self,
        connection_id: &str,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        content_type: Option<&str>,
    ) -> Result<K8sResponse, ShellError> {
        let client = self.client(connection_id)?;
        client.request(method, path, body, content_type).await
    }

    /// Starts following one list, answering the stream id that stops it.
    ///
    /// The stream ends on its own when the server closes it, when a relist is needed, or when
    /// somebody stops it — and either way the caller is told exactly once, through `end`.
    pub fn watch_start(
        &self,
        connection_id: &str,
        path: &str,
        events: UnboundedSender<K8sWatchEvent>,
        end: UnboundedSender<K8sStreamEnd>,
    ) -> Result<String, ShellError> {
        let client = self.client(connection_id)?;
        // Checked before anything is spawned, so a path the server would refuse is an error at the
        // call rather than an event nobody asked for.
        rest::check_path(path)?;

        let stream_id = uuid::Uuid::new_v4().to_string();
        let (stop, receiver) = cancel_channel::channel(false);
        self.streams.insert(&stream_id, connection_id, move || {
            // Ignored: a stop is idempotent, and the receiver may already be gone.
            let _ = stop.send(true);
        });

        let streams = Arc::clone(&self.streams);
        let task_id = stream_id.clone();
        let path = path.to_owned();
        tokio::spawn(async move {
            watch::run(client, &path, &task_id, events, end, receiver).await;
            streams.remove(&task_id);
        });

        Ok(stream_id)
    }

    /// Stops one stream by id. Answers whether there was one.
    pub fn stop_stream(&self, stream_id: &str) -> bool {
        self.streams.stop(stream_id)
    }

    /// Registers a stream a stop should end.
    ///
    /// A watch is one caller and a pod's log stream is the other: the interface has one stop button
    /// and one id shape, so the two are stopped the same way. Whoever registers is also responsible
    /// for calling [`K8sManager::forget_stream`] when the stream ends by itself, so that a later
    /// stop is not answered as if something were still running.
    pub fn register_stream(&self, stream_id: &str, connection_id: &str, stop: impl Fn() + Send + Sync + 'static) {
        self.streams.insert(stream_id, connection_id, stop);
    }

    /// Forgets a stream that ended on its own.
    pub fn forget_stream(&self, stream_id: &str) {
        self.streams.remove(stream_id);
    }

    /// The streams that are open right now, for the tests that cannot see the map.
    pub fn open_streams(&self) -> usize {
        self.streams.count()
    }

    /// Follows one container's log and hands every line to `events`.
    ///
    /// The stop is registered before the task starts, so an interface that stops the stream the
    /// moment it gets the id finds something to stop rather than a race.
    pub async fn logs_start(
        &self,
        connection_id: &str,
        request: &sevenshell_types::K8sLogRequest,
        events: tokio::sync::mpsc::UnboundedSender<sevenshell_types::K8sStreamLine>,
        end: tokio::sync::mpsc::UnboundedSender<sevenshell_types::K8sStreamEnd>,
    ) -> Result<String, ShellError> {
        let client = self.client(connection_id)?;
        let mut stream = crate::logs::open_logs(&client, request)?;
        let handle = stream.handle();

        let stream_id = uuid::Uuid::new_v4().to_string();
        self.register_stream(&stream_id, connection_id, move || handle.stop());

        let streams = Arc::clone(&self.streams);
        let task_id = stream_id.clone();
        tokio::spawn(async move {
            while let Some(line) = stream.next_line().await {
                let event = sevenshell_types::K8sStreamLine {
                    stream_id: task_id.clone(),
                    line,
                };
                if events.send(event).is_err() {
                    break;
                }
            }
            let error = stream.wait_ended().await.map(|error| error.to_string());
            streams.remove(&task_id);
            let _ = end.send(sevenshell_types::K8sStreamEnd {
                stream_id: task_id,
                error,
            });
        });

        Ok(stream_id)
    }

    /// Opens one pod exec channel. The caller owns the returned session from then on.
    pub async fn exec_open(
        &self,
        connection_id: &str,
        request: &sevenshell_types::K8sExecRequest,
        columns: u32,
        rows: u32,
    ) -> Result<crate::exec::ExecSession, ShellError> {
        let client = self.client(connection_id)?;
        crate::exec::open_exec(&client, request, columns, rows).await
    }

    fn client(&self, connection_id: &str) -> Result<Arc<K8sClient>, ShellError> {
        self.connections()
            .get(connection_id)
            .cloned()
            .ok_or_else(|| ShellError::k8s_not_connected(connection_id))
    }

    fn connections(&self) -> MutexGuard<'_, HashMap<String, Arc<K8sClient>>> {
        // A poisoned lock is recovered rather than reported, for the same reason the vault recovers:
        // the connections are still the best copy of what is open, and refusing every call would
        // turn one panic into a window that can no longer reach any cluster.
        match self.connections.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Default for K8sManager {
    fn default() -> Self {
        Self::new()
    }
}

/// The streams that are running, so one can be stopped and a disconnect can stop them all.
///
/// Type-erased on purpose: a watch and a log stream are stopped the same way and nothing else about
/// them is the map's business.
#[derive(Default)]
pub(crate) struct Streams {
    entries: Mutex<HashMap<String, StreamEntry>>,
}

struct StreamEntry {
    connection: String,
    stop: Box<dyn Fn() + Send + Sync>,
}

impl Streams {
    pub(crate) fn insert(&self, stream_id: &str, connection: &str, stop: impl Fn() + Send + Sync + 'static) {
        self.entries().insert(
            stream_id.to_owned(),
            StreamEntry {
                connection: connection.to_owned(),
                stop: Box::new(stop),
            },
        );
    }

    /// Forgets a stream that ended by itself. The next call after this cannot stop it again, which
    /// is why the caller's stop answers whether there was anything left to stop.
    pub(crate) fn remove(&self, stream_id: &str) {
        self.entries().remove(stream_id);
    }

    pub(crate) fn stop(&self, stream_id: &str) -> bool {
        match self.entries().remove(stream_id) {
            Some(entry) => {
                (entry.stop)();
                true
            }
            None => false,
        }
    }

    pub(crate) fn stop_connection(&self, connection: &str) -> usize {
        let mut entries = self.entries();
        let ids: Vec<String> = entries
            .iter()
            .filter(|(_, entry)| entry.connection == connection)
            .map(|(id, _)| id.clone())
            .collect();
        let mut stopped = 0;
        for id in ids {
            if let Some(entry) = entries.remove(&id) {
                (entry.stop)();
                stopped += 1;
            }
        }
        stopped
    }

    pub(crate) fn count(&self) -> usize {
        self.entries().len()
    }

    fn entries(&self) -> MutexGuard<'_, HashMap<String, StreamEntry>> {
        match self.entries.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

#[cfg(test)]
mod key_tests {
    use super::*;

    #[test]
    fn an_sec1_ec_client_key_is_reencoded_as_pkcs8() {
        let key = include_bytes!("../testdata/keys/sec1-ec.pem");
        let converted = normalized_key(key).expect("an SEC1 EC key is a shape we convert");
        let text = String::from_utf8(converted).expect("PEM is text");
        assert!(text.contains("-----BEGIN PRIVATE KEY-----"), "the result is PKCS#8");
        assert!(
            !text.contains("-----BEGIN EC PRIVATE KEY-----"),
            "the SEC1 marker is gone"
        );
    }

    #[test]
    fn a_pkcs1_rsa_client_key_is_reencoded_as_pkcs8() {
        let key = include_bytes!("../testdata/keys/pkcs1-rsa.pem");
        let converted = normalized_key(key).expect("a PKCS#1 RSA key is a shape we convert");
        let text = String::from_utf8(converted).expect("PEM is text");
        assert!(text.contains("-----BEGIN PRIVATE KEY-----"), "the result is PKCS#8");
        assert!(
            !text.contains("-----BEGIN RSA PRIVATE KEY-----"),
            "the PKCS#1 marker is gone"
        );
    }

    #[test]
    fn a_key_in_no_known_pem_shape_is_refused_by_name() {
        let error = normalized_key(b"not a key").expect_err("rubbish is not a key");
        assert_eq!(error.code(), sevenshell_error::ErrorCode::K8sKubeconfigInvalid);
    }
}
