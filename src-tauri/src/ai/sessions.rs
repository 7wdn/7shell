//! A conversation the assistant keeps between runs.
//!
//! The vault stores one opaque string per conversation, keyed by its id (see
//! `sevenshell_vault::Vault::ai_sessions`). The shape inside is this module's: a [`StoredSession`]
//! serialised as JSON, which the vault neither parses nor validates. Two consequences follow from
//! that split, and both are deliberate:
//!
//! * **A document this build cannot read is skipped, never rewritten.** A conversation is the
//!   person's own typing; discarding one because a newer build wrote a block shape this build does
//!   not know would be data loss disguised as robustness. A corrupt string is logged and passed
//!   over, and the rest of the list still draws.
//! * **The document is bounded here, not in the vault.** Fifty conversations, 200 messages and
//!   512 KiB each, 4 MiB in total. Over the per-conversation bound the oldest *exchanges* go — a
//!   tool result whose call went with it is a shape no provider accepts — and a single exchange too
//!   large to store is `AI-SESSION-TOO-LARGE` rather than a silently dropped answer.
//!
//! Timestamps are seconds since the epoch, like every other timestamp that crosses the wire: the
//! list sorts by `updated_at`, and retention is a subtraction from the clock, not a date library.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use sevenshell_ai::ChatMessage;
use sevenshell_error::{ErrorCode, ShellError};
use sevenshell_types::{AiContextSummary, AiSessionDetail, AiSessionSummary, AiToolOutcome, AppSettings};
use sevenshell_vault::Vault;

use crate::ai::catalog_error;
use crate::ai::context;

/// The document version written into every session. A later build that changes the shape checks it
/// before reading; this build only writes [`SESSION_VERSION`] and accepts a document of any version
/// whose fields still deserialize.
pub const SESSION_VERSION: u32 = 1;

/// The most conversations this installation keeps. Over it the oldest is evicted, because a list
/// nobody can read to the end is worse than a short one.
pub const MAX_SESSIONS: usize = 50;

/// The most messages one conversation keeps.
pub const MAX_MESSAGES: usize = 200;

/// The most one stored document may weigh, in bytes.
pub const MAX_SESSION_BYTES: usize = 512 * 1024;

/// The most all conversations together may weigh, in bytes.
pub const MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;

/// How much of the first question becomes the name.
pub const NAME_MAX_CHARS: usize = 60;

fn session_version_default() -> u32 {
    SESSION_VERSION
}

/// One conversation, as the vault holds it.
///
/// Field order is the contract's; `version` leads so a person looking at a decrypted document sees
/// what it is before what it says.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredSession {
    #[serde(default = "session_version_default")]
    pub version: u32,
    pub id: String,
    /// The name the list draws: the first question, scrubbed and shortened, or a person's rename.
    #[serde(default)]
    pub name: String,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    /// One row per tool call, keyed by the call id its tool-use block names.
    #[serde(default)]
    pub outcomes: BTreeMap<String, AiToolOutcome>,
    /// What replaced the older half of the transcript, when something did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_summary: Option<AiContextSummary>,
}

impl StoredSession {
    /// A conversation with nothing in it. Not written to the vault yet: an empty conversation in
    /// the list would be a row nobody can open to anything.
    pub fn new(now: u64) -> Self {
        Self {
            version: SESSION_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            name: String::new(),
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
            outcomes: BTreeMap::new(),
            context_summary: None,
        }
    }

    /// A conversation with a known id, for the one case that needs one: a send that named a
    /// conversation the vault no longer holds.
    pub fn with_id(id: &str, now: u64) -> Self {
        Self {
            id: id.to_owned(),
            ..Self::new(now)
        }
    }

    pub fn summary(&self) -> AiSessionSummary {
        AiSessionSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            message_count: self.messages.len() as u32,
            has_summary: self.context_summary.is_some(),
        }
    }

    /// The conversation as the transcript draws it.
    ///
    /// Messages travel as JSON values rather than as typed rows, because that is the shape the wire
    /// type was built for: a block this build does not know passes through instead of failing the
    /// whole read. Serialising our own messages cannot fail, and the `Null` fallback is there so a
    /// hypothetical failure shortens one row rather than shifting every row after it.
    pub fn detail(&self) -> AiSessionDetail {
        AiSessionDetail {
            summary: self.summary(),
            messages: self
                .messages
                .iter()
                .map(|message| serde_json::to_value(message).unwrap_or(serde_json::Value::Null))
                .collect(),
            outcomes: self.outcomes.clone(),
            context_summary: self.context_summary.clone(),
        }
    }

    /// The document exactly as it goes into the vault.
    pub fn document(&self) -> Result<String, ShellError> {
        serde_json::to_string(self)
            .map_err(|error| ShellError::app_internal(format!("a conversation could not be written: {error}")))
    }

    /// What this document weighs, as the vault will store it.
    pub fn size_bytes(&self) -> usize {
        // A serialisation of this type has no failure path, and the limit is a guard against a long
        // conversation rather than against malformed data.
        self.document().map(|document| document.len()).unwrap_or(usize::MAX)
    }

    /// Drops the oldest exchanges until the document fits its bound.
    ///
    /// Messages are dropped in exchange-sized bites and never down to nothing: the newest question
    /// is what the next run is about, so a document that kept the *old* half of a conversation and
    /// lost the new one would be keeping the wrong half.
    pub fn trim_to_limits(&mut self) -> Result<(), ShellError> {
        while self.messages.len() > MAX_MESSAGES || self.size_bytes() > MAX_SESSION_BYTES {
            if self.messages.len() <= 1 {
                break;
            }
            let cut = oldest_exchange_end(&self.messages);
            self.messages.drain(..cut.max(1));
            self.prune_outcomes();
        }

        if self.size_bytes() > MAX_SESSION_BYTES {
            return Err(catalog_error(ErrorCode::AiSessionTooLarge));
        }
        Ok(())
    }

    /// Forgets the outcomes of tool calls no remaining message names.
    fn prune_outcomes(&mut self) {
        if self.outcomes.is_empty() {
            return;
        }
        let mut referenced = std::collections::BTreeSet::new();
        for message in &self.messages {
            for block in &message.blocks {
                if let sevenshell_ai::ContentBlock::ToolUse { id, .. } = block {
                    referenced.insert(id.as_str());
                }
            }
        }
        self.outcomes.retain(|call_id, _| referenced.contains(call_id.as_str()));
    }
}

/// One line out of a question, scrubbed, as the name of a conversation.
///
/// Escape sequences and control characters go for the reason they go everywhere in this tree: the
/// name is drawn in a list, and a terminal title in it would be unreadable. Newlines and tabs stay
/// long enough to be folded into single spaces — a question written over two lines should not become
/// one word. Credentials are scrubbed because a question may quote a command line — and a
/// conversation name is written to the vault and drawn on screen for as long as the conversation
/// exists.
pub fn name_from_question(text: &str) -> String {
    let plain = context::strip_control(text, true);
    let scrubbed = sevenshell_error::scrub(&plain);
    let line = scrubbed.split_whitespace().collect::<Vec<_>>().join(" ");
    line.chars().take(NAME_MAX_CHARS).collect()
}

/// A name a person typed, cut to the same length and stripped the same way.
pub fn clean_name(text: &str) -> String {
    let plain = context::strip_control(text, false);
    plain
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(NAME_MAX_CHARS)
        .collect()
}

/// The index one past the oldest exchange, which is where a trim starts.
///
/// An exchange begins at a user message carrying something other than a tool result; the second
/// such message is therefore the end of the first. A conversation with no second question is one
/// exchange, and `1` is the only cut that makes progress without emptying it.
fn oldest_exchange_end(messages: &[ChatMessage]) -> usize {
    let mut seen = 0;
    for (index, message) in messages.iter().enumerate() {
        if context::starts_exchange(message) {
            seen += 1;
            if seen == 2 {
                return index;
            }
        }
    }
    messages.len().saturating_sub(1).max(1)
}

/// Seconds since the epoch, the unit every timestamp on this side uses.
pub fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// The stored conversations, newest first, with this installation's retention applied.
///
/// The same read answers both questions — what is on screen and what has aged out — because a prune
/// against a table read a moment earlier could delete a conversation that a second copy still said
/// was current.
pub fn list(vault: &Vault, settings: &AppSettings, now: u64) -> Result<Vec<AiSessionSummary>, ShellError> {
    let stored = vault.ai_sessions()?;
    let retention_days = settings.ai_session_retention_days.clamp(1, 365) as u64;
    let oldest = now.saturating_sub(retention_days * 86_400);

    let mut rows = Vec::with_capacity(stored.len());
    for (id, document) in stored {
        let Some(session) = parse(&id, &document) else {
            continue;
        };
        if session.updated_at < oldest {
            // Retention is a setting, and the list is where it is applied: a conversation somebody
            // has not opened in a month is older than the assistant is useful for, and it is gone
            // from the vault rather than merely hidden from the list.
            if let Err(error) = vault.delete_ai_session(&id) {
                log::warn!("an assistant conversation older than the retention stayed in the vault: {error}");
            }
            continue;
        }
        rows.push(session.summary());
    }

    rows.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| b.id.cmp(&a.id)));
    Ok(rows)
}

/// One conversation, exactly as it was stored.
///
/// A document this build cannot read answers `None` — the same answer as a conversation that is not
/// there — because the caller's next move is the same either way: open an empty one and carry on.
pub fn load(vault: &Vault, id: &str) -> Result<Option<StoredSession>, ShellError> {
    let Some(document) = vault.ai_session(id)? else {
        return Ok(None);
    };
    Ok(parse(id, &document))
}

/// Writes one conversation, evicting older ones if the table has no room.
///
/// The caller trims the document first ([`StoredSession::trim_to_limits`]); this function is the
/// answer to "may it be stored at all", and its refusal is the catalog's `AI-SESSION-TOO-LARGE`.
pub fn save(vault: &Vault, session: &StoredSession) -> Result<(), ShellError> {
    let document = session.document()?;
    if document.len() > MAX_SESSION_BYTES {
        return Err(catalog_error(ErrorCode::AiSessionTooLarge));
    }
    make_room(vault, &session.id, document.len())?;
    vault.save_ai_session(&session.id, Some(document))
}

/// Renames a stored conversation, answering it as the list will now draw it.
///
/// `None` means there was nothing to rename. A name that is empty after cleaning is *not* a rename:
/// the person asked for a blank label, and the row keeps the name it had rather than losing it.
pub fn rename(vault: &Vault, id: &str, name: &str) -> Result<Option<StoredSession>, ShellError> {
    let name = clean_name(name);
    if name.is_empty() {
        return load(vault, id);
    }
    if !vault.rename_ai_session(id, &name)? {
        return Ok(None);
    }
    load(vault, id)
}

/// Forgets one conversation. Deleting one already gone is not a failure: the state asked for holds.
pub fn delete(vault: &Vault, id: &str) -> Result<(), ShellError> {
    vault.delete_ai_session(id)
}

/// Empties a stored conversation in place, keeping its id, name and creation time.
///
/// The person asked for this conversation to start over; the row keeps its place in the list under
/// the name they know it by, which is why the name is not cleared with the messages.
pub fn clear(vault: &Vault, session: &mut StoredSession, now: u64) -> Result<(), ShellError> {
    session.messages.clear();
    session.outcomes.clear();
    session.context_summary = None;
    session.updated_at = now;
    save(vault, session)
}

/// Parses one stored document, saying why when it cannot.
fn parse(id: &str, document: &str) -> Option<StoredSession> {
    match serde_json::from_str::<StoredSession>(document) {
        Ok(mut session) => {
            // The key a table row was stored under is the id, whatever the document says: a
            // document that drifted from its key would otherwise answer to a name that cannot
            // delete or rename it.
            session.id = id.to_owned();
            Some(session)
        }
        Err(error) => {
            log::warn!("an assistant conversation could not be read and was left alone: {error}");
            None
        }
    }
}

/// Evicts the oldest conversations until this one fits within the count and size bounds.
fn make_room(vault: &Vault, id: &str, size: usize) -> Result<(), ShellError> {
    let stored = vault.ai_sessions()?;
    let mut others: Vec<(String, u64, usize)> = stored
        .iter()
        .filter(|(other, _)| other.as_str() != id)
        .map(|(other, document)| {
            let updated_at = serde_json::from_str::<StoredSession>(document)
                .map(|session| session.updated_at)
                // A document nobody can read has no timestamp and is evicted first: it cannot be
                // drawn, so it cannot be missed.
                .unwrap_or(0);
            (other.clone(), updated_at, document.len())
        })
        .collect();

    if size > MAX_TOTAL_BYTES {
        return Err(catalog_error(ErrorCode::AiSessionTooLarge));
    }

    let mut count = others.len() + 1;
    let mut total = others.iter().map(|(_, _, length)| length).sum::<usize>() + size;
    if count <= MAX_SESSIONS && total <= MAX_TOTAL_BYTES {
        return Ok(());
    }

    others.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    for (other, _, length) in others {
        if count <= MAX_SESSIONS && total <= MAX_TOTAL_BYTES {
            break;
        }
        vault.delete_ai_session(&other)?;
        count -= 1;
        total -= length;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sevenshell_ai::ContentBlock;

    struct Harness {
        _directory: tempfile::TempDir,
        vault: Vault,
    }

    fn harness() -> Harness {
        let directory = tempfile::tempdir().expect("a temp directory");
        let vault = Vault::open_at(directory.path().join("vault.json"));
        vault.create("master").expect("a vault");
        Harness {
            _directory: directory,
            vault,
        }
    }

    fn settings(days: u32) -> AppSettings {
        AppSettings {
            ai_session_retention_days: days,
            ..AppSettings::default()
        }
    }

    fn session_with(id: &str, updated_at: u64, messages: &[&str]) -> StoredSession {
        StoredSession {
            version: SESSION_VERSION,
            id: id.to_owned(),
            name: format!("question for {id}"),
            created_at: updated_at.saturating_sub(60),
            updated_at,
            messages: messages.iter().map(|text| ChatMessage::user(*text)).collect(),
            outcomes: BTreeMap::new(),
            context_summary: None,
        }
    }

    #[test]
    fn a_document_round_trips_through_the_vault() {
        let harness = harness();
        let mut session = session_with("session-1", 1_700_000_000, &["why is nginx 502"]);
        session
            .messages
            .push(ChatMessage::assistant(vec![ContentBlock::ToolUse {
                id: "call-1".to_owned(),
                name: "sshell_sessions_list".to_owned(),
                input: serde_json::json!({}),
            }]));
        session.outcomes.insert(
            "call-1".to_owned(),
            AiToolOutcome {
                call_id: "call-1".to_owned(),
                ok: false,
                code: Some("POLICY-DENIED".to_owned()),
                truncated: true,
                duration_ms: 12,
            },
        );

        save(&harness.vault, &session).expect("stored");
        let back = load(&harness.vault, "session-1").expect("read").expect("there");

        assert_eq!(back, session, "the document the host wrote is the document it reads");
        assert_eq!(back.detail().outcomes["call-1"].call_id, "call-1");
    }

    #[test]
    fn the_name_is_the_first_question_without_escape_sequences_or_credentials() {
        let name = name_from_question("\u{1b}[31mwhy is\u{1b}[0m  nginx\n502?  ");
        assert_eq!(
            name, "why is nginx 502?",
            "a question written over two lines is one name, not one word"
        );

        let quoted = name_from_question("password=hunter2 why did the deploy fail?");
        assert!(
            !quoted.contains("hunter2"),
            "a credential quoted in a question must not become the row's label: {quoted}"
        );
        assert!(quoted.contains("why did the deploy fail?"));

        let long = name_from_question(&"x".repeat(200));
        assert_eq!(long.chars().count(), NAME_MAX_CHARS);
    }

    #[test]
    fn an_empty_rename_keeps_the_name_the_conversation_had() {
        let harness = harness();
        let session = session_with("session-1", 10, &["hello"]);
        save(&harness.vault, &session).expect("stored");

        let renamed = rename(&harness.vault, "session-1", "   ")
            .expect("asked")
            .expect("there");

        assert_eq!(renamed.name, session.name);
    }

    #[test]
    fn a_corrupt_document_is_skipped_and_the_rest_of_the_list_survives() {
        let harness = harness();
        save(&harness.vault, &session_with("good", 2_000, &["hello"])).expect("stored");
        harness
            .vault
            .save_ai_session("broken", Some("[1,2,3]".to_owned()))
            .expect("stored");

        let rows = list(&harness.vault, &settings(30), 2_100).expect("listed");

        assert_eq!(rows.len(), 1, "a document this build cannot read is passed over");
        assert_eq!(rows[0].id, "good");
        assert_eq!(
            harness.vault.ai_session("broken").expect("read").as_deref(),
            Some("[1,2,3]"),
            "and it is left exactly as it was rather than rewritten from a guess"
        );
    }

    #[test]
    fn a_locked_vault_refuses_the_list_rather_than_drawing_an_empty_one() {
        let harness = harness();
        save(&harness.vault, &session_with("session-1", 1_000, &["hello"])).expect("stored");
        harness.vault.lock();

        let error = list(&harness.vault, &settings(30), 2_000).expect_err("the vault cannot say");

        assert_eq!(
            error.code(),
            ErrorCode::VaultLocked,
            "an empty list would tell somebody their conversations were lost"
        );
    }

    #[test]
    fn a_conversation_older_than_the_retention_is_deleted_by_the_list() {
        let harness = harness();
        let now = 4_000_000_000_u64;
        save(&harness.vault, &session_with("old", now - 40 * 86_400, &["hello"])).expect("stored");
        save(&harness.vault, &session_with("new", now - 2 * 86_400, &["hello"])).expect("stored");

        let rows = list(&harness.vault, &settings(30), now).expect("listed");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "new");
        assert!(
            harness.vault.ai_session("old").expect("read").is_none(),
            "retention removes the document, not just the row"
        );
    }

    #[test]
    fn the_list_answers_newest_first() {
        let harness = harness();
        save(&harness.vault, &session_with("older", 1_000, &["a"])).expect("stored");
        save(&harness.vault, &session_with("newer", 3_000, &["b"])).expect("stored");

        let rows = list(&harness.vault, &settings(30), 3_100).expect("listed");

        assert_eq!(
            rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            vec!["newer", "older"]
        );
    }

    #[test]
    fn a_conversation_over_the_message_limit_loses_its_oldest_exchanges() {
        let mut session = session_with("long", 10, &[]);
        for index in 0..(MAX_MESSAGES + 20) {
            session.messages.push(ChatMessage::user(format!("question {index}")));
            session.messages.push(ChatMessage::assistant(vec![ContentBlock::Text {
                text: format!("answer {index}"),
            }]));
        }
        let newest = session.messages.last().cloned();

        session.trim_to_limits().expect("it fits after trimming");

        assert!(session.messages.len() <= MAX_MESSAGES);
        assert_eq!(
            session.messages.last().cloned(),
            newest,
            "the newest answer is the part kept"
        );
        assert!(
            session.messages.first().is_some_and(context::starts_exchange),
            "what is left begins with a question, not half an exchange"
        );
    }

    #[test]
    fn one_message_too_large_to_store_is_refused_rather_than_silently_dropped() {
        let mut session = session_with("huge", 10, &[]);
        session
            .messages
            .push(ChatMessage::user("x".repeat(MAX_SESSION_BYTES + 1)));

        let error = session.trim_to_limits().expect_err("one message cannot be cut in half");

        assert_eq!(error.code(), ErrorCode::AiSessionTooLarge);
        assert_eq!(error.message_key(), "errors.ai.sessionTooLarge");
    }

    #[test]
    fn storing_one_more_conversation_evicts_the_oldest_when_the_table_is_full() {
        let harness = harness();
        for index in 0..MAX_SESSIONS {
            let session = session_with(&format!("session-{index}"), 1_000 + index as u64, &["hello"]);
            save(&harness.vault, &session).expect("stored");
        }

        save(&harness.vault, &session_with("newest", 9_000, &["hello"])).expect("stored");

        let stored = harness.vault.ai_sessions().expect("read");
        assert_eq!(stored.len(), MAX_SESSIONS, "the table stays at its bound");
        assert!(stored.contains_key("newest"));
        assert!(
            !stored.contains_key("session-0"),
            "the oldest conversation is the one that goes"
        );
    }

    #[test]
    fn an_outcome_whose_call_was_trimmed_away_goes_with_it() {
        let mut session = session_with("session-1", 10, &[]);
        session.messages.push(ChatMessage::user("run uptime"));
        session
            .messages
            .push(ChatMessage::assistant(vec![ContentBlock::ToolUse {
                id: "call-1".to_owned(),
                name: "sshell_exec_run".to_owned(),
                input: serde_json::json!({ "command": "uptime" }),
            }]));
        session.outcomes.insert(
            "call-1".to_owned(),
            AiToolOutcome {
                call_id: "call-1".to_owned(),
                ok: true,
                code: None,
                truncated: false,
                duration_ms: 4,
            },
        );
        // Enough later exchanges that the first one is over the message bound.
        for index in 0..MAX_MESSAGES {
            session.messages.push(ChatMessage::user(format!("padding {index}")));
        }

        session.trim_to_limits().expect("it fits after trimming");

        assert!(
            !session
                .messages
                .iter()
                .any(|message| message.blocks.iter().any(|block| matches!(
                    block,
                    ContentBlock::ToolUse { id, .. } if id == "call-1"
                ))),
            "the exchange holding the tool call is the one that was dropped"
        );
        assert!(
            !session.outcomes.contains_key("call-1"),
            "an outcome whose call is no longer in the document is a card nothing can draw"
        );
    }
}
