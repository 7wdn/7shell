//! What a snapshot is: everything worth carrying, in one file.
//!
//! Two forms, the same two the session export has, and for the same reason — a file that carries
//! credentials is sealed, and a file that is not sealed carries none. The plain form is written
//! when nobody asked for credentials, or when they asked and gave no passphrase to seal them under;
//! either way the `selection` it records says what it actually holds, so a file is never described
//! as carrying passwords it does not have.
//!
//! What a snapshot carries that an export does not: the command library, the words added to it, the
//! imported keys, the trusted host keys, the settings that describe how somebody works, and — for
//! whoever asks for it — the record of what they typed. The record and the trusted keys are off by
//! default: one is a diary, and the other decides what a machine trusts before it has ever
//! connected anywhere.
//!
//! A snapshot carries the whole of each of those. The selection's four id lists exist so that a
//! file written by something that *does* narrow one day can be read here — this build writes them
//! empty and applies whatever a file holds, which is the right answer for a narrowed file too.
//!
//! Nothing here reaches the interface. A catalog is counts, and the values stay in this crate, in a
//! `SecretEntry` or on their way into a payload.

use serde::{Deserialize, Serialize};
use sevenshell_error::ShellError;
use sevenshell_types::{
    AppSettings, CommandHistoryEntry, CompletionWord, KnownHostEntry, Session, SessionGroup, SnippetGroup,
    SyncContents, SyncSelection,
};
use sevenshell_vault::{
    KeyEntry, MIN_PASSPHRASE_LEN, PayloadPurpose, SealedPayload, SecretEntry, Vault, open_payload, seal_payload,
};

use crate::settings::PortableSettings;

/// The `format` field this build writes, and the only one it reads.
pub const SNAPSHOT_FORMAT: &str = "sevenshell-sync";

/// The plain layout: everything a snapshot carries, and not one credential.
pub const SNAPSHOT_VERSION: u32 = 1;

/// The sealed layout: an envelope naming its derivation parameters, and one payload.
///
/// Its own number rather than a field on the old one, for the reason the export gives: a build that
/// predates this cannot open a payload, and must be refused rather than left reading the halves of
/// a file it does not understand.
pub const SNAPSHOT_SEALED_VERSION: u32 = 2;

/// The only `protection` value this build writes or opens.
pub const PROTECTION_PASSPHRASE: &str = "passphrase";

/// A snapshot larger than this is not one.
///
/// Generous compared with the session export's few kilobytes, because a snapshot carries the whole
/// command library and every key that was imported — and still bounded, so a mistyped path that
/// lands on a disk image is refused before it is read into memory.
pub const MAX_SNAPSHOT_BYTES: u64 = 32 * 1024 * 1024;

impl std::fmt::Debug for BuiltSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuiltSnapshot")
            .field("contents", &self.contents)
            .field("sealed", &self.sealed)
            .field("text", &format_args!("<{} bytes>", self.text.len()))
            .finish()
    }
}

/// Everything a snapshot holds.
///
/// Not `Debug` through a derive: the two credential lists are in here, and a derived one would put
/// a stored login one `{:?}` away from a log line. The manual implementation counts.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSnapshot {
    pub format: String,
    pub version: u32,
    /// Seconds since the Unix epoch, for whoever reads the file by hand.
    pub exported_at: u64,
    /// The application that wrote it, for the person looking at a file they did not write.
    pub app_version: String,
    /// What the writer meant to carry. The writers of this build narrow nothing, so the four lists
    /// are absent; a reader still reads them, because a file written by something later may have.
    pub selection: SyncSelection,
    #[serde(default)]
    pub sessions: Vec<Session>,
    #[serde(default)]
    pub groups: Vec<SessionGroup>,
    #[serde(default)]
    pub snippets: Vec<SnippetGroup>,
    #[serde(default)]
    pub words: Vec<CompletionWord>,
    /// Absent when the snapshot was not asked for the sent-command record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<CommandHistoryEntry>>,
    /// Absent when it was not asked for the trusted host keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_hosts: Option<Vec<KnownHostEntry>>,
    /// Absent when it was not asked for the settings that travel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<PortableSettings>,
    /// Only ever inside a sealed payload: the plain form has none, because the plain builder never
    /// reads the vault's secrets at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secrets: Option<Vec<SecretEntry>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<Vec<KeyEntry>>,
}

impl std::fmt::Debug for SyncSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncSnapshot")
            .field("format", &self.format)
            .field("version", &self.version)
            .field("sessions", &self.sessions.len())
            .field("groups", &self.groups.len())
            .field("snippets", &self.snippets.len())
            .field("words", &self.words.len())
            .field("settings", &self.settings.is_some())
            .field("secrets", &self.secrets.as_ref().map(Vec::len))
            .field("keys", &self.keys.as_ref().map(Vec::len))
            .finish()
    }
}

impl SyncSnapshot {
    /// What this holds, as counts.
    pub fn contents(&self) -> SyncContents {
        SyncContents {
            sessions: self.sessions.len() as u32,
            groups: self.groups.len() as u32,
            snippets: self.snippets.iter().map(|group| group.commands.len()).sum::<usize>() as u32,
            words: self.words.len() as u32,
            history: self.history.as_ref().map_or(0, |entries| entries.len() as u32),
            known_hosts: self.known_hosts.as_ref().map_or(0, |entries| entries.len() as u32),
            settings: self.settings.as_ref().map_or(0, PortableSettings::count),
            secrets: self.secrets.as_ref().map_or(0, |entries| entries.len() as u32),
            keys: self.keys.as_ref().map_or(0, |entries| entries.len() as u32),
        }
    }

    /// The text to write into the file. Pretty-printed on purpose: a snapshot is a thing people
    /// open, diff and occasionally hand-edit, and one line of JSON is none of those.
    pub fn to_json(&self) -> Result<String, ShellError> {
        serde_json::to_string_pretty(self).map_err(|error| ShellError::sync_io(error.to_string()))
    }
}

/// A sealed snapshot: the parameters to open it with, and one payload.
///
/// What is *not* here is any hint about what the payload holds: not a session count, not a group
/// name, not a user name. A file somebody finds learns nothing by being looked at — which is also
/// why the catalog of a sealed file cannot be shown before its passphrase is given.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotEnvelope {
    pub format: String,
    pub version: u32,
    pub exported_at: u64,
    /// Written out rather than left to be inferred: a person who opens the file should be able to
    /// see that a passphrase stands between them and what is inside.
    pub protection: String,
    #[serde(flatten)]
    pub sealed: SealedPayload,
}

impl SnapshotEnvelope {
    /// Opens the payload with a passphrase, giving back the snapshot that went in.
    ///
    /// A wrong passphrase, a payload truncated on the way here and one sealed for another purpose
    /// are one answer. The caller that can act on it is the question being asked next, which keeps
    /// its field on screen and lets the person try again.
    pub fn unseal(&self, passphrase: &str) -> Result<SyncSnapshot, ShellError> {
        let payload = open_payload(passphrase, PayloadPurpose::SyncSnapshot, &self.sealed)?;

        let snapshot: SyncSnapshot = serde_json::from_slice(payload.as_slice())
            .map_err(|error| ShellError::sync_snapshot_invalid(format!("the file could not be read: {error}")))?;

        check_snapshot(&snapshot)?;
        Ok(snapshot)
    }

    pub fn to_json(&self) -> Result<String, ShellError> {
        serde_json::to_string_pretty(self).map_err(|error| ShellError::sync_io(error.to_string()))
    }
}

/// What a snapshot's own fields promise about it, checked wherever one is read.
fn check_snapshot(snapshot: &SyncSnapshot) -> Result<(), ShellError> {
    if snapshot.format != SNAPSHOT_FORMAT {
        return Err(ShellError::sync_snapshot_invalid("the file is not a 7Shell backup"));
    }
    if snapshot.version > SNAPSHOT_VERSION {
        return Err(ShellError::store_schema_newer(snapshot.version, SNAPSHOT_VERSION));
    }
    Ok(())
}

/// A snapshot on its way to a file, with what it holds.
///
/// `Debug` by hand for the reason the export's summary gives: the text is a library with no
/// credential in it or a sealed payload, and a future variant that carried a third thing must not be
/// printable by default.
pub struct BuiltSnapshot {
    pub text: String,
    pub contents: SyncContents,
    /// Whether a passphrase stands between the file and what is in it.
    pub sealed: bool,
}

/// Reads everything the selection asks for out of the vault, and takes the settings as a value.
///
/// The settings are passed in rather than read here: this function is a rule about a file, and a
/// rule that reads the machine it is running on cannot be tested against a machine that is not.
///
/// The credentials are the one thing that needs a passphrase, and asking for them without one is
/// not an error: the snapshot is written without them and records that it does. The alternative
/// would be a refusal for something a person may well mean — "back up everything except the
/// passwords" is the ordinary case, not a mistake. **The passphrase is here only to make that
/// decision**; the credentials themselves are read by [`with_credentials`], which every caller runs
/// last.
pub fn gather(
    vault: &Vault,
    settings: &AppSettings,
    selection: &SyncSelection,
    passphrase: Option<&str>,
    now: u64,
) -> Result<SyncSnapshot, ShellError> {
    let mut carried = selection.clone();
    let sealing = match (selection.credentials, passphrase) {
        (true, Some(value)) if value.chars().count() < MIN_PASSPHRASE_LEN => {
            return Err(ShellError::sync_passphrase_short(MIN_PASSPHRASE_LEN));
        }
        (true, Some(value)) => Some(value),
        _ => None,
    };
    carried.credentials = sealing.is_some();

    let mut snapshot = SyncSnapshot {
        format: SNAPSHOT_FORMAT.to_owned(),
        version: SNAPSHOT_VERSION,
        exported_at: now,
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        selection: carried,
        sessions: Vec::new(),
        groups: Vec::new(),
        snippets: Vec::new(),
        words: Vec::new(),
        history: None,
        known_hosts: None,
        settings: None,
        secrets: None,
        keys: None,
    };

    // Whole lists, every time. Narrowing one is what the selection's lists are shaped for, and it is
    // the one thing this build does not do: a narrowed session would have to lose the group it sits
    // in or the hop it goes through, and both of those are drops somebody should be told about.
    // Carrying everything and saying so is the honest version until a picker exists that can count
    // what it left out.
    snapshot.sessions = vault.sessions()?;
    snapshot.groups = vault.groups()?;
    snapshot.snippets = vault.snippet_groups()?;
    snapshot.words = vault.completion_words()?;

    if selection.history {
        snapshot.history = Some(vault.command_history()?);
    }
    if selection.known_hosts {
        snapshot.known_hosts = Some(vault.known_hosts()?);
    }
    if selection.settings {
        snapshot.settings = Some(PortableSettings::of(settings));
    }

    Ok(snapshot)
}

/// Adds the stored logins and key material to a snapshot that was asked to carry them.
///
/// Its own step rather than part of [`gather`], and it is the last thing every caller does: reading
/// them is the last operation that can fail, so there is no path where they are in memory and then
/// dropped because something else went wrong.
pub fn with_credentials(vault: &Vault, snapshot: &mut SyncSnapshot) -> Result<(), ShellError> {
    snapshot.secrets = Some(vault.secret_entries()?);
    snapshot.keys = Some(vault.key_entries()?);
    Ok(())
}

/// Refuses a snapshot that says it carries credentials and does not.
///
/// The flag is what a reader uses to decide whether to ask for a passphrase and what to tell somebody
/// about the file, so a sealed snapshot claiming credentials it does not hold is a file that lies
/// about itself — the one failure a backup must not have. Both sealers check it, which is what makes
/// "read the credentials last" a rule a caller can forget without the mistake reaching a file: they
/// get a loud internal error instead of a plausible-looking file.
pub(crate) fn ensure_credentials_came(snapshot: &SyncSnapshot) -> Result<(), ShellError> {
    if snapshot.selection.credentials && (snapshot.secrets.is_none() || snapshot.keys.is_none()) {
        return Err(ShellError::app_internal(
            "a snapshot was sealed as carrying credentials without them having been read",
        ));
    }
    Ok(())
}

/// A snapshot in the form a file, a server or a snippet carries.
pub fn build(
    vault: &Vault,
    settings: &AppSettings,
    selection: &SyncSelection,
    passphrase: Option<&str>,
    now: u64,
) -> Result<BuiltSnapshot, ShellError> {
    let mut snapshot = gather(vault, settings, selection, passphrase, now)?;
    let contents = snapshot.contents();

    // What `gather` decided about the credentials, which is the answer to whether this goes out
    // sealed: the two can never disagree, because the flag it wrote is the one read here.
    let sealing = passphrase.filter(|_| snapshot.selection.credentials);
    let Some(passphrase) = sealing else {
        return Ok(BuiltSnapshot {
            text: snapshot.to_json()?,
            contents,
            sealed: false,
        });
    };

    with_credentials(vault, &mut snapshot)?;
    ensure_credentials_came(&snapshot)?;

    let payload = zeroize::Zeroizing::new(
        serde_json::to_vec(&snapshot)
            .map_err(|error| ShellError::sync_io(format!("the snapshot could not be written: {error}")))?,
    );
    let envelope = SnapshotEnvelope {
        format: SNAPSHOT_FORMAT.to_owned(),
        version: SNAPSHOT_SEALED_VERSION,
        exported_at: now,
        protection: PROTECTION_PASSPHRASE.to_owned(),
        sealed: seal_payload(
            passphrase,
            PayloadPurpose::SyncSnapshot,
            payload.as_slice(),
            &Default::default(),
        )?,
    };

    Ok(BuiltSnapshot {
        text: envelope.to_json()?,
        contents,
        sealed: true,
    })
}

/// What a chosen file turned out to be.
///
/// The two forms are answered separately rather than as one "snapshot with maybe a passphrase",
/// because what happens next differs: one can be described and applied now, the other is a question
/// to ask first.
#[derive(Debug)]
pub enum OpenedSnapshot {
    Plain(Box<SyncSnapshot>),
    Sealed(Box<SnapshotEnvelope>),
}

/// The part of every snapshot that says what the file is, read before anything else is.
#[derive(Deserialize)]
struct SnapshotHeader {
    format: String,
    version: u32,
}

/// Reads a chosen file's text, refusing anything this build does not understand.
pub fn open(text: &str) -> Result<OpenedSnapshot, ShellError> {
    let header: SnapshotHeader = serde_json::from_str(text)
        .map_err(|error| ShellError::sync_snapshot_invalid(format!("the file could not be read: {error}")))?;

    if header.format != SNAPSHOT_FORMAT {
        return Err(ShellError::sync_snapshot_invalid("the file is not a 7Shell backup"));
    }
    if header.version > SNAPSHOT_SEALED_VERSION {
        return Err(ShellError::store_schema_newer(header.version, SNAPSHOT_SEALED_VERSION));
    }

    match header.version {
        SNAPSHOT_VERSION => {
            let snapshot: SyncSnapshot = serde_json::from_str(text)
                .map_err(|error| ShellError::sync_snapshot_invalid(format!("the file could not be read: {error}")))?;
            check_snapshot(&snapshot)?;
            Ok(OpenedSnapshot::Plain(Box::new(snapshot)))
        }
        SNAPSHOT_SEALED_VERSION => {
            let envelope: SnapshotEnvelope = serde_json::from_str(text)
                .map_err(|error| ShellError::sync_snapshot_invalid(format!("the file could not be read: {error}")))?;
            if envelope.protection != PROTECTION_PASSPHRASE {
                return Err(ShellError::sync_snapshot_invalid(format!(
                    "the file is protected with '{}', which this build cannot open",
                    envelope.protection
                )));
            }
            Ok(OpenedSnapshot::Sealed(Box::new(envelope)))
        }
        other => Err(ShellError::sync_snapshot_invalid(format!(
            "the file says it is version {other}, which is not one this application writes"
        ))),
    }
}
