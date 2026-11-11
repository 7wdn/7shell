//! What the assistant tells the window while it works.
//!
//! One run answers with a stream of these, and every one of them names its `runId`: the window may
//! have started a second conversation, and an event from the first must not paint into the second.
//! `messageId` groups the blocks of one assistant turn — text, thinking and tool cards — because a
//! turn can contain more than one of them and they arrive interleaved.
//!
//! The host maps these onto the `ai:*` IPC events and does no interpretation of its own; anything a
//! person reads (a localized sentence, a tool's title) is drawn by the interface from the codes and
//! keys here. That is why a failure carries a catalog code and a message key rather than a sentence.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sevenshell_ai::Usage;

/// Why a run ended.
///
/// Four answers, one of which is not a provider's: `maxTurns` and `cancelled` are the loop's own, and
/// `error` is a failure the [`super::event::AgentEvent::Failed`] before it explains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunStopReason {
    /// The model finished its turn and no tool called for another.
    EndTurn,
    /// The loop reached its tool-call limit.
    MaxTurns,
    /// Somebody called the run off.
    Cancelled,
    /// The run failed; the failure's own event says why.
    Error,
}

/// One thing the run did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AgentEvent {
    /// More text of the assistant's answer.
    TextDelta {
        run_id: String,
        message_id: String,
        block_index: u32,
        text: String,
    },
    /// More of the model's reasoning, for the part of the window that folds it away.
    ThinkingDelta {
        run_id: String,
        message_id: String,
        block_index: u32,
        text: String,
    },
    /// The model asked for a tool. The call's id is what its result will answer.
    ToolCall {
        run_id: String,
        call_id: String,
        tool: String,
        input: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
    /// A tool answered. `preview` is bounded and scrubbed by the loop before it is sent, because it is
    /// drawn in the window and may be copied out of it.
    ToolResult {
        run_id: String,
        call_id: String,
        tool: String,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
        preview: String,
        truncated: bool,
        duration_ms: u64,
    },
    /// The run finished.
    Done {
        run_id: String,
        stop_reason: RunStopReason,
        turns: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
    },
    /// The run failed. `message_params` are already scrubbed; a credential has no path into this event.
    Failed {
        run_id: String,
        code: String,
        message_key: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        message_params: BTreeMap<String, String>,
        retryable: bool,
    },
}

impl AgentEvent {
    /// The run this event belongs to, which is what the window files it under.
    pub fn run_id(&self) -> &str {
        match self {
            Self::TextDelta { run_id, .. }
            | Self::ThinkingDelta { run_id, .. }
            | Self::ToolCall { run_id, .. }
            | Self::ToolResult { run_id, .. }
            | Self::Done { run_id, .. }
            | Self::Failed { run_id, .. } => run_id,
        }
    }
}

/// Where a run's events go.
///
/// Unbounded on purpose: an event is a few hundred bytes and the consumer is the host, which forwards
/// it to a window. A bounded channel would make the loop wait on the interface, and an interface that
/// stopped reading would stall a network stream rather than drop a frame.
pub type EventSink = tokio::sync::mpsc::UnboundedSender<AgentEvent>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_delta_names_its_run_and_message() {
        let event = AgentEvent::TextDelta {
            run_id: "run-1".to_owned(),
            message_id: "message-1".to_owned(),
            block_index: 0,
            text: "hello".to_owned(),
        };

        let json = serde_json::to_value(&event).expect("an event is JSON");

        assert_eq!(json["kind"], serde_json::json!("textDelta"));
        assert_eq!(json["runId"], serde_json::json!("run-1"));
        assert_eq!(json["messageId"], serde_json::json!("message-1"));
        assert_eq!(json["blockIndex"], serde_json::json!(0));
    }

    #[test]
    fn a_tool_result_omits_the_code_it_does_not_have() {
        let event = AgentEvent::ToolResult {
            run_id: "run-1".to_owned(),
            call_id: "call-1".to_owned(),
            tool: "sshell_exec_run".to_owned(),
            ok: true,
            code: None,
            preview: "up 3 days".to_owned(),
            truncated: false,
            duration_ms: 120,
        };

        let json = serde_json::to_value(&event).expect("an event is JSON");

        assert_eq!(json["kind"], serde_json::json!("toolResult"));
        assert!(json.get("code").is_none(), "a success has no failure code to draw");
        assert_eq!(json["durationMs"], serde_json::json!(120));
    }

    #[test]
    fn a_finished_run_says_why_and_how_many_turns_it_took() {
        let event = AgentEvent::Done {
            run_id: "run-1".to_owned(),
            stop_reason: RunStopReason::MaxTurns,
            turns: 12,
            usage: Some(Usage {
                input_tokens: 900,
                output_tokens: 120,
                cache_read_tokens: None,
            }),
        };

        let json = serde_json::to_value(&event).expect("an event is JSON");

        assert_eq!(json["kind"], serde_json::json!("done"));
        assert_eq!(json["stopReason"], serde_json::json!("maxTurns"));
        assert_eq!(json["usage"]["inputTokens"], serde_json::json!(900));
        assert_eq!(event.run_id(), "run-1");
    }

    #[test]
    fn a_failure_carries_a_code_and_a_key_rather_than_a_sentence() {
        let event = AgentEvent::Failed {
            run_id: "run-1".to_owned(),
            code: "AI-NO-KEY".to_owned(),
            message_key: "errors.ai.noKey".to_owned(),
            message_params: BTreeMap::new(),
            retryable: false,
        };

        let json = serde_json::to_value(&event).expect("an event is JSON");

        assert_eq!(json["kind"], serde_json::json!("failed"));
        assert_eq!(json["code"], serde_json::json!("AI-NO-KEY"));
        assert_eq!(json["messageKey"], serde_json::json!("errors.ai.noKey"));
        assert!(
            json.get("messageParams").is_none(),
            "a failure with nothing to fill in writes no parameter object"
        );
    }
}
