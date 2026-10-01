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
    /// Saved sessions to reach this one through, in order: the first is connected to directly.
    ///
    /// References rather than copies, because a hop is a session like any other: editing it once
    /// should change every route that goes through it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub jump: Vec<String>,
    /// Ports to forward while this session is open.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forwards: Vec<LocalForward>,
    /// Seconds since the Unix epoch, for ordering by recency.
    pub updated_at: u64,
}

/// A local port answered from the far side of a connection.
///
/// The name is the `ssh -L` shape: a listener on this machine, and whatever it should reach on
/// the remote side. Only local forwarding exists so far; remote and dynamic forwarding are
/// declared in the descriptors as planned and are refused with `APP-UNSUPPORTED`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalForward {
    pub id: String,
    /// The address to listen on. `127.0.0.1` unless somebody deliberately says otherwise.
    pub bind_host: String,
    /// The port to listen on. Zero asks the system for a free one.
    pub bind_port: u16,
    pub target_host: String,
    pub target_port: u16,
}

/// A forward that is up right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenTunnel {
    pub id: String,
    /// The terminal this tunnel belongs to; closing the terminal closes the tunnel.
    pub handle: String,
    pub session_id: String,
    /// What is actually listening, which is worth knowing when the port was chosen for us.
    pub bind_host: String,
    pub bind_port: u16,
    pub target_host: String,
    pub target_port: u16,
    /// How many connections it has carried since it came up.
    pub connections: u64,
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
// `rename_all` renames the *variants*; `rename_all_fields` is what renames the fields inside
// them. Without it this enum grew `session_id` and `tunnel_failures` while every struct around it
// spoke camelCase, and the interface read `undefined` for both without complaining.
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum OpenOutcome {
    /// The host key has never been seen. Nothing has been sent to the server.
    HostKeyUnknown { key: PresentedHostKey },
    /// The presented key differs from the trusted one. A hard stop, not a prompt.
    HostKeyMismatch {
        key: PresentedHostKey,
        trusted: KnownHostEntry,
    },
    /// The session is open and its terminal is producing output.
    Opened {
        handle: String,
        session_id: String,
        /// The forwards this session brought up, so the interface can show them immediately.
        tunnels: Vec<OpenTunnel>,
        /// The forwards that did not come up, each with its reason. A port already in use is
        /// worth saying out loud; it is not worth losing the terminal over.
        tunnel_failures: Vec<TunnelFailure>,
    },
}

/// What an entry in a directory is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    Directory,
    File,
    /// A link, which is shown as one and never followed automatically: following one by accident
    /// is how a file browser reads something outside the directory it is looking at.
    Symlink,
    /// A socket, fifo or device: visible, not something to download.
    Other,
}

/// One entry in a remote directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpEntry {
    pub name: String,
    /// The full path, so the interface never has to join strings and get it wrong.
    pub path: String,
    pub kind: EntryKind,
    /// Bytes, for a file. Zero for anything else.
    pub size: u64,
    /// Seconds since the Unix epoch, when the server said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<u64>,
    /// The permission bits (for example `0o644`), when the server sent them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    /// Resolved link target, for a symlink the server told us about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_target: Option<String>,
}

/// A directory as it was read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpListing {
    /// The canonical path the server resolved, which is what the breadcrumbs should show.
    pub path: String,
    pub entries: Vec<SftpEntry>,
}

/// What opening a file session answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SftpOutcome {
    /// The host key has never been seen. Nothing has been sent to the server.
    HostKeyUnknown { key: PresentedHostKey },
    /// The presented key differs from the trusted one. A hard stop, not a prompt.
    HostKeyMismatch {
        key: PresentedHostKey,
        trusted: KnownHostEntry,
    },
    /// The file session is open, and this is where it starts.
    Opened {
        handle: String,
        session_id: String,
        listing: SftpListing,
    },
}

/// Which way a transfer is going, from this machine's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferDirection {
    Download,
    Upload,
}

/// A transfer that is running, or has stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferInfo {
    pub id: String,
    pub direction: TransferDirection,
    /// The name shown to a person: the file's own name, not a path on either side.
    pub name: String,
    pub remote_path: String,
    pub size: u64,
    pub transferred: u64,
    /// `cancelled` only ever comes from the person who asked for it.
    pub state: TransferState,
    /// Whether the files travelled as one archive, packed on the side they came from.
    ///
    /// It is separate from the name because the two answer different questions — the name says what
    /// moved, this says what the transfer *did* — and because the interface needs it for the one
    /// thing it cannot otherwise work out: which directory to read again afterwards. A packed upload
    /// lands its files in the directory it was aimed at; an ordinary one lands a file inside it.
    pub packed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum TransferState {
    Running,
    Done,
    Failed { detail: String },
    Cancelled,
}

/// A forward that was asked for and could not be opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelFailure {
    pub forward_id: String,
    pub bind_host: String,
    pub bind_port: u16,
    /// What went wrong, already scrubbed of anything sensitive.
    pub detail: String,
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
