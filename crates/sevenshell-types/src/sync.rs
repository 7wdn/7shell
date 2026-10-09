//! What the interface is told about a backup, and what it decides about one.
//!
//! Nothing here carries a secret. A snapshot's contents cross the boundary as *counts* — how many
//! sessions, how many stored logins — and the values themselves never do: they are read out of the
//! vault, sealed into a file and read back by the shell, so the interface chooses what travels and
//! is told what happened without ever holding what it was about.

use serde::{Deserialize, Serialize};

/// What a snapshot is asked to carry.
///
/// The four lists are `None` for "every one there is", which is what this build means by a snapshot
/// it was not asked to narrow: a file written before per-item choosing existed carries `None` too,
/// so the two are the same thing rather than two states a reader has to tell apart.
///
/// The four flags are the things a person actually decides about. Everything a snapshot could carry
/// is here, including what it can *not*: the defaults are the conservative ones, and
/// [`SyncSelection::everything`] is the one that turns them all on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSelection {
    /// Session ids, or `None` for all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sessions: Option<Vec<String>>,
    /// Session-group ids, or `None` for all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<String>>,
    /// Command ids, or `None` for all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippets: Option<Vec<String>>,
    /// Completion-word ids, or `None` for all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<String>>,
    /// Whether the stored logins and key material travel, sealed under a passphrase.
    ///
    /// A preference rather than a promise: a snapshot asked for credentials with no passphrase is
    /// written without them, and says so here, because the alternative is a file that looks like it
    /// carries passwords and does not.
    pub credentials: bool,
    /// Whether the commands that were sent travel. Their own switch because a history is a diary.
    pub history: bool,
    /// Whether the trusted host keys travel. Their own switch because carrying them means a machine
    /// accepting keys it never verified.
    pub known_hosts: bool,
    /// Whether the portable settings travel.
    pub settings: bool,
}

impl Default for SyncSelection {
    /// Everything that is nobody's surprise, and neither of the two that are.
    ///
    /// The history and the trusted host keys are the two things a snapshot can carry that somebody
    /// would not expect it to: one is a record of what they typed, and the other decides what their
    /// machine trusts before it has ever connected anywhere. They are one checkbox away rather than
    /// on, and this is the shape a first backup has.
    fn default() -> Self {
        Self {
            sessions: None,
            groups: None,
            snippets: None,
            words: None,
            credentials: false,
            history: false,
            known_hosts: false,
            settings: true,
        }
    }
}

/// What a snapshot holds, as counts.
///
/// Counts rather than contents, and this is the same rule the session export follows: what crosses
/// the boundary is how much, never what. A catalog is also what a *prospective* snapshot reports,
/// so the dialog that asks what to include can say what the answer would be before anything is
/// written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncContents {
    pub sessions: u32,
    pub groups: u32,
    pub snippets: u32,
    pub words: u32,
    /// Zero when the snapshot does not carry a history at all.
    pub history: u32,
    /// Zero when it does not carry trusted keys.
    pub known_hosts: u32,
    /// How many settings travel. Zero when the snapshot carries none.
    pub settings: u32,
    /// How many stored logins travel. Always zero in the plain form.
    pub secrets: u32,
    /// How many keys travel, material and all. Always zero in the plain form.
    pub keys: u32,
}

/// What a file holds, for the dialog that asks what to do with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncCatalog {
    pub contents: SyncContents,
    /// Whether a passphrase stands between a reader and what is inside.
    pub sealed: bool,
    /// Seconds since the Unix epoch, from the file itself.
    pub exported_at: u64,
    /// The version of the application that wrote it, for the person reading the dialog.
    pub app_version: String,
}

/// What writing a backup did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReceipt {
    pub contents: SyncContents,
    /// Whether a passphrase was used, which is what the sentence afterwards has to say.
    pub sealed: bool,
    /// The file's own name, never a path: the interface has no business knowing where it is.
    pub file_name: String,
}

/// What restoring a backup did.
///
/// `replaced` and `copied` are the same event counted under the two meanings it can have: an arrival
/// whose id was already here. Asked to replace, it was written over the one that was here; not
/// asked, it arrived as a second copy under an id of its own. `missing_keys` and `dropped_hops` are
/// the export's warnings again: what they count was applied anyway, and the sentence they feed is
/// the only sign that a session may not connect the way it did where the file came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncApplySummary {
    pub contents: SyncContents,
    pub replaced: u32,
    pub copied: u32,
    pub missing_keys: u32,
    pub dropped_hops: u32,
}

/// What came of asking to read a backup file.
///
/// Two answers and neither is a failure: either the file was read and what it holds can be shown,
/// or it is sealed and the passphrase is the next question. A dismissed file dialog is the absence
/// of this value — nothing happened, so there is nothing to say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// `rename_all` renames the variants; `rename_all_fields` renames the fields inside them.
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SyncInspectOutcome {
    /// The file was read, and this is what it holds.
    Read { catalog: SyncCatalog, file_name: String },
    /// The file is sealed under a passphrase this build has not been given yet.
    NeedsPassphrase { file_name: String },
}

/// What this installation has configured for a sync server.
///
/// The two booleans are all the interface is told about what is kept, and both are answers rather
/// than values: `has_password` says the vault is holding one, which is what the box beside the field
/// is drawn from, and nothing here ever carries the password or the passphrase itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncServiceStatus {
    /// Whether the vault holds a password for the server.
    pub has_password: bool,
    /// Whether it holds the passphrase an upload is sealed under.
    pub has_passphrase: bool,
}

/// What an upload to a server did.
///
/// Its own shape rather than [`SyncReceipt`], because a server has no file name to speak of: what a
/// person wants to see is where it went, and the remote path is configuration rather than a secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncUploadReceipt {
    pub contents: SyncContents,
    /// Whether a passphrase stood between the file and what is in it.
    pub sealed: bool,
    pub remote_path: String,
}

/// What comes of asking a server for the file it holds.
///
/// Three answers rather than two: a server with nothing there yet is not a failure and not a
/// question — it is the ordinary state of a sync that has never run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// `rename_all` renames the variants; `rename_all_fields` renames the fields inside them.
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SyncDownloadOutcome {
    /// The server has no file there yet.
    NothingYet,
    /// The file was read, and this is what it holds.
    Read { catalog: SyncCatalog },
    /// It is sealed under a passphrase this build has not been given.
    NeedsPassphrase,
}

/// Which service a backup is parked in as a snippet.
///
/// Here rather than in the crate that talks to them, because it is three names the interface is told
/// and three names the settings file holds: the dropdown, the stored configuration and the request
/// all have to say the same word, and this is the one place that word is written down. The names are
/// the services' own — lower case, and not camel-cased out of the middle of `GitHub` — which is also
/// why they are not translated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SnippetProvider {
    GitHub,
    Gitee,
    GitLab,
}

impl SnippetProvider {
    /// Every provider, in the order a list offers them: the one most people have first.
    pub const ALL: &'static [SnippetProvider] =
        &[SnippetProvider::GitHub, SnippetProvider::Gitee, SnippetProvider::GitLab];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::Gitee => "gitee",
            Self::GitLab => "gitlab",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|provider| provider.as_str() == id)
    }
}

impl std::fmt::Display for SnippetProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The one nobody has chosen is the first of the list.
///
/// A default rather than an error, because "no service has been picked yet" is the state of the
/// panel before it is used, and the honest thing to draw there is the dropdown's first entry rather
/// than an empty control nobody can press a button under. It is also what the shell falls back to,
/// so the form and the request mean the same service without anybody having touched either.
impl Default for SnippetProvider {
    fn default() -> Self {
        Self::GitHub
    }
}

/// What this installation has for the snippet account it is configured for.
///
/// Two things, and both are the shell's answers: what the account is called, and whether a token is
/// kept for it. The token itself never crosses this boundary — that is a lookup the shell does.
///
/// The account's *name* crosses because the page files the snippet id under it, and the page is where
/// that id lives: it is a setting, drawn on the form and written with the other settings. What names
/// an account is the shell's rule, in one language, rather than a second copy that would drift the
/// day a service gains an address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetStatus {
    /// What this service, or this installation of it, is called: the key a snippet id is filed
    /// under. Not a secret, and not a name a person reads — the settings file holds it too.
    pub account: String,
    /// Whether the vault holds a token for this account.
    pub has_token: bool,
}

/// What an upload to a snippet did.
///
/// The id is why this shape exists rather than [`SyncUploadReceipt`]: the first upload *creates* the
/// gist or snippet, and the id it was given is what every later upload writes to. It comes back so
/// it can be written down; without it, each upload would make another one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetUploadReceipt {
    pub contents: SyncContents,
    /// Whether a passphrase stood between the file and what is in it. It always does here — a
    /// snippet belongs to somebody else's service — and it is said anyway rather than assumed.
    pub sealed: bool,
    pub provider: SnippetProvider,
    pub snippet_id: String,
}
