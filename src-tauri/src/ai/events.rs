//! The assistant's half of the IPC vocabulary: eight events, and the seam that delivers them.
//!
//! The names and payloads are the contract the window was written against:
//!
//! | event | payload |
//! |---|---|
//! | `ai:delta` | `runId, messageId, blockIndex, text` |
//! | `ai:thinking` | `runId, messageId, blockIndex, text` |
//! | `ai:tool_call` | `runId, callId, tool, input, sessionId?` |
//! | `ai:tool_result` | `runId, callId, tool, ok, code?, preview, truncated, durationMs` |
//! | `ai:done` | `runId, stopReason, turns, usage?` |
//! | `ai:error` | `runId, code, messageKey, messageParams, retryable` |
//! | `ai:session_saved` | `sessionId, updatedAt` |
//! | `ai:compacted` | `sessionId, summarizedMessages, keptMessages, degraded` |
//!
//! The first six are one run's, and every payload carries that run's id: a second question may be
//! asked while the first answer is still arriving, and an event from one run must never be painted
//! into the other. The last two are the conversation's rather than a run's — a save outlives the run
//! that caused it, and a summary is a fact about the document, not about one answer — so they carry
//! a session id and no run id at all.
//!
//! Delivery is a trait rather than a direct `AppHandle` so the run can be exercised without a
//! webview — the same seam [`crate::mcp::approval::ApprovalPresenter`] gives the approval dialog. The
//! real one emits; a test records what it was given and asserts the shape.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;
use sevenshell_agent::{AgentEvent, RunStopReason};
use sevenshell_ai::Usage;
use tauri::{AppHandle, Emitter as _};

/// The event names, one constant each so a typo is a compile error at the emitter.
pub const DELTA_EVENT: &str = "ai:delta";
pub const THINKING_EVENT: &str = "ai:thinking";
pub const TOOL_CALL_EVENT: &str = "ai:tool_call";
pub const TOOL_RESULT_EVENT: &str = "ai:tool_result";
pub const DONE_EVENT: &str = "ai:done";
pub const ERROR_EVENT: &str = "ai:error";
pub const SESSION_SAVED_EVENT: &str = "ai:session_saved";
pub const COMPACTED_EVENT: &str = "ai:compacted";

/// One thing the assistant says to the window.
#[derive(Debug, Clone, PartialEq)]
pub enum AiChatEvent {
    Delta {
        run_id: String,
        message_id: String,
        block_index: u32,
        text: String,
    },
    Thinking {
        run_id: String,
        message_id: String,
        block_index: u32,
        text: String,
    },
    ToolCall {
        run_id: String,
        call_id: String,
        tool: String,
        input: Value,
        session_id: Option<String>,
    },
    ToolResult {
        run_id: String,
        call_id: String,
        tool: String,
        ok: bool,
        code: Option<String>,
        preview: String,
        truncated: bool,
        duration_ms: u64,
    },
    Done {
        run_id: String,
        stop_reason: &'static str,
        turns: u32,
        usage: Option<Usage>,
    },
    Error {
        run_id: String,
        code: String,
        message_key: String,
        message_params: BTreeMap<String, String>,
        retryable: bool,
    },
    /// A conversation was written to the vault, and when.
    ///
    /// The timestamp rather than the transcript: the list only needs the new `updatedAt`, and a
    /// payload carrying the messages would make every save a second copy of them on the wire.
    SessionSaved { session_id: String, updated_at: u64 },
    /// The older half of a conversation was replaced by something shorter.
    Compacted {
        session_id: String,
        summarized_messages: u32,
        kept_messages: u32,
        /// The model did not write the summary; the host grouped-and-cut instead.
        degraded: bool,
    },
}

impl AiChatEvent {
    /// The loop's event, in the window's vocabulary. One-for-one: the loop says what happened and
    /// this says it in the names the interface subscribes to.
    pub fn from_agent(event: &AgentEvent) -> Self {
        match event {
            AgentEvent::TextDelta {
                run_id,
                message_id,
                block_index,
                text,
            } => Self::Delta {
                run_id: run_id.clone(),
                message_id: message_id.clone(),
                block_index: *block_index,
                text: text.clone(),
            },
            AgentEvent::ThinkingDelta {
                run_id,
                message_id,
                block_index,
                text,
            } => Self::Thinking {
                run_id: run_id.clone(),
                message_id: message_id.clone(),
                block_index: *block_index,
                text: text.clone(),
            },
            AgentEvent::ToolCall {
                run_id,
                call_id,
                tool,
                input,
                session_id,
            } => Self::ToolCall {
                run_id: run_id.clone(),
                call_id: call_id.clone(),
                tool: tool.clone(),
                input: input.clone(),
                session_id: session_id.clone(),
            },
            AgentEvent::ToolResult {
                run_id,
                call_id,
                tool,
                ok,
                code,
                preview,
                truncated,
                duration_ms,
            } => Self::ToolResult {
                run_id: run_id.clone(),
                call_id: call_id.clone(),
                tool: tool.clone(),
                ok: *ok,
                code: code.clone(),
                preview: preview.clone(),
                truncated: *truncated,
                duration_ms: *duration_ms,
            },
            AgentEvent::Done {
                run_id,
                stop_reason,
                turns,
                usage,
            } => Self::Done {
                run_id: run_id.clone(),
                stop_reason: stop_reason_name(*stop_reason),
                turns: *turns,
                usage: *usage,
            },
            AgentEvent::Failed {
                run_id,
                code,
                message_key,
                message_params,
                retryable,
            } => Self::Error {
                run_id: run_id.clone(),
                code: code.clone(),
                message_key: message_key.clone(),
                message_params: message_params.clone(),
                retryable: *retryable,
            },
        }
    }

    /// The IPC event name this one is delivered under.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Delta { .. } => DELTA_EVENT,
            Self::Thinking { .. } => THINKING_EVENT,
            Self::ToolCall { .. } => TOOL_CALL_EVENT,
            Self::ToolResult { .. } => TOOL_RESULT_EVENT,
            Self::Done { .. } => DONE_EVENT,
            Self::Error { .. } => ERROR_EVENT,
            Self::SessionSaved { .. } => SESSION_SAVED_EVENT,
            Self::Compacted { .. } => COMPACTED_EVENT,
        }
    }

    /// The payload exactly as the contract names it: camelCase, optional fields left out.
    pub fn payload(&self) -> Value {
        match self {
            Self::Delta {
                run_id,
                message_id,
                block_index,
                text,
            } => serde_json::to_value(DeltaPayload {
                run_id,
                message_id,
                block_index: *block_index,
                text,
            }),
            Self::Thinking {
                run_id,
                message_id,
                block_index,
                text,
            } => serde_json::to_value(DeltaPayload {
                run_id,
                message_id,
                block_index: *block_index,
                text,
            }),
            Self::ToolCall {
                run_id,
                call_id,
                tool,
                input,
                session_id,
            } => serde_json::to_value(ToolCallPayload {
                run_id,
                call_id,
                tool,
                input,
                session_id: session_id.as_deref(),
            }),
            Self::ToolResult {
                run_id,
                call_id,
                tool,
                ok,
                code,
                preview,
                truncated,
                duration_ms,
            } => serde_json::to_value(ToolResultPayload {
                run_id,
                call_id,
                tool,
                ok: *ok,
                code: code.as_deref(),
                preview,
                truncated: *truncated,
                duration_ms: *duration_ms,
            }),
            Self::Done {
                run_id,
                stop_reason,
                turns,
                usage,
            } => serde_json::to_value(DonePayload {
                run_id,
                stop_reason,
                turns: *turns,
                usage: *usage,
            }),
            Self::Error {
                run_id,
                code,
                message_key,
                message_params,
                retryable,
            } => serde_json::to_value(ErrorPayload {
                run_id,
                code,
                message_key,
                message_params,
                retryable: *retryable,
            }),
            Self::SessionSaved { session_id, updated_at } => serde_json::to_value(SessionSavedPayload {
                session_id,
                updated_at: *updated_at,
            }),
            Self::Compacted {
                session_id,
                summarized_messages,
                kept_messages,
                degraded,
            } => serde_json::to_value(CompactedPayload {
                session_id,
                summarized_messages: *summarized_messages,
                kept_messages: *kept_messages,
                degraded: *degraded,
            }),
        }
        .expect("an assistant event payload is JSON: its fields are strings, numbers and JSON values")
    }

    /// The end of a run the loop did not announce itself.
    ///
    /// The loop owns its own `Done`, but a run that ends without one — a failure the host produced,
    /// or a loop that returned and forgot — would leave the window drawing an answer forever. The
    /// host sends this in that case, with the outcome it read.
    pub fn done(run_id: &str, stop_reason: RunStopReason, turns: u32, usage: Option<Usage>) -> Self {
        Self::Done {
            run_id: run_id.to_owned(),
            stop_reason: stop_reason_name(stop_reason),
            turns,
            usage,
        }
    }

    /// A conversation reached the vault.
    pub fn session_saved(session_id: &str, updated_at: u64) -> Self {
        Self::SessionSaved {
            session_id: session_id.to_owned(),
            updated_at,
        }
    }

    /// The older half of a conversation was replaced.
    pub fn compacted(session_id: &str, summarized_messages: u32, kept_messages: u32, degraded: bool) -> Self {
        Self::Compacted {
            session_id: session_id.to_owned(),
            summarized_messages,
            kept_messages,
            degraded,
        }
    }

    /// The same event, filed under the run id the window was told.
    ///
    /// The loop mints its own run id and puts it in every event; the window was given the host's id
    /// before the first event arrived, and an event filed under an id the window does not know is an
    /// event it drops. Re-filing here keeps the loop's id an internal detail.
    pub fn under(self, run_id: &str) -> Self {
        match self {
            Self::Delta {
                message_id,
                block_index,
                text,
                ..
            } => Self::Delta {
                run_id: run_id.to_owned(),
                message_id,
                block_index,
                text,
            },
            Self::Thinking {
                message_id,
                block_index,
                text,
                ..
            } => Self::Thinking {
                run_id: run_id.to_owned(),
                message_id,
                block_index,
                text,
            },
            Self::ToolCall {
                call_id,
                tool,
                input,
                session_id,
                ..
            } => Self::ToolCall {
                run_id: run_id.to_owned(),
                call_id,
                tool,
                input,
                session_id,
            },
            Self::ToolResult {
                call_id,
                tool,
                ok,
                code,
                preview,
                truncated,
                duration_ms,
                ..
            } => Self::ToolResult {
                run_id: run_id.to_owned(),
                call_id,
                tool,
                ok,
                code,
                preview,
                truncated,
                duration_ms,
            },
            Self::Done {
                stop_reason,
                turns,
                usage,
                ..
            } => Self::Done {
                run_id: run_id.to_owned(),
                stop_reason,
                turns,
                usage,
            },
            Self::Error {
                code,
                message_key,
                message_params,
                retryable,
                ..
            } => Self::Error {
                run_id: run_id.to_owned(),
                code,
                message_key,
                message_params,
                retryable,
            },
            // A conversation's events name a session and no run; there is nothing to re-file.
            other @ (Self::SessionSaved { .. } | Self::Compacted { .. }) => other,
        }
    }
}

/// The wire name of a stop reason. The window's union is these four strings.
fn stop_reason_name(reason: RunStopReason) -> &'static str {
    match reason {
        RunStopReason::EndTurn => "endTurn",
        RunStopReason::MaxTurns => "maxTurns",
        RunStopReason::Cancelled => "cancelled",
        RunStopReason::Error => "error",
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeltaPayload<'a> {
    run_id: &'a str,
    message_id: &'a str,
    block_index: u32,
    text: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolCallPayload<'a> {
    run_id: &'a str,
    call_id: &'a str,
    tool: &'a str,
    input: &'a Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolResultPayload<'a> {
    run_id: &'a str,
    call_id: &'a str,
    tool: &'a str,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'a str>,
    preview: &'a str,
    truncated: bool,
    duration_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DonePayload<'a> {
    run_id: &'a str,
    stop_reason: &'a str,
    turns: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<Usage>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorPayload<'a> {
    run_id: &'a str,
    code: &'a str,
    message_key: &'a str,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    message_params: &'a BTreeMap<String, String>,
    retryable: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionSavedPayload<'a> {
    session_id: &'a str,
    updated_at: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CompactedPayload<'a> {
    session_id: &'a str,
    summarized_messages: u32,
    kept_messages: u32,
    degraded: bool,
}

/// Where a run's events go. `false` means nothing received it.
pub trait AiEventPresenter: Send + Sync {
    fn emit(&self, event: &AiChatEvent) -> bool;
}

/// The real presenter: one event to the window, under its own name.
pub struct WindowEvents {
    app: AppHandle,
}

impl WindowEvents {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl AiEventPresenter for WindowEvents {
    fn emit(&self, event: &AiChatEvent) -> bool {
        // A window that is gone is not an error worth reporting: closing it cancels the run, and the
        // events that were already in flight have nowhere left to be drawn. The answer says so, and
        // the caller only logs it.
        self.app.emit(event.name(), event.payload()).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingPresenter {
        seen: Mutex<Vec<(String, Value)>>,
    }

    impl RecordingPresenter {
        fn seen(&self) -> Vec<(String, Value)> {
            self.seen.lock().expect("not poisoned").clone()
        }
    }

    impl AiEventPresenter for RecordingPresenter {
        fn emit(&self, event: &AiChatEvent) -> bool {
            self.seen
                .lock()
                .expect("not poisoned")
                .push((event.name().to_owned(), event.payload()));
            true
        }
    }

    #[test]
    fn every_event_is_delivered_under_the_contract_name_with_camel_case_fields() {
        let presenter = RecordingPresenter::default();
        let events = [
            AiChatEvent::Delta {
                run_id: "run-1".to_owned(),
                message_id: "message-1".to_owned(),
                block_index: 2,
                text: "hello".to_owned(),
            },
            AiChatEvent::Thinking {
                run_id: "run-1".to_owned(),
                message_id: "message-1".to_owned(),
                block_index: 0,
                text: "considering".to_owned(),
            },
            AiChatEvent::ToolCall {
                run_id: "run-1".to_owned(),
                call_id: "call-1".to_owned(),
                tool: "sshell_exec_run".to_owned(),
                input: serde_json::json!({ "command": "uptime" }),
                session_id: Some("session-1".to_owned()),
            },
            AiChatEvent::ToolResult {
                run_id: "run-1".to_owned(),
                call_id: "call-1".to_owned(),
                tool: "sshell_exec_run".to_owned(),
                ok: true,
                code: None,
                preview: "up 3 days".to_owned(),
                truncated: false,
                duration_ms: 120,
            },
            AiChatEvent::done("run-1", RunStopReason::EndTurn, 2, None),
            AiChatEvent::Error {
                run_id: "run-1".to_owned(),
                code: "AI-NO-KEY".to_owned(),
                message_key: "errors.ai.noKey".to_owned(),
                message_params: BTreeMap::new(),
                retryable: false,
            },
        ];

        for event in &events {
            assert!(presenter.emit(event));
        }

        let seen = presenter.seen();
        assert_eq!(
            seen.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(),
            vec![
                "ai:delta",
                "ai:thinking",
                "ai:tool_call",
                "ai:tool_result",
                "ai:done",
                "ai:error"
            ]
        );

        assert_eq!(seen[0].1["runId"], serde_json::json!("run-1"));
        assert_eq!(seen[0].1["messageId"], serde_json::json!("message-1"));
        assert_eq!(seen[0].1["blockIndex"], serde_json::json!(2));

        assert_eq!(seen[2].1["input"]["command"], serde_json::json!("uptime"));
        assert_eq!(seen[2].1["sessionId"], serde_json::json!("session-1"));

        assert!(seen[3].1.get("code").is_none(), "a success has no failure code");
        assert_eq!(seen[3].1["durationMs"], serde_json::json!(120));

        assert_eq!(seen[4].1["stopReason"], serde_json::json!("endTurn"));
        assert!(
            seen[4].1.get("usage").is_none(),
            "a run that reported no usage writes none"
        );

        assert_eq!(seen[5].1["code"], serde_json::json!("AI-NO-KEY"));
        assert_eq!(seen[5].1["messageKey"], serde_json::json!("errors.ai.noKey"));
        assert_eq!(seen[5].1["retryable"], serde_json::json!(false));
        assert!(
            seen[5].1.get("messageParams").is_none(),
            "an error with nothing to fill in writes no parameter object"
        );
    }

    #[test]
    fn a_loop_event_is_carried_through_unchanged() {
        let agent_event = AgentEvent::TextDelta {
            run_id: "run-1".to_owned(),
            message_id: "message-1".to_owned(),
            block_index: 1,
            text: "more".to_owned(),
        };

        let event = AiChatEvent::from_agent(&agent_event);

        assert_eq!(event.name(), DELTA_EVENT);
        assert_eq!(event.payload()["text"], serde_json::json!("more"));
    }

    #[test]
    fn an_event_is_refiled_under_the_run_the_window_was_told_about() {
        // The loop names its own run; the window was given the host's before the first event. An
        // event under an id the window never saw is one it drops.
        let event = AiChatEvent::Delta {
            run_id: "the-loop's-id".to_owned(),
            message_id: "message-1".to_owned(),
            block_index: 0,
            text: "hello".to_owned(),
        };

        let refiled = event.under("the-window's-id");

        assert_eq!(refiled.payload()["runId"], serde_json::json!("the-window's-id"));
        assert_eq!(
            refiled.payload()["messageId"],
            serde_json::json!("message-1"),
            "only the run id is re-filed"
        );
    }

    #[test]
    fn a_conversations_events_name_the_session_and_no_run() {
        let presenter = RecordingPresenter::default();
        let events = [
            AiChatEvent::session_saved("session-1", 1_700_000_000),
            AiChatEvent::compacted("session-1", 12, 4, true),
        ];

        for event in &events {
            assert!(presenter.emit(event));
        }

        let seen = presenter.seen();
        assert_eq!(seen[0].0, SESSION_SAVED_EVENT);
        assert_eq!(
            seen[0].1,
            serde_json::json!({ "sessionId": "session-1", "updatedAt": 1_700_000_000_u64 })
        );
        assert!(
            seen[0].1.get("runId").is_none(),
            "a save is the conversation's, not one run's"
        );

        assert_eq!(seen[1].0, COMPACTED_EVENT);
        assert_eq!(
            seen[1].1,
            serde_json::json!({
                "sessionId": "session-1",
                "summarizedMessages": 12,
                "keptMessages": 4,
                "degraded": true
            })
        );

        // Re-filing under a run id would put a session's name where a run's belongs; the pump only
        // ever re-files what the loop said, and the loop says nothing about a conversation.
        assert_eq!(
            events[0].clone().under("run-1").payload(),
            seen[0].1,
            "a save is the conversation's, whatever run is in flight"
        );
    }

    #[test]
    fn every_stop_reason_has_the_name_the_window_draws() {
        for (reason, name) in [
            (RunStopReason::EndTurn, "endTurn"),
            (RunStopReason::MaxTurns, "maxTurns"),
            (RunStopReason::Cancelled, "cancelled"),
            (RunStopReason::Error, "error"),
        ] {
            let event = AiChatEvent::done("run-1", reason, 1, None);
            assert_eq!(event.payload()["stopReason"], serde_json::json!(name));
        }
    }
}
