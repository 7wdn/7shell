//! Opening, driving and closing terminals, the routes they take, and the forwards they bring up.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sevenshell_error::ShellError;
use sevenshell_session::{OpenTerminal, RecordingSpec, SessionRegistry};
use sevenshell_ssh::{
    Credential, LocalForward as HopForward, OpenRequest, Purpose, Route, SshSession, Target, TerminalSize, TunnelHandle,
};
use sevenshell_types::{
    AuthMethod, ForwardMode, KnownHostEntry, LocalForward, OpenOutcome, OpenTunnel, PresentedHostKey, Session,
    TransferInfo, TunnelFailure,
};
use sevenshell_vault::Vault;
use tokio::sync::{Mutex, mpsc};

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

/// A transfer that is running, or has finished running.
pub struct TransferRecord {
    pub(crate) info: TransferInfo,
    pub(crate) progress: Arc<sevenshell_sftp::TransferProgress>,
    /// The task driving it, so a cancellation can wait for the file handles to be released.
    pub(crate) task: tokio::task::JoinHandle<()>,
}

/// A forward that is up, and what it belongs to.
struct TunnelRecord {
    id: String,
    handle: String,
    session_id: String,
    spec: LocalForward,
    tunnel: TunnelHandle,
}

/// The operations that need the vault, the transport and the registry in a particular order.
pub struct Core {
    pub(crate) vault: Arc<Vault>,
    pub(crate) registry: Arc<SessionRegistry>,
    pub(crate) files: Mutex<HashMap<String, Arc<crate::FileSession>>>,
    /// Behind an `Arc` as well as a mutex: the task driving a transfer updates its own state
    /// long after the call that started it has returned.
    pub(crate) transfers: Arc<Mutex<HashMap<String, crate::TransferRecord>>>,
    pub(crate) next_file: std::sync::atomic::AtomicU64,
    pub(crate) next_transfer: std::sync::atomic::AtomicU64,
    tunnels: Mutex<HashMap<String, TunnelRecord>>,
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
                next_file: std::sync::atomic::AtomicU64::new(1),
                next_transfer: std::sync::atomic::AtomicU64::new(1),
                tunnels: Mutex::new(HashMap::new()),
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
        let session = self.session(session_id)?;
        let credential = self.credential(&session)?;
        let target = Target {
            host: session.host.clone(),
            port: session.port,
            user: session.user.clone(),
        };

        let hop = match self.open_route(&session).await? {
            RouteOutcome::Ready(hop) => hop,
            RouteOutcome::Decision(decision) => return Ok(terminal_decision(*decision)),
        };

        let presented = match &hop {
            Some(hop) => sevenshell_ssh::probe_host_key_via(hop, &target.host, target.port, self.timeout).await?,
            None => sevenshell_ssh::probe_host_key(&target.host, target.port, self.timeout).await?,
        };

        match self.vault.find_known_host(&target.host, target.port)? {
            None => return Ok(OpenOutcome::HostKeyUnknown { key: presented }),
            Some(trusted) if trusted.public_key != presented.public_key => {
                return Ok(OpenOutcome::HostKeyMismatch {
                    key: presented,
                    trusted,
                });
            }
            Some(_) => {}
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

        let handle = self
            .registry
            .open(session_id, request, self.recording_for(&session)?)
            .await?;
        let (tunnels, tunnel_failures) = self.bring_up_forwards(&handle, &session).await;

        Ok(OpenOutcome::Opened {
            handle,
            session_id: session_id.to_owned(),
            tunnels,
            tunnel_failures,
        })
    }

    /// Sends keystrokes or pasted text.
    pub async fn write(&self, handle: &str, data: &[u8]) -> Result<(), ShellError> {
        self.registry.write(handle, data).await
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

    /// Connects every jump host in order, and answers with the last one.
    ///
    /// Each hop is a connection of its own, on the same terms as the final one: verified before
    /// anything is sent through it, and authenticated with its own saved credential. A hop whose
    /// key is unknown ends the attempt with a decision about *that* host, so asking again after
    /// trusting it moves the chain forward one link at a time.
    pub(crate) async fn open_route(&self, session: &Session) -> Result<RouteOutcome, ShellError> {
        let mut current: Option<Arc<SshSession>> = None;

        for hop_id in &session.jump {
            if hop_id == &session.id {
                return Err(ShellError::app_unsupported("a session that is its own jump host"));
            }

            let hop_session = self.session(hop_id)?;
            let credential = self.credential(&hop_session)?;
            let target = Target {
                host: hop_session.host.clone(),
                port: hop_session.port,
                user: hop_session.user.clone(),
            };

            let presented = match &current {
                Some(hop) => sevenshell_ssh::probe_host_key_via(hop, &target.host, target.port, self.timeout).await?,
                None => sevenshell_ssh::probe_host_key(&target.host, target.port, self.timeout).await?,
            };

            match self.vault.find_known_host(&target.host, target.port)? {
                None => return Ok(RouteOutcome::Decision(Box::new(HostKeyDecision::Unknown(presented)))),
                Some(trusted) if trusted.public_key != presented.public_key => {
                    return Ok(RouteOutcome::Decision(Box::new(HostKeyDecision::Mismatch {
                        key: presented,
                        trusted,
                    })));
                }
                Some(_) => {}
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
                keepalive_seconds: hop_session.terminal.keepalive_seconds,
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
            return Ok(ConnectionOutcome::Ready(self.registry.session_of(&handle).await?));
        }

        let credential = self.credential(session)?;
        let target = Target {
            host: session.host.clone(),
            port: session.port,
            user: session.user.clone(),
        };

        let hop = match self.open_route(session).await? {
            RouteOutcome::Ready(hop) => hop,
            RouteOutcome::Decision(decision) => return Ok(ConnectionOutcome::Decision(decision)),
        };

        let presented = match &hop {
            Some(hop) => sevenshell_ssh::probe_host_key_via(hop, &target.host, target.port, self.timeout).await?,
            None => sevenshell_ssh::probe_host_key(&target.host, target.port, self.timeout).await?,
        };

        match self.vault.find_known_host(&target.host, target.port)? {
            None => {
                return Ok(ConnectionOutcome::Decision(Box::new(HostKeyDecision::Unknown(
                    presented,
                ))));
            }
            Some(trusted) if trusted.public_key != presented.public_key => {
                return Ok(ConnectionOutcome::Decision(Box::new(HostKeyDecision::Mismatch {
                    key: presented,
                    trusted,
                })));
            }
            Some(_) => {}
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
        Ok(ConnectionOutcome::Ready(Arc::new(
            SshSession::open(request, output).await?,
        )))
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

    /// Collects what a session authenticates with. The material is held only long enough to be
    /// handed to the transport, which zeroizes it when the connection is done with it.
    pub(crate) fn credential(&self, session: &Session) -> Result<Credential, ShellError> {
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
                let key_ref = session
                    .auth
                    .key
                    .as_ref()
                    .and_then(|key| key.key_ref.as_deref())
                    .ok_or_else(|| ShellError::ssh_key_invalid("this session has no key attached"))?;

                let material = self
                    .vault
                    .key_material(key_ref)?
                    .ok_or_else(|| ShellError::ssh_key_invalid("the key this session used is no longer stored"))?;

                let (contents, passphrase) = material.into_parts();
                Ok(Credential::Key { contents, passphrase })
            }
            // Two different things, kept apart on purpose: the machine's own agent is asked for a
            // signature, while Pageant speaks a protocol this build does not — and saying *which*
            // one is missing is what tells somebody whether to start a program or stop looking.
            AuthMethod::Agent => Ok(Credential::Agent),
            AuthMethod::Pageant => Ok(Credential::Pageant),
        }
    }
}

/// Either the end of a route, or the decision that has to be made before it can be finished.
pub(crate) enum RouteOutcome {
    Ready(Option<Arc<SshSession>>),
    /// Boxed because a decision carries keys and fingerprints, and it is the rare answer.
    Decision(Box<HostKeyDecision>),
}

/// Either a connection that carries something other than a terminal, or the decision that stands
/// in the way of opening one.
pub(crate) enum ConnectionOutcome {
    Ready(Arc<SshSession>),
    Decision(Box<HostKeyDecision>),
}

/// The terminal's answer to a decision the route could not make on its own.
fn terminal_decision(decision: HostKeyDecision) -> OpenOutcome {
    match decision {
        HostKeyDecision::Unknown(key) => OpenOutcome::HostKeyUnknown { key },
        HostKeyDecision::Mismatch { key, trusted } => OpenOutcome::HostKeyMismatch { key, trusted },
    }
}

pub(crate) fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
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
            protocol: sevenshell_types::Protocol::Ssh,
            host: "127.0.0.1".to_owned(),
            port: 2222,
            user: "someone".to_owned(),
            auth: SessionAuth::with_password(AuthMethod::Password),
            terminal: TerminalSettings::default(),
            jump: Vec::new(),
            forwards: Vec::new(),
            updated_at: 0,
        }
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
}
