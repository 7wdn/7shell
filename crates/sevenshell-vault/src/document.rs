//! The decrypted content of a vault, and the rules it holds itself to.
//!
//! A `Document` is the plaintext behind the envelope: the sessions, the groups that order them,
//! the imported keys, the per-session secrets and the trusted host keys. Three rules live here
//! rather than in the caller:
//!
//! * **A stored secret is wiped when it is dropped.** Key material and session secrets are held
//!   in [`SecretString`], so locking the vault really does erase them instead of leaving them
//!   for the allocator to hand to the next allocation.
//! * **A secret is never printed.** Nothing here derives `Debug`; the manual implementations
//!   count what is stored, they never show it.
//! * **Absent means unchanged.** Saving a session without a secret keeps whatever secret that
//!   session already had — the interface never has to send a password back to leave it alone.

use std::fmt;

use serde::{Deserialize, Serialize};
use sevenshell_error::ShellError;
use sevenshell_types::{AuthMethod, KeyInfo, KnownHostEntry, Session, SessionGroup};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

/// Bumped only when the stored layout changes in a way older builds cannot read.
pub const VAULT_SCHEMA_VERSION: u32 = 1;

/// Everything a vault holds, as it is serialized into the encrypted payload.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub schema_version: u32,
    #[serde(default)]
    pub sessions: Vec<Session>,
    #[serde(default)]
    pub groups: Vec<SessionGroup>,
    #[serde(default)]
    pub keys: Vec<StoredKey>,
    #[serde(default)]
    pub secrets: Vec<StoredSecret>,
    #[serde(default)]
    pub known_hosts: Vec<KnownHostEntry>,
}

impl Document {
    /// An empty document at the current schema version.
    pub(crate) fn empty() -> Self {
        Self {
            schema_version: VAULT_SCHEMA_VERSION,
            sessions: Vec::new(),
            groups: Vec::new(),
            keys: Vec::new(),
            secrets: Vec::new(),
            known_hosts: Vec::new(),
        }
    }

    /// The sessions, with `has_secret` set from the secrets table rather than from whatever
    /// the caller last wrote. The table is the authority; the flag is a report about it.
    pub(crate) fn sessions(&self) -> Vec<Session> {
        let mut sessions = self.sessions.clone();
        for session in &mut sessions {
            session.auth.has_secret = self.stores_secret_for(&session.id);
        }
        sessions
    }

    /// Adds or replaces a session, and its secret when one is given.
    ///
    /// `secret: None` leaves an existing secret alone; there is no way to say "delete the
    /// secret" here, because the interface cannot tell the difference between "the user did
    /// not change it" and "the user cleared it" — and of the two readings, keeping the
    /// password is the one that cannot lose data.
    pub(crate) fn save_session(&mut self, mut session: Session, secret: Option<SessionSecret>) -> Session {
        if let Some(secret) = &secret {
            let value = SecretString::new(secret.expose().to_owned());
            match self.secrets.iter_mut().find(|stored| stored.session_id == session.id) {
                Some(stored) => stored.secret = value,
                None => self.secrets.push(StoredSecret {
                    session_id: session.id.clone(),
                    secret: value,
                }),
            }
        }

        session.auth.has_secret = self.stores_secret_for(&session.id);
        match self.sessions.iter_mut().find(|existing| existing.id == session.id) {
            Some(existing) => *existing = session.clone(),
            None => self.sessions.push(session.clone()),
        }
        session
    }

    /// Removes a session and the secret that belonged to it. Removing something already absent
    /// is not an error: the world ends up in the state the caller asked for either way.
    pub(crate) fn delete_session(&mut self, id: &str) {
        self.sessions.retain(|session| session.id != id);
        self.secrets.retain(|secret| secret.session_id != id);
    }

    pub(crate) fn save_group(&mut self, group: SessionGroup) -> SessionGroup {
        match self.groups.iter_mut().find(|existing| existing.id == group.id) {
            Some(existing) => *existing = group.clone(),
            None => self.groups.push(group.clone()),
        }
        group
    }

    /// Removes a group. Sessions and child groups that pointed at it fall back to the root of
    /// the tree: deleting a folder must never delete what was inside it.
    pub(crate) fn delete_group(&mut self, id: &str) {
        self.groups.retain(|group| group.id != id);
        for session in &mut self.sessions {
            if session.group_id.as_deref() == Some(id) {
                session.group_id = None;
            }
        }
        for group in &mut self.groups {
            if group.parent_id.as_deref() == Some(id) {
                group.parent_id = None;
            }
        }
    }

    /// The trusted key for exactly this endpoint, if one has been recorded.
    pub(crate) fn find_known_host(&self, host: &str, port: u16) -> Option<KnownHostEntry> {
        self.known_hosts
            .iter()
            .find(|entry| entry.host == host && entry.port == port)
            .cloned()
    }

    /// Records a trusted key for an endpoint.
    ///
    /// One entry per host and port: the comparison the SSH layer makes is against *the* key it
    /// expected, and a rotated key is recorded by replacing the old one. This is also the only
    /// way a user accepts a legitimately changed key.
    pub(crate) fn trust_host(&mut self, entry: KnownHostEntry) {
        self.known_hosts
            .retain(|existing| !(existing.host == entry.host && existing.port == entry.port));
        self.known_hosts.push(entry);
        self.known_hosts
            .sort_by(|a, b| a.host.cmp(&b.host).then(a.port.cmp(&b.port)));
    }

    /// Forgets every key recorded for an endpoint.
    pub(crate) fn forget_host(&mut self, host: &str, port: u16) {
        self.known_hosts
            .retain(|entry| !(entry.host == host && entry.port == port));
    }

    /// Stores key material and returns the `KeyInfo` the interface should remember: the handle
    /// the vault assigned, and the truth about whether a passphrase is held for it.
    pub(crate) fn import_key(
        &mut self,
        contents: String,
        passphrase: Option<String>,
        mut info: KeyInfo,
    ) -> Result<KeyInfo, ShellError> {
        if contents.trim().is_empty() {
            return Err(ShellError::ssh_key_invalid("the key file is empty"));
        }

        let key_ref = format!("key-{}", Uuid::new_v4());
        info.key_ref = Some(key_ref.clone());
        info.has_passphrase = passphrase.is_some();
        self.keys.push(StoredKey {
            key_ref,
            info: info.clone(),
            contents: SecretString::new(contents),
            passphrase: passphrase.map(SecretString::new),
        });
        Ok(info)
    }

    pub(crate) fn key_material(&self, key_ref: &str) -> Option<KeyMaterial> {
        let stored = self.keys.iter().find(|key| key.key_ref == key_ref)?;
        Some(KeyMaterial {
            contents: stored.contents.expose().to_owned(),
            passphrase: stored
                .passphrase
                .as_ref()
                .map(|passphrase| passphrase.expose().to_owned()),
        })
    }

    /// Removes imported key material. Removing something already absent is not an error, for
    /// the same reason deleting a session is not.
    pub(crate) fn forget_key(&mut self, key_ref: &str) {
        self.keys.retain(|key| key.key_ref != key_ref);
    }

    /// The secret stored for a session.
    ///
    /// The kind is not stored next to the value — the format keeps a secret and its session id
    /// and nothing else, so that a stray file listing cannot say what a value is for. It is
    /// derived from the session's authentication method instead: a key-based session holds a
    /// passphrase, anything else holds a password.
    pub(crate) fn secret_for_session(&self, id: &str) -> Option<SessionSecret> {
        let stored = self.secrets.iter().find(|secret| secret.session_id == id)?;
        let method = self
            .sessions
            .iter()
            .find(|session| session.id == id)
            .map(|session| session.auth.method);

        Some(match method {
            Some(AuthMethod::PublicKey) => SessionSecret::KeyPassphrase(stored.secret.expose().to_owned()),
            _ => SessionSecret::Password(stored.secret.expose().to_owned()),
        })
    }

    fn stores_secret_for(&self, session_id: &str) -> bool {
        self.secrets.iter().any(|secret| secret.session_id == session_id)
    }
}

impl fmt::Debug for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Counts only. A document is where every secret lives; a derived `Debug` here would put
        // them one `{:?}` away from a log line.
        f.debug_struct("Document")
            .field("schema_version", &self.schema_version)
            .field("sessions", &self.sessions.len())
            .field("groups", &self.groups.len())
            .field("keys", &self.keys.len())
            .field("secrets", &self.secrets.len())
            .field("known_hosts", &self.known_hosts.len())
            .finish()
    }
}

/// An imported key as it is stored: its handle, what the interface knows about it, and the
/// material itself.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredKey {
    pub key_ref: String,
    pub info: KeyInfo,
    contents: SecretString,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    passphrase: Option<SecretString>,
}

/// A secret stored for one session, keyed by the session it belongs to.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredSecret {
    pub session_id: String,
    secret: SecretString,
}

/// Key material on its way out of the vault.
///
/// Wiped when it is dropped, and never printed. The fields are public because the caller owns
/// this value and reads it once, but note that the `Drop` implementation is what keeps it from
/// lingering: formatting one is impossible, by construction.
#[derive(Clone)]
pub struct KeyMaterial {
    pub contents: String,
    pub passphrase: Option<String>,
}

impl KeyMaterial {
    /// Takes the material apart so it can be handed to a transport, which zeroizes it when done.
    ///
    /// Consuming `self` keeps one owner for the secret. Cloning it would leave a second copy in
    /// memory for the allocator to hand to whoever asks next.
    pub fn into_parts(mut self) -> (String, Option<String>) {
        (std::mem::take(&mut self.contents), self.passphrase.take())
    }
}

impl Drop for KeyMaterial {
    fn drop(&mut self) {
        self.contents.zeroize();
        if let Some(passphrase) = &mut self.passphrase {
            passphrase.zeroize();
        }
    }
}

impl fmt::Debug for KeyMaterial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KeyMaterial(<redacted>)")
    }
}

/// What a session needs in order to authenticate: a login password, or the passphrase of a key
/// file it points at.
///
/// Wiped when it is dropped, and never printed.
#[derive(Clone)]
pub enum SessionSecret {
    Password(String),
    KeyPassphrase(String),
}

impl SessionSecret {
    /// The value, for the one caller that has to hand it to a connection.
    pub(crate) fn expose(&self) -> &str {
        match self {
            Self::Password(value) | Self::KeyPassphrase(value) => value,
        }
    }

    /// Moves the value out so a connection can own it, leaving nothing behind to wipe here.
    ///
    /// The caller becomes responsible for the secret: whatever receives it must zeroize it, which
    /// `sevenshell_ssh::Credential` does.
    pub fn into_value(mut self) -> String {
        match &mut self {
            Self::Password(value) | Self::KeyPassphrase(value) => std::mem::take(value),
        }
    }
}

impl Drop for SessionSecret {
    fn drop(&mut self) {
        match self {
            Self::Password(value) | Self::KeyPassphrase(value) => value.zeroize(),
        }
    }
}

impl fmt::Debug for SessionSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Which kind of secret it is is safe to say; what it is, is not.
        let kind = match self {
            Self::Password(_) => "Password",
            Self::KeyPassphrase(_) => "KeyPassphrase",
        };
        write!(f, "SessionSecret::{kind}(<redacted>)")
    }
}

/// A string inside a document that is wiped on drop and can never be printed.
#[derive(Clone, Default)]
struct SecretString(Zeroizing<String>);

impl SecretString {
    fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl Serialize for SecretString {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.expose())
    }
}

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // The deserialized allocation is moved straight into the zeroizing wrapper, so no
        // un-wiped copy of a secret is left behind for the allocator to hand out.
        String::deserialize(deserializer).map(Self::new)
    }
}

/// Brings a document written by an older build up to [`VAULT_SCHEMA_VERSION`].
///
/// One arm today, because version 1 is the only shape that exists. A future version adds its
/// step here (`1 => { ...; 2 }`) and `unlock` runs the chain before anything reads the
/// document. A document from a *newer* build is refused instead of guessed at: silently
/// dropping fields a future version needs is how a vault loses data.
pub(crate) fn migrate(document: Document) -> Result<Document, ShellError> {
    match document.schema_version {
        VAULT_SCHEMA_VERSION => Ok(document),
        found => Err(ShellError::store_schema_newer(found, VAULT_SCHEMA_VERSION)),
    }
}

/// Parses a decrypted payload. A payload that decrypted correctly but does not parse was
/// written by something that is not this format, and is reported as an unlock failure so the
/// two cannot be told apart from outside.
pub(crate) fn from_slice(payload: &[u8]) -> Result<Document, ShellError> {
    let document = serde_json::from_slice::<Document>(payload).map_err(|_| ShellError::vault_unlock_failed())?;
    migrate(document)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sevenshell_types::{SessionAuth, TerminalSettings};

    fn session(id: &str, name: &str) -> Session {
        Session {
            id: id.to_owned(),
            name: name.to_owned(),
            group_id: None,
            protocol: sevenshell_types::Protocol::Ssh,
            host: "host.example".to_owned(),
            port: 22,
            user: "root".to_owned(),
            auth: SessionAuth::with_password(AuthMethod::Password),
            terminal: TerminalSettings::default(),
            jump: Vec::new(),
            forwards: Vec::new(),
            updated_at: 1,
        }
    }

    #[test]
    fn saving_without_a_secret_keeps_the_one_already_stored() {
        let mut document = Document::empty();
        document.save_session(session("a", "web"), Some(SessionSecret::Password("hunter2".to_owned())));

        document.save_session(session("a", "web renamed"), None);

        assert_eq!(
            document
                .secret_for_session("a")
                .map(|secret| secret.expose().to_owned()),
            Some("hunter2".to_owned())
        );
        assert!(document.sessions()[0].auth.has_secret);
    }

    #[test]
    fn saving_with_a_secret_replaces_the_old_one() {
        let mut document = Document::empty();
        document.save_session(session("a", "web"), Some(SessionSecret::Password("first".to_owned())));
        document.save_session(
            session("a", "web"),
            Some(SessionSecret::KeyPassphrase("second".to_owned())),
        );

        let stored = document.secret_for_session("a").expect("a secret");
        assert_eq!(stored.expose(), "second");
        assert!(
            matches!(stored, SessionSecret::Password(_)),
            "the method decides the kind"
        );
    }

    #[test]
    fn deleting_a_session_takes_its_secret_with_it() {
        let mut document = Document::empty();
        document.save_session(session("a", "web"), Some(SessionSecret::Password("hunter2".to_owned())));

        document.delete_session("a");

        assert!(document.sessions.is_empty());
        assert!(document.secrets.is_empty());
    }

    #[test]
    fn deleting_a_group_keeps_what_was_inside_it() {
        let mut document = Document::empty();
        document.save_group(SessionGroup {
            id: "g1".to_owned(),
            name: "servers".to_owned(),
            parent_id: None,
            order: 0,
        });
        document.save_group(SessionGroup {
            id: "g2".to_owned(),
            name: "nested".to_owned(),
            parent_id: Some("g1".to_owned()),
            order: 0,
        });
        let mut nested = session("a", "web");
        nested.group_id = Some("g1".to_owned());
        document.save_session(nested, None);

        document.delete_group("g1");

        assert_eq!(document.groups.len(), 1);
        assert_eq!(document.groups[0].parent_id, None);
        assert_eq!(document.sessions[0].group_id, None);
    }

    #[test]
    fn a_newer_document_is_refused_with_the_schema_code() {
        let mut future = Document::empty();
        future.schema_version = 2;

        let error = migrate(future).expect_err("version 2 is not this build's shape");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::StoreSchemaNewer);
        assert_eq!(error.message_params().get("found").map(String::as_str), Some("2"));
        assert_eq!(error.message_params().get("supported").map(String::as_str), Some("1"));
    }

    #[test]
    fn a_document_without_a_schema_version_does_not_parse() {
        let error = from_slice(br#"{"sessions":[]}"#).expect_err("schemaVersion is required");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::VaultUnlockFailed);
    }

    #[test]
    fn a_secret_is_never_printed() {
        let mut document = Document::empty();
        document.save_session(session("a", "web"), Some(SessionSecret::Password("hunter2".to_owned())));

        let rendered = format!("{document:?} {:?}", document.secret_for_session("a"));

        assert!(
            !rendered.contains("hunter2"),
            "a secret reached Debug output: {rendered}"
        );
        assert!(rendered.contains("redacted"));
    }
}
