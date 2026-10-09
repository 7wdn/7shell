//! A batch of writes that arrived in a file, applied in one go.
//!
//! The vault writes its whole document on every mutation, which is what makes one change atomic.
//! Applying a snapshot one item at a time would therefore be hundreds of writes of a file that
//! grows as it goes, and a failure half way through would leave somebody with half a restore and no
//! way to tell which half. So a restore is worked out somewhere else, handed here as one value, and
//! written once — the same shape the session import already uses.
//!
//! What this does *not* decide: whether an id may be taken, where a session sits in the tree, what
//! to do about a key this machine already holds, and whether an item that is already here is
//! replaced or copied. Those are decisions about the file that arrived, and whoever read it makes
//! them. This writes what it is given.

use sevenshell_types::{CommandHistoryEntry, CompletionWord, KnownHostEntry, Session, SessionGroup, SnippetGroup};

use crate::credential::KeyEntry;
use crate::document::{Document, SessionSecret};

/// One session to write, with the login that came with it.
pub struct RestoredSession {
    pub session: Session,
    /// `None` leaves whatever this machine already holds for that id alone — the vault's "absent
    /// means unchanged" rule, which is the direction that cannot lose a password somebody has.
    pub secret: Option<SessionSecret>,
}

/// Everything a restore writes.
#[derive(Default)]
pub struct Restore {
    pub groups: Vec<SessionGroup>,
    pub sessions: Vec<RestoredSession>,
    pub snippet_groups: Vec<SnippetGroup>,
    pub words: Vec<CompletionWord>,
    pub history: Vec<CommandHistoryEntry>,
    pub known_hosts: Vec<KnownHostEntry>,
    pub keys: Vec<KeyEntry>,
}

/// What a restore wrote, counted as it was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RestoreCounts {
    pub groups: u32,
    pub sessions: u32,
    pub secrets: u32,
    pub snippets: u32,
    pub words: u32,
    pub history: u32,
    pub known_hosts: u32,
    pub keys: u32,
}

impl Document {
    /// Writes everything one restore carries, and counts it on the way past.
    ///
    /// The order is the one that keeps references valid: a group before the sessions that name it,
    /// a command group before the commands that live in it. Nothing here re-checks any of that —
    /// whoever planned the restore has already closed it under itself — so this is deliberately
    /// mechanical.
    pub(crate) fn apply_restore(&mut self, restore: Restore) -> RestoreCounts {
        let mut counts = RestoreCounts::default();

        for group in restore.groups {
            self.save_group(group);
            counts.groups += 1;
        }
        for entry in restore.sessions {
            if entry.secret.is_some() {
                counts.secrets += 1;
            }
            self.save_session(entry.session, entry.secret);
            counts.sessions += 1;
        }
        for group in restore.snippet_groups {
            // The group itself first, then its commands: a command whose group is not there yet
            // would land in the default one and stay there.
            counts.snippets += group.commands.len() as u32;
            let commands = group.commands.clone();
            let mut empty = group;
            empty.commands.clear();
            self.save_snippet_group(empty);
            for command in commands {
                self.save_snippet(command);
            }
        }
        for word in restore.words {
            self.save_completion_word(word);
            counts.words += 1;
        }
        counts.history = self.merge_history(restore.history);
        for entry in restore.known_hosts {
            self.trust_host(entry);
            counts.known_hosts += 1;
        }
        for key in restore.keys {
            let key_ref = key.key_ref.clone();
            let info = key.info.clone();
            let (contents, passphrase) = key.into_parts();

            // A key the vault will not take is not counted. The plan has already dropped the ones
            // that could never be stored, so this is a guard rather than a path — and counting a
            // key that is not there would make the receipt a lie.
            if self.import_key_as(key_ref, contents, passphrase, info).is_ok() {
                counts.keys += 1;
            }
        }

        counts
    }

    /// Adds the entries of a history this machine does not already have, and answers how many.
    ///
    /// Merged rather than replaced: a snapshot is a copy taken at some moment, and the commands
    /// typed here since are not in it. An entry is the same entry when it says the same thing at
    /// the same second, which is what makes restoring the same file twice do nothing the second
    /// time.
    pub(crate) fn merge_history(&mut self, entries: Vec<CommandHistoryEntry>) -> u32 {
        let mut added = 0;
        for entry in entries {
            if self
                .command_history
                .iter()
                .any(|existing| existing.command == entry.command && existing.at == entry.at)
            {
                continue;
            }
            self.command_history.push(entry);
            added += 1;
        }

        // Newest first, which is the order this list is in everywhere else, and bounded by the same
        // limit the history has always had.
        self.command_history.sort_by_key(|entry| std::cmp::Reverse(entry.at));
        self.command_history.truncate(crate::document::COMMAND_HISTORY_LIMIT);

        added
    }
}
