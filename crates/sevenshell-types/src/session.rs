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

/// What kind of forward a saved forward is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ForwardMode {
    /// One local port, one fixed destination: the `ssh -L` shape.
    #[default]
    Local,
    /// One local port that speaks SOCKS5, so the destination is chosen per connection: the
    /// `ssh -D` shape. What asks it for an address is a browser or another tool, not this build.
    Dynamic,
}

/// A local port answered from the far side of a connection.
///
/// The name is the `ssh -L` shape — a listener on this machine, and whatever it should reach on
/// the remote side — and it is the shape of both kinds this build has: a [`ForwardMode::Local`] one
/// has a destination of its own, a [`ForwardMode::Dynamic`] one is told the destination by whoever
/// connects. Remote forwarding, the third kind, is still declared as planned and refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalForward {
    pub id: String,
    /// The address to listen on. `127.0.0.1` unless somebody deliberately says otherwise.
    pub bind_host: String,
    /// The port to listen on. Zero asks the system for a free one.
    pub bind_port: u16,
    /// Local or dynamic. A document written before dynamic forwarding existed carries no mode, and
    /// a missing one means a local forward — which is what every forward was.
    #[serde(default)]
    pub mode: ForwardMode,
    /// Where a local forward sends what arrives. A dynamic one ignores it, and it stays in the
    /// document rather than being dropped so that switching a forward's kind back and forth does
    /// not lose the address somebody typed.
    #[serde(default)]
    pub target_host: String,
    #[serde(default)]
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
    /// What the listener is for. A dynamic one has no single destination to show, and the interface
    /// draws "SOCKS5" where the other kind draws an address.
    #[serde(default)]
    pub mode: ForwardMode,
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

/// The kinds of key this build can make.
///
/// Three, which are the three OpenSSH itself offers a person on every platform. The names are short
/// and lower case — `ed25519`, not `ssh-ed25519` — because they are what a dropdown stores and what
/// a request carries, while the *key type* the wire and the vault hold is `ssh-ed25519`, and that
/// comes back from reading the key rather than from this list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyAlgorithm {
    Ed25519,
    Ecdsa,
    Rsa,
}

impl KeyAlgorithm {
    /// Every kind, in the order a list offers them: the one to pick when in doubt first.
    pub const ALL: &'static [KeyAlgorithm] = &[KeyAlgorithm::Ed25519, KeyAlgorithm::Ecdsa, KeyAlgorithm::Rsa];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ed25519 => "ed25519",
            Self::Ecdsa => "ecdsa",
            Self::Rsa => "rsa",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|algorithm| algorithm.as_str() == id)
    }

    /// What OpenSSH calls the key it writes, which is also what the vault ends up calling it.
    ///
    /// Here rather than in the crate that makes the key because this is the string a person sees in
    /// the list afterwards — `describe_key` reads it back out of the key itself, and the two have to
    /// be the same name.
    pub const fn openssh_name(self) -> &'static str {
        match self {
            Self::Ed25519 => "ssh-ed25519",
            Self::Ecdsa => "ecdsa-sha2-nistp256",
            Self::Rsa => "ssh-rsa",
        }
    }
}

impl std::fmt::Display for KeyAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The kind nobody has chosen is the first of the list.
///
/// Ed25519, because it is what a server made in the last decade is configured for and what OpenSSH
/// itself offers first — so a form that opens on this one is a form that is right most of the time,
/// and the person who needs RSA knows it.
impl Default for KeyAlgorithm {
    fn default() -> Self {
        Self::Ed25519
    }
}

/// A key that was just made, and the half of it that goes on a server.
///
/// The public line is here because that is the whole point of making a key in this application: the
/// private half stays in the vault and authenticates from inside it, and what a person carries away
/// is one line to paste into `authorized_keys`. A public key is not a secret — it is meant to be
/// copied onto machines — which is why it is the one key-shaped thing that crosses this boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedKey {
    pub info: KeyInfo,
    /// The `authorized_keys` line: the type, the base64 key, and the comment.
    pub public_key: String,
}

/// What a session asks of its terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSettings {
    pub terminal_type: String,
    pub keepalive_seconds: u16,
    /// Whether the far side may use the key agent on this machine while the session lasts.
    ///
    /// Off by default, and off is the safe answer: while it is on, anything running on that server
    /// — including something an attacker put there — can ask this machine's agent to sign. It is a
    /// deliberate lending of the keys, for as long as the session is open.
    #[serde(default)]
    pub forward_agent: bool,
    /// Whether this session is written down as it happens.
    ///
    /// A recording is a plain file on disk holding everything the terminal ever showed — including
    /// anything printed that was not meant to be kept — so it is off unless somebody says so.
    #[serde(default)]
    pub record: bool,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            terminal_type: "xterm-256color".to_owned(),
            keepalive_seconds: 30,
            forward_agent: false,
            record: false,
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
    /// Whether the group is drawn above the groups it was made beside.
    ///
    /// A fact of its own rather than a very small `order`: pinning is laid over the arrangement and
    /// leaves it alone, so unpinning puts the group back where somebody dragged it to.
    #[serde(default)]
    pub pinned: bool,
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
    /// Who owns the entry. The server reports this as a number, and what arrives here is the name
    /// it keeps for that number when there is one — a listing is read by a person, and `1000` where
    /// every other file manager says `labuser` is an answer to a question nobody asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// The group the entry belongs to, named the same way as `owner`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
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

/// What an upload did: sent everything, asked about a name, or nothing at all.
///
/// An upload is the one transfer that can destroy something without being asked to — a file that is
/// already there is what the bytes land on — so the names on the far side are looked at before
/// anything is sent, and a name that is taken comes back as a question. The local paths stay where
/// they were chosen meanwhile; the token is what stands for them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum UploadOutcome {
    /// The transfers are running: one id per file, or one for a batch that travels packed.
    Started { ids: Vec<String> },
    /// Some of what was chosen is already there. Nothing has been sent.
    Conflict {
        /// What stands for this upload while the question is open.
        token: String,
        /// The names already on the far side, which is what the question is about.
        names: Vec<String>,
        /// Whether "send only what is missing" is a real answer — it is not, for one archive.
        skippable: bool,
    },
    /// Nothing was chosen, or the question was answered with "no".
    Cancelled,
}

/// The answer to that question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UploadAnswer {
    /// Send them anyway: what is on the far side is replaced.
    Overwrite,
    /// Send only the files whose names are free.
    Skip,
    /// Send nothing, and forget the choice.
    Cancel,
}

/// Where downloads land, as the interface sees it.
///
/// Names, never paths — the same rule the export summary follows with the file it wrote. The folder
/// itself is kept on the other side, which is what makes "download into it" something the interface
/// can ask for without being able to name it: a row is named, this side answers with where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadFolderView {
    /// The folder downloads land in, by its own name, or nothing when every download asks.
    pub current: Option<String>,
    /// The folders chosen before, newest first, by their own names. The current one is the first.
    pub history: Vec<String>,
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
    /// Which session it is moving bytes for.
    ///
    /// A transfer outlives the file browser that started it, so a list that is no longer one
    /// browser's own has to be able to say whose it is — and what the interface knows is sessions,
    /// not the handles this side keeps to itself.
    pub session_id: String,
    pub size: u64,
    pub transferred: u64,
    /// `cancelled` only ever comes from the person who asked for it.
    pub state: TransferState,
    /// Whether this end holds a local path for the transfer — not what it is.
    ///
    /// The path was chosen in a native dialog and has never come back through the interface, and it
    /// still does not: opening the file, or showing where it is, stays a call *about this row* that
    /// this side answers from what it kept. What the interface needs in order to draw the entries
    /// that ask for that is only whether asking is possible at all.
    pub has_local_path: bool,
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
    /// Waiting because somebody asked it to, with its connection and its part of the file intact.
    ///
    /// Not a kind of stopped: a paused transfer is still a transfer that is going to happen, and the
    /// button beside it says "go on" rather than "start again".
    Paused,
    Done,
    Failed {
        detail: String,
    },
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

/// One archived vault file that sits beside the active one.
///
/// A library nobody could open is renamed rather than deleted — `vault.json.old`, then
/// `vault.json.2.old`, and so on — and this is how one of those is shown and chosen again. The name
/// travels and the directory does not: the interface may say *which* archive, never where archives
/// live, and the shell joins the two on its own side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultArchive {
    /// The file's own name, exactly as it is on disk.
    pub name: String,
    /// When the file was last written, in seconds since the Unix epoch.
    pub modified_at: u64,
    /// How large the file is, in bytes.
    pub size_bytes: u64,
}

/// A secret travelling *inwards* from the interface, never outwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretInput {
    pub value: String,
}

/// What an export of the session library wrote.
///
/// Counts and a file name, and nothing else: the file itself, and whatever is in it, stays where it
/// was written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionExportSummary {
    pub sessions: u32,
    pub groups: u32,
    /// How many stored credentials travelled. Zero unless a passphrase sealed them into the file.
    pub secrets: u32,
    /// Whether a passphrase stands between the file and what is in it.
    pub encrypted: bool,
    /// The file's own name, never a path: the interface has no business knowing where it is.
    pub file_name: String,
}

/// What an import added.
///
/// `missing_keys` and `dropped_hops` are not failures: what they count was imported anyway, and the
/// sentence they feed is the only warning that a session will not connect the way it did where the
/// file came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionImportSummary {
    pub sessions: u32,
    pub groups: u32,
    /// How many stored credentials came with the file.
    pub secrets: u32,
    /// Imported sessions that name a key this device does not hold.
    pub missing_keys: u32,
    /// Imported sessions that jumped through a session the file did not carry.
    pub dropped_hops: u32,
}

/// What came of asking to import a file.
///
/// Two answers and neither is a failure: either the library was written, or the file is sealed and
/// the passphrase is the next question. A dismissed file dialog is the third answer, and it is the
/// absence of this value rather than a variant of it — nothing happened, and there is nothing to
/// say about it.
///
/// `file_name` travels with the question because the person answering it chose that file a moment
/// ago and is owed the reminder of which one it was; it is the file's name and never its path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// `rename_all` renames the variants; `rename_all_fields` renames the fields inside them.
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SessionImportOutcome {
    /// The library was written, and this is what it added.
    Imported { summary: SessionImportSummary },
    /// The file is sealed under a passphrase this build has not been given yet.
    NeedsPassphrase { file_name: String },
}
