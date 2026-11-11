//! A stored assistant conversation, as the interface reads it back.
//!
//! The vault keeps each conversation as one opaque string keyed by its id (see
//! `sevenshell_vault::Document::ai_sessions`), because the vault has no business learning what a
//! content block is: a build that predates a block shape would have to drop it on its next write.
//! This module is the *other* end — the shape the window is given once the host has opened that
//! string and turned it into the facts a list and a transcript draw.
//!
//! Two decisions shape the fields:
//!
//! * **Messages stay JSON.** A transcript is a list of messages whose blocks are the provider
//!   vocabulary; typing them here would put a second copy of `ContentBlock` in a `rank 0` crate and
//!   make an unknown block a parse failure. Passing the stored values through means a block this
//!   build does not draw is still there after a round trip through the window.
//! * **One outcome per tool call, named by the call.** The tool cards are rebuilt from the messages'
//!   tool-use blocks *and* these rows: a card's tick, its error code and whether its result was
//!   truncated are facts about a call that the message itself does not carry.
//!
//! Nothing here has a field for a key, a prompt or a path: a conversation is text somebody typed and
//! what the model answered, and it travels to the window the way it was stored.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The version written into a [`AiContextSummary`].
///
/// A version rather than nothing, because the summary is text a *model* wrote: a later build that
/// changes how a summary is phrased has to be able to tell an old one from a new one, and a stored
/// document is where it finds that out.
pub const AI_CONTEXT_SUMMARY_VERSION: u32 = 1;

pub fn ai_context_summary_version_default() -> u32 {
    AI_CONTEXT_SUMMARY_VERSION
}

/// One stored conversation, as the list of them names it.
///
/// The name is a name, not a question: the host derives it from the first question and a person may
/// rename it, which is why it is stored in the document rather than recomputed from `messages`
/// here — a recomputed name would undo the rename.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSessionSummary {
    pub id: String,
    /// What the list draws. Empty only for a session nobody has named yet.
    #[serde(default)]
    pub name: String,
    pub created_at: u64,
    /// When the conversation last grew, which is what the list sorts by.
    pub updated_at: u64,
    /// How many messages are in the document. The list says "12 messages" without opening it.
    #[serde(default)]
    pub message_count: u32,
    /// Whether the older half has been replaced by a [`AiContextSummary`] already.
    #[serde(default)]
    pub has_summary: bool,
}

/// One tool call's outcome, as a restored card needs it.
///
/// `call_id` repeats the key it arrives under, and it is deliberate: a card drawn from the map
/// itself has the id, while a card drawn from the value — handed to a component one row at a time —
/// would otherwise have nothing to answer its own tool-use block with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiToolOutcome {
    pub call_id: String,
    /// Whether the tool did what was asked. A refused approval is `false` with a code, which is a
    /// fact the model read rather than a turn that failed.
    pub ok: bool,
    /// The catalog code of the failure, when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Whether the result the model saw was cut down to the tool layer's size limit.
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub duration_ms: u64,
}

/// The older half of a conversation, after it was replaced by something shorter.
///
/// A failed summarisation is not an error anybody sees as a code: the host falls back to a
/// deterministic truncation and keeps going, so `AI-SUMMARY-FAILED` exists in logs and events only
/// and is deliberately absent from the error catalog — there is nothing a person could do about a
/// summary that failed, and the conversation still works.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiContextSummary {
    #[serde(default = "ai_context_summary_version_default")]
    pub version: u32,
    /// What the model wrote, in place of the messages it replaced.
    pub content: String,
    /// The message the summary covers up to and including; everything after it is still verbatim
    /// in `messages`, and this is how the window knows where the summary ends.
    pub through_message_id: String,
    pub created_at: u64,
}

/// One conversation, in full, as the transcript draws it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSessionDetail {
    /// The same row the list draws, so a restored conversation can name itself.
    pub summary: AiSessionSummary,
    /// The messages exactly as they were stored. Each is a `{ role, blocks }` object; anything a
    /// newer build wrote is passed through rather than dropped.
    #[serde(default)]
    pub messages: Vec<serde_json::Value>,
    /// One row per tool call, keyed by the call id the assistant message names.
    #[serde(default)]
    pub outcomes: BTreeMap<String, AiToolOutcome>,
    /// The summary that replaced the oldest messages, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_summary: Option<AiContextSummary>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn summary() -> AiSessionSummary {
        AiSessionSummary {
            id: "session-1".to_owned(),
            name: "why is nginx 502".to_owned(),
            created_at: 1_700_000_000,
            updated_at: 1_700_000_100,
            message_count: 4,
            has_summary: true,
        }
    }

    #[test]
    fn a_session_summary_speaks_the_names_the_list_reads() {
        let json = serde_json::to_value(summary()).expect("a summary is JSON");

        assert_eq!(json["id"], json!("session-1"));
        assert_eq!(json["createdAt"], json!(1_700_000_000_u64));
        assert_eq!(json["updatedAt"], json!(1_700_000_100_u64));
        assert_eq!(json["messageCount"], json!(4));
        assert_eq!(json["hasSummary"], json!(true));
        assert!(
            json.get("message_count").is_none(),
            "a snake_case key is one the window reads as `undefined`: {json}"
        );
    }

    #[test]
    fn a_detail_round_trips_with_its_outcomes_and_its_passed_through_messages() {
        let detail = AiSessionDetail {
            summary: summary(),
            // A block this build has never heard of: it has to come back unchanged rather than as
            // an empty message, because a transcript that silently loses a block is worse than one
            // that draws an odd row.
            messages: vec![
                json!({ "role": "user", "blocks": [{ "type": "text", "text": "why 502" }] }),
                json!({ "role": "assistant", "blocks": [{ "type": "someFutureBlock", "payload": 1 }] }),
            ],
            outcomes: BTreeMap::from([(
                "call-1".to_owned(),
                AiToolOutcome {
                    call_id: "call-1".to_owned(),
                    ok: false,
                    code: Some("POLICY-DENIED".to_owned()),
                    truncated: true,
                    duration_ms: 12,
                },
            )]),
            context_summary: Some(AiContextSummary {
                version: AI_CONTEXT_SUMMARY_VERSION,
                content: "the older half, summarised".to_owned(),
                through_message_id: "message-2".to_owned(),
                created_at: 1_700_000_200,
            }),
        };

        let json = serde_json::to_value(&detail).expect("a detail is JSON");
        assert_eq!(json["messages"][1]["blocks"][0]["type"], json!("someFutureBlock"));
        assert_eq!(json["outcomes"]["call-1"]["callId"], json!("call-1"));
        assert_eq!(json["outcomes"]["call-1"]["durationMs"], json!(12));
        assert_eq!(json["contextSummary"]["throughMessageId"], json!("message-2"));

        let back: AiSessionDetail = serde_json::from_value(json).expect("round trip");
        assert_eq!(back, detail);
    }

    #[test]
    fn a_detail_written_without_the_optional_halves_reads_as_having_none() {
        let detail: AiSessionDetail = serde_json::from_value(json!({
            "summary": {
                "id": "session-1",
                "name": "a question",
                "createdAt": 1,
                "updatedAt": 2
            }
        }))
        .expect("a document that predates outcomes is still readable");

        assert!(detail.messages.is_empty());
        assert!(detail.outcomes.is_empty());
        assert!(detail.context_summary.is_none());
        assert_eq!(detail.summary.message_count, 0);
        assert!(!detail.summary.has_summary);
    }

    #[test]
    fn a_summary_written_without_a_version_reads_as_the_first_one() {
        let summary: AiContextSummary = serde_json::from_value(json!({
            "content": "the older half",
            "throughMessageId": "message-2",
            "createdAt": 1
        }))
        .expect("a summary is readable");

        assert_eq!(summary.version, AI_CONTEXT_SUMMARY_VERSION);
    }
}
