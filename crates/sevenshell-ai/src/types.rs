//! The conversation, the request and what comes back — in a vocabulary both providers express.
//!
//! These types are the contract between the agent loop and a provider adapter. The adapter is the
//! only place that knows how Anthropic or OpenAI spells a tool call, a stop reason or a usage block;
//! everything above it reads these.
//!
//! Two rules shape the field lists:
//!
//! * **No secret has a field here.** A provider key is passed to an adapter when it is built, never
//!   carried in a request or an event.
//! * **A message is blocks, not a string.** Text, thinking, a tool call and a tool result are the
//!   four things a turn can contain, and an assistant turn may contain several of them at once.
//!   Flattening one of them into text would lose the tool call's id, which is what a result answers.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// The non-secret provider configuration is a wire type other crates already share: the settings
// page writes it, the host sends it back, and this crate must read the same names rather than keep
// a second copy that could drift.
pub use sevenshell_types::{AiProviderConfig, AiProviderKind, AiProviderView};

/// Who a message is from.
///
/// Only two: the system prompt is [`ProviderRequest::system`], because both APIs put it somewhere of
/// their own and a `System` message here would have to be moved by every adapter anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// One piece of a turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ContentBlock {
    /// What somebody said, or what the model wrote.
    Text { text: String },
    /// The model's reasoning, when the provider returns it and the person asked to see it.
    Thinking { text: String },
    /// A tool the model asked for. `id` is what the result answers; `input` is the arguments object
    /// exactly as the model produced it.
    ToolUse { id: String, name: String, input: Value },
    /// What a tool answered. `is_error` is a fact about the tool, not about the model: a refused
    /// command is a result the model reads, not a turn that failed.
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}

/// One turn of the conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub role: Role,
    pub blocks: Vec<ContentBlock>,
}

impl ChatMessage {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            blocks: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    /// What the model said, including any tool calls it asked for.
    pub fn assistant(blocks: Vec<ContentBlock>) -> Self {
        Self {
            role: Role::Assistant,
            blocks,
        }
    }

    /// What a tool answered, as one block of the next user turn.
    pub fn tool_result(tool_use_id: impl Into<String>, content: impl Into<String>, is_error: bool) -> Self {
        Self {
            role: Role::User,
            blocks: vec![ContentBlock::ToolResult {
                tool_use_id: tool_use_id.into(),
                content: content.into(),
                is_error,
            }],
        }
    }
}

/// One tool the model may call, as the request describes it.
///
/// The name is the tool layer's own (`sshell_exec_run`), and the schema is the closed object the tool
/// declares: the model is told exactly what the registry will accept, which is what keeps a call from
/// being refused for an argument the schema never offered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// One request to one provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRequest {
    pub model: String,
    /// The system prompt, when there is one. Separate from the messages because both APIs treat it
    /// as a field of the request rather than as a turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolSpec>,
    pub max_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

/// What one request cost, as the provider reported it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Tokens answered from the provider's prompt cache, when it reports them. Absent means the
    /// provider does not say, which is not the same as zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u32>,
}

/// Why the model stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    /// It finished the turn.
    EndTurn,
    /// It asked for a tool and is waiting for the result.
    ToolUse,
    /// The output budget ran out.
    MaxTokens,
    /// It reached one of the stop sequences the request named.
    StopSequence,
    /// The provider's own safety layer refused to continue.
    Refusal,
}

/// One thing that happened while the answer was arriving.
///
/// `index` is the position of the content block in the assistant message, which is what lets a
/// streamed tool call be assembled from fragments that arrive out of order relative to text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum StreamEvent {
    TextDelta {
        index: u32,
        text: String,
    },
    ThinkingDelta {
        index: u32,
        text: String,
    },
    ToolUseStart {
        index: u32,
        id: String,
        name: String,
    },
    /// A fragment of the tool's JSON arguments, exactly as it arrived. It is not parsed here: a
    /// fragment is not a document, and the assembled value arrives in [`StreamEvent::ToolUseDone`].
    ToolInputDelta {
        index: u32,
        json: String,
    },
    ToolUseDone {
        index: u32,
        id: String,
        name: String,
        input: Value,
    },
    Usage(Usage),
    Stop {
        reason: StopReason,
    },
}

/// A failure every adapter can produce.
///
/// The variants are the distinctions the agent loop acts on — retry, tell the person, stop — rather
/// than a mirror of either provider's error body. [`ProviderError::code`] is the one mapping to the
/// catalog's stable AI codes, so no adapter invents a code of its own.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    #[error("no API key is stored for this provider")]
    NoKey,
    #[error("the provider rejected the credentials")]
    Auth,
    /// The provider asked for a later retry, with a hint when it gave one.
    #[error("the provider asked for the request to be slowed down")]
    RateLimited { retry_after_ms: Option<u64> },
    #[error("the provider is overloaded")]
    Overloaded,
    #[error("the request is larger than the model's context window")]
    ContextTooLong,
    #[error("the provider could not be reached: {0}")]
    Network(String),
    #[error("the provider did not answer in time")]
    Timeout,
    /// Something arrived that this build cannot read: a stream shape, a field, a finish reason.
    #[error("the provider answered something this build does not understand: {0}")]
    Protocol(String),
    #[error("the provider answered with status {status}: {detail}")]
    Server { status: u16, detail: String },
    #[error("the request was cancelled")]
    Cancelled,
}

impl ProviderError {
    /// The catalog code this failure travels under.
    ///
    /// Every transport-shaped failure shares `AI-PROVIDER-UNAVAILABLE`: what a person can do about
    /// them is the same, and the distinction the loop needs — whether a retry is worth it — is the
    /// variant itself, not the code the interface draws.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoKey => "AI-NO-KEY",
            Self::ContextTooLong => "AI-CONTEXT-TOO-LONG",
            Self::Cancelled => "AI-CANCELLED",
            Self::Auth
            | Self::RateLimited { .. }
            | Self::Overloaded
            | Self::Network(_)
            | Self::Timeout
            | Self::Protocol(_)
            | Self::Server { .. } => "AI-PROVIDER-UNAVAILABLE",
        }
    }

    /// Whether trying the same request again could succeed.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::RateLimited { .. } | Self::Overloaded | Self::Network(_) | Self::Timeout => true,
            Self::Server { status, .. } => *status >= 500,
            Self::NoKey | Self::Auth | Self::ContextTooLong | Self::Protocol(_) | Self::Cancelled => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_tool_call_serializes_with_the_names_the_adapters_read() {
        let block = ContentBlock::ToolUse {
            id: "call-1".to_owned(),
            name: "sshell_exec_run".to_owned(),
            input: json!({ "command": "uptime" }),
        };

        let json = serde_json::to_value(&block).expect("a block is JSON");

        assert_eq!(json["type"], serde_json::json!("toolUse"));
        assert_eq!(json["id"], serde_json::json!("call-1"));
        assert_eq!(json["input"]["command"], serde_json::json!("uptime"));
    }

    #[test]
    fn a_tool_result_answers_the_call_it_names() {
        let message = ChatMessage::tool_result("call-1", "up 3 days", false);
        let json = serde_json::to_value(&message).expect("a message is JSON");

        assert_eq!(json["role"], serde_json::json!("user"));
        assert_eq!(json["blocks"][0]["type"], serde_json::json!("toolResult"));
        assert_eq!(json["blocks"][0]["toolUseId"], serde_json::json!("call-1"));
        assert_eq!(json["blocks"][0]["isError"], serde_json::json!(false));
    }

    #[test]
    fn a_stream_event_serializes_with_camel_case_fields() {
        let started = serde_json::to_value(StreamEvent::ToolUseStart {
            index: 2,
            id: "call-1".to_owned(),
            name: "sshell_sftp_list".to_owned(),
        })
        .expect("an event is JSON");
        assert_eq!(started["type"], serde_json::json!("toolUseStart"));
        assert_eq!(started["index"], serde_json::json!(2));

        let usage = serde_json::to_value(StreamEvent::Usage(Usage {
            input_tokens: 12,
            output_tokens: 3,
            cache_read_tokens: Some(10),
        }))
        .expect("an event is JSON");
        assert_eq!(usage["type"], serde_json::json!("usage"));
        assert_eq!(usage["cacheReadTokens"], serde_json::json!(10));

        let stop = serde_json::to_value(StreamEvent::Stop {
            reason: StopReason::ToolUse,
        })
        .expect("an event is JSON");
        assert_eq!(stop["reason"], serde_json::json!("toolUse"));
    }

    #[test]
    fn a_request_omits_the_parts_nobody_set() {
        let request = ProviderRequest {
            model: "claude-sonnet-4-5".to_owned(),
            system: None,
            messages: vec![ChatMessage::user("hello")],
            tools: Vec::new(),
            max_tokens: 1_024,
            temperature: None,
        };

        let json = serde_json::to_value(&request).expect("a request is JSON");

        assert_eq!(json["maxTokens"], serde_json::json!(1_024));
        assert!(json.get("system").is_none(), "an absent system prompt is not written");
        assert!(json.get("tools").is_none(), "nor is a tool list nobody offered");
        assert!(json.get("temperature").is_none());
    }

    #[test]
    fn every_failure_maps_to_a_catalog_code_and_only_some_are_worth_retrying() {
        let cases = [
            (ProviderError::NoKey, "AI-NO-KEY", false),
            (ProviderError::Auth, "AI-PROVIDER-UNAVAILABLE", false),
            (
                ProviderError::RateLimited {
                    retry_after_ms: Some(1_000),
                },
                "AI-PROVIDER-UNAVAILABLE",
                true,
            ),
            (ProviderError::Overloaded, "AI-PROVIDER-UNAVAILABLE", true),
            (ProviderError::ContextTooLong, "AI-CONTEXT-TOO-LONG", false),
            (
                ProviderError::Network("dns".to_owned()),
                "AI-PROVIDER-UNAVAILABLE",
                true,
            ),
            (ProviderError::Timeout, "AI-PROVIDER-UNAVAILABLE", true),
            (
                ProviderError::Protocol("unknown frame".to_owned()),
                "AI-PROVIDER-UNAVAILABLE",
                false,
            ),
            (
                ProviderError::Server {
                    status: 502,
                    detail: "bad gateway".to_owned(),
                },
                "AI-PROVIDER-UNAVAILABLE",
                true,
            ),
            (ProviderError::Cancelled, "AI-CANCELLED", false),
        ];

        for (error, code, retryable) in cases {
            assert_eq!(error.code(), code, "{error} mapped to the wrong code");
            assert_eq!(
                error.is_retryable(),
                retryable,
                "{error} says the wrong thing about retrying"
            );
        }
    }
}
