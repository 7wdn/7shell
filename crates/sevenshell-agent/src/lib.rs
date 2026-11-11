//! The assistant's turn loop.
//!
//! This crate is the part between a provider and the tool layer: it holds the conversation, sends one
//! request per turn, reads the streamed answer, runs the tool the model asked for, appends the result
//! and goes around again until the model stops or a limit is reached. What it emits is
//! [`event::AgentEvent`]s; what it calls is [`sevenshell_api::ToolRegistry`] through the context the
//! host built, so scope, approval and audit behave exactly as they do for an MCP client.
//!
//! It deliberately does not know about Tauri, the vault or the interface: the host supplies the
//! provider, the registry, the context and a sink for the events. That is what makes the loop
//! testable with a fake provider and fake tools, which is how `tests/loop.rs` exercises it.

pub mod event;
pub mod prompt;
pub mod run;

pub use event::{AgentEvent, EventSink, RunStopReason};
pub use prompt::AI_SYSTEM_PROMPT_DEFAULT;
pub use run::{
    AI_MAX_RESULT_BYTES_DEFAULT, AI_MAX_TURNS_DEFAULT, AI_MODEL_DEFAULT, AI_TOOL_PREVIEW_CHARS, AI_TOOL_SKIPPED_CODE,
    AI_TOOL_SKIPPED_CONTENT, AI_TRUNCATION_MARKER, Agent, AgentError, RunOutcome,
};
