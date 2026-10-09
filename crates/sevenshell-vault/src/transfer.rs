//! A bundle of sessions, groups and their secrets, on its way to or from a file.
//!
//! Export and import live here rather than in the host that opens the file dialog, because this is
//! where the secrets already are: a file written under a passphrase carries logins, and the code
//! that can produce one should be the code that holds them and wipes them. The host moves bytes and
//! picks a file; nothing above this crate sees a secret in either direction, and the file is the
//! only copy of one that ever leaves.
//!
//! **Credentials travel sealed, or they do not travel.** A file this build writes carries the
//! stored passwords in one of exactly two ways: not at all, or inside a payload sealed under a
//! passphrase whoever wrote the file chose. There is deliberately no third form. The one this crate
//! used to have — a plain JSON file with the passwords sitting in it — is a file that cannot be told
//! apart from one that is safe to mail, by the only person who could still stop it: the one tidying
//! up their downloads folder a week later. So the two forms are two functions, and no way to write
//! the third.
//!
//! Reading is more forgiving than writing. A file written before that rule existed still imports,
//! secrets and all: it is the only copy of a library somebody may no longer be able to reach, and
//! refusing it would punish them for having used the feature as it was.
//!
//! Three rules shape what an import does, and all three are about not surprising the person who
//! asked for it:
//!
//! 1. **An import adds, and never overwrites.** An id that is already taken is replaced with a
//!    fresh one, so importing the same file twice makes a second copy rather than quietly
//!    replacing what somebody had.
//! 2. **An imported thing only refers to things in the same bundle.** A group whose parent is not
//!    in the file lands at the root, a session whose group is absent lands in none, and a hop the
//!    file does not carry is dropped — a file must not be able to re-point at whatever happens to
//!    be on this machine, and an id in a file somebody mailed over says nothing about intent.
//! 3. **What could not be carried is counted.** A session naming a key this device does not hold,
//!    or jumping through a session the file did not include, is still imported and then reported:
//!    the alternative is a session that fails to connect later for a reason nobody can see.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use sevenshell_error::ShellError;
use sevenshell_types::{AuthMethod, Session, SessionGroup};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::credential::{SecretEntry, SecretKind};
use crate::crypto::{self, KdfParams, SealedPayload};
use crate::document::{Document, SessionSecret};

/// The `format` field this build writes, and the only one it reads.
pub const BUNDLE_FORMAT: &str = "sevenshell-sessions";

/// The layout of an unsealed bundle: sessions and groups, and no credential anywhere in it.
///
/// Still the number this build writes for that form, and still the number the files it wrote
/// before the rule above existed carry — those are read, not written.
pub const BUNDLE_VERSION: u32 = 1;

/// The layout of a sealed bundle: an envelope naming its KDF parameters, and one payload.
///
/// A separate number rather than a field on the old one, because a build that predates this cannot
/// open a payload and must not be left thinking it read the whole file: `parse` refuses a version
/// it does not know, which turns "the import silently dropped every password" into a sentence.
pub const BUNDLE_SEALED_VERSION: u32 = 2;

/// The only `protection` value this build writes or opens.
pub const PROTECTION_PASSPHRASE: &str = "passphrase";

/// The shortest passphrase this build will seal a file under.
///
/// Not a claim about strength — a four-character passphrase is not a secret worth deriving a key
/// from. It is a floor against a slip, and the one thing it really rules out is an empty field
/// accepted because nobody said no.
pub const MIN_PASSPHRASE_LEN: usize = 4;

/// A session file larger than this is not one. A bundle is a few kilobytes at its most; the limit
/// is here so a mistyped path that lands on a disk image is refused before it is read into memory.
pub const MAX_BUNDLE_BYTES: u64 = 8 * 1024 * 1024;

/// A file of sessions, groups and — when they were asked for — the secrets that go with them.
///
/// Not `Debug` for the same reason [`SecretEntry`] is not: this struct is mostly a list of them.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionBundle {
    pub format: String,
    pub version: u32,
    /// Seconds since the Unix epoch, for whoever reads the file by hand.
    pub exported_at: u64,
    #[serde(default)]
    pub sessions: Vec<Session>,
    #[serde(default)]
    pub groups: Vec<SessionGroup>,
    /// Absent when the export was asked to leave credentials behind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<SecretEntry>,
}

impl std::fmt::Debug for SessionBundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionBundle")
            .field("format", &self.format)
            .field("version", &self.version)
            .field("sessions", &self.sessions.len())
            .field("groups", &self.groups.len())
            .field("secrets", &self.secrets.len())
            .finish()
    }
}

impl SessionBundle {
    /// Reads a file's text as a plain bundle, refusing anything this build does not understand.
    ///
    /// The plain form only: a sealed file read this way would parse — every field it does not have
    /// is optional — and come back as a library with nothing in it. [`open`] is what tells the two
    /// apart, and it is the door a chosen file goes through.
    pub fn parse(text: &str) -> Result<Self, ShellError> {
        let bundle: Self = serde_json::from_str(text)
            .map_err(|error| ShellError::session_bundle_invalid(format!("the file could not be read: {error}")))?;

        check_bundle(&bundle)?;
        Ok(bundle)
    }

    /// The text to write into the file. Pretty-printed on purpose: an export is a thing people
    /// open, diff and occasionally hand-edit, and one line of JSON is none of those.
    pub fn to_json(&self) -> Result<String, ShellError> {
        serde_json::to_string_pretty(self).map_err(|error| ShellError::session_bundle_io(error.to_string()))
    }
}

/// What a bundle's own fields promise about it, checked wherever one is read.
fn check_bundle(bundle: &SessionBundle) -> Result<(), ShellError> {
    if bundle.format != BUNDLE_FORMAT {
        return Err(ShellError::session_bundle_invalid(
            "the file is not a 7Shell session export",
        ));
    }
    if bundle.version > BUNDLE_VERSION {
        return Err(ShellError::store_schema_newer(bundle.version, BUNDLE_VERSION));
    }
    Ok(())
}

/// A sealed bundle: the parameters to open it with, and the payload itself.
///
/// Everything a reader needs is in here, including the derivation cost — the file is the authority
/// on how it was written, so raising the defaults later cannot make an existing export unopenable.
/// What is *not* in here is any hint about what the payload holds: not a session count, not a
/// group name, not a user name. A file somebody finds learns nothing from being looked at.
///
/// `Debug` is derived because nothing here is a secret: the payload is ciphertext, and whoever is
/// reading a log line with this in it is already holding the file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleEnvelope {
    pub format: String,
    pub version: u32,
    /// Seconds since the Unix epoch, for whoever reads the file by hand.
    pub exported_at: u64,
    /// Written out rather than left to be inferred: a person who opens the file should be able to
    /// see that a passphrase stands between them and what is inside it.
    pub protection: String,
    /// Flattened, so the file's shape is the same as it was when these three fields were spelled
    /// out here — and so a snapshot's envelope and this one stay one implementation.
    #[serde(flatten)]
    pub sealed: SealedPayload,
}

impl BundleEnvelope {
    /// Opens the payload with a passphrase, giving back the library that went in.
    ///
    /// A wrong passphrase, a payload truncated on the way here and one that was sealed under
    /// another context are one answer, the same way they are for the vault and for the same
    /// reason. The caller that can act on it is the question being asked next, which keeps its
    /// field on screen and lets the person try again.
    pub fn unseal(&self, passphrase: &str) -> Result<SessionBundle, ShellError> {
        let payload = crypto::open_payload(passphrase, crypto::PayloadPurpose::ExportBundle, &self.sealed)?;

        let bundle: SessionBundle = serde_json::from_slice(payload.as_slice())
            .map_err(|error| ShellError::session_bundle_invalid(format!("the file could not be read: {error}")))?;

        // What the payload says about itself, checked after it opens: a payload sealed by a future
        // build under this same envelope would otherwise be read as this build's layout.
        check_bundle(&bundle)?;
        Ok(bundle)
    }

    /// The text to write into the file. Pretty-printed for the same reason the plain form is.
    pub fn to_json(&self) -> Result<String, ShellError> {
        serde_json::to_string_pretty(self).map_err(|error| ShellError::session_bundle_io(error.to_string()))
    }
}

/// What a chosen file turned out to be.
///
/// The two forms are answered separately rather than as one "bundle with maybe a passphrase",
/// because what happens next differs: one is imported now, the other is a question to ask first.
///
/// `Debug` is derived because neither variant can print a secret: a bundle prints its counts, and
/// an envelope holds ciphertext.
#[derive(Debug)]
pub enum OpenedBundle {
    /// Sessions and groups, readable as they are.
    Plain(Box<SessionBundle>),
    /// A payload and the parameters needed to open it.
    Sealed(Box<BundleEnvelope>),
}

/// The part of a bundle that says what the file is, read before anything else is.
///
/// A header rather than a parse of the whole file, because the two layouts do not overlap in a way
/// a single parse can survive: the fields a sealed file does not have are the optional ones.
#[derive(Deserialize)]
struct BundleHeader {
    format: String,
    version: u32,
}

/// Reads a chosen file's text, refusing anything this build does not understand.
///
/// Both refusals happen before any of it is applied: a wrong `format` is somebody else's file, and
/// a newer `version` is one this build would drop fields from on the way in. The detail carries
/// what the parser said, which for a truncated file is the only clue there is.
pub fn open(text: &str) -> Result<OpenedBundle, ShellError> {
    let header: BundleHeader = serde_json::from_str(text)
        .map_err(|error| ShellError::session_bundle_invalid(format!("the file could not be read: {error}")))?;

    if header.format != BUNDLE_FORMAT {
        return Err(ShellError::session_bundle_invalid(
            "the file is not a 7Shell session export",
        ));
    }
    if header.version > BUNDLE_SEALED_VERSION {
        return Err(ShellError::store_schema_newer(header.version, BUNDLE_SEALED_VERSION));
    }

    match header.version {
        BUNDLE_VERSION => Ok(OpenedBundle::Plain(Box::new(SessionBundle::parse(text)?))),
        BUNDLE_SEALED_VERSION => {
            let envelope: BundleEnvelope = serde_json::from_str(text)
                .map_err(|error| ShellError::session_bundle_invalid(format!("the file could not be read: {error}")))?;
            if envelope.protection != PROTECTION_PASSPHRASE {
                return Err(ShellError::session_bundle_invalid(format!(
                    "the file is protected with '{}', which this build cannot open",
                    envelope.protection
                )));
            }
            Ok(OpenedBundle::Sealed(Box::new(envelope)))
        }
        other => Err(ShellError::session_bundle_invalid(format!(
            "the file says it is version {other}, which is not one this application writes"
        ))),
    }
}

/// An export on its way out: the text for the file, and the counts the interface is told.
///
/// Counts rather than the bundle itself, because a sealed file's text is not a bundle anything
/// above this crate could read back — the numbers are all that is left to say about it.
pub struct ExportedBundle {
    pub text: String,
    pub sessions: u32,
    pub groups: u32,
    pub secrets: u32,
    /// Whether a passphrase stands between the file and what is in it.
    pub encrypted: bool,
}

/// Hand-written for the reason [`SessionBundle`]'s is: the text here is either a library with no
/// credential in it or a sealed payload, and a future variant that carried a third thing must not
/// be printable by default.
impl std::fmt::Debug for ExportedBundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportedBundle")
            .field("sessions", &self.sessions)
            .field("groups", &self.groups)
            .field("secrets", &self.secrets)
            .field("encrypted", &self.encrypted)
            .field("text", &format_args!("<{} bytes>", self.text.len()))
            .finish()
    }
}

/// The plain form: every saved session and group, and not one credential.
///
/// The values are left out entirely rather than blanked: a bundle that carries a `secrets` array of
/// empty strings reads as a file whose passwords were lost, and one with no array at all reads as a
/// file that was asked to leave them behind.
pub(crate) fn build_plain_bundle(document: &Document, now: u64) -> Result<ExportedBundle, ShellError> {
    let bundle = assemble(document, false, now);

    Ok(ExportedBundle {
        text: bundle.to_json()?,
        sessions: bundle.sessions.len() as u32,
        groups: bundle.groups.len() as u32,
        secrets: bundle.secrets.len() as u32,
        encrypted: false,
    })
}

/// The sealed form: the same, plus every credential the library holds, under a passphrase.
///
/// The passphrase is checked before the secrets are read out of the document, so a slip in the
/// dialog cannot be answered with a file that already has the logins in it.
pub(crate) fn build_sealed_bundle(
    document: &Document,
    passphrase: &str,
    kdf: &KdfParams,
    now: u64,
) -> Result<ExportedBundle, ShellError> {
    if passphrase.chars().count() < MIN_PASSPHRASE_LEN {
        return Err(ShellError::session_bundle_passphrase_short(MIN_PASSPHRASE_LEN));
    }

    let bundle = assemble(document, true, now);
    let counts = (
        bundle.sessions.len() as u32,
        bundle.groups.len() as u32,
        bundle.secrets.len() as u32,
    );

    // The one buffer here that holds a login in the clear: wiped when it goes out of scope, and
    // never formatted anywhere.
    let payload = Zeroizing::new(
        serde_json::to_vec(&bundle)
            .map_err(|error| ShellError::session_bundle_io(format!("the sessions could not be written: {error}")))?,
    );

    let envelope = BundleEnvelope {
        format: BUNDLE_FORMAT.to_owned(),
        version: BUNDLE_SEALED_VERSION,
        exported_at: now,
        protection: PROTECTION_PASSPHRASE.to_owned(),
        sealed: crypto::seal_payload(
            passphrase,
            crypto::PayloadPurpose::ExportBundle,
            payload.as_slice(),
            kdf,
        )?,
    };
    Ok(ExportedBundle {
        text: envelope.to_json()?,
        sessions: counts.0,
        groups: counts.1,
        secrets: counts.2,
        encrypted: true,
    })
}

/// What a bundle holds, read off a document.
///
/// `with_secrets` is private to this module, which is the point: the plain form is built without
/// the credentials and the sealed form with them, and there is no third call that could put them
/// into a file that anyone can read.
fn assemble(document: &Document, with_secrets: bool, now: u64) -> SessionBundle {
    let sessions = document.sessions();
    let secrets = if with_secrets {
        sessions
            .iter()
            .filter_map(|session| {
                let secret = document.secret_for_session(&session.id)?;
                let (kind, value) = match &secret {
                    SessionSecret::Password(value) => (SecretKind::Password, value.clone()),
                    SessionSecret::KeyPassphrase(value) => (SecretKind::KeyPassphrase, value.clone()),
                };
                Some(SecretEntry {
                    session_id: session.id.clone(),
                    kind,
                    value,
                })
            })
            .collect()
    } else {
        Vec::new()
    };

    SessionBundle {
        format: BUNDLE_FORMAT.to_owned(),
        version: BUNDLE_VERSION,
        exported_at: now,
        sessions,
        groups: document.groups.clone(),
        secrets,
    }
}

/// One session an import will write, with the secret that came with it.
pub(crate) struct PlannedSession {
    pub session: Session,
    pub secret: Option<SessionSecret>,
}

/// Everything an import will write, worked out before anything is written.
pub(crate) struct ImportPlan {
    pub sessions: Vec<PlannedSession>,
    pub groups: Vec<SessionGroup>,
    /// Sessions that named a key this vault does not hold.
    pub missing_keys: u32,
    /// Sessions that jumped through a session the bundle did not carry.
    pub dropped_hops: u32,
}

/// Works out what an import does to a document, without touching it.
///
/// A plan rather than a sequence of writes so that the whole answer — every id, every reference,
/// every count — is decided in one place and can be asserted in a test without a vault file. The
/// caller applies it; nothing here mutates anything.
pub(crate) fn plan_import(bundle: SessionBundle, existing: &Document) -> ImportPlan {
    let taken_groups: HashSet<&str> = existing.groups.iter().map(|group| group.id.as_str()).collect();
    let taken_sessions: HashSet<&str> = existing.sessions.iter().map(|session| session.id.as_str()).collect();
    let held_keys: HashSet<&str> = existing.keys.iter().map(|key| key.key_ref.as_str()).collect();

    // Groups first: a session's place in the tree is only known once the folder it lands in has an
    // id, and a group's parent is another group, so both passes are over the same list.
    //
    // The ids are collected per entry as well as per name, because a file somebody edited may name
    // the same group twice: each of those becomes a group of its own, while anything that referred
    // to that name refers to the first one. Nothing in a file is dropped for being odd.
    let mut group_ids: HashMap<String, String> = HashMap::new();
    let mut group_entries: Vec<String> = Vec::with_capacity(bundle.groups.len());
    let mut used: HashSet<String> = taken_groups.iter().map(|id| (*id).to_owned()).collect();
    for group in &bundle.groups {
        let id = free_id(&group.id, &mut used);
        group_ids.entry(group.id.clone()).or_insert_with(|| id.clone());
        group_entries.push(id);
    }

    let groups = bundle
        .groups
        .iter()
        .zip(group_entries)
        .map(|(group, id)| SessionGroup {
            id,
            name: group.name.clone(),
            // Only a parent from this file: an id that merely matches something already here is a
            // coincidence, and a group that arrived from elsewhere must not nest itself inside it.
            parent_id: group
                .parent_id
                .as_ref()
                .and_then(|parent| group_ids.get(parent))
                .cloned(),
            order: group.order,
            // How the group was arranged is part of what the file carries: an import that quietly
            // unpinned it would rearrange somebody's list without saying so.
            pinned: group.pinned,
        })
        .collect();

    let mut session_ids: HashMap<String, String> = HashMap::new();
    let mut used_sessions: HashSet<String> = taken_sessions.iter().map(|id| (*id).to_owned()).collect();
    for session in &bundle.sessions {
        let id = free_id(&session.id, &mut used_sessions);
        session_ids.insert(session.id.clone(), id);
    }

    let secrets: HashMap<&str, &SecretEntry> = bundle
        .secrets
        .iter()
        .map(|secret| (secret.session_id.as_str(), secret))
        .collect();

    let mut missing_keys = 0;
    let mut dropped_hops = 0;
    let sessions = bundle
        .sessions
        .iter()
        .map(|session| {
            let mut imported = session.clone();
            imported.id = session_ids[&session.id].clone();
            imported.group_id = session
                .group_id
                .as_ref()
                .and_then(|group| group_ids.get(group))
                .cloned();

            let mut hops = Vec::with_capacity(session.jump.len());
            for hop in &session.jump {
                match session_ids.get(hop) {
                    Some(id) => hops.push(id.clone()),
                    None => dropped_hops += 1,
                }
            }
            imported.jump = hops;

            if session.auth.method == AuthMethod::PublicKey
                && let Some(key_ref) = imported.auth.key.as_ref().and_then(|key| key.key_ref.as_deref())
                && !held_keys.contains(key_ref)
            {
                missing_keys += 1;
            }

            // The flag is about *this* vault's secrets table, not about the file: what the file says
            // was stored on the machine it came from is not a promise about this one.
            imported.auth.has_secret = false;

            PlannedSession {
                session: imported,
                secret: secrets.get(session.id.as_str()).map(|secret| match secret.kind {
                    SecretKind::Password => SessionSecret::Password(secret.value.clone()),
                    SecretKind::KeyPassphrase => SessionSecret::KeyPassphrase(secret.value.clone()),
                }),
            }
        })
        .collect();

    ImportPlan {
        sessions,
        groups,
        missing_keys,
        dropped_hops,
    }
}

/// An id nothing uses: the one the file carried when it is free, a fresh one when it is not.
fn free_id(id: &str, used: &mut HashSet<String>) -> String {
    if !used.contains(id) && !id.is_empty() {
        used.insert(id.to_owned());
        return id.to_owned();
    }
    loop {
        let candidate = Uuid::new_v4().to_string();
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sevenshell_types::{SessionAuth, TerminalSettings};

    /// Derivation costs a file does not need to be a real one to be read back: the tests here are
    /// about the layout and the rules, and a full-cost Argon2id per case would only make them slow.
    fn fast_params() -> KdfParams {
        KdfParams {
            memory_kib: 8,
            iterations: 1,
            parallelism: 1,
            ..KdfParams::default()
        }
    }

    fn session(id: &str, name: &str) -> Session {
        Session {
            id: id.to_owned(),
            name: name.to_owned(),
            group_id: None,
            protocol: sevenshell_types::Protocol::Ssh,
            host: "example.test".to_owned(),
            port: 22,
            user: "root".to_owned(),
            auth: SessionAuth::with_password(AuthMethod::Password),
            terminal: TerminalSettings::default(),
            jump: Vec::new(),
            forwards: Vec::new(),
            updated_at: 1_700_000_000,
        }
    }

    fn group(id: &str, name: &str) -> SessionGroup {
        SessionGroup {
            id: id.to_owned(),
            name: name.to_owned(),
            parent_id: None,
            order: 0,
            pinned: false,
        }
    }

    fn bundle(sessions: Vec<Session>, groups: Vec<SessionGroup>) -> SessionBundle {
        SessionBundle {
            format: BUNDLE_FORMAT.to_owned(),
            version: BUNDLE_VERSION,
            exported_at: 0,
            sessions,
            groups,
            secrets: Vec::new(),
        }
    }

    #[test]
    fn a_bundle_that_was_exported_here_reads_back() {
        let text = bundle(vec![session("a", "web")], vec![group("g", "Work")])
            .to_json()
            .expect("write");

        let read = SessionBundle::parse(&text).expect("read");

        assert_eq!(read.sessions.len(), 1);
        assert_eq!(read.sessions[0].name, "web");
        assert_eq!(read.groups[0].name, "Work");
    }

    #[test]
    fn a_file_from_somewhere_else_is_refused_by_its_format_field() {
        let text = r#"{"format":"other-tool","version":1,"exportedAt":0,"sessions":[]}"#;

        let error = SessionBundle::parse(text).expect_err("not ours");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SessionBundleInvalid);
    }

    #[test]
    fn a_newer_bundle_is_refused_rather_than_read_partially() {
        let text = r#"{"format":"sevenshell-sessions","version":99,"exportedAt":0,"sessions":[]}"#;

        let error = SessionBundle::parse(text).expect_err("newer than this build");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::StoreSchemaNewer);
    }

    #[test]
    fn a_damaged_file_says_so_instead_of_panicking() {
        let error = SessionBundle::parse("{\"format\": \"sevenshell-sessions\",").expect_err("truncated");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SessionBundleInvalid);
    }

    #[test]
    fn an_import_keeps_the_ids_it_can_and_replaces_the_ones_already_taken() {
        let mut existing = Document::empty();
        existing.save_session(session("a", "already here"), None);

        let plan = plan_import(bundle(vec![session("a", "from the file")], Vec::new()), &existing);

        assert_eq!(plan.sessions.len(), 1);
        let imported = &plan.sessions[0].session;
        assert_ne!(imported.id, "a", "the id was taken, so the import got its own");
        assert_eq!(imported.name, "from the file", "and nothing about the old one changed");
        assert_eq!(existing.sessions.len(), 1, "the plan writes nothing by itself");
    }

    #[test]
    fn an_imported_group_only_nests_under_groups_from_the_same_file() {
        let parent = group("g1", "Work");
        let mut nested = group("g2", "Rack");
        nested.parent_id = Some("g1".to_owned());
        let mut orphan = group("g3", "Lab");
        orphan.parent_id = Some("not-in-this-file".to_owned());

        let plan = plan_import(bundle(Vec::new(), vec![parent, nested, orphan]), &Document::empty());

        let rack = plan.groups.iter().find(|group| group.name == "Rack").expect("rack");
        let lab = plan.groups.iter().find(|group| group.name == "Lab").expect("lab");
        assert_eq!(rack.parent_id.as_deref(), Some("g1"));
        assert_eq!(lab.parent_id, None, "a parent the file does not carry means the root");
    }

    #[test]
    fn a_group_that_collides_takes_its_children_with_it() {
        let parent = group("g1", "Work");
        let mut nested = group("g2", "Rack");
        nested.parent_id = Some("g1".to_owned());

        let mut existing = Document::empty();
        existing.save_group(group("g1", "Somebody else's group"));

        let plan = plan_import(bundle(Vec::new(), vec![parent, nested]), &existing);

        let imported_parent = plan.groups.iter().find(|group| group.name == "Work").expect("work");
        let imported_child = plan.groups.iter().find(|group| group.name == "Rack").expect("rack");
        assert_ne!(imported_parent.id, "g1");
        assert_eq!(
            imported_child.parent_id.as_deref(),
            Some(imported_parent.id.as_str()),
            "the child follows the parent's new id rather than landing at the root"
        );
    }

    #[test]
    fn a_file_that_names_the_same_group_twice_makes_two_groups() {
        // A file somebody edited by hand is data, not a promise: both are made, and the reference
        // resolves to the first — which is what a reader of that file would expect it to mean.
        let first = group("g1", "Work");
        let second = group("g1", "Work (second)");
        let mut child = group("g2", "Rack");
        child.parent_id = Some("g1".to_owned());

        let plan = plan_import(bundle(Vec::new(), vec![first, second, child]), &Document::empty());

        assert_eq!(plan.groups.len(), 3, "nothing in the file is dropped for being odd");
        let ids: HashSet<&str> = plan.groups.iter().map(|group| group.id.as_str()).collect();
        assert_eq!(ids.len(), 3, "and every one of them has an id of its own");
        assert_eq!(
            plan.groups[2].parent_id.as_deref(),
            Some("g1"),
            "the child points at the first group that claimed the name"
        );
    }

    #[test]
    fn a_session_lands_in_the_imported_group_and_nowhere_else() {
        let mut welcome = group("g1", "Welcome");
        welcome.order = 3;
        let mut existing = Document::empty();
        existing.save_group(group("g1", "Somebody else's group"));

        let mut session = session("a", "web");
        session.group_id = Some("g1".to_owned());

        let plan = plan_import(bundle(vec![session], vec![welcome]), &existing);

        let imported_group = &plan.groups[0];
        assert_eq!(
            plan.sessions[0].session.group_id.as_deref(),
            Some(imported_group.id.as_str())
        );
    }

    #[test]
    fn a_hop_the_file_does_not_carry_is_dropped_and_counted() {
        let mut through = session("a", "web");
        through.jump = vec!["bastion".to_owned(), "b".to_owned()];

        let plan = plan_import(
            bundle(vec![through, session("b", "jump")], Vec::new()),
            &Document::empty(),
        );

        let web = plan
            .sessions
            .iter()
            .find(|entry| entry.session.name == "web")
            .expect("web");
        let jump = plan
            .sessions
            .iter()
            .find(|entry| entry.session.name == "jump")
            .expect("jump");
        assert_eq!(web.session.jump, vec![jump.session.id.clone()]);
        assert_eq!(plan.dropped_hops, 1);
    }

    #[test]
    fn a_session_that_names_a_key_this_device_does_not_hold_is_counted() {
        let mut keyed = session("a", "web");
        keyed.auth = SessionAuth {
            method: AuthMethod::PublicKey,
            key: Some(sevenshell_types::KeyInfo {
                key_ref: Some("key-somewhere-else".to_owned()),
                display_name: "id_ed25519".to_owned(),
                algorithm: "ssh-ed25519".to_owned(),
                fingerprint: "SHA256:AAAA".to_owned(),
                has_passphrase: false,
            }),
            has_secret: false,
        };

        let plan = plan_import(bundle(vec![keyed], Vec::new()), &Document::empty());

        assert_eq!(plan.missing_keys, 1);
        assert_eq!(plan.sessions.len(), 1, "it is still imported, and then reported");
    }

    #[test]
    fn a_secret_from_the_file_is_planned_against_the_session_it_came_with() {
        let mut bundle = bundle(vec![session("a", "web")], Vec::new());
        bundle.secrets.push(SecretEntry {
            session_id: "a".to_owned(),
            kind: SecretKind::Password,
            value: "hunter2".to_owned(),
        });

        let plan = plan_import(bundle, &Document::empty());

        let secret = plan.sessions[0].secret.as_ref().expect("the secret travelled with it");
        assert_eq!(secret.expose(), "hunter2");
    }

    #[test]
    fn a_plain_export_carries_no_credential_and_the_flag_still_reports_one_is_stored() {
        let mut document = Document::empty();
        document.save_session(session("a", "web"), Some(SessionSecret::Password("hunter2".to_owned())));

        let written = build_plain_bundle(&document, 10).expect("write");
        let bundle = SessionBundle::parse(&written.text).expect("read");

        assert!(!written.encrypted);
        assert_eq!(written.secrets, 0);
        assert!(
            bundle.secrets.is_empty(),
            "leaving secrets behind means no array at all"
        );
        assert!(
            bundle.sessions[0].auth.has_secret,
            "the flag describes this vault's secrets table, not the file"
        );
    }

    #[test]
    fn a_sealed_export_carries_the_credentials_and_opens_with_its_passphrase() {
        let mut document = Document::empty();
        document.save_session(session("a", "web"), Some(SessionSecret::Password("hunter2".to_owned())));

        let written = build_sealed_bundle(&document, "correct horse", &fast_params(), 10).expect("seal");
        let OpenedBundle::Sealed(envelope) = open(&written.text).expect("open") else {
            panic!("a sealed file must be read as sealed");
        };

        assert!(written.encrypted);
        assert_eq!((written.sessions, written.groups, written.secrets), (1, 0, 1));
        let bundle = envelope.unseal("correct horse").expect("unseal");
        assert_eq!(bundle.secrets[0].value, "hunter2");
    }

    #[test]
    fn a_sealed_file_says_nothing_about_what_is_inside_it() {
        let mut document = Document::empty();
        document.save_session(session("a", "web"), Some(SessionSecret::Password("hunter2".to_owned())));

        let written = build_sealed_bundle(&document, "correct horse", &fast_params(), 10).expect("seal");

        for leak in ["hunter2", "web", "example.test", "root"] {
            assert!(
                !written.text.contains(leak),
                "a sealed file must not name what it holds, and it spells out {leak}"
            );
        }
    }

    #[test]
    fn the_wrong_passphrase_opens_nothing() {
        let mut document = Document::empty();
        document.save_session(session("a", "web"), Some(SessionSecret::Password("hunter2".to_owned())));
        let written = build_sealed_bundle(&document, "correct horse", &fast_params(), 10).expect("seal");
        let OpenedBundle::Sealed(envelope) = open(&written.text).expect("open") else {
            panic!("sealed");
        };

        let error = envelope.unseal("Correct Horse").expect_err("not the passphrase");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SessionBundlePassphrase);
    }

    #[test]
    fn a_passphrase_too_short_to_be_one_is_refused_before_anything_is_read_out() {
        let mut document = Document::empty();
        document.save_session(session("a", "web"), Some(SessionSecret::Password("hunter2".to_owned())));

        let error = build_sealed_bundle(&document, "abc", &fast_params(), 10).expect_err("too short");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SessionBundlePassphraseShort);
        assert_eq!(error.message_params().get("minimum").map(String::as_str), Some("4"));
    }

    #[test]
    fn a_sealed_file_is_not_read_as_a_plain_one_with_nothing_in_it() {
        // The failure this guards against is silent: every field a sealed envelope does not have is
        // optional, so a plain read of one would hand back an empty library rather than an error.
        let document = Document::empty();
        let written = build_sealed_bundle(&document, "correct horse", &fast_params(), 10).expect("seal");

        let opened = open(&written.text).expect("open");

        assert!(matches!(opened, OpenedBundle::Sealed(_)));
    }

    #[test]
    fn a_sealed_file_from_a_future_build_is_refused_rather_than_guessed_at() {
        let sealed = r#"{"format":"sevenshell-sessions","version":99,"exportedAt":0,"protection":"passphrase"}"#;

        let error = open(sealed).expect_err("newer than this build");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::StoreSchemaNewer);
    }

    #[test]
    fn a_protection_this_build_cannot_open_is_refused_by_name() {
        let sealed = format!(
            r#"{{"format":"sevenshell-sessions","version":{},"exportedAt":0,"protection":"age",
                 "kdf":{{"name":"argon2id","memoryKib":65536,"iterations":3,"parallelism":1}},
                 "salt":"AAAAAAAAAAAAAAAAAAAAAA==","payload":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}}"#,
            BUNDLE_SEALED_VERSION
        );

        let error = open(&sealed).expect_err("a protection this build does not implement");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SessionBundleInvalid);
        assert!(
            error.detail().is_some_and(|detail| detail.contains("age")),
            "the refusal should name what the file asked for, and it said {:?}",
            error.detail()
        );
    }

    #[test]
    fn an_export_written_before_credentials_ever_travelled_sealed_still_imports() {
        // A file this build used to write: plain, with the passwords in it. It is somebody's only
        // copy of a library, so it reads — the rule that changed is about writing.
        let mut legacy = bundle(vec![session("a", "web")], Vec::new());
        legacy.secrets.push(SecretEntry {
            session_id: "a".to_owned(),
            kind: SecretKind::Password,
            value: "hunter2".to_owned(),
        });
        let text = legacy.to_json().expect("write");

        let OpenedBundle::Plain(read) = open(&text).expect("open") else {
            panic!("a plain file must be read as plain");
        };

        let plan = plan_import(*read, &Document::empty());
        assert_eq!(
            plan.sessions[0].secret.as_ref().map(SessionSecret::expose),
            Some("hunter2")
        );
    }

    #[test]
    fn a_secret_never_prints_and_neither_does_the_bundle() {
        let mut document = Document::empty();
        document.save_session(session("a", "web"), Some(SessionSecret::Password("hunter2".to_owned())));
        let sealed = build_sealed_bundle(&document, "correct horse", &fast_params(), 10).expect("seal");
        let OpenedBundle::Sealed(envelope) = open(&sealed.text).expect("open") else {
            panic!("sealed");
        };
        let written = envelope.unseal("correct horse").expect("unseal");

        let printed = format!("{:?} {:?} {:?}", written, written.secrets[0], envelope);

        assert!(
            !printed.contains("hunter2"),
            "a secret must not be printable: {printed}"
        );
    }
}
