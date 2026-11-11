//! Kubeconfig and the client built from it.
//!
//! One client per cluster connection, and everything a request needs comes from that one client:
//! one TLS story, one `Authorization` header, one base URL. A list, a watch and (in the other
//! module) an exec channel therefore cannot come to trust different things than each other.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use sevenshell_error::ShellError;
use sevenshell_types::{K8sResponse, K8sStreamEnd, K8sWatchEvent};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch as cancel_channel;
use tokio::sync::watch as stop_channel;
use tokio::task::JoinHandle;

use crate::kubeconfig::{Cluster, Selected, User};
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
    /// The loopback end of a tunnel, when this connection is carried by one.
    ///
    /// Part of the transport rather than of the HTTP client because every shape of traffic has to
    /// take it: a list, a watch, a followed log and a pod terminal all reach the same cluster, and a
    /// stream that quietly went around the tunnel would be a different promise than the one the
    /// connection made.
    pub(crate) bridge: Option<Bridge>,
}

/// The loopback listener a tunnel answers on, and the one server it will carry.
///
/// The pair travels together on purpose: a client that knows only the endpoint would have to guess
/// what to ask it for, and the bridge refuses everything but this server anyway.
#[derive(Clone)]
pub(crate) struct Bridge {
    pub(crate) endpoint: SocketAddr,
    pub(crate) target: ServerAddress,
}

impl K8sClient {
    pub async fn connect(config: &Kubeconfig, context: Option<&str>) -> Result<Self, ShellError> {
        let selected = config.select(context)?;
        let transport = transport_of(&selected)?;
        let http = http_client(&transport.tls, None)?;

        Ok(Self { transport, http })
    }

    /// Opens the listener and builds the client that reaches the server through `dialer`.
    ///
    /// The URL keeps the host the kubeconfig named, so the certificate is checked against the
    /// cluster and not against the listener; every connection underneath it goes to the listener,
    /// which carries it through the dialer.
    async fn open_via(
        prepared: TunneledCluster,
        dialer: Arc<dyn ApiDialer>,
    ) -> Result<(Self, TunnelEndpoint), ShellError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.map_err(|error| {
            ShellError::k8s_unavailable(format!("the local end of the tunnel could not listen: {error}"))
        })?;
        let address = listener.local_addr().map_err(|error| {
            ShellError::k8s_unavailable(format!("the local end of the tunnel has no address: {error}"))
        })?;

        let mut transport = prepared.transport;
        transport.bridge = Some(Bridge {
            endpoint: address,
            target: prepared.target.clone(),
        });
        let http = http_client(&transport.tls, Some(address))?;
        let tunnel = TunnelEndpoint::start(listener, prepared.target, dialer);
        Ok((Self { transport, http }, tunnel))
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

/// One connection to the API server, as whatever carries it produces it.
///
/// The trait exists so the client can be reached through something this crate does not depend on:
/// an SSH channel, in practice. Both halves are required because an HTTP connection is a
/// conversation, not a single request.
pub trait ApiStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> ApiStream for T {}

/// The stream a dialer answers with.
pub type BoxedApiStream = Box<dyn ApiStream>;

/// The future one dial answers with.
pub type DialFuture = Pin<Box<dyn std::future::Future<Output = Result<BoxedApiStream, ShellError>> + Send>>;

/// Opens connections to the API server from wherever the caller's network is.
///
/// Called once per connection the local listener carries, so an implementation that reuses one
/// session — an SSH connection, in practice — has to be cheap to call repeatedly and safe from
/// several tasks at once.
pub trait ApiDialer: Send + Sync + 'static {
    /// Opens one byte stream to `host:port` as that network sees it.
    fn dial(&self, host: String, port: u16) -> DialFuture;
}

/// Where the API server listens, as the kubeconfig spells it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerAddress {
    /// Without brackets: this string is also what a direct-tcpip channel names.
    host: String,
    port: u16,
}

impl ServerAddress {
    /// `host:port` as a CONNECT authority spells it.
    ///
    /// Brackets appear here, not in the dialer: a CONNECT authority for an IPv6 address has to be
    /// `[::1]:6443`, while the channel that carries it is named `::1`. The bridge compares the two
    /// spellings as addresses, so this is the shape that reaches it.
    pub(crate) fn authority(&self) -> String {
        match self.host.parse::<std::net::Ipv6Addr>() {
            Ok(_) => format!("[{}]:{}", self.host, self.port),
            Err(_) => format!("{}:{}", self.host, self.port),
        }
    }
}

/// Reads the host and port a tunnel has to reach.
///
/// The address is validated by the same function a direct connection uses, so a tunnel refuses
/// exactly what a direct connection refuses; only the default port, which the direct path leaves
/// inside the URL, is made explicit here.
fn server_address(server: &str) -> Result<ServerAddress, ShellError> {
    let parsed = rest::parse_server(server)?;
    let url = reqwest::Url::parse(&parsed.base)
        .map_err(|error| ShellError::k8s_kubeconfig_invalid(format!("the cluster address is not a URL: {error}")))?;
    let port = url.port_or_known_default().ok_or_else(|| {
        ShellError::k8s_kubeconfig_invalid("the cluster address has no port and its scheme has no default")
    })?;
    Ok(ServerAddress {
        host: parsed.host,
        port,
    })
}

/// A cluster prepared to be reached through a dialer.
///
/// Everything the kubeconfig alone can refuse is refused before this value exists, so a cluster
/// that cannot be carried through a tunnel never costs a listener, an SSH connection or a
/// host-key question.
pub struct TunneledCluster {
    transport: K8sTransport,
    target: ServerAddress,
}

impl TunneledCluster {
    /// Reads and checks what a tunnelled connection needs, without opening anything.
    pub fn prepare(config: &Kubeconfig, context: Option<&str>) -> Result<Self, ShellError> {
        let selected = config.select(context)?;
        let transport = transport_of(&selected)?;

        // The local end of a tunnel answers CONNECT and nothing else. A plain-HTTP server would
        // have its requests sent to that listener as absolute URLs, which is a forward proxy this
        // side does not implement; refusing the address is clearer than a tunnel that refuses
        // every request it is sent.
        if !transport.base.starts_with("https://") {
            return Err(ShellError::app_unsupported(
                "a cluster reached through an SSH session has to be an https address",
            ));
        }

        let target = server_address(&selected.cluster.server)?;
        Ok(Self { transport, target })
    }
}

/// Everything a connection needs that the kubeconfig names.
fn transport_of(selected: &Selected) -> Result<K8sTransport, ShellError> {
    let server = rest::parse_server(&selected.cluster.server)?;
    let tls = connector(&selected.cluster, &selected.user)?;
    Ok(K8sTransport {
        base: server.base,
        server: server.host,
        tls,
        authorization: selected.user.authorization(),
        bridge: None,
    })
}

/// How long the local end of a tunnel has to say what it wants carried.
const BRIDGE_HEAD_TIMEOUT: Duration = Duration::from_secs(15);

/// The largest head the bridge will read. A CONNECT request is a few lines; anything bigger is
/// not one, and buffering it would be buffering whatever found the port.
const MAX_BRIDGE_HEAD: usize = 8 * 1024;

/// What the local end is told when its request is carried.
const BRIDGE_ESTABLISHED: &[u8] = b"HTTP/1.1 200 Connection Established\r\n\r\n";

/// What the local end is told when it asked for a server this bridge does not carry.
const BRIDGE_REFUSED: &[u8] = b"HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";

/// What the local end is told when the far side of the tunnel could not be reached.
const BRIDGE_UNREACHABLE: &[u8] = b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";

/// The local end of a tunnel: a loopback listener the HTTP client is pointed at, and the task that
/// carries each accepted connection through the dialer.
///
/// Dropping this stops the listener and signals every connection it is carrying. The client holds
/// it for exactly as long as the connection, so a disconnect is a drop.
struct TunnelEndpoint {
    stop: stop_channel::Sender<bool>,
    task: JoinHandle<()>,
}

impl TunnelEndpoint {
    fn start(listener: TcpListener, target: ServerAddress, dialer: Arc<dyn ApiDialer>) -> Self {
        let (stop, stopped) = stop_channel::channel(false);
        let task = tokio::spawn(accept_connections(listener, target, dialer, stopped));
        Self { stop, task }
    }
}

impl Drop for TunnelEndpoint {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        // Aborted as well as signalled: while the task is waiting for a connection, the signal
        // would not be looked at until the next one arrived.
        self.task.abort();
    }
}

/// Accepts local connections and carries each one through the dialer.
async fn accept_connections(
    listener: TcpListener,
    target: ServerAddress,
    dialer: Arc<dyn ApiDialer>,
    mut stop: stop_channel::Receiver<bool>,
) {
    loop {
        let accepted = tokio::select! {
            _ = stop.changed() => break,
            accepted = listener.accept() => accepted,
        };

        let Ok((client, _peer)) = accepted else {
            // The listener itself is gone; nothing is left to wait for.
            break;
        };

        let target = target.clone();
        let dialer = Arc::clone(&dialer);
        let mut stopping = stop.clone();
        tokio::spawn(async move {
            let _ = carry(client, target, dialer, &mut stopping).await;
        });
    }
}

/// One connection as the local end phrased it: the head, and whatever followed it.
struct BridgeHead {
    text: String,
    leftover: Vec<u8>,
}

/// Carries one local connection through the dialer, after checking it asked for this server.
async fn carry(
    mut client: TcpStream,
    target: ServerAddress,
    dialer: Arc<dyn ApiDialer>,
    stop: &mut stop_channel::Receiver<bool>,
) -> Result<(), ShellError> {
    let head = tokio::select! {
        _ = stop.changed() => return Ok(()),
        head = tokio::time::timeout(BRIDGE_HEAD_TIMEOUT, read_head(&mut client)) => match head {
            Ok(head) => head?,
            // A connection that says nothing is not an error worth reporting: it is somebody's
            // probe, and there is nothing to answer.
            Err(_) => return Ok(()),
        },
    };

    if !asks_for_target(&head.text, &target) {
        // A loopback port is reachable by every process on this machine; carrying whatever it asks
        // for would turn the saved SSH session into a proxy for anyone who can connect, so only
        // the server this tunnel was opened for is ever carried.
        let _ = client.write_all(BRIDGE_REFUSED).await;
        return Ok(());
    }

    let mut tunnel = tokio::select! {
        _ = stop.changed() => return Ok(()),
        dialed = dialer.dial(target.host.clone(), target.port) => match dialed {
            Ok(tunnel) => tunnel,
            Err(error) => {
                let _ = client.write_all(BRIDGE_UNREACHABLE).await;
                return Err(error);
            }
        },
    };

    client.write_all(BRIDGE_ESTABLISHED).await.map_err(|error| {
        ShellError::k8s_unavailable(format!("the local end of the tunnel could not be answered: {error}"))
    })?;
    if !head.leftover.is_empty() {
        tunnel
            .write_all(&head.leftover)
            .await
            .map_err(|error| ShellError::k8s_unavailable(format!("the tunnel could not be written to: {error}")))?;
    }

    tokio::select! {
        _ = stop.changed() => {}
        copied = tokio::io::copy_bidirectional(&mut client, &mut tunnel) => {
            copied.map_err(|error| ShellError::k8s_unavailable(format!("the tunnel ended: {error}")))?;
        }
    }
    Ok(())
}

/// Reads one request head, keeping the bytes that followed it.
async fn read_head(stream: &mut TcpStream) -> Result<BridgeHead, ShellError> {
    let mut buffer = Vec::with_capacity(512);
    let mut chunk = [0u8; 512];
    loop {
        let read = stream.read(&mut chunk).await.map_err(|error| {
            ShellError::k8s_unavailable(format!("the local end of the tunnel could not be read: {error}"))
        })?;
        if read == 0 {
            return Err(ShellError::k8s_unavailable(
                "the local end of the tunnel ended before it said what it wanted",
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);

        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            return Ok(BridgeHead {
                text: String::from_utf8_lossy(&buffer[..end]).into_owned(),
                leftover: buffer[end + 4..].to_vec(),
            });
        }
        if buffer.len() > MAX_BRIDGE_HEAD {
            return Err(ShellError::k8s_unavailable(
                "the local end of the tunnel sent a head larger than a tunnel request",
            ));
        }
    }
}

/// Whether a request head asks for the server this bridge is pinned to.
fn asks_for_target(head: &str, target: &ServerAddress) -> bool {
    let Some(line) = head.lines().next() else {
        return false;
    };
    let mut parts = line.split_whitespace();
    if parts.next() != Some("CONNECT") {
        return false;
    }
    let Some(authority) = parts.next() else {
        return false;
    };
    authority_is(authority, target)
}

/// Whether `host:port` is the pinned server.
///
/// The spelling differs by where the address came from: a URL lower-cases a domain and compresses
/// an IPv6 address, while a kubeconfig may spell either another way, and both name the same
/// server.
fn authority_is(authority: &str, target: &ServerAddress) -> bool {
    let Some((host, port)) = split_host_port(authority) else {
        return false;
    };
    port == target.port && same_host(&host, &target.host)
}

/// Splits `host:port` and `[v6]:port`, the two shapes a CONNECT authority takes.
fn split_host_port(authority: &str) -> Option<(String, u16)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        let port = after.strip_prefix(':')?.parse().ok()?;
        return Some((host.to_owned(), port));
    }
    let (host, port) = authority.rsplit_once(':')?;
    Some((host.to_owned(), port.parse().ok()?))
}

fn same_host(left: &str, right: &str) -> bool {
    if let (Ok(left), Ok(right)) = (left.parse::<std::net::IpAddr>(), right.parse::<std::net::IpAddr>()) {
        return left == right;
    }
    left.eq_ignore_ascii_case(right)
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

fn http_client(tls: &native_tls::TlsConnector, endpoint: Option<SocketAddr>) -> Result<reqwest::Client, ShellError> {
    // No total timeout on the client: a watch is supposed to stay open, and a client-wide deadline
    // would cut it off after five minutes. Ordinary requests apply their own deadline instead.
    let mut builder = reqwest::Client::builder()
        .tls_backend_preconfigured(tls.clone())
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(concat!("7Shell/", env!("CARGO_PKG_VERSION")));

    if let Some(endpoint) = endpoint {
        // A proxy rather than a DNS override: an address written as an IP literal is connected to
        // without ever being resolved, so nothing can stand in for it at the resolver. The CONNECT
        // this client sends is what the listener answers, and the TLS handshake that follows is
        // between this client and the cluster, with the host name the kubeconfig gave.
        let proxy = reqwest::Proxy::all(format!("http://{endpoint}")).map_err(|error| {
            ShellError::k8s_unavailable(format!("the local end of the tunnel is not a usable proxy: {error}"))
        })?;
        builder = builder.proxy(proxy);
    }

    builder
        .build()
        .map_err(|error| ShellError::k8s_unavailable(format!("the HTTP client could not be built: {error}")))
}

/// The cluster connections a window holds open, and the streams they are feeding.
pub struct K8sManager {
    connections: Mutex<HashMap<String, Connection>>,
    streams: Arc<Streams>,
}

/// One open connection: the client, and the local end of the tunnel when there is one.
struct Connection {
    client: Arc<K8sClient>,
    /// Held, not read: dropping it stops the listener and the connections it is carrying.
    #[allow(dead_code)]
    tunnel: Option<TunnelEndpoint>,
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
        self.connections()
            .insert(id.clone(), Connection { client, tunnel: None });
        Ok(id)
    }

    /// Opens a connection whose traffic goes through `dialer` instead of this machine's network.
    pub async fn connect_via(
        &self,
        prepared: TunneledCluster,
        dialer: Arc<dyn ApiDialer>,
    ) -> Result<String, ShellError> {
        // Built before the id exists, for the same reason a direct connection is: a listener with
        // no id would be a listener nobody can stop.
        let (client, tunnel) = K8sClient::open_via(prepared, dialer).await?;
        let id = uuid::Uuid::new_v4().to_string();
        self.connections().insert(
            id.clone(),
            Connection {
                client: Arc::new(client),
                tunnel: Some(tunnel),
            },
        );
        Ok(id)
    }

    /// Drops a connection and everything it was streaming. Answers whether it was open.
    pub fn disconnect(&self, connection_id: &str) -> bool {
        // Taken out of the map and dropped before this returns: that drop is what stops a tunnel's
        // listener and releases the dialer it held, so nothing keeps carrying traffic for a tab
        // that is gone.
        let connection = self.connections().remove(connection_id);
        // Streams are stopped even when the connection was already gone: the two maps are written
        // separately, and a watch whose connection was dropped is still a watch holding a socket.
        self.streams.stop_connection(connection_id);
        connection.is_some()
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

    /// Opens a command that runs to completion: no terminal, two output streams, and a status.
    ///
    /// File operations use this and never the terminal path: a pty merges standard error into
    /// standard output and translates line endings, and a tar archive survives neither.
    pub async fn exec_run(
        &self,
        connection_id: &str,
        request: &sevenshell_types::K8sExecRequest,
        stdin: bool,
    ) -> Result<crate::exec::RunSession, ShellError> {
        let client = self.client(connection_id)?;
        crate::exec::open_run(&client, request, stdin).await
    }

    fn client(&self, connection_id: &str) -> Result<Arc<K8sClient>, ShellError> {
        let connections = self.connections();
        connections
            .get(connection_id)
            .map(|connection| Arc::clone(&connection.client))
            .ok_or_else(|| ShellError::k8s_not_connected(connection_id))
    }

    fn connections(&self) -> MutexGuard<'_, HashMap<String, Connection>> {
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

#[cfg(test)]
mod tunnel_tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// A kubeconfig pointing at `server`, with a token so the document is a usable one.
    fn document(server: &str) -> String {
        format!(
            r#"apiVersion: v1
kind: Config
current-context: test
clusters:
  - name: test
    cluster:
      server: {server}
contexts:
  - name: test
    context:
      cluster: test
      user: test
users:
  - name: test
    user:
      token: not-a-real-token
"#
        )
    }

    /// A dialer that answers with a socket to `address`, recording what it was asked for.
    struct LocalDialer {
        address: SocketAddr,
        asked: Mutex<Vec<(String, u16)>>,
        calls: AtomicUsize,
    }

    impl LocalDialer {
        fn new(address: SocketAddr) -> Arc<Self> {
            Arc::new(Self {
                address,
                asked: Mutex::new(Vec::new()),
                calls: AtomicUsize::new(0),
            })
        }

        fn asked(&self) -> Vec<(String, u16)> {
            self.asked.lock().expect("the asked lock").clone()
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl ApiDialer for LocalDialer {
        fn dial(&self, host: String, port: u16) -> DialFuture {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.asked.lock().expect("the asked lock").push((host, port));
            let address = self.address;
            Box::pin(async move {
                let stream = TcpStream::connect(address).await.map_err(|error| {
                    ShellError::k8s_unavailable(format!("the test tunnel could not connect: {error}"))
                })?;
                let stream: BoxedApiStream = Box::new(stream);
                Ok(stream)
            })
        }
    }

    /// A TCP server that echoes back whatever one connection sends, so a carried connection has
    /// something to show for itself.
    async fn echo_once() -> SocketAddr {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.expect("a free port");
        let address = listener.local_addr().expect("an address");
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buffer = [0u8; 64];
                    if let Ok(read) = stream.read(&mut buffer).await
                        && read > 0
                    {
                        let _ = stream.write_all(&buffer[..read]).await;
                    }
                });
            }
        });
        address
    }

    fn target(host: &str, port: u16) -> ServerAddress {
        ServerAddress {
            host: host.to_owned(),
            port,
        }
    }

    /// Reads one HTTP answer head, so an assertion is about the status line rather than a length.
    async fn read_answer_head(stream: &mut TcpStream) -> String {
        let mut answer = Vec::new();
        let mut chunk = [0u8; 256];
        while !answer.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = stream.read(&mut chunk).await.expect("the answer");
            assert!(read > 0, "the bridge closed before it answered");
            answer.extend_from_slice(&chunk[..read]);
        }
        String::from_utf8_lossy(&answer).into_owned()
    }

    #[test]
    fn a_server_address_carries_the_port_its_scheme_defaults_to() {
        assert_eq!(
            server_address("https://api.example.test").expect("an https address"),
            target("api.example.test", 443)
        );
        assert_eq!(
            server_address("https://api.example.test:6443/k8s").expect("an explicit port"),
            target("api.example.test", 6443)
        );
        assert_eq!(
            server_address("http://api.example.test").expect("an http address"),
            target("api.example.test", 80)
        );
    }

    #[test]
    fn an_ipv6_server_address_is_dialled_without_its_brackets() {
        // Brackets are how a URL spells an IPv6 authority; a direct-tcpip channel names the
        // address, and a host spelled with brackets is not one.
        assert_eq!(
            server_address("https://[::1]:6443").expect("an ipv6 address"),
            target("::1", 6443)
        );
        assert_eq!(
            server_address("https://[2001:db8::1]").expect("an ipv6 address without a port"),
            target("2001:db8::1", 443)
        );
    }

    #[test]
    fn an_address_that_is_not_a_cluster_address_is_refused_as_a_kubeconfig_problem() {
        let error = server_address("api.example.test:6443").expect_err("no scheme is not an address");
        assert_eq!(error.code(), sevenshell_error::ErrorCode::K8sKubeconfigInvalid);
    }

    #[test]
    fn a_tunnel_request_names_the_server_whatever_the_urls_spelling() {
        let target = target("api.example.test", 443);
        assert!(asks_for_target("CONNECT api.example.test:443 HTTP/1.1", &target));
        // A URL lower-cases a domain; a kubeconfig may not.
        assert!(asks_for_target("CONNECT API.EXAMPLE.TEST:443 HTTP/1.1", &target));
        assert!(!asks_for_target("CONNECT api.example.test:6443 HTTP/1.1", &target));
        assert!(!asks_for_target("CONNECT other.example.test:443 HTTP/1.1", &target));
        // An ordinary request in absolute form is not a tunnel request.
        assert!(!asks_for_target(
            "GET http://api.example.test/api/v1/pods HTTP/1.1",
            &target
        ));
    }

    #[test]
    fn an_ipv6_tunnel_request_is_compared_as_an_address() {
        let target = target("2001:db8::1", 6443);
        assert!(asks_for_target("CONNECT [2001:db8::1]:6443 HTTP/1.1", &target));
        assert!(asks_for_target(
            "CONNECT [2001:0db8:0000:0000:0000:0000:0000:0001]:6443 HTTP/1.1",
            &target
        ));
    }

    #[test]
    fn the_authority_the_client_sends_is_the_one_the_bridge_accepts() {
        // The two sides of one contract: the WebSocket path writes this line, and the listener
        // judges it. A bracketed IPv6 address is where the two spellings would part company.
        for (host, port) in [("api.example.test", 6443), ("2001:db8::1", 6443), ("127.0.0.1", 1)] {
            let target = target(host, port);
            let line = format!("CONNECT {} HTTP/1.1", target.authority());
            assert!(
                asks_for_target(&line, &target),
                "the bridge refused its own client: {line}"
            );
        }
        assert_eq!(target("2001:db8::1", 6443).authority(), "[2001:db8::1]:6443");
    }

    #[test]
    fn a_plain_http_cluster_is_refused_before_a_listener_exists() {
        let config = Kubeconfig::parse(document("http://api.example.test").as_bytes()).expect("a kubeconfig");
        let prepared = TunneledCluster::prepare(&config, None);
        let error = match prepared {
            Ok(_) => panic!("a plain-HTTP cluster was prepared for a tunnel"),
            Err(error) => error,
        };
        assert_eq!(error.code(), sevenshell_error::ErrorCode::AppUnsupported);
    }

    #[tokio::test]
    async fn the_bridge_answers_a_connect_for_its_server_and_carries_the_bytes_through_the_dialer() {
        let far = echo_once().await;
        let dialer = LocalDialer::new(far);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.expect("a free port");
        let address = listener.local_addr().expect("an address");
        let _endpoint = TunnelEndpoint::start(listener, target("api.example.test", 6443), dialer.clone());

        let mut local = TcpStream::connect(address).await.expect("the listener");
        local
            .write_all(b"CONNECT api.example.test:6443 HTTP/1.1\r\nHost: api.example.test:6443\r\n\r\n")
            .await
            .expect("the tunnel request");
        let answer = read_answer_head(&mut local).await;
        assert!(
            answer.starts_with("HTTP/1.1 200"),
            "the tunnel request was not carried: {answer}"
        );

        local.write_all(b"hello").await.expect("bytes for the far side");
        let mut echoed = [0u8; 5];
        local.read_exact(&mut echoed).await.expect("the far side's answer");
        assert_eq!(&echoed, b"hello");

        assert_eq!(dialer.asked(), vec![("api.example.test".to_owned(), 6443)]);
    }

    #[tokio::test]
    async fn the_bridge_refuses_a_connect_for_another_server_instead_of_carrying_it() {
        let far = echo_once().await;
        let dialer = LocalDialer::new(far);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.expect("a free port");
        let address = listener.local_addr().expect("an address");
        let _endpoint = TunnelEndpoint::start(listener, target("api.example.test", 443), dialer.clone());

        let mut local = TcpStream::connect(address).await.expect("the listener");
        local
            .write_all(b"CONNECT elsewhere.example.test:443 HTTP/1.1\r\nHost: elsewhere.example.test:443\r\n\r\n")
            .await
            .expect("the tunnel request");
        let answer = read_answer_head(&mut local).await;

        assert!(
            answer.starts_with("HTTP/1.1 403"),
            "a request for another server was not refused: {answer}"
        );
        assert_eq!(
            dialer.calls(),
            0,
            "the bridge dialled for a server it was not opened for"
        );
    }

    #[tokio::test]
    async fn dropping_the_endpoint_stops_the_listener() {
        let far = echo_once().await;
        let dialer = LocalDialer::new(far);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.expect("a free port");
        let address = listener.local_addr().expect("an address");
        let endpoint = TunnelEndpoint::start(listener, target("api.example.test", 443), dialer);

        assert!(
            TcpStream::connect(address).await.is_ok(),
            "the listener did not answer while the endpoint lived"
        );
        drop(endpoint);

        // The task is aborted, so the socket is released when the runtime next polls it; a few
        // attempts cover that without making the answer depend on the scheduler.
        let mut closed = false;
        for _ in 0..100 {
            if TcpStream::connect(address).await.is_err() {
                closed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(closed, "the listener outlived the endpoint");
    }
}
