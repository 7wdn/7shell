//! The domain the interface and the core share: sessions, groups, keys and known hosts.
//!
//! The rule this module exists to enforce: **a secret has no field here.** Passwords, passphrases
//! and key material travel inwards to the vault and never come back out. What comes out is a
//! boolean saying one is stored, and a fingerprint — enough for a person to recognise what they
//! saved, and useless to anyone reading over their shoulder.

use serde::{Deserialize, Serialize};

use crate::generated::{AuthMethod, Protocol};

/// A saved way to reach a machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SessionAuth,
    pub terminal: TerminalSettings,
    /// Seconds since the Unix epoch, for ordering by recency.
    pub updated_at: u64,
}

/// How a session authenticates, without the material that does it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionAuth {
    pub method: AuthMethod,
    /// Present when the method uses a key that has been imported into the vault.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<KeyInfo>,
    /// Whether a password or passphrase is stored for this session.
    pub has_secret: bool,
}

impl SessionAuth {
    pub fn with_password(method: AuthMethod) -> Self {
        Self {
            method,
            key: None,
            has_secret: false,
        }
    }
}

/// What the interface is allowed to know about an imported key.
///
/// The key itself, and its passphrase, live in the vault under `key_ref`. The display name is a
/// file name without a directory, so no local path is ever named outside the process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_ref: Option<String>,
    pub display_name: String,
    pub algorithm: String,
    pub fingerprint: String,
    pub has_passphrase: bool,
}

/// What a session asks of its terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSettings {
    pub terminal_type: String,
    pub keepalive_seconds: u16,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            terminal_type: "xterm-256color".to_owned(),
            keepalive_seconds: 30,
        }
    }
}

/// A folder in the session tree. Groups nest, and are ordered by `order`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionGroup {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub order: u32,
}

/// A host key that has been trusted for a host, so it is not asked about again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownHostEntry {
    pub host: String,
    pub port: u16,
    /// The key algorithm, for example `ssh-ed25519`.
    pub key_type: String,
    /// The server's public key, base64 as it travels in the protocol.
    pub public_key: String,
    /// The human-readable fingerprint, which is what a person actually compares.
    pub fingerprint: String,
    /// Seconds since the Unix epoch.
    pub added_at: u64,
}

/// A host key as presented during a handshake, before anyone has decided to trust it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentedHostKey {
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub public_key: String,
    pub fingerprint: String,
}

/// What the interface is told when opening a session needs a decision, or has succeeded.
///
/// A decision carries no handle: the interface either trusts the key (and asks to open again) or
/// it does not. Keeping the connection half-open while a person thinks would mean holding a
/// verified socket and a credential in memory for no reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OpenOutcome {
    /// The host key has never been seen. Nothing has been sent to the server.
    HostKeyUnknown { key: PresentedHostKey },
    /// The presented key differs from the trusted one. A hard stop, not a prompt.
    HostKeyMismatch {
        key: PresentedHostKey,
        trusted: KnownHostEntry,
    },
    /// The session is open and its terminal is producing output.
    Opened { handle: String, session_id: String },
}

/// Whether the vault exists and whether it is open, as the interface sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VaultState {
    /// No vault on this device yet: the interface offers to create one.
    Missing,
    Locked,
    Unlocked,
}

/// A secret travelling *inwards* from the interface, never outwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretInput {
    pub value: String,
}
