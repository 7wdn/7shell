//! Commands kept for later: the library, the slots in a command, and what was sent.
//!
//! A saved command is a line somebody runs often, with up to [`SNIPPET_SLOTS`] numbered places to
//! put values into — `service #1 restart`, `tail -f #1`. The values used before are remembered per
//! slot, because the reason to save a command is that it is the same command with the same few
//! things changed, and retyping `nginx` every time is the part that was not worth saving.
//!
//! Substitution is the sender's business, not the storage's: what is stored is the line with its
//! slots, and what is sent is the line with the values already in it. That keeps one rule — what
//! the editor shows is what the far side receives — instead of a template language nobody can see.

use serde::{Deserialize, Serialize};

/// How many `#n` slots a saved command may use: `#1` through `#5`.
pub const SNIPPET_SLOTS: usize = 5;

/// Whether a command is offered by the completion, for a library written before it could be turned
/// off: absent means on, because a command somebody saved is one they meant to use.
fn enabled_by_default() -> bool {
    true
}

/// The group a command lands in when nobody has made one yet.
///
/// It exists from the start — a library with no group is a library with nowhere to put the first
/// command — and it is identified by this id rather than by its name. Its name is empty in storage
/// on purpose: it is the one group whose label is a *translation*, and a stored name would be a
/// name written in the language that happened to be active when the vault was created.
pub const DEFAULT_GROUP_ID: &str = "default";

/// One group of saved commands, in the order the interface lists them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetGroup {
    pub id: String,
    pub name: String,
    /// Position in the list. Stored rather than implied: the order somebody drags commands into is
    /// theirs, and an id order would quietly disagree with it after every edit.
    pub order: u32,
    #[serde(default)]
    pub commands: Vec<Snippet>,
}

/// One saved command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    pub id: String,
    pub group_id: String,
    /// What the list shows. It may differ from the command itself: `restart nginx` is a better
    /// name than `systemctl restart nginx`, and the name is what somebody scans for.
    pub name: String,
    /// The short text that brings the line up.
    ///
    /// Typing it in the command line offers this command, which is what makes three letters worth
    /// more than a whole name: the trigger is what somebody reaches for, and it is shorter than
    /// either the name or the line. Empty means none — the name and the line itself are what the
    /// completion searches then, which is how a command saved before this existed still behaves.
    #[serde(default)]
    pub prefix: String,
    /// The line, with `#1`…`#5` where values go.
    pub command: String,
    /// Whether sending it presses Enter afterwards. Off means the line is placed in the shell
    /// without running it — which is how a saved `cd /srv/app` gets to be amended before it runs.
    pub append_cr: bool,
    /// Whether the completion offers it.
    ///
    /// Off keeps a line somebody is not using right now out of the list without deleting it, which is
    /// what a library of shipped and half-forgotten commands needs. A command saved before this
    /// existed is on: that is what the default below means.
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    pub order: u32,
    /// Values used in each slot before, newest first: index 0 belongs to `#1`.
    ///
    /// Stored inside the command rather than in a history of its own because a slot means nothing
    /// outside the line it sits in — `#1` of a restart command and `#1` of a log command have
    /// nothing to do with each other.
    #[serde(default)]
    pub slot_history: Vec<Vec<String>>,
}

impl Snippet {
    /// The values remembered for one slot, or an empty list when it has none yet.
    pub fn slot_values(&self, slot: usize) -> &[String] {
        self.slot_history.get(slot).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Keeps a value at the front of its slot's history, and the history bounded.
    ///
    /// A move rather than a copy: a value already remembered moves to the front, because the list
    /// is about what was used last and repeating it does not make it more likely than the value
    /// someone just typed.
    pub fn remember(&mut self, slot: usize, value: &str) {
        let value = value.trim();
        if value.is_empty() || slot >= SNIPPET_SLOTS {
            return;
        }

        while self.slot_history.len() <= slot {
            self.slot_history.push(Vec::new());
        }

        let values = &mut self.slot_history[slot];
        values.retain(|existing| existing != value);
        values.insert(0, value.to_owned());
        values.truncate(SLOT_HISTORY);
    }
}

/// How many values are remembered per slot. Enough to offer the few things a host is called, and
/// not so many that the suggestion list becomes a search.
pub const SLOT_HISTORY: usize = 12;

/// One command that was sent, and when.
///
/// Deliberately not the machine it went to: the same line is usually sent to whichever session is
/// on screen, the interesting thing about history is the text, and a per-session history would grow
/// with the sessions rather than with the work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandHistoryEntry {
    pub command: String,
    /// Seconds since the epoch, so the list can be ordered after a restart and can be aged out.
    pub at: u64,
}

/// Where a command is sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SendTarget {
    /// The terminals open right now, by handle: the sessions somebody has on screen.
    ///
    /// Handles rather than session ids because this is a terminal send — the command is typed into
    /// a shell that is already running, which is why it needs no connection of its own and why the
    /// answer is "written", not an exit code.
    Terminals { handles: Vec<String> },
}

/// What sending did, per terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendReport {
    /// The handles the command was written to.
    pub sent: Vec<String>,
    /// The handles it could not be written to, each with why.
    pub failures: Vec<SendFailure>,
}

/// One terminal a command could not be sent to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendFailure {
    pub handle: String,
    /// The stable code from the error catalog, e.g. `SESSION-NOT-FOUND`.
    pub code: String,
    pub message_key: String,
    #[serde(default)]
    pub message_params: std::collections::BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// What sending answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SendOutcome {
    /// The command is dangerous enough that sending it somewhere at once waits for a person.
    ///
    /// Only for more than one terminal: a line typed into one shell is the person deciding, which
    /// is the same reason the terminal itself is not gated.
    ApprovalRequired { risk: crate::RiskAssessment },
    /// It was written to the terminals that could take it.
    Sent { report: SendReport },
}
