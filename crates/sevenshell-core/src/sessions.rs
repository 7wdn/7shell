//! Opening, driving and closing terminals, the routes they take, and the forwards they bring up.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sevenshell_error::ShellError;
use sevenshell_session::{OpenTerminal, RecordingSpec, SessionRegistry};
use sevenshell_ssh::openssh::SystemHostKey;
use sevenshell_ssh::{
    ChannelEvent, Credential, LocalForward as HopForward, OpenRequest, Purpose, Route, SshSession, Target,
    TerminalSize, TunnelHandle,
};
use sevenshell_term::{LocalSpec, SerialSpec, TcpSpec, TelnetSpec, WslSpec};
use sevenshell_types::{
    AuthMethod, ConnectionTestOutcome, ForwardMode, KnownHostEntry, LocalForward, LocalShell, OpenOutcome, OpenTunnel,
    PresentedHostKey, Protocol, SecretInput, SerialPortInfo, Session, TerminalEncoding, TransferInfo, TunnelFailure,
};
use sevenshell_vault::Vault;
use tokio::sync::{Mutex, Notify, mpsc};

/// One event from an open terminal, on its way to the host.
pub use sevenshell_session::SessionEvent as CoreEvent;

/// How long connecting, authenticating and opening a channel may take in total.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(30);

/// The size a terminal starts at, until the interface says otherwise.
const INITIAL_SIZE: TerminalSize = TerminalSize { columns: 80, rows: 24 };

/// A decision a host key needs before anything can be sent.
///
/// Shared by both kinds of session — a terminal and a file browser — because the decision is the
/// same one, and answering it in two places is how the two answers drift apart.
pub(crate) enum HostKeyDecision {
    Unknown(PresentedHostKey),
    Mismatch {
        key: PresentedHostKey,
        trusted: KnownHostEntry,
    },
}

/// A session after OpenSSH's configuration has had its say.
///
/// The saved session keeps the alias; this is the copy a connection actually uses. `config` is the
/// resolved entry when there was one, so the credential step can read the identities it names
/// without the interface ever learning a path. `proxy` is the `ProxyJump` route the same file
/// names, outermost hop first; it is empty when the session keeps its own saved jump list, which
/// is the person's explicit route.
#[derive(Debug)]
pub(crate) struct PreparedSession {
    pub(crate) session: Session,
    pub(crate) config: Option<sevenshell_ssh::openssh::ResolvedHost>,
    pub(crate) proxy: Vec<crate::openssh::ProxyHop>,
}

/// One connection a route makes before it reaches the target.
struct HopPlan {
    host: String,
    port: u16,
    user: String,
    keepalive_seconds: u16,
    source: HopSource,
}

enum HopSource {
    /// A saved session the route names, resolved on its own terms.
    Saved(Box<PreparedSession>),
    /// A hop the target's own `ProxyJump` names.
    Config(Box<crate::openssh::ProxyHop>),
}

/// A key file larger than this is not a key.
const MAX_IDENTITY_BYTES: u64 = 1024 * 1024;

/// What this machine's own `known_hosts` says about the key a server presented.
fn system_known_host(host: &str, port: u16, presented: &PresentedHostKey) -> SystemHostKey {
    let Some(path) = sevenshell_platform::ssh_dir().map(|directory| directory.join("known_hosts")) else {
        return SystemHostKey::Unknown;
    };
    sevenshell_ssh::openssh::check_system_known_host(&path, host, port, &presented.key_type, &presented.public_key)
}

/// What the two record keepers together say about the key a server presented.
enum KeyStanding {
    Accepted,
    Unknown,
    /// The record that stops the connection, for the prompt that shows both fingerprints.
    Changed(KnownHostEntry),
}

/// Reads the vault's record and this machine's `known_hosts` as one answer.
///
/// Either keeper accepting the key is enough: both are records of keys a person has trusted, and
/// asking again because only one of them knows the key would be the same question answered twice.
/// A record of a *different algorithm* is not a record of this key — OpenSSH treats that as unseen
/// and asks, and stopping there is how a server that added or switched a key type looked like a
/// changed host key. Only a record of the same algorithm with different material is the real stop.
/// Callers have already refused on `SystemHostKey::Changed`.
fn standing(trusted: Option<KnownHostEntry>, system: SystemHostKey, presented: &PresentedHostKey) -> KeyStanding {
    match trusted {
        Some(entry) if entry.accepts(presented) => KeyStanding::Accepted,
        Some(entry) if entry.key_type == presented.key_type => KeyStanding::Changed(entry),
        _ if system == SystemHostKey::Trusted => KeyStanding::Accepted,
        _ => KeyStanding::Unknown,
    }
}

/// Where an `IdentityFile` value points on this machine.
///
/// `~` and `%d` are the home directory; a bare relative path is read under `~/.ssh`, which is where
/// a configuration file's relative identities almost always live. The path stays in this process.
fn identity_path(value: &str, home: &std::path::Path, ssh_dir: &std::path::Path) -> std::path::PathBuf {
    // `~/.ssh` is the directory `ssh_dir()` names, which a controlled run may have moved. Expanding
    // the tilde against the real home first would read the person's keys in a test that asked for
    // its own.
    for prefix in ["~/.ssh/", "~\\.ssh\\", "%d/.ssh/", "%d\\.ssh\\"] {
        if let Some(rest) = value.strip_prefix(prefix) {
            return ssh_dir.join(rest);
        }
    }
    if value == "~/.ssh" || value == "~\\.ssh" || value == "%d/.ssh" || value == "%d\\.ssh" {
        return ssh_dir.to_path_buf();
    }

    if value == "~"
        || value.starts_with("~/")
        || value.starts_with("~\\")
        || value.starts_with("%d/")
        || value.starts_with("%d\\")
    {
        return sevenshell_ssh::openssh::expand_home(value, home);
    }

    let path = std::path::PathBuf::from(value);
    if path.is_absolute() { path } else { ssh_dir.join(path) }
}

/// A transfer that is running, or has finished running.
pub struct TransferRecord {
    pub(crate) info: TransferInfo,
    pub(crate) progress: Arc<sevenshell_sftp::TransferProgress>,
    /// The task driving it, so whoever stops it can wait for the file handles to be released.
    ///
    /// An `Option` because a handle can be awaited exactly once: stopping and removing both take it,
    /// and what is left behind is the truth — a transfer with nothing running behind it any more.
    pub(crate) task: Option<tokio::task::JoinHandle<()>>,
    /// Where this transfer's local end is, when there is one.
    ///
    /// Kept here rather than in `info` because it must not cross to the interface: it was chosen in
    /// a native dialog, and what the interface asks for instead is *this row* — the id — so that
    /// opening the file, or showing where it is, happens entirely on this side.
    pub(crate) local: Option<std::path::PathBuf>,
    /// How to run this transfer again, for the retry button.
    ///
    /// The core is the only place that can hold this: the local end of a transfer was chosen in a
    /// native dialog and deliberately never came back through the interface, so a transfer that
    /// failed cannot be re-issued by the page that started it — it can only be asked for again here.
    pub(crate) run: TransferRun,
}

/// One attempt at a transfer, as a future that a task can outlive the call that made it with.
///
/// Boxed because the four kinds of transfer have four different futures and the registry holds them
/// all in one map.
pub(crate) type TransferAttempt = std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ShellError>> + Send>>;

/// One attempt at a transfer, as a closure that can be called more than once.
///
/// A `Fn` closure is what makes a retry the *same* transfer rather than a new one. Each call has to
/// produce a fresh future that owns everything it uses — an `async` block borrowing the closure's
/// captures would not outlive the call, which is why every call site clones what it needs.
pub(crate) type TransferRun = Arc<dyn Fn(Arc<sevenshell_sftp::TransferProgress>) -> TransferAttempt + Send + Sync>;

/// A forward that is up, and what it belongs to.
struct TunnelRecord {
    id: String,
    handle: String,
    session_id: String,
    spec: LocalForward,
    tunnel: TunnelHandle,
}

/// The stops for streams a container tab is following, keyed by the stream id the host emitted.
pub(crate) type ContainerStreamStops = HashMap<String, Arc<dyn Fn() + Send + Sync>>;

/// The operations that need the vault, the transport and the registry in a particular order.
pub struct Core {
    pub(crate) vault: Arc<Vault>,
    pub(crate) registry: Arc<SessionRegistry>,
    pub(crate) files: Mutex<HashMap<String, Arc<crate::FileSession>>>,
    /// Behind an `Arc` as well as a mutex: the task driving a transfer updates its own state
    /// long after the call that started it has returned.
    pub(crate) transfers: Arc<Mutex<HashMap<String, crate::TransferRecord>>>,
    /// Container connections, one per open container tab. The manager owns the runtime side; the
    /// core owns the order in which a tab is connected, used and let go.
    #[allow(dead_code)]
    pub(crate) containers: Arc<sevenshell_container::ContainerManager>,
    /// Kubernetes connections, one per open cluster tab.
    #[allow(dead_code)]
    pub(crate) clusters: Arc<sevenshell_k8s::K8sManager>,
    /// Stops for the line streams a container tab is following. Keyed by the id the host emitted,
    /// so a stop button and a closing window use the same path.
    pub(crate) container_streams: Arc<Mutex<ContainerStreamStops>>,
    /// Tokens for places on this machine a native dialog picked. Paths stay here; the token is
    /// what the interface and the tools see.
    pub(crate) vmrest_auth: std::sync::Mutex<Option<(String, String)>>,
    pub(crate) next_file: std::sync::atomic::AtomicU64,
    pub(crate) next_transfer: std::sync::atomic::AtomicU64,
    /// The matrix and the audit trail behind every unattended action. Replaced together, read
    /// without waiting on a disk because the journal is called after the snapshot is released.
    pub(crate) policy: std::sync::Mutex<crate::policy::PolicySnapshot>,
    tunnels: Mutex<HashMap<String, TunnelRecord>>,
    /// The opens that are still going, so one of them can be called off.
    ///
    /// Keyed by session rather than by attempt: the interface knows which row was clicked, not which
    /// request it made, and a second attempt for the same session while one is in flight is already
    /// refused by the interface.
    opens_in_flight: Mutex<HashMap<String, Arc<Notify>>>,
    pub(crate) timeout: Duration,
}

impl Core {
    /// Builds the core around a vault, and returns it with the receiver its terminal output arrives on.
    pub fn new(vault: Arc<Vault>) -> (Arc<Self>, mpsc::Receiver<CoreEvent>) {
        let (registry, events) = SessionRegistry::start();
        (
            Arc::new(Self {
                vault,
                registry,
                files: Mutex::new(HashMap::new()),
                transfers: Arc::new(Mutex::new(HashMap::new())),
                containers: Arc::new(sevenshell_container::ContainerManager::new()),
                clusters: Arc::new(sevenshell_k8s::K8sManager::new()),
                container_streams: Arc::new(Mutex::new(HashMap::new())),
                vmrest_auth: std::sync::Mutex::new(None),
                next_file: std::sync::atomic::AtomicU64::new(1),
                next_transfer: std::sync::atomic::AtomicU64::new(1),
                // The default rule until a host replaces it: safe and caution run, the destructive
                // levels ask, and nothing is journaled because there is no trail yet.
                policy: std::sync::Mutex::new(crate::policy::PolicySnapshot::default()),
                tunnels: Mutex::new(HashMap::new()),
                opens_in_flight: Mutex::new(HashMap::new()),
                timeout: OPEN_TIMEOUT,
            }),
            events,
        )
    }

    /// The vault this core reads from, for the commands that only touch storage.
    pub fn vault(&self) -> &Vault {
        &self.vault
    }

    /// Opens a terminal, or reports which decision has to be made first.
    ///
    /// The order is the whole point:
    ///
    /// 1. The session and its credential are resolved locally, so a session that cannot possibly
    ///    authenticate fails without opening a socket.
    /// 2. Every jump host in the route is connected in turn, and each one's key is verified before
    ///    anything is sent through it.
    /// 3. The target's host key is read with a probe over the last hop — a connection that sends
    ///    nothing — so the answer is the key the real connection will see.
    /// 4. If a key has never been seen, or has changed, the caller is told and nothing more
    ///    happens. Changing a key is a stop, not a warning.
    /// 5. Only then is the real connection opened, and it verifies the key again inside itself.
    ///
    /// A decision names the host it is about, which is what makes a route of any length work with
    /// one prompt: trust that host, ask again, and the next unknown one asks in its turn.
    pub async fn open_terminal(&self, session_id: &str) -> Result<OpenOutcome, ShellError> {
        let cancel = self.begin_open(session_id).await;
        let outcome = self.open_terminal_until_cancelled(session_id, &cancel).await;
        self.opens_in_flight.lock().await.remove(session_id);
        outcome
    }

    /// Calls off an open that is still going, and says whether there was one.
    ///
    /// The answer is what the interface shows: a click that cancelled something says so, and one
    /// that arrived after the terminal was already up does not pretend it stopped anything.
    pub async fn cancel_open(&self, session_id: &str) -> bool {
        let open = self.opens_in_flight.lock().await.get(session_id).cloned();
        match open {
            Some(cancel) => {
                // `notify_one` rather than `notify_waiters`: the step that is waiting is between two
                // awaits half the time, and a notification with nobody listening is kept until the
                // next step does listen.
                cancel.notify_one();
                true
            }
            None => false,
        }
    }

    /// The token for one attempt, registered so a cancel can find it.
    async fn begin_open(&self, session_id: &str) -> Arc<Notify> {
        let cancel = Arc::new(Notify::new());
        self.opens_in_flight
            .lock()
            .await
            .insert(session_id.to_owned(), cancel.clone());
        cancel
    }

    async fn open_terminal_until_cancelled(
        &self,
        session_id: &str,
        cancel: &Arc<Notify>,
    ) -> Result<OpenOutcome, ShellError> {
        let prepared = self.prepare_session(&self.session(session_id)?)?;
        let session = &prepared.session;
        match session.protocol {
            Protocol::Ssh => {}
            Protocol::Rdp => {
                return Err(ShellError::app_unsupported(
                    "this session is a remote desktop, not a terminal",
                ));
            }
            Protocol::WebDav => {
                return Err(ShellError::app_unsupported(
                    "this session is a WebDAV folder, not a terminal",
                ));
            }
            Protocol::Ftp => {
                return Err(ShellError::app_unsupported(
                    "this session is an FTP folder, not a terminal",
                ));
            }
            Protocol::Container => {
                return Err(ShellError::app_unsupported(
                    "this session is a container engine; open it from the container workspace",
                ));
            }
            Protocol::Vmware => {
                return Err(ShellError::app_unsupported(
                    "this session is a virtual machine; open it from the VM workspace",
                ));
            }
            Protocol::Kubernetes => {
                return Err(ShellError::app_unsupported(
                    "this session is a Kubernetes cluster; open it from the cluster workspace",
                ));
            }
            Protocol::Telnet | Protocol::RawTcp | Protocol::Serial | Protocol::Local | Protocol::Wsl => {
                return self.open_stream_terminal_until_cancelled(session, cancel).await;
            }
        }
        let credential = self.credential(session, prepared.config.as_ref())?;
        let target = Target {
            host: session.host.clone(),
            port: session.port,
            user: session.user.clone(),
        };

        let route = until_cancelled(self.open_route(&prepared, None), cancel).await?;
        let hop = match route {
            RouteOutcome::Ready(hop) => hop,
            RouteOutcome::Decision(decision) => return Ok(terminal_decision(*decision)),
        };

        let presented = match &hop {
            Some(hop) => {
                until_cancelled(
                    sevenshell_ssh::probe_host_key_via(hop, &target.host, target.port, self.timeout),
                    cancel,
                )
                .await?
            }
            None => {
                until_cancelled(
                    sevenshell_ssh::probe_host_key(&target.host, target.port, self.timeout),
                    cancel,
                )
                .await?
            }
        };

        let system = system_known_host(&target.host, target.port, &presented);
        if system == SystemHostKey::Changed {
            return Err(ShellError::ssh_host_key_system_mismatch(
                &target.host,
                target.port,
                &presented.key_type,
                &presented.fingerprint,
            ));
        }
        match standing(
            self.vault.find_known_host(&target.host, target.port)?,
            system,
            &presented,
        ) {
            KeyStanding::Accepted => {}
            KeyStanding::Unknown => return Ok(OpenOutcome::HostKeyUnknown { key: presented }),
            KeyStanding::Changed(trusted) => {
                return Ok(OpenOutcome::HostKeyMismatch {
                    key: presented,
                    trusted,
                });
            }
        }

        let route = match hop {
            Some(hop) => Route::Through {
                hop,
                host: target.host.clone(),
                port: target.port,
            },
            None => Route::Direct,
        };

        let request = OpenRequest {
            target,
            credential,
            terminal_type: session.terminal.terminal_type.clone(),
            keepalive_seconds: session.terminal.keepalive_seconds,
            // Only the terminal: it is the channel whose far side runs a shell, and a shell is
            // what has an environment for the agent's socket. A carrier carries channels.
            forward_agent: session.terminal.forward_agent,
            timeout: self.timeout,
            expected_host_key: presented,
            size: INITIAL_SIZE,
            route,
            purpose: Purpose::Terminal,
        };

        let opened = until_cancelled(
            self.registry.open(session_id, request, self.recording_for(session)?),
            cancel,
        )
        .await;
        let handle = match opened {
            Ok(handle) => handle,
            Err(cancelled) => {
                // Registering the terminal is the last thing that function does, so a cancellation
                // can land a moment after it — and a terminal nobody was told the handle of is a
                // connection left running with nothing on screen to close it. Closing whatever this
                // session opened is what makes "cancelled" true rather than nearly true.
                self.close_terminals_for(session_id).await;
                return Err(cancelled);
            }
        };
        let (tunnels, tunnel_failures) = self.bring_up_forwards(&handle, session).await;

        Ok(OpenOutcome::Opened {
            handle,
            session_id: session_id.to_owned(),
            tunnels,
            tunnel_failures,
        })
    }

    /// Closes every terminal this session has, which is what a cancelled open leaves behind it.
    async fn close_terminals_for(&self, session_id: &str) {
        for open in self.registry.open_sessions().await {
            if open.session_id == session_id {
                let _ = self.close_terminal(&open.handle).await;
            }
        }
    }

    /// Tests a session's connection without opening a terminal.
    ///
    /// The order is the open's order, because a test that skipped a step would be testing a
    /// different connection than the one the person would get: every jump host is authenticated,
    /// the target's key is probed and checked against what is trusted, and only then is a
    /// credential offered. What comes up is a carrier — no session channel and no pty — so the
    /// server sees a login and nothing else. Calling this "connected" therefore means the key was
    /// verified and the credential was accepted, which is the question a test is for.
    ///
    /// The session is the editor's draft rather than a saved id: a person should be able to learn
    /// that a password is wrong before saving it. The credential is the one typed into the dialog
    /// when there is one, and the vault's when there is not.
    ///
    /// Telnet and a raw TCP console have no login; for them the honest test is whether the
    /// address answers. Every other transport has no connection of its own to test.
    pub async fn test_session(
        &self,
        session: &Session,
        secret: Option<SecretInput>,
    ) -> Result<ConnectionTestOutcome, ShellError> {
        let started = Instant::now();
        let prepared = self.prepare_session(session)?;
        let session = &prepared.session;

        match session.protocol {
            Protocol::Ssh => self.test_ssh(&prepared, secret, started).await,
            Protocol::Telnet | Protocol::RawTcp => {
                let stream = match tokio::time::timeout(
                    self.timeout,
                    tokio::net::TcpStream::connect((session.host.as_str(), session.port)),
                )
                .await
                {
                    Ok(Ok(stream)) => stream,
                    Ok(Err(error)) => {
                        return Err(ShellError::terminal_unreachable(
                            &session.host,
                            session.port,
                            error.to_string(),
                        ));
                    }
                    Err(_) => {
                        return Err(ShellError::terminal_unreachable(
                            &session.host,
                            session.port,
                            format!("nothing answered within {} seconds", self.timeout.as_secs()),
                        ));
                    }
                };
                drop(stream);
                Ok(ConnectionTestOutcome::Connected {
                    latency_ms: elapsed_millis(started),
                    authenticated: false,
                    fingerprint: None,
                })
            }
            _ => Err(ShellError::app_unsupported(
                "this kind of session has no connection to test",
            )),
        }
    }

    /// The SSH half of [`Core::test_session`]: verify the route and the key, authenticate, hang up.
    async fn test_ssh(
        &self,
        prepared: &PreparedSession,
        secret: Option<SecretInput>,
        started: Instant,
    ) -> Result<ConnectionTestOutcome, ShellError> {
        let session = &prepared.session;
        let credential = self.credential_for_test(session, prepared.config.as_ref(), secret.as_ref())?;
        let target = Target {
            host: session.host.clone(),
            port: session.port,
            user: session.user.clone(),
        };

        let route = self.open_route(prepared, secret.as_ref()).await?;
        let hop = match route {
            RouteOutcome::Ready(hop) => hop,
            RouteOutcome::Decision(decision) => return Ok(test_decision(*decision)),
        };

        let presented = match &hop {
            Some(hop) => sevenshell_ssh::probe_host_key_via(hop, &target.host, target.port, self.timeout).await?,
            None => sevenshell_ssh::probe_host_key(&target.host, target.port, self.timeout).await?,
        };

        let system = system_known_host(&target.host, target.port, &presented);
        if system == SystemHostKey::Changed {
            return Err(ShellError::ssh_host_key_system_mismatch(
                &target.host,
                target.port,
                &presented.key_type,
                &presented.fingerprint,
            ));
        }
        match standing(
            self.vault.find_known_host(&target.host, target.port)?,
            system,
            &presented,
        ) {
            KeyStanding::Accepted => {}
            KeyStanding::Unknown => return Ok(ConnectionTestOutcome::HostKeyUnknown { key: presented }),
            KeyStanding::Changed(trusted) => {
                return Ok(ConnectionTestOutcome::HostKeyMismatch {
                    key: presented,
                    trusted,
                });
            }
        }

        let route = match hop {
            Some(hop) => Route::Through {
                hop,
                host: target.host.clone(),
                port: target.port,
            },
            None => Route::Direct,
        };
        let fingerprint = presented.fingerprint.clone();
        let request = OpenRequest {
            target,
            credential,
            terminal_type: session.terminal.terminal_type.clone(),
            // A connection with no terminal has no life to keep.
            keepalive_seconds: 0,
            forward_agent: false,
            timeout: self.timeout,
            expected_host_key: presented,
            size: INITIAL_SIZE,
            route,
            purpose: Purpose::Carrier,
        };

        let (events, _unused) = mpsc::channel::<ChannelEvent>(1);
        let connected = SshSession::open(request, events).await?;
        let outcome = ConnectionTestOutcome::Connected {
            latency_ms: elapsed_millis(started),
            authenticated: true,
            fingerprint: Some(fingerprint),
        };
        // The test is over either way. A disconnect that fails changes nothing about what the
        // connection already answered.
        let _ = connected.close().await;
        Ok(outcome)
    }

    /// The credential a test offers: the typed one when there is one, the stored one otherwise.
    ///
    /// Only the login-and-password method has anything new to offer here. A key's material is in
    /// the vault or on disk already, and an imported key carries its own passphrase with it.
    fn credential_for_test(
        &self,
        session: &Session,
        config: Option<&sevenshell_ssh::openssh::ResolvedHost>,
        secret: Option<&SecretInput>,
    ) -> Result<Credential, ShellError> {
        match (session.auth.method, secret) {
            (AuthMethod::Password | AuthMethod::KeyboardInteractive, Some(input)) => {
                Ok(Credential::Password(input.value.clone()))
            }
            _ => self.credential(session, config),
        }
    }

    /// Opens a terminal whose transport is not SSH.
    ///
    /// Nothing about a host key applies here: Telnet and serial ports have no identity to verify,
    /// and a local shell is a process on this machine. There is also no route and no forwarding —
    /// the capabilities those need belong to SSH, and the descriptors do not claim them for these
    /// protocols.
    async fn open_stream_terminal_until_cancelled(
        &self,
        session: &Session,
        cancel: &Arc<Notify>,
    ) -> Result<OpenOutcome, ShellError> {
        let size = sevenshell_term::Size {
            columns: INITIAL_SIZE.columns,
            rows: INITIAL_SIZE.rows,
        };
        let (stream, events) = match session.protocol {
            Protocol::Telnet => {
                let spec = TelnetSpec {
                    host: session.host.clone(),
                    port: session.port,
                    enter: session.telnet.enter,
                    backspace: session.telnet.backspace,
                    timeout: self.timeout,
                };
                until_cancelled(sevenshell_term::open_telnet(spec, size), cancel).await?
            }
            Protocol::Serial => {
                let spec = SerialSpec::from(&session.serial);
                until_cancelled(sevenshell_term::open_serial(spec), cancel).await?
            }
            Protocol::RawTcp => {
                let spec = TcpSpec {
                    host: session.host.clone(),
                    port: session.port,
                    timeout: self.timeout,
                };
                until_cancelled(sevenshell_term::open_raw_tcp(spec), cancel).await?
            }
            Protocol::Local => {
                let spec = LocalSpec {
                    shell: session.local.shell.clone(),
                    args: session.local.args.clone(),
                    cwd: None,
                };
                until_cancelled(sevenshell_term::open_local(spec, size), cancel).await?
            }
            Protocol::Wsl => {
                let spec = WslSpec {
                    distro: session.wsl.distro.clone(),
                    args: session.wsl.args.clone(),
                };
                until_cancelled(sevenshell_term::open_wsl(spec, size), cancel).await?
            }
            Protocol::Ssh
            | Protocol::Rdp
            | Protocol::WebDav
            | Protocol::Ftp
            | Protocol::Container
            | Protocol::Vmware
            | Protocol::Kubernetes => {
                unreachable!("only the stream protocols reach this function")
            }
        };
        let handle = self
            .registry
            .open_stream(&session.id, stream, events, self.recording_for(session)?)
            .await?;
        Ok(OpenOutcome::Opened {
            handle,
            session_id: session.id.clone(),
            tunnels: Vec::new(),
            tunnel_failures: Vec::new(),
        })
    }

    /// The shells this machine can start, for the editor's picker.
    pub fn local_shells(&self) -> Vec<LocalShell> {
        sevenshell_term::local_shells()
    }

    /// The WSL distributions this machine has. Empty where WSL does not exist.
    pub fn wsl_distros(&self) -> Vec<String> {
        sevenshell_term::wsl_distros()
    }

    /// The serial ports this machine has, for the editor's picker.
    pub fn serial_ports(&self) -> Vec<SerialPortInfo> {
        sevenshell_term::serial_ports()
    }

    /// Sends keystrokes or pasted text.
    pub async fn write(&self, handle: &str, data: &[u8]) -> Result<(), ShellError> {
        self.registry.write(handle, data).await
    }

    /// Sends text in the encoding the terminal session asked for.
    ///
    /// The terminal on this side always speaks UTF-8; a switch or an old Unix host may not. Encoding
    /// here rather than in the interface keeps the browser edition and the desktop edition on the
    /// same rule, and keeps the conversion next to the only code that writes to the pty.
    pub async fn write_text(&self, handle: &str, text: &str, encoding: TerminalEncoding) -> Result<(), ShellError> {
        let bytes = encoded_text(text, encoding);
        self.write(handle, &bytes).await
    }

    /// Tells the remote side the terminal changed size.
    pub async fn resize(&self, handle: &str, columns: u32, rows: u32) -> Result<(), ShellError> {
        self.registry.resize(handle, columns, rows).await
    }

    /// Closes one terminal, and the forwards that were carrying its traffic.
    pub async fn close_terminal(&self, handle: &str) -> Result<(), ShellError> {
        self.close_tunnels_for(handle).await;
        self.registry.close(handle).await
    }

    /// The terminals that are open.
    pub async fn open_terminals(&self) -> Vec<OpenTerminal> {
        self.registry.open_sessions().await
    }

    /// The forwards that are up, with how much they have carried.
    pub async fn tunnels(&self) -> Vec<OpenTunnel> {
        let tunnels = self.tunnels.lock().await;
        let mut open: Vec<OpenTunnel> = tunnels
            .values()
            .map(|record| OpenTunnel {
                id: record.id.clone(),
                handle: record.handle.clone(),
                session_id: record.session_id.clone(),
                bind_host: record.tunnel.listener.ip().to_string(),
                bind_port: record.tunnel.listener.port(),
                mode: record.spec.mode,
                target_host: record.spec.target_host.clone(),
                target_port: record.spec.target_port,
                connections: record.tunnel.connections(),
            })
            .collect();
        open.sort_by(|left, right| left.id.cmp(&right.id));
        open
    }

    /// Stops one forward. A forward that is already gone is not an error: the caller asked for it
    /// to not be there, and it is not.
    pub async fn close_tunnel(&self, id: &str) -> Result<(), ShellError> {
        let Some(record) = self.tunnels.lock().await.remove(id) else {
            return Ok(());
        };
        record.tunnel.stop().await;
        Ok(())
    }

    /// Stops every forward belonging to one terminal.
    pub async fn close_tunnels_for(&self, handle: &str) {
        let ids: Vec<String> = {
            let tunnels = self.tunnels.lock().await;
            tunnels
                .values()
                .filter(|record| record.handle == handle)
                .map(|record| record.id.clone())
                .collect()
        };

        for id in ids {
            let _ = self.close_tunnel(&id).await;
        }
    }

    /// Closes every terminal and every forward, for when the interface goes away.
    pub async fn close_all(&self) {
        let ids: Vec<String> = self.tunnels.lock().await.keys().cloned().collect();
        for id in ids {
            let _ = self.close_tunnel(&id).await;
        }
        self.registry.close_all().await;
    }

    /// Records a host key a person has just accepted.
    pub fn trust_host_key(&self, key: &PresentedHostKey) -> Result<(), ShellError> {
        self.vault.trust_host(KnownHostEntry {
            host: key.host.clone(),
            port: key.port,
            key_type: key.key_type.clone(),
            public_key: key.public_key.clone(),
            fingerprint: key.fingerprint.clone(),
            added_at: now_seconds(),
        })
    }

    pub(crate) fn session(&self, id: &str) -> Result<Session, ShellError> {
        self.vault
            .sessions()?
            .into_iter()
            .find(|session| session.id == id)
            .ok_or_else(|| ShellError::session_not_found(id))
    }

    /// Connects every hop in order, and answers with the last one.
    ///
    /// The route is the session's own saved jump list when it has one; otherwise it is the
    /// `ProxyJump` chain its alias names, already walked outward first. Each hop is a connection of
    /// its own, verified before anything is sent through it, and authenticated with its own
    /// credential — a saved hop's stored one, or the target's for a hop the configuration names. A
    /// hop whose key is unknown ends the attempt with a decision about *that* host, so asking again
    /// after trusting it moves the chain forward one link at a time.
    ///
    /// `secret` is a password typed into the connection test. It answers for every hop whose
    /// credential is inherited from the target, which is what makes testing a draft before it is
    /// saved say something true.
    pub(crate) async fn open_route(
        &self,
        prepared: &PreparedSession,
        secret: Option<&SecretInput>,
    ) -> Result<RouteOutcome, ShellError> {
        let plan = self.route_plan(prepared)?;
        let mut current: Option<Arc<SshSession>> = None;

        for hop in plan {
            let credential = self.hop_credential(prepared, &hop, secret)?;
            let target = Target {
                host: hop.host.clone(),
                port: hop.port,
                user: hop.user.clone(),
            };

            let presented = match &current {
                Some(hop) => sevenshell_ssh::probe_host_key_via(hop, &target.host, target.port, self.timeout).await?,
                None => sevenshell_ssh::probe_host_key(&target.host, target.port, self.timeout).await?,
            };

            let system = system_known_host(&target.host, target.port, &presented);
            if system == SystemHostKey::Changed {
                return Err(ShellError::ssh_host_key_system_mismatch(
                    &target.host,
                    target.port,
                    &presented.key_type,
                    &presented.fingerprint,
                ));
            }
            match standing(
                self.vault.find_known_host(&target.host, target.port)?,
                system,
                &presented,
            ) {
                KeyStanding::Accepted => {}
                KeyStanding::Unknown => {
                    return Ok(RouteOutcome::Decision(Box::new(HostKeyDecision::Unknown(presented))));
                }
                KeyStanding::Changed(trusted) => {
                    return Ok(RouteOutcome::Decision(Box::new(HostKeyDecision::Mismatch {
                        key: presented,
                        trusted,
                    })));
                }
            }

            let route = match current {
                Some(hop) => Route::Through {
                    hop,
                    host: target.host.clone(),
                    port: target.port,
                },
                None => Route::Direct,
            };

            let request = OpenRequest {
                target,
                credential,
                // A hop carries a connection; it is not a terminal, and asking it for a shell
                // would leave a process running on the jump host for no reason.
                terminal_type: String::new(),
                keepalive_seconds: hop.keepalive_seconds,
                forward_agent: false,
                timeout: self.timeout,
                expected_host_key: presented,
                size: INITIAL_SIZE,
                route,
                purpose: Purpose::Carrier,
            };

            // A hop has no terminal, so its output has nowhere to go: the sender is dropped as
            // soon as the connection is open, which is correct — nothing reads it.
            let (output, _unused) = mpsc::channel(1);
            current = Some(Arc::new(sevenshell_ssh::SshSession::open(request, output).await?));
        }

        Ok(RouteOutcome::Ready(current))
    }

    /// Every hop from this machine outward, in the order they are connected.
    ///
    /// Saved hops keep their own routes: a hop may name jumps of its own, and that chain is walked
    /// before the hop itself. A config hop has no saved route, so its `ProxyJump` was already
    /// walked when the prepared session was built. `visiting` is the sessions on the current path,
    /// which is what turns two sessions naming each other into an error instead of a loop.
    fn route_plan(&self, prepared: &PreparedSession) -> Result<Vec<HopPlan>, ShellError> {
        let mut plan = Vec::new();
        let mut visiting = std::collections::HashSet::new();
        self.collect_hops(prepared, &mut visiting, &mut plan)?;
        Ok(plan)
    }

    fn collect_hops(
        &self,
        prepared: &PreparedSession,
        visiting: &mut std::collections::HashSet<String>,
        plan: &mut Vec<HopPlan>,
    ) -> Result<(), ShellError> {
        let session = &prepared.session;

        if !session.jump.is_empty() {
            if !visiting.insert(session.id.clone()) {
                let id = &session.id;
                return Err(ShellError::app_unsupported(&format!(
                    "a route that loops back through session {id}"
                )));
            }
            for hop_id in &session.jump {
                if hop_id == &session.id {
                    return Err(ShellError::app_unsupported("a session that is its own jump host"));
                }
                let hop = Box::new(self.prepare_session(&self.session(hop_id)?)?);
                self.collect_hops(&hop, visiting, plan)?;
                plan.push(HopPlan {
                    host: hop.session.host.clone(),
                    port: hop.session.port,
                    user: hop.session.user.clone(),
                    keepalive_seconds: hop.session.terminal.keepalive_seconds,
                    source: HopSource::Saved(hop),
                });
            }
            visiting.remove(&session.id);
            return Ok(());
        }

        for hop in &prepared.proxy {
            plan.push(HopPlan {
                host: hop.host.clone(),
                port: hop.port,
                user: hop.user.clone(),
                keepalive_seconds: session.terminal.keepalive_seconds,
                source: HopSource::Config(Box::new(hop.clone())),
            });
        }
        Ok(())
    }

    /// The credential one hop offers.
    ///
    /// A saved hop answers with what its own session stores. A hop named by the configuration has
    /// no session of its own, so it uses that hop's `IdentityFile` when it names one; otherwise it
    /// inherits the target's credential, the way one would expect a route written into the same
    /// file to share an identity.
    fn hop_credential(
        &self,
        prepared: &PreparedSession,
        hop: &HopPlan,
        secret: Option<&SecretInput>,
    ) -> Result<Credential, ShellError> {
        match &hop.source {
            HopSource::Saved(hop) => self.credential(&hop.session, hop.config.as_ref()),
            HopSource::Config(hop) => self.proxy_hop_credential(prepared, hop, secret),
        }
    }

    fn proxy_hop_credential(
        &self,
        prepared: &PreparedSession,
        hop: &crate::openssh::ProxyHop,
        secret: Option<&SecretInput>,
    ) -> Result<Credential, ShellError> {
        let session = &prepared.session;
        let method = session.auth.method;

        match method {
            AuthMethod::Password | AuthMethod::KeyboardInteractive => {
                if let Some(input) = secret {
                    return Ok(Credential::Password(input.value.clone()));
                }
                match self.vault.secret_for_session(&session.id)? {
                    Some(stored) => Ok(Credential::Password(stored.into_value())),
                    None => Err(ShellError::ssh_auth_failed(&hop.host, hop.port, "password")
                        .with_detail("the jump host inherits this session's password, and none is stored for it")),
                }
            }
            AuthMethod::PublicKey => {
                let hop_names_identity = hop
                    .config
                    .as_ref()
                    .is_some_and(|config| !config.identity_files.is_empty());
                // A key attached to the session is the person's explicit choice, and is offered to
                // the hops unless the hop's own block names the identities it wants.
                if !hop_names_identity
                    && let Some(key_ref) = session.auth.key.as_ref().and_then(|key| key.key_ref.as_deref())
                {
                    let material = self
                        .vault
                        .key_material(key_ref)?
                        .ok_or_else(|| ShellError::ssh_key_invalid("the key this session used is no longer stored"))?;
                    let (contents, passphrase) = material.into_parts();
                    return Ok(Credential::Key { contents, passphrase });
                }
                let hop_config = hop.config.clone().unwrap_or_default();
                self.key_from_config(&hop.name, &hop_config)
            }
            AuthMethod::Agent => Ok(Credential::Agent),
            AuthMethod::Pageant => Ok(Credential::Pageant),
        }
    }

    /// Starts the forwards a session saves, and reports what happened to each.
    ///
    /// A forward that cannot come up does not fail the terminal: losing a shell because a local
    /// port happened to be taken would be a strange trade. The reason travels back instead.
    async fn bring_up_forwards(&self, handle: &str, session: &Session) -> (Vec<OpenTunnel>, Vec<TunnelFailure>) {
        let mut up = Vec::new();
        let mut failed = Vec::new();

        if session.forwards.is_empty() {
            return (up, failed);
        }

        let Ok(connection) = self.registry.session_of(handle).await else {
            return (up, failed);
        };

        for (index, spec) in session.forwards.iter().enumerate() {
            let forward_id = if spec.id.is_empty() {
                format!("f{}", index + 1)
            } else {
                spec.id.clone()
            };

            let requested = HopForward {
                bind_host: spec.bind_host.clone(),
                bind_port: spec.bind_port,
                target_host: spec.target_host.clone(),
                target_port: spec.target_port,
            };

            // Which kind of listener to bring up is the whole difference: a local forward sends
            // everything to one address, a dynamic one is told by whoever connects.
            let brought_up = match spec.mode {
                ForwardMode::Local => Arc::clone(&connection).forward_local(requested).await,
                ForwardMode::Dynamic => Arc::clone(&connection).forward_dynamic(requested).await,
            };

            match brought_up {
                Ok(tunnel) => {
                    let id = format!("{handle}-{forward_id}");
                    let open = OpenTunnel {
                        id: id.clone(),
                        handle: handle.to_owned(),
                        session_id: session.id.clone(),
                        bind_host: tunnel.listener.ip().to_string(),
                        bind_port: tunnel.listener.port(),
                        mode: spec.mode,
                        target_host: spec.target_host.clone(),
                        target_port: spec.target_port,
                        connections: tunnel.connections(),
                    };

                    self.tunnels.lock().await.insert(
                        id.clone(),
                        TunnelRecord {
                            id,
                            handle: handle.to_owned(),
                            session_id: session.id.clone(),
                            spec: spec.clone(),
                            tunnel,
                        },
                    );
                    up.push(open);
                }
                Err(error) => failed.push(TunnelFailure {
                    forward_id,
                    bind_host: spec.bind_host.clone(),
                    bind_port: spec.bind_port,
                    detail: error.detail().map(str::to_owned).unwrap_or_else(|| error.to_string()),
                }),
            }
        }

        (up, failed)
    }

    /// A connection to this session for something that is not a terminal.
    ///
    /// Reuses an open terminal's connection when there is one — one authentication is enough, and
    /// closing that terminal later does not take this connection down, because the caller holds a
    /// reference of its own. Otherwise it opens a carrier: the same route, the same probe and the
    /// same verification a terminal gets, without asking the far side for a shell.
    pub(crate) async fn open_carrier(&self, session: &Session) -> Result<ConnectionOutcome, ShellError> {
        let existing = self
            .registry
            .open_sessions()
            .await
            .into_iter()
            .find(|open| open.session_id == session.id)
            .map(|open| open.handle);

        if let Some(handle) = existing {
            return Ok(ConnectionOutcome::Ready(Carrier {
                connection: self.registry.session_of(&handle).await?,
                shared: true,
            }));
        }

        self.open_fresh_carrier(session).await
    }

    /// A carrier connection of this session's own, which no terminal shares.
    ///
    /// Used when the terminal's connection is the wrong place for a new channel: one SSH
    /// connection carries only so many, and the terminal still needs room for its monitor.
    pub(crate) async fn open_fresh_carrier(&self, session: &Session) -> Result<ConnectionOutcome, ShellError> {
        let prepared = self.prepare_session(session)?;
        let session = &prepared.session;
        let credential = self.credential(session, prepared.config.as_ref())?;
        let target = Target {
            host: session.host.clone(),
            port: session.port,
            user: session.user.clone(),
        };

        let hop = match self.open_route(&prepared, None).await? {
            RouteOutcome::Ready(hop) => hop,
            RouteOutcome::Decision(decision) => return Ok(ConnectionOutcome::Decision(decision)),
        };

        let presented = match &hop {
            Some(hop) => sevenshell_ssh::probe_host_key_via(hop, &target.host, target.port, self.timeout).await?,
            None => sevenshell_ssh::probe_host_key(&target.host, target.port, self.timeout).await?,
        };

        let system = system_known_host(&target.host, target.port, &presented);
        if system == SystemHostKey::Changed {
            return Err(ShellError::ssh_host_key_system_mismatch(
                &target.host,
                target.port,
                &presented.key_type,
                &presented.fingerprint,
            ));
        }
        match standing(
            self.vault.find_known_host(&target.host, target.port)?,
            system,
            &presented,
        ) {
            KeyStanding::Accepted => {}
            KeyStanding::Unknown => {
                return Ok(ConnectionOutcome::Decision(Box::new(HostKeyDecision::Unknown(
                    presented,
                ))));
            }
            KeyStanding::Changed(trusted) => {
                return Ok(ConnectionOutcome::Decision(Box::new(HostKeyDecision::Mismatch {
                    key: presented,
                    trusted,
                })));
            }
        }

        let route = match hop {
            Some(hop) => Route::Through {
                hop,
                host: target.host.clone(),
                port: target.port,
            },
            None => Route::Direct,
        };

        let request = OpenRequest {
            target,
            credential,
            terminal_type: String::new(),
            keepalive_seconds: session.terminal.keepalive_seconds,
            forward_agent: false,
            timeout: self.timeout,
            expected_host_key: presented,
            size: INITIAL_SIZE,
            route,
            // A carrier: what is coming is a channel, not a shell.
            purpose: Purpose::Carrier,
        };

        let (output, _unused) = mpsc::channel(1);
        Ok(ConnectionOutcome::Ready(Carrier {
            connection: Arc::new(SshSession::open(request, output).await?),
            shared: false,
        }))
    }

    /// Where this session's recording goes, when it is being recorded.
    ///
    /// The name comes from the session and the clock, and both are cleaned up on the way into a
    /// path: a session may be called anything at all, and a name with a slash in it would put the
    /// recording somewhere nobody chose.
    fn recording_for(&self, session: &Session) -> Result<Option<RecordingSpec>, ShellError> {
        if !session.terminal.record {
            return Ok(None);
        }

        let directory = sevenshell_platform::config_dir().ok_or_else(|| {
            ShellError::store_io("the configuration directory could not be located, so a recording has nowhere to go")
        })?;

        let stamp = now_seconds();
        let cleaned: String = session
            .name
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character
                } else {
                    '-'
                }
            })
            .collect();
        let name = if cleaned.trim_matches('-').is_empty() {
            "session".to_owned()
        } else {
            cleaned.trim_matches('-').to_owned()
        };

        Ok(Some(RecordingSpec {
            path: directory.join("recordings").join(format!("{name}-{stamp}.cast")),
            width: INITIAL_SIZE.columns,
            height: INITIAL_SIZE.rows,
            title: session.name.clone(),
        }))
    }

    /// The session a connection uses, with an OpenSSH alias resolved to its real target.
    ///
    /// A session that does not use the configuration is returned as it was saved. One that does is
    /// resolved again on every connection, so editing `~/.ssh/config` changes the next connection
    /// without editing the session. Every directive that would change where the connection goes and
    /// that this build cannot honour refuses here, before a socket.
    ///
    /// A `ProxyJump` becomes the route unless the session already names saved jumps: those are the
    /// person's explicit route, and a file must not silently replace them.
    pub(crate) fn prepare_session(&self, session: &Session) -> Result<PreparedSession, ShellError> {
        if !session.auth.ssh_config || session.protocol != Protocol::Ssh {
            return Ok(PreparedSession {
                session: session.clone(),
                config: None,
                proxy: Vec::new(),
            });
        }

        let Some(config) = crate::openssh::config() else {
            return Err(ShellError::app_unsupported("OpenSSH's configuration directory"));
        };
        let resolved = config.resolve(&session.host);
        if !resolved.matched {
            return Err(ShellError::ssh_config_unknown_alias(&session.host));
        }

        let mut unsupported = resolved.unsupported.clone();
        if resolved.proxy_command.is_some() && !unsupported.iter().any(|name| name == "ProxyCommand") {
            unsupported.push("ProxyCommand".to_owned());
        }
        if let Some(directive) = unsupported.first() {
            return Err(ShellError::ssh_config_unsupported(directive));
        }

        let mut prepared = session.clone();
        prepared.host = resolved.address(&session.host);
        if let Some(user) = &resolved.user {
            prepared.user = user.clone();
        }
        if let Some(port) = resolved.port {
            prepared.port = port;
        }

        let proxy = if prepared.jump.is_empty() {
            crate::openssh::proxy_hops(&config, &resolved, &prepared.user).map_err(proxy_chain_error)?
        } else {
            Vec::new()
        };

        Ok(PreparedSession {
            session: prepared,
            config: Some(resolved),
            proxy,
        })
    }

    /// The real address a session reaches, for a policy layer that must not be bypassed by an
    /// alias. A session that cannot be resolved keeps its saved values; the connection that
    /// follows will refuse with the reason.
    pub(crate) fn policy_target_for(&self, session: &Session) -> (String, u16) {
        match self.prepare_session(session) {
            Ok(prepared) => (prepared.session.host, prepared.session.port),
            Err(_) => (session.host.clone(), session.port),
        }
    }

    /// Collects what a session authenticates with. The material is held only long enough to be
    /// handed to the transport, which zeroizes it when the connection is done with it.
    pub(crate) fn credential(
        &self,
        session: &Session,
        config: Option<&sevenshell_ssh::openssh::ResolvedHost>,
    ) -> Result<Credential, ShellError> {
        let method = session.auth.method;

        match method {
            AuthMethod::Password | AuthMethod::KeyboardInteractive => {
                match self.vault.secret_for_session(&session.id)? {
                    // Moved rather than copied: the transport takes ownership and wipes it when the
                    // connection is done with it.
                    Some(secret) => Ok(Credential::Password(secret.into_value())),
                    None => Err(
                        ShellError::ssh_auth_failed(&session.host, session.port, method.as_str())
                            .with_detail("no password is stored for this session"),
                    ),
                }
            }
            AuthMethod::PublicKey => {
                if let Some(key_ref) = session.auth.key.as_ref().and_then(|key| key.key_ref.as_deref()) {
                    let material = self
                        .vault
                        .key_material(key_ref)?
                        .ok_or_else(|| ShellError::ssh_key_invalid("the key this session used is no longer stored"))?;

                    let (contents, passphrase) = material.into_parts();
                    return Ok(Credential::Key { contents, passphrase });
                }

                // A session that takes its identity from OpenSSH's configuration names files, not
                // vault handles. They are read here and never leave the process.
                if let Some(config) = config {
                    return self.key_from_config(&session.host, config);
                }

                Err(ShellError::ssh_key_invalid("this session has no key attached"))
            }
            // Two different things, kept apart on purpose: the machine's own agent is asked for a
            // signature, while Pageant speaks a protocol this build does not — and saying *which*
            // one is missing is what tells somebody whether to start a program or stop looking.
            AuthMethod::Agent => Ok(Credential::Agent),
            AuthMethod::Pageant => Ok(Credential::Pageant),
        }
    }

    /// Reads the first usable identity the configuration names, then OpenSSH's defaults.
    ///
    /// An encrypted key is reported by its file name only: this build has no interactive
    /// passphrase step in this path, and guessing would either skip a key the person meant to use
    /// or ask for a secret in a place that cannot ask. The answer names the alternative — the
    /// agent, or importing the key into the vault.
    fn key_from_config(
        &self,
        alias: &str,
        config: &sevenshell_ssh::openssh::ResolvedHost,
    ) -> Result<Credential, ShellError> {
        let Some(home) = sevenshell_platform::home_dir() else {
            return Err(ShellError::ssh_config_no_key(alias));
        };
        let ssh_dir = sevenshell_platform::ssh_dir().unwrap_or_else(|| home.join(".ssh"));

        let mut candidates: Vec<std::path::PathBuf> = config
            .identity_files
            .iter()
            .map(|value| identity_path(value, &home, &ssh_dir))
            .collect();
        for name in sevenshell_ssh::openssh::DEFAULT_IDENTITY_FILES {
            candidates.push(ssh_dir.join(name));
        }

        let mut locked: Option<String> = None;
        for path in candidates {
            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            if !metadata.is_file() || metadata.len() > MAX_IDENTITY_BYTES {
                continue;
            }
            let Ok(contents) = std::fs::read_to_string(&path) else {
                continue;
            };
            match sevenshell_ssh::describe_key(&contents, None) {
                Ok(description) if !description.has_passphrase => {
                    return Ok(Credential::Key {
                        contents,
                        passphrase: None,
                    });
                }
                Ok(_) => {
                    if locked.is_none() {
                        locked = path.file_name().map(|name| name.to_string_lossy().into_owned());
                    }
                }
                Err(_) => {}
            }
        }

        match locked {
            Some(name) => Err(ShellError::ssh_config_key_passphrase(&name)),
            None => Err(ShellError::ssh_config_no_key(alias)),
        }
    }
}

/// Either the end of a route, or the decision that has to be made before it can be finished.
pub(crate) enum RouteOutcome {
    Ready(Option<Arc<SshSession>>),
    /// Boxed because a decision carries keys and fingerprints, and it is the rare answer.
    Decision(Box<HostKeyDecision>),
}

/// A connection that carries something other than a terminal, and whether a terminal shares it.
///
/// `shared` matters because one SSH connection has a channel budget: OpenSSH's default
/// `MaxSessions` is ten, and a terminal's connection is also where its monitor runs its one-shot
/// commands. A caller that wants to keep a channel open for a while checks this before taking one.
pub(crate) struct Carrier {
    pub connection: Arc<SshSession>,
    pub shared: bool,
}

/// Either a connection that carries something other than a terminal, or the decision that stands
/// in the way of opening one.
pub(crate) enum ConnectionOutcome {
    Ready(Carrier),
    Decision(Box<HostKeyDecision>),
}

/// The terminal's answer to a decision the route could not make on its own.
fn terminal_decision(decision: HostKeyDecision) -> OpenOutcome {
    match decision {
        HostKeyDecision::Unknown(key) => OpenOutcome::HostKeyUnknown { key },
        HostKeyDecision::Mismatch { key, trusted } => OpenOutcome::HostKeyMismatch { key, trusted },
    }
}

/// The same two decisions, in the vocabulary a connection test answers in.
fn test_decision(decision: HostKeyDecision) -> ConnectionTestOutcome {
    match decision {
        HostKeyDecision::Unknown(key) => ConnectionTestOutcome::HostKeyUnknown { key },
        HostKeyDecision::Mismatch { key, trusted } => ConnectionTestOutcome::HostKeyMismatch { key, trusted },
    }
}

/// A stopwatch reading as whole milliseconds, which is the precision the interface prints.
fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// A `ProxyJump` chain that cannot be followed, in the vocabulary of the error envelope.
fn proxy_chain_error(error: crate::openssh::ProxyChainError) -> ShellError {
    use crate::openssh::ProxyChainError;

    match error {
        ProxyChainError::Directive(directive) => ShellError::ssh_config_unsupported(&directive),
        ProxyChainError::Invalid(hop) => ShellError::ssh_config_proxy_chain(&hop),
        ProxyChainError::Loop(chain) => ShellError::ssh_config_proxy_chain(&chain),
    }
}

pub(crate) fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

/// One step of an open, unless it is called off first.
///
/// The step is a future rather than a closure, so a cancellation *drops* it at an await point: the
/// socket being dialled is closed and whatever it had already built goes with it. What that must
/// not swallow — a terminal that made it into the registry — is handled by the caller.
async fn until_cancelled<T>(
    step: impl std::future::Future<Output = Result<T, ShellError>>,
    cancel: &Arc<Notify>,
) -> Result<T, ShellError> {
    tokio::select! {
        result = step => result,
        () = cancel.notified() => Err(ShellError::session_open_cancelled()),
    }
}

/// A terminal's text in the bytes its far side prints and expects.
fn encoded_text(text: &str, encoding: TerminalEncoding) -> Vec<u8> {
    match encoding {
        TerminalEncoding::Utf8 => text.as_bytes().to_vec(),
        TerminalEncoding::Gbk => encoding_rs::GBK.encode(text).0.into_owned(),
        TerminalEncoding::Gb18030 => encoding_rs::GB18030.encode(text).0.into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use sevenshell_types::{AuthMethod, SessionAuth, TerminalSettings};
    use tempfile::{TempDir, tempdir};

    use super::*;

    /// Returns the temp directory as well as the vault: dropping it would take the vault file with it.
    fn vault_with_a_session() -> (TempDir, Arc<Vault>, String) {
        let directory = tempdir().expect("temp dir");
        let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
        vault.create("correct horse battery staple").expect("create");

        vault.save_session(sample("session-1", "example"), None).expect("save");

        (directory, vault, "session-1".to_owned())
    }

    fn sample(id: &str, name: &str) -> Session {
        Session {
            id: id.to_owned(),
            name: name.to_owned(),
            group_id: None,
            order: None,
            protocol: sevenshell_types::Protocol::Ssh,
            host: "127.0.0.1".to_owned(),
            port: 2222,
            user: "someone".to_owned(),
            auth: SessionAuth::with_password(AuthMethod::Password),
            terminal: TerminalSettings::default(),
            desktop: sevenshell_types::DesktopSettings::default(),
            telnet: sevenshell_types::TelnetSettings::default(),
            serial: sevenshell_types::SerialSettings::default(),
            local: sevenshell_types::LocalSettings::default(),
            wsl: sevenshell_types::WslSettings::default(),
            container: sevenshell_types::ContainerSettings::default(),
            kubernetes: sevenshell_types::KubernetesSettings::default(),
            webdav: sevenshell_types::WebDavSettings::default(),
            ftp: sevenshell_types::FtpSettings::default(),
            vmware: sevenshell_types::VmwareSettings::default(),
            jump: Vec::new(),
            forwards: Vec::new(),
            updated_at: 0,
        }
    }

    /// A Chinese switch expects two bytes for a character an xterm would send as three.
    #[test]
    fn typed_text_becomes_the_bytes_the_session_s_encoding_names() {
        assert_eq!(encoded_text("中文", TerminalEncoding::Utf8), "中文".as_bytes());
        assert_eq!(
            encoded_text("中文", TerminalEncoding::Gbk),
            vec![0xD6, 0xD0, 0xCE, 0xC4]
        );
        assert_eq!(
            encoded_text("😀", TerminalEncoding::Gb18030).len(),
            4,
            "GB18030 reaches every scalar as four bytes at most; GBK would have replaced this one"
        );
    }

    #[tokio::test]
    async fn a_local_shell_opens_and_closes_through_the_registry_without_a_host_key() {
        let directory = tempdir().expect("temp dir");
        let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
        vault.create("correct horse battery staple").expect("create");
        let mut session = sample("local-1", "a shell here");
        session.protocol = sevenshell_types::Protocol::Local;
        session.host = String::new();
        session.port = 0;
        session.user = String::new();
        session.local.shell = if cfg!(windows) { "cmd".to_owned() } else { String::new() };
        vault.save_session(session, None).expect("save");

        let (core, _events) = Core::new(vault);
        let outcome = core.open_terminal("local-1").await.expect("a local shell opens");
        let OpenOutcome::Opened { handle, .. } = outcome else {
            panic!("a local shell needs no host key decision, got {outcome:?}");
        };
        core.close_terminal(&handle).await.expect("the shell closes");
    }

    #[tokio::test]
    async fn opening_a_session_that_is_not_saved_says_so() {
        let (_directory, vault, _) = vault_with_a_session();
        let (core, _events) = Core::new(vault);

        let error = core
            .open_terminal("session-that-never-existed")
            .await
            .expect_err("no such session");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SessionNotFound);
    }

    #[tokio::test]
    async fn a_raw_tcp_test_reports_reachability_without_authenticating() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a free port");
        let port = listener.local_addr().expect("the bound address").port();
        let (_directory, vault, _) = vault_with_a_session();
        let (core, _events) = Core::new(vault);
        let mut session = sample("raw", "raw");
        session.protocol = sevenshell_types::Protocol::RawTcp;
        session.port = port;
        session.user = String::new();

        let outcome = core.test_session(&session, None).await.expect("the address answers");
        match outcome {
            ConnectionTestOutcome::Connected {
                authenticated,
                fingerprint,
                ..
            } => {
                assert!(!authenticated, "a raw TCP console has no login to authenticate");
                assert_eq!(fingerprint, None, "and no key to report");
            }
            other => panic!("expected a connected test, got {other:?}"),
        }

        drop(listener);
        let error = core
            .test_session(&session, None)
            .await
            .expect_err("nothing is listening");
        assert_eq!(error.code(), sevenshell_error::ErrorCode::TerminalUnreachable);
    }

    #[tokio::test]
    async fn a_session_without_a_stored_password_fails_before_any_network_use() {
        let (_directory, vault, id) = vault_with_a_session();
        let (core, _events) = Core::new(vault);

        // Nothing is listening on the port in the fixture, so a network attempt would surface as
        // unreachable. Getting the credential error instead proves the order is local-first.
        let error = core.open_terminal(&id).await.expect_err("no password stored");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SshAuthFailed);
        assert_eq!(error.detail(), Some("no password is stored for this session"));
    }

    #[tokio::test]
    async fn a_locked_vault_stops_a_session_from_opening() {
        let (_directory, vault, id) = vault_with_a_session();
        let (core, _events) = Core::new(Arc::clone(&vault));
        vault.lock();

        let error = core.open_terminal(&id).await.expect_err("locked");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::VaultLocked);
    }

    #[tokio::test]
    async fn a_hop_that_is_not_saved_is_named_in_the_failure() {
        let (_directory, vault, id) = vault_with_a_session();
        let mut session = sample("session-2", "behind-a-ghost");
        session.jump = vec!["session-that-never-existed".to_owned()];
        vault
            .save_session(
                session,
                Some(sevenshell_vault::SessionSecret::Password("hunter2".to_owned())),
            )
            .expect("save");

        let (core, _events) = Core::new(vault);
        let error = core.open_terminal("session-2").await.expect_err("the hop is gone");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SessionNotFound);
        assert_eq!(
            error.diagnostics().get("sessionId").map(String::as_str),
            Some("session-that-never-existed")
        );
        assert_ne!(
            error.diagnostics().get("sessionId").map(String::as_str),
            Some(id.as_str())
        );
    }

    #[tokio::test]
    async fn a_session_that_is_its_own_jump_host_is_refused() {
        let (_directory, vault, id) = vault_with_a_session();
        let mut session = sample("session-3", "loop");
        session.jump = vec!["session-3".to_owned()];
        vault
            .save_session(
                session,
                Some(sevenshell_vault::SessionSecret::Password("hunter2".to_owned())),
            )
            .expect("save");

        let (core, _events) = Core::new(vault);
        let error = core
            .open_terminal("session-3")
            .await
            .expect_err("a loop is not a route");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::AppUnsupported);
        assert!(id != "session-3");
    }

    #[tokio::test]
    async fn stopping_a_tunnel_that_is_not_there_is_not_an_error() {
        let (_directory, vault, _) = vault_with_a_session();
        let (core, _events) = Core::new(vault);

        assert!(core.close_tunnel("never-existed").await.is_ok());
        assert!(core.tunnels().await.is_empty());
        assert!(core.open_terminals().await.is_empty());
    }

    #[test]
    fn trusting_a_key_records_it_for_that_host_and_port() {
        let (_directory, vault, _) = vault_with_a_session();
        let (core, _events) = Core::new(Arc::clone(&vault));

        core.trust_host_key(&PresentedHostKey {
            host: "example.test".to_owned(),
            port: 22,
            key_type: "ssh-ed25519".to_owned(),
            public_key: "AAAAB3NzaC1lZDI1NTE5AAAAIexample".to_owned(),
            fingerprint: "SHA256:example".to_owned(),
        })
        .expect("trust");

        let stored = vault
            .find_known_host("example.test", 22)
            .expect("read")
            .expect("stored");
        assert_eq!(stored.fingerprint, "SHA256:example");
        assert!(stored.added_at > 0, "the entry is timestamped");
        assert!(
            vault.find_known_host("example.test", 2222).expect("read").is_none(),
            "ports are not interchangeable"
        );
    }

    #[tokio::test]
    async fn calling_off_an_open_that_is_not_running_says_so() {
        // The answer is what the interface shows: a click that cancelled nothing must not say it
        // stopped something.
        let (_directory, vault, _) = vault_with_a_session();
        let (core, _events) = Core::new(vault);

        assert!(!core.cancel_open("session-1").await);
    }

    #[tokio::test]
    async fn a_called_off_open_stops_before_it_gets_a_terminal() {
        // A listener that accepts and then says nothing: the SSH handshake waits for a banner that
        // never arrives, which is a connect that hangs — exactly what somebody looking at a row
        // that says "connecting" wants to call off.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a free port");
        let port = listener.local_addr().expect("the bound address").port();
        tokio::spawn(async move {
            // Held rather than dropped: accepting and letting the stream go would close the socket,
            // and a probe that sees the far side hang up fails instead of waiting.
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                held.push(stream);
            }
        });

        let (_directory, vault, _) = vault_with_a_session();
        let mut hanging = sample("session-1", "hanging");
        hanging.port = port;
        // With a password to send, the attempt gets as far as the socket: a session with no
        // credential is refused before anything is dialled, which is not what this is about.
        vault
            .save_session(
                hanging,
                Some(sevenshell_vault::SessionSecret::Password("hunter2".to_owned())),
            )
            .expect("save");
        let (core, _events) = Core::new(vault);

        let opening = tokio::spawn({
            let core = core.clone();
            async move { core.open_terminal("session-1").await }
        });

        // Long enough for the socket to be dialled, far inside the 30 second budget.
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(core.cancel_open("session-1").await, "there is an attempt to call off");

        let error = opening
            .await
            .expect("the task ran")
            .expect_err("no terminal came out of it");
        assert_eq!(error.code(), sevenshell_error::ErrorCode::SessionOpenCancelled);
        assert!(
            core.registry.open_sessions().await.is_empty(),
            "and the attempt left nothing behind it"
        );
    }

    #[tokio::test]
    async fn a_cancelled_open_is_forgotten_so_the_next_one_works() {
        // The token is per attempt: cancelling once must not leave the session marked as busy, or
        // the row would stop being openable at all.
        let (_directory, vault, _) = vault_with_a_session();
        let (core, _events) = Core::new(vault);

        let opening = tokio::spawn({
            let core = core.clone();
            async move { core.open_terminal("session-1").await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = core.cancel_open("session-1").await;
        let _ = opening.await;

        assert!(!core.cancel_open("session-1").await, "nothing is in flight any more");
    }

    /// The SSH-directory override is process-wide, so these tests take turns.
    static SSH_ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct SshDir {
        previous: Option<std::ffi::OsString>,
    }

    impl SshDir {
        fn at(path: &std::path::Path) -> Self {
            let previous = std::env::var_os(sevenshell_platform::SSH_DIR_OVERRIDE);
            unsafe { std::env::set_var(sevenshell_platform::SSH_DIR_OVERRIDE, path) };
            Self { previous }
        }
    }

    impl Drop for SshDir {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => unsafe { std::env::set_var(sevenshell_platform::SSH_DIR_OVERRIDE, value) },
                None => unsafe { std::env::remove_var(sevenshell_platform::SSH_DIR_OVERRIDE) },
            }
        }
    }

    fn ssh_config(text: &str) -> (TempDir, SshDir) {
        let directory = tempdir().expect("temp dir");
        let ssh = directory.path().join("ssh");
        std::fs::create_dir_all(&ssh).expect("ssh dir");
        std::fs::write(ssh.join("config"), text).expect("config");
        let env = SshDir::at(&ssh);
        (directory, env)
    }

    fn aliased(method: AuthMethod) -> Session {
        let mut session = sample("web-1", "web");
        session.host = "web".to_owned();
        session.port = 22;
        session.user = "placeholder".to_owned();
        session.auth = SessionAuth {
            method,
            key: None,
            has_secret: false,
            ssh_config: true,
        };
        session
    }

    fn core_with(session: Session) -> (TempDir, Arc<Core>) {
        let directory = tempdir().expect("temp dir");
        let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
        vault.create("correct horse battery staple").expect("create");
        vault.save_session(session, None).expect("save");
        (directory, Core::new(vault).0)
    }

    #[test]
    fn a_session_that_uses_openssh_resolves_its_alias_by_config() {
        let _guard = SSH_ENV.lock().expect("the environment lock");
        let (_directory, _env) = ssh_config("Host web\n  HostName 10.0.0.5\n  User deploy\n  Port 2222\n");
        let (_vault_dir, core) = core_with(aliased(AuthMethod::Agent));

        let prepared = core
            .prepare_session(&core.session("web-1").expect("session"))
            .expect("the alias resolves");

        assert_eq!(prepared.session.host, "10.0.0.5");
        assert_eq!(prepared.session.user, "deploy");
        assert_eq!(prepared.session.port, 2222);
    }

    #[test]
    fn a_proxy_jump_becomes_the_route_the_connection_will_take() {
        let _guard = SSH_ENV.lock().expect("the environment lock");
        let (_directory, _env) = ssh_config(
            "Host web\n  HostName 10.0.0.5\n  ProxyJump bastion\n\
             Host bastion\n  HostName 10.0.0.9\n  Port 2200\n  User ops\n",
        );
        let (_vault_dir, core) = core_with(aliased(AuthMethod::Agent));

        let prepared = core
            .prepare_session(&core.session("web-1").expect("session"))
            .expect("the alias resolves");

        assert_eq!(prepared.session.host, "10.0.0.5");
        assert_eq!(prepared.proxy.len(), 1);
        assert_eq!(prepared.proxy[0].host, "10.0.0.9");
        assert_eq!(prepared.proxy[0].port, 2200);
        assert_eq!(prepared.proxy[0].user, "ops");
    }

    #[test]
    fn saved_jumps_win_over_a_route_in_the_file() {
        let _guard = SSH_ENV.lock().expect("the environment lock");
        let (_directory, _env) =
            ssh_config("Host web\n  HostName 10.0.0.5\n  ProxyJump bastion\nHost bastion\n  HostName 10.0.0.9\n");

        let mut leaf = aliased(AuthMethod::Agent);
        leaf.jump = vec!["hop".to_owned()];
        let mut hop = sample("hop", "bastion");
        hop.host = "10.0.0.9".to_owned();

        let directory = tempdir().expect("temp dir");
        let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
        vault.create("correct horse battery staple").expect("create");
        vault.save_session(leaf, None).expect("save the leaf");
        vault.save_session(hop, None).expect("save the hop");
        let core = Core::new(vault).0;

        let prepared = core
            .prepare_session(&core.session("web-1").expect("session"))
            .expect("the alias resolves");

        assert!(
            prepared.proxy.is_empty(),
            "the saved jumps are the person's own route, not the file's"
        );
    }

    #[test]
    fn a_proxy_cycle_refuses_with_the_chain_it_walked() {
        let _guard = SSH_ENV.lock().expect("the environment lock");
        let (_directory, _env) = ssh_config("Host web\n  ProxyJump gateway\nHost gateway\n  ProxyJump web\n");
        let (_vault_dir, core) = core_with(aliased(AuthMethod::Agent));

        let error = core
            .prepare_session(&core.session("web-1").expect("session"))
            .expect_err("a cycle is not a route");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SshConfigProxyChain);
    }

    #[test]
    fn a_proxy_directive_refuses_before_a_socket_is_dialled() {
        let _guard = SSH_ENV.lock().expect("the environment lock");
        let (_directory, _env) = ssh_config("Host web\n  ProxyCommand ssh -W %h:%p bastion\n");
        let (_vault_dir, core) = core_with(aliased(AuthMethod::Agent));

        let error = core
            .prepare_session(&core.session("web-1").expect("session"))
            .expect_err("a ProxyCommand is not guessed at");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SshConfigUnsupported);
    }

    #[test]
    fn an_alias_the_configuration_does_not_name_is_refused_by_name() {
        let _guard = SSH_ENV.lock().expect("the environment lock");
        let (_directory, _env) = ssh_config("Host other\n  HostName 10.0.0.9\n");
        let (_vault_dir, core) = core_with(aliased(AuthMethod::Agent));

        let error = core
            .prepare_session(&core.session("web-1").expect("session"))
            .expect_err("the alias is not in the file");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SshConfigUnknownAlias);
    }

    #[test]
    fn an_identity_named_by_the_config_authenticates_without_a_vault_key() {
        let _guard = SSH_ENV.lock().expect("the environment lock");
        let directory = tempdir().expect("temp dir");
        let ssh = directory.path().join("ssh");
        std::fs::create_dir_all(&ssh).expect("ssh dir");
        std::fs::write(ssh.join("config"), "Host web\n  IdentityFile ~/.ssh/id_test\n").expect("config");
        let generated =
            sevenshell_ssh::generate(sevenshell_types::KeyAlgorithm::Ed25519, "test", None).expect("a generated key");
        std::fs::write(ssh.join("id_test"), generated.private_text()).expect("the key file");
        let _env = SshDir::at(&ssh);

        let (_vault_dir, core) = core_with(aliased(AuthMethod::PublicKey));
        let prepared = core
            .prepare_session(&core.session("web-1").expect("session"))
            .expect("the alias resolves");
        let credential = core
            .credential(&prepared.session, prepared.config.as_ref())
            .expect("the key file is read");

        let Credential::Key { contents, passphrase } = &credential else {
            panic!("the configuration names a key file");
        };
        assert!(contents.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
        assert!(passphrase.is_none());
    }

    #[test]
    fn an_encrypted_identity_is_named_rather_than_silently_skipped() {
        let _guard = SSH_ENV.lock().expect("the environment lock");
        let directory = tempdir().expect("temp dir");
        let ssh = directory.path().join("ssh");
        std::fs::create_dir_all(&ssh).expect("ssh dir");
        std::fs::write(ssh.join("config"), "Host web\n  IdentityFile ~/.ssh/id_locked\n").expect("config");
        let generated = sevenshell_ssh::generate(sevenshell_types::KeyAlgorithm::Ed25519, "test", Some("a passphrase"))
            .expect("a generated key");
        std::fs::write(ssh.join("id_locked"), generated.private_text()).expect("the key file");
        let _env = SshDir::at(&ssh);

        let (_vault_dir, core) = core_with(aliased(AuthMethod::PublicKey));
        let prepared = core
            .prepare_session(&core.session("web-1").expect("session"))
            .expect("the alias resolves");
        let error = core
            .credential(&prepared.session, prepared.config.as_ref())
            .expect_err("a locked key cannot be used without a passphrase");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SshConfigKeyPassphrase);
    }

    #[test]
    fn a_key_in_the_system_known_hosts_is_trusted_without_the_vault_asking() {
        let _guard = SSH_ENV.lock().expect("the environment lock");
        let directory = tempdir().expect("temp dir");
        let ssh = directory.path().join("ssh");
        std::fs::create_dir_all(&ssh).expect("ssh dir");
        let generated =
            sevenshell_ssh::generate(sevenshell_types::KeyAlgorithm::Ed25519, "test", None).expect("a generated key");
        let mut parts = generated.public_line().split_whitespace();
        let key_type = parts.next().expect("a key type").to_owned();
        let public_key = parts.next().expect("a public key").to_owned();
        std::fs::write(
            ssh.join("known_hosts"),
            format!("web.example {key_type} {public_key}\n"),
        )
        .expect("known_hosts");
        let _env = SshDir::at(&ssh);

        let presented = PresentedHostKey {
            host: "web.example".to_owned(),
            port: 22,
            key_type,
            public_key,
            fingerprint: String::new(),
        };

        assert_eq!(system_known_host("web.example", 22, &presented), SystemHostKey::Trusted);
    }

    fn as_presented(key_type: &str, public_key: &str) -> PresentedHostKey {
        PresentedHostKey {
            host: "web.example".to_owned(),
            port: 22,
            key_type: key_type.to_owned(),
            public_key: public_key.to_owned(),
            fingerprint: String::new(),
        }
    }

    fn as_recorded(key_type: &str, public_key: &str) -> KnownHostEntry {
        KnownHostEntry {
            host: "web.example".to_owned(),
            port: 22,
            key_type: key_type.to_owned(),
            public_key: public_key.to_owned(),
            fingerprint: String::new(),
            added_at: 0,
        }
    }

    #[test]
    fn a_record_of_another_algorithm_asks_instead_of_stopping() {
        let presented = as_presented("ssh-ed25519", "AAAA");
        let answer = standing(Some(as_recorded("ssh-rsa", "BBBB")), SystemHostKey::Unknown, &presented);
        assert!(
            matches!(answer, KeyStanding::Unknown),
            "a record of another algorithm says nothing about this key"
        );

        let answer = standing(Some(as_recorded("ssh-rsa", "BBBB")), SystemHostKey::Trusted, &presented);
        assert!(
            matches!(answer, KeyStanding::Accepted),
            "the system file knows this very key, so there is nothing to ask"
        );
    }

    #[test]
    fn a_record_of_the_same_algorithm_with_other_material_is_a_stop() {
        let presented = as_presented("ssh-ed25519", "AAAA");
        let answer = standing(
            Some(as_recorded("ssh-ed25519", "BBBB")),
            SystemHostKey::Unknown,
            &presented,
        );
        assert!(matches!(answer, KeyStanding::Changed(_)));
    }

    #[test]
    fn a_matching_vault_record_is_accepted() {
        let presented = as_presented("ssh-ed25519", "AAAA");
        let answer = standing(
            Some(as_recorded("ssh-ed25519", "AAAA")),
            SystemHostKey::Unknown,
            &presented,
        );
        assert!(matches!(answer, KeyStanding::Accepted));
    }
}
