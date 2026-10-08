//! The words somebody adds to what follows a command.
//!
//! The table of what each command is usually followed by ships with the build and is the same for
//! everybody. This is the other half of it: the tools a machine has that the table does not know —
//! an in-house CLI, a package manager, the verbs somebody uses in their own cluster. A word added
//! here is offered before a shipped word of the same standing, which is the rule the command library
//! has too: what somebody made comes first.

use serde::{Deserialize, Serialize};

/// One word, and the command it follows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionWord {
    pub id: String,
    /// The command the word follows, as it is typed: `kubectl`.
    pub command: String,
    /// The word itself, offered after that command and a space.
    pub word: String,
    /// What it means, in the words of whoever added it.
    ///
    /// Their own text rather than a translation, because nobody else can know what
    /// `acme-deploy drain` does to their cluster. Empty means the list shows what kind of word it is
    /// — the same fallback a shipped word without a line has.
    #[serde(default)]
    pub note: String,
}
