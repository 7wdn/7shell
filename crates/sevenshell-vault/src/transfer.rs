//! A bundle of sessions, groups and their secrets, on its way to or from a file.
//!
//! Export and import live here rather than in the host that opens the file dialog, because this is
//! where the secrets already are: a bundle written with `include_secrets` carries logins, and the
//! code that can produce one should be the code that holds them and wipes them. The host moves
//! bytes and picks a file; nothing above this crate sees a secret in either direction, and the file
//! is the only copy of one that ever leaves.
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
use zeroize::Zeroize;

use crate::document::{Document, SessionSecret};

/// The `format` field this build writes, and the only one it reads.
pub const BUNDLE_FORMAT: &str = "sevenshell-sessions";

/// The bundle layout this build writes. A newer number is refused rather than guessed at, the same
/// way a newer vault document is.
pub const BUNDLE_VERSION: u32 = 1;

/// A session file larger than this is not one. A bundle is a few kilobytes at its most; the limit
/// is here so a mistyped path that lands on a disk image is refused before it is read into memory.
pub const MAX_BUNDLE_BYTES: u64 = 8 * 1024 * 1024;

/// Which secret an entry holds.
///
/// Written out rather than derived from the session on the way in: the file should say what it
/// carries, so a reader — and a person looking at it — does not have to know the authentication
/// rules to see that a password is in there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BundleSecretKind {
    Password,
    KeyPassphrase,
}

/// One stored secret in a bundle.
///
/// No `Debug`: a derived one would print the value into any log line an unwary caller wrote. What
/// this is *about* is safe to say, and that is all the manual implementation says.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleSecret {
    /// The session the secret belongs to, by the id it had in the file.
    pub session_id: String,
    pub kind: BundleSecretKind,
    pub value: String,
}

impl std::fmt::Debug for BundleSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BundleSecret({:?}, <redacted>)", self.kind)
    }
}

/// The value is wiped when the bundle is dropped.
///
/// The file is the one copy of a secret that leaves this process, and the copy that was held here
/// while it was written has no reason to outlive the write — a `String` handed back to the
/// allocator is a password the next allocation may find. This is also why the field cannot be moved
/// out of the struct by value: taking the value out and leaving the struct behind is exactly the
/// thing that would skip the wipe.
impl Drop for BundleSecret {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

/// A file of sessions, groups and — when they were asked for — the secrets that go with them.
///
/// Not `Debug` for the same reason [`BundleSecret`] is not: this struct is mostly a list of them.
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
    pub secrets: Vec<BundleSecret>,
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
    /// Reads a file's text as a bundle, refusing anything this build does not understand.
    ///
    /// Both refusals are before any of it is applied: a wrong `format` is somebody else's file, and
    /// a newer `version` is one this build would drop fields from on the way in. The detail carries
    /// what the parser said, which for a truncated file is the only clue there is.
    pub fn parse(text: &str) -> Result<Self, ShellError> {
        let bundle: Self = serde_json::from_str(text)
            .map_err(|error| ShellError::session_bundle_invalid(format!("the file could not be read: {error}")))?;

        if bundle.format != BUNDLE_FORMAT {
            return Err(ShellError::session_bundle_invalid(
                "the file is not a 7Shell session export",
            ));
        }
        if bundle.version > BUNDLE_VERSION {
            return Err(ShellError::store_schema_newer(bundle.version, BUNDLE_VERSION));
        }
        Ok(bundle)
    }

    /// The text to write into the file. Pretty-printed on purpose: an export is a thing people
    /// open, diff and occasionally hand-edit, and one line of JSON is none of those.
    pub fn to_json(&self) -> Result<String, ShellError> {
        serde_json::to_string_pretty(self).map_err(|error| ShellError::session_bundle_io(error.to_string()))
    }
}

/// What a bundle holds, read off a document.
///
/// Passing `include_secrets: false` leaves the values out entirely rather than blanking them: a
/// bundle that carries a `secrets` array of empty strings reads as a file whose passwords were
/// lost, and one with no array at all reads as a file that was asked to leave them behind.
pub(crate) fn build_bundle(document: &Document, include_secrets: bool, now: u64) -> SessionBundle {
    let sessions = document.sessions();
    let secrets = if include_secrets {
        sessions
            .iter()
            .filter_map(|session| {
                let secret = document.secret_for_session(&session.id)?;
                let (kind, value) = match &secret {
                    SessionSecret::Password(value) => (BundleSecretKind::Password, value.clone()),
                    SessionSecret::KeyPassphrase(value) => (BundleSecretKind::KeyPassphrase, value.clone()),
                };
                Some(BundleSecret {
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
        })
        .collect();

    let mut session_ids: HashMap<String, String> = HashMap::new();
    let mut used_sessions: HashSet<String> = taken_sessions.iter().map(|id| (*id).to_owned()).collect();
    for session in &bundle.sessions {
        let id = free_id(&session.id, &mut used_sessions);
        session_ids.insert(session.id.clone(), id);
    }

    let secrets: HashMap<&str, &BundleSecret> = bundle
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
                    BundleSecretKind::Password => SessionSecret::Password(secret.value.clone()),
                    BundleSecretKind::KeyPassphrase => SessionSecret::KeyPassphrase(secret.value.clone()),
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
        bundle.secrets.push(BundleSecret {
            session_id: "a".to_owned(),
            kind: BundleSecretKind::Password,
            value: "hunter2".to_owned(),
        });

        let plan = plan_import(bundle, &Document::empty());

        let secret = plan.sessions[0].secret.as_ref().expect("the secret travelled with it");
        assert_eq!(secret.expose(), "hunter2");
    }

    #[test]
    fn an_export_without_secrets_carries_none_and_the_flag_still_reports_none() {
        let mut document = Document::empty();
        document.save_session(session("a", "web"), Some(SessionSecret::Password("hunter2".to_owned())));

        let with = build_bundle(&document, true, 10);
        let without = build_bundle(&document, false, 10);

        assert_eq!(with.secrets.len(), 1);
        assert_eq!(with.secrets[0].value, "hunter2");
        assert!(with.sessions[0].auth.has_secret);
        assert!(
            without.secrets.is_empty(),
            "leaving secrets behind means no array at all"
        );
        assert!(
            without.sessions[0].auth.has_secret,
            "the flag describes this vault's secrets table, not the file"
        );
    }

    #[test]
    fn a_secret_never_prints_and_neither_does_the_bundle() {
        let mut document = Document::empty();
        document.save_session(session("a", "web"), Some(SessionSecret::Password("hunter2".to_owned())));
        let written = build_bundle(&document, true, 10);

        let printed = format!("{written:?} {:?}", written.secrets[0]);

        assert!(
            !printed.contains("hunter2"),
            "a secret must not be printable: {printed}"
        );
    }
}
