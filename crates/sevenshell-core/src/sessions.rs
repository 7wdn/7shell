//! Opening, driving and closing terminals.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sevenshell_error::ShellError;
use sevenshell_session::{OpenTerminal, SessionRegistry};
use sevenshell_ssh::{Credential, OpenRequest, Target, TerminalSize};
use sevenshell_types::{AuthMethod, KnownHostEntry, OpenOutcome, PresentedHostKey, Session};
use sevenshell_vault::Vault;
use tokio::sync::mpsc;

/// One event from an open terminal, on its way to the host.
pub use sevenshell_session::SessionEvent as CoreEvent;

/// How long connecting, authenticating and opening a channel may take in total.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(30);

/// The size a terminal starts at, until the interface says otherwise.
const INITIAL_SIZE: TerminalSize = TerminalSize { columns: 80, rows: 24 };

/// The operations that need the vault, the transport and the registry in a particular order.
pub struct Core {
    vault: Arc<Vault>,
    registry: Arc<SessionRegistry>,
    timeout: Duration,
}

impl Core {
    /// Builds the core around a vault, and returns it with the receiver its terminal output arrives on.
    pub fn new(vault: Arc<Vault>) -> (Arc<Self>, mpsc::Receiver<CoreEvent>) {
        let (registry, events) = SessionRegistry::start();
        (
            Arc::new(Self {
                vault,
                registry,
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
    /// 2. The host key is read with a probe that sends nothing — no credential, no username.
    /// 3. If the key has never been seen, or has changed, the caller is told and nothing more
    ///    happens. Changing a key is a stop, not a warning.
    /// 4. Only then is the real connection opened, and it verifies the key again inside itself:
    ///    a key that changes between the probe and the connection is refused there.
    pub async fn open_terminal(&self, session_id: &str) -> Result<OpenOutcome, ShellError> {
        let session = self.session(session_id)?;
        let credential = self.credential(&session)?;
        let target = Target {
            host: session.host.clone(),
            port: session.port,
            user: session.user.clone(),
        };

        let presented = sevenshell_ssh::probe_host_key(&target.host, target.port, self.timeout).await?;

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

        let request = OpenRequest {
            target,
            credential,
            terminal_type: session.terminal.terminal_type.clone(),
            keepalive_seconds: session.terminal.keepalive_seconds,
            timeout: self.timeout,
            expected_host_key: presented,
            size: INITIAL_SIZE,
        };

        let handle = self.registry.open(session_id, request).await?;
        Ok(OpenOutcome::Opened {
            handle,
            session_id: session_id.to_owned(),
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

    /// Closes one terminal.
    pub async fn close_terminal(&self, handle: &str) -> Result<(), ShellError> {
        self.registry.close(handle).await
    }

    /// The terminals that are open.
    pub async fn open_terminals(&self) -> Vec<OpenTerminal> {
        self.registry.open_sessions().await
    }

    /// Closes every terminal, for when the interface goes away.
    pub async fn close_all(&self) {
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

    fn session(&self, id: &str) -> Result<Session, ShellError> {
        self.vault
            .sessions()?
            .into_iter()
            .find(|session| session.id == id)
            .ok_or_else(|| ShellError::session_not_found(id))
    }

    /// Collects what a session authenticates with. The material is held only long enough to be
    /// handed to the transport, which zeroizes it when the connection is done with it.
    fn credential(&self, session: &Session) -> Result<Credential, ShellError> {
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
            // The transport answers the agent variants with `app_unsupported`, which names what is
            // missing instead of looking like a credential that was rejected.
            AuthMethod::Agent | AuthMethod::Pageant => Ok(Credential::Agent),
        }
    }
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use sevenshell_types::{SessionAuth, TerminalSettings};
    use tempfile::{TempDir, tempdir};

    use super::*;

    /// Returns the temp directory as well as the vault: dropping it would take the vault file with it.
    fn vault_with_a_session() -> (TempDir, Arc<Vault>, String) {
        let directory = tempdir().expect("temp dir");
        let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
        vault.create("correct horse battery staple").expect("create");

        let session = Session {
            id: "session-1".to_owned(),
            name: "example".to_owned(),
            group_id: None,
            protocol: sevenshell_types::Protocol::Ssh,
            host: "127.0.0.1".to_owned(),
            port: 2222,
            user: "someone".to_owned(),
            auth: SessionAuth::with_password(AuthMethod::Password),
            terminal: TerminalSettings::default(),
            updated_at: 0,
        };
        vault.save_session(session, None).expect("save");

        (directory, vault, "session-1".to_owned())
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
