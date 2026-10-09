//! What a restore will write, worked out before anything is written.
//!
//! Everything the rules need — which ids are already taken, which keys this machine holds, which
//! references do not survive the trip — is decided here. The write afterwards is mechanical, and
//! the answer can be asserted without a vault.
//!
//! Two of the rules are the session export's, because a file that arrived is a file that arrived:
//! **an item whose id is taken is not written over unless somebody said so**, and **nothing in the
//! file points at anything that is not in the file**. The third is the restore's own: a key this
//! machine already holds is recognised by its fingerprint and the arrival points at the handle that
//! is here, rather than importing a second copy of the same key.

use std::collections::{HashMap, HashSet};

use sevenshell_error::ShellError;
use sevenshell_types::{DEFAULT_GROUP_ID, SessionGroup, SnippetGroup, SyncApplySummary, SyncContents};
use sevenshell_vault::{KeyEntry, Restore, RestoredSession, SecretEntry, SessionSecret, Vault};
use uuid::Uuid;

use crate::settings::PortableSettings;
use crate::snapshot::SyncSnapshot;

/// What is already on this machine, as the plan needs to see it.
#[derive(Default)]
pub struct Destination {
    session_ids: HashSet<String>,
    group_ids: HashSet<String>,
    snippet_group_ids: HashSet<String>,
    snippet_ids: HashSet<String>,
    word_ids: HashSet<String>,
    /// The command-and-word pairs that are already rows, because that pair is what a word *is*.
    word_pairs: HashSet<(String, String)>,
    key_refs: HashSet<String>,
    /// Fingerprint to the handle this machine already holds that key under.
    key_ref_by_fingerprint: HashMap<String, String>,
}

impl Destination {
    /// Reads what is here, through the vault's own answers.
    pub fn of(vault: &Vault) -> Result<Self, ShellError> {
        let sessions = vault.sessions()?;
        let groups = vault.groups()?;
        let snippet_groups = vault.snippet_groups()?;
        let words = vault.completion_words()?;
        let keys = vault.keys()?;

        Ok(Self {
            session_ids: sessions.into_iter().map(|session| session.id).collect(),
            group_ids: groups.into_iter().map(|group| group.id).collect(),
            snippet_group_ids: snippet_groups.iter().map(|group| group.id.clone()).collect(),
            snippet_ids: snippet_groups
                .iter()
                .flat_map(|group| group.commands.iter().map(|command| command.id.clone()))
                .collect(),
            word_ids: words.iter().map(|word| word.id.clone()).collect(),
            word_pairs: words.into_iter().map(|word| (word.command, word.word)).collect(),
            key_refs: keys.iter().filter_map(|key| key.key_ref.clone()).collect(),
            key_ref_by_fingerprint: keys
                .into_iter()
                .filter_map(|key| key.key_ref.map(|reference| (key.fingerprint, reference)))
                .collect(),
        })
    }
}

/// The ids a restore is writing into.
///
/// One of these per kind of thing, because an id only has to be free among its own kind: a session
/// and a group may both be called `a` without anybody being confused.
struct Ids {
    used: HashSet<String>,
    replace: bool,
    /// How many arrivals found their id already here. What that meant is decided by `replace`.
    collided: u32,
}

impl Ids {
    fn new(used: HashSet<String>, replace: bool) -> Self {
        Self {
            used,
            replace,
            collided: 0,
        }
    }

    /// The id an arriving item lands under.
    ///
    /// The file's own id when it is free, and also when the restore was asked to replace what is
    /// here — where landing on the same id is exactly what makes it a replacement rather than a
    /// copy. A fresh one otherwise, so that restoring the same file twice makes a second copy
    /// rather than quietly replacing what somebody had.
    fn land(&mut self, id: &str) -> String {
        let taken = !id.is_empty() && self.used.contains(id);
        if taken {
            self.collided += 1;
        }

        let landing = if !id.is_empty() && (self.replace || !taken) {
            id.to_owned()
        } else {
            loop {
                let candidate = Uuid::new_v4().to_string();
                if self.used.insert(candidate.clone()) {
                    break candidate;
                }
            }
        };

        self.used.insert(landing.clone());
        landing
    }
}

/// What a restore will write, and what to say about it.
pub struct RestorePlan {
    restore: Restore,
    settings: Option<PortableSettings>,
    summary: SyncApplySummary,
}

impl RestorePlan {
    /// What this plan will write, for a caller that wants to show it before it happens.
    pub fn summary(&self) -> SyncApplySummary {
        self.summary.clone()
    }

    /// Takes the plan apart for the write.
    pub fn into_parts(self) -> (Restore, Option<PortableSettings>, SyncApplySummary) {
        (self.restore, self.settings, self.summary)
    }
}

/// How many arrivals found their id already here, over every kind of thing that has one.
fn collided(sets: &[&Ids]) -> u32 {
    sets.iter().map(|set| set.collided).sum()
}

/// Works out what a snapshot does to this machine, without touching it.
pub fn plan_restore(snapshot: SyncSnapshot, destination: &Destination, replace: bool) -> RestorePlan {
    let SyncSnapshot {
        sessions,
        groups,
        snippets,
        words,
        history,
        known_hosts,
        settings,
        secrets,
        keys,
        ..
    } = snapshot;

    let mut group_ids = Ids::new(destination.group_ids.clone(), replace);
    let mut planned_group_ids: HashMap<String, String> = HashMap::new();
    let planned_groups: Vec<SessionGroup> = groups
        .into_iter()
        .map(|group| {
            let id = group_ids.land(&group.id);
            // The first entry that claimed a name answers for it: a file somebody edited may name
            // the same group twice, and a reader of that file would expect the reference to mean
            // the first one. Both are still made.
            planned_group_ids.entry(group.id.clone()).or_insert_with(|| id.clone());

            SessionGroup {
                id,
                name: group.name,
                // Only a parent from this file: an id that merely matches something already here
                // is a coincidence, and what arrived must not nest itself inside it.
                parent_id: group
                    .parent_id
                    .as_ref()
                    .and_then(|parent| planned_group_ids.get(parent))
                    .cloned(),
                order: group.order,
                pinned: group.pinned,
            }
        })
        .collect();

    // Keys first, because the sessions below point at them and the session's reference is what
    // decides whether the handle it carries here means anything.
    let mut key_ids = Ids::new(destination.key_refs.clone(), replace);
    let mut key_landings: HashMap<String, String> = HashMap::new();
    let mut planned_keys: Vec<KeyEntry> = Vec::new();
    for key in keys.into_iter().flatten() {
        // An entry with no material is not a key: there is nothing to store, and nothing a session
        // could meaningfully point at. It is dropped rather than counted, because a hand-edited
        // file is not worth a sentence in a receipt.
        if key.contents.trim().is_empty() {
            continue;
        }

        // A key this machine already holds, recognised by its fingerprint: the arrival points at
        // the handle that is here instead of importing the same key a second time under a second
        // name. This is a recognition, not a replacement — nothing about the key that is here is
        // written over.
        if let Some(existing) = destination.key_ref_by_fingerprint.get(&key.info.fingerprint) {
            key_landings
                .entry(key.key_ref.clone())
                .or_insert_with(|| existing.clone());
            continue;
        }

        let landing = key_ids.land(&key.key_ref);
        key_landings
            .entry(key.key_ref.clone())
            .or_insert_with(|| landing.clone());

        let mut entry = key;
        entry.key_ref = landing;
        planned_keys.push(entry);
    }

    let mut session_ids = Ids::new(destination.session_ids.clone(), replace);
    let mut session_landings: HashMap<String, String> = HashMap::new();
    let planned_session_ids: Vec<String> = sessions
        .iter()
        .map(|session| {
            let landing = session_ids.land(&session.id);
            session_landings
                .entry(session.id.clone())
                .or_insert_with(|| landing.clone());
            landing
        })
        .collect();

    // The logins, by the session they belong to. Taken by the id the file used, because that is the
    // id the session below is still carrying at this point.
    let mut logins: HashMap<String, SecretEntry> = secrets
        .into_iter()
        .flatten()
        .map(|entry| (entry.session_id.clone(), entry))
        .collect();

    let mut dropped_hops = 0;
    let mut missing_keys = 0;
    let mut written_secrets = 0;
    let planned_sessions: Vec<RestoredSession> = sessions
        .into_iter()
        .zip(planned_session_ids)
        .map(|(session, id)| {
            let mut imported = session.clone();
            imported.id = id;
            imported.group_id = session
                .group_id
                .as_ref()
                .and_then(|group| planned_group_ids.get(group))
                .cloned();

            imported.jump = session
                .jump
                .iter()
                .filter_map(|hop| match session_landings.get(hop) {
                    Some(landing) => Some(landing.clone()),
                    None => {
                        dropped_hops += 1;
                        None
                    }
                })
                .collect();

            // The key this session names, moved to the handle it landed on here. A handle this
            // restore did not bring is left exactly as the file had it and counted: it is still
            // imported — a session somebody can open and fix — and the count is the only sign that
            // it will not connect the way it did where the file came from.
            if let Some(reference) = imported.auth.key.as_mut().and_then(|key| key.key_ref.as_mut()) {
                match key_landings.get(reference.as_str()) {
                    Some(landing) => *reference = landing.clone(),
                    None => missing_keys += 1,
                }
            }

            // The flag is about *this* vault's secrets table, not about the file, and the write
            // below is what sets it: a login that did not come with the file leaves this false.
            imported.auth.has_secret = false;

            let secret = logins.remove(&session.id).map(|entry| match entry.kind {
                sevenshell_vault::SecretKind::Password => SessionSecret::Password(entry.into_value()),
                sevenshell_vault::SecretKind::KeyPassphrase => SessionSecret::KeyPassphrase(entry.into_value()),
            });
            if secret.is_some() {
                written_secrets += 1;
            }

            RestoredSession {
                session: imported,
                secret,
            }
        })
        .collect();
    // A login whose session is not in the file belongs to nothing. It is dropped rather than
    // written against an id that is not there — there is no session it could belong to.

    let mut snippet_group_ids = Ids::new(destination.snippet_group_ids.clone(), replace);
    let mut snippet_ids = Ids::new(destination.snippet_ids.clone(), replace);
    let mut written_snippets = 0;
    let planned_snippet_groups: Vec<SnippetGroup> = snippets
        .into_iter()
        .map(|group| {
            // The default group is the one group every vault has, whatever a file calls it. It is
            // the same group rather than a collision, so it lands on its own id every time — and a
            // fresh id here would make a nameless second group instead.
            let id = if group.id == DEFAULT_GROUP_ID {
                DEFAULT_GROUP_ID.to_owned()
            } else {
                snippet_group_ids.land(&group.id)
            };

            let commands = group
                .commands
                .into_iter()
                .map(|mut command| {
                    command.id = snippet_ids.land(&command.id);
                    command.group_id = id.clone();
                    written_snippets += 1;
                    command
                })
                .collect();

            SnippetGroup {
                id,
                name: group.name,
                order: group.order,
                commands,
            }
        })
        .collect();

    let mut word_ids = Ids::new(destination.word_ids.clone(), replace);
    let mut planned_words = Vec::new();
    for word in words {
        // The pair of command and word is what a row *is*, so a pair that is already here is the
        // same row: without replacement the row that is here is left alone, and with it the file's
        // version is written over it.
        if !replace
            && destination
                .word_pairs
                .contains(&(word.command.clone(), word.word.clone()))
        {
            continue;
        }

        let mut planned = word;
        planned.id = word_ids.land(&planned.id);
        planned_words.push(planned);
    }

    let written_settings = settings.as_ref().map_or(0, PortableSettings::count);
    let words_written = planned_words.len() as u32;
    let history_written = history.as_ref().map_or(0, Vec::len) as u32;
    let hosts_written = known_hosts.as_ref().map_or(0, Vec::len) as u32;

    let summary = SyncApplySummary {
        contents: SyncContents {
            sessions: planned_sessions.len() as u32,
            groups: planned_groups.len() as u32,
            snippets: written_snippets,
            words: words_written,
            history: history_written,
            known_hosts: hosts_written,
            settings: written_settings,
            secrets: written_secrets,
            keys: planned_keys.len() as u32,
        },
        // What the collisions meant, said in the two ways they can mean something. A word whose
        // pair was already here is not in either count when nothing is being replaced: it was left
        // alone rather than copied, so counting it as a copy would be a lie.
        replaced: if replace {
            collided(&[&session_ids, &group_ids, &snippet_ids, &word_ids])
        } else {
            0
        },
        copied: if replace {
            0
        } else {
            collided(&[&session_ids, &group_ids, &snippet_ids, &word_ids])
        },
        missing_keys,
        dropped_hops,
    };

    RestorePlan {
        restore: Restore {
            groups: planned_groups,
            sessions: planned_sessions,
            snippet_groups: planned_snippet_groups,
            words: planned_words,
            history: history.unwrap_or_default(),
            known_hosts: known_hosts.unwrap_or_default(),
            keys: planned_keys,
        },
        settings,
        summary,
    }
}
