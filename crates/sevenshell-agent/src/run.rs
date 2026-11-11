//! The loop itself.
//!
//! A run is one conversation: each turn streams one request from the provider, at most one tool
//! runs per turn, its result is appended, and the loop goes around until the model stops asking for
//! tools, the run is called off, or [`Agent::max_turns`] is reached. What the loop emits is described
//! in [`crate::event`]; what it calls is the tool registry through the [`ToolContext`] the host
//! built, so approval, scope and audit are the same rules an MCP call meets.
//!
//! Three properties are deliberate:
//!
//! * **The loop never asks anything itself.** Approval and the timeout that ends it belong to the
//!   tool, reached through the context. A refusal therefore arrives as a tool result the model
//!   reads, not as a dialog the loop opened.
//! * **The limits are the host's.** `max_turns` and `max_result_bytes` are fields of the agent, set
//!   by whoever built it, never taken from the model's output.
//! * **A tool call is the model's, verbatim.** The arguments go to the registry exactly as they
//!   arrived; the loop adds nothing and strips nothing, which is what keeps `acknowledged` an
//!   invalid argument rather than a hint the loop could satisfy.
//!
//! Only complete turns become history. A turn that was cancelled or that failed is shown in the
//! window as far as it got, but it is not part of [`RunOutcome::history`]: a half turn sent back to
//! a provider is either an empty answer or a tool call without its result, and neither is a
//! conversation the next request can carry.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::Value;
use sevenshell_ai::{
    ChatMessage, ContentBlock, Provider, ProviderError, ProviderRequest, Role, StreamEvent, ToolSpec, Usage,
};
use sevenshell_api::{Cancel, ToolCall, ToolContext, ToolError, ToolOutcome, ToolRegistry, scope_allows};
use sevenshell_error::{ErrorCode, scrub_bounded};
use sevenshell_types::{AI_MAX_OUTPUT_TOKENS_DEFAULT, McpScope};
use uuid::Uuid;

use crate::event::{AgentEvent, EventSink, RunStopReason};
use crate::prompt::AI_SYSTEM_PROMPT_DEFAULT;

/// How many turns one run may take before it stops itself.
///
/// Twelve is enough for an investigation that reads a listing, opens a session, runs a command and
/// looks at the result, and small enough that a model going in circles stops in a minute rather than
/// an hour. A run that hits it reports `maxTurns` rather than an empty success, and does not draw a
/// failure: nothing went wrong, the limit did its job.
pub const AI_MAX_TURNS_DEFAULT: u32 = 12;

/// The most one tool result may contribute to the conversation, in bytes.
///
/// Smaller than the bridge's own limit because this result travels inside a conversation that has
/// to fit a context window: a directory listing of a huge directory or a long log would otherwise
/// push out the question it was fetched for.
pub const AI_MAX_RESULT_BYTES_DEFAULT: u64 = 32 * 1024;

/// How much of a tool result the window is given to draw.
///
/// The card shows what the tool found; the model reads the whole (bounded) result. A preview is
/// scrubbed before it is sent, because it is text a person reads and may copy.
pub const AI_TOOL_PREVIEW_CHARS: usize = 2_048;

/// The model a request names when the host did not say which one.
///
/// Not a recommendation. The host always knows the configured provider and its model, and a run
/// that reaches a provider with this name is a run whose caller forgot [`Agent::with_model`] — the
/// provider's own "unknown model" error is a clearer report of that than an empty string would be.
pub const AI_MODEL_DEFAULT: &str = "claude-sonnet-4-5";

/// The code a tool call that was not run travels under.
///
/// Two calls in one turn is one tool too many; the second is answered with this rather than being
/// queued, because a model that batches unrelated actions should be told to take them one at a time.
pub const AI_TOOL_SKIPPED_CODE: &str = "AI-TOOL-SKIPPED";

/// What the model is told about a call this turn did not run.
///
/// Prompt material, like a tool description: the model reads it, so it is a plain sentence naming
/// what to do next rather than a translated one.
pub const AI_TOOL_SKIPPED_CONTENT: &str =
    "AI-TOOL-SKIPPED: only one tool runs per turn. Ask for this one again on the next turn if you still need it.";

/// What is appended to a result that did not fit the byte limit.
///
/// A cut JSON document is one that parses as nothing, so the cut is admitted: a model that knows it
/// was given a prefix can ask a narrower question instead of reading a broken document.
pub const AI_TRUNCATION_MARKER: &str = "\n…[truncated]";

/// What a run came to.
#[derive(Debug, Clone, PartialEq)]
pub struct RunOutcome {
    pub run_id: String,
    pub stop_reason: RunStopReason,
    /// How many requests were sent to the provider. Zero for a run that was called off before its
    /// first turn.
    pub turns: u32,
    /// What the provider said the conversation cost, added up over the turns. Absent when the
    /// provider never said.
    pub usage: Option<Usage>,
    /// The conversation as it stands after this run, oldest first: what the host hands back as the
    /// history of the next one. It is `history` plus the turns that completed, and it is the reason
    /// this type exists rather than just the stop reason — a run that returned only a verdict would
    /// make the next question arrive with no conversation behind it.
    pub history: Vec<ChatMessage>,
}

/// A failure the loop itself produces.
///
/// Everything below the loop fails into an [`AgentEvent::Failed`] and a [`RunStopReason::Error`],
/// because a provider's refusal or a tool's refusal is still an answer the window has to draw, and
/// the conversation up to that point is still worth keeping. This one is different: the window that
/// would read the answer is gone, so there is nobody to tell and nothing left to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentError {
    /// The event sink was closed. The run stops rather than calling tools for nobody.
    EventsClosed,
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EventsClosed => formatter.write_str("nobody is listening for the assistant's events"),
        }
    }
}

impl std::error::Error for AgentError {}

/// One assistant, as the host holds it: a provider, the tool list, the model and the limits.
pub struct Agent {
    provider: Arc<dyn Provider>,
    registry: Arc<ToolRegistry>,
    max_turns: u32,
    model: String,
    system: String,
    max_tokens: u32,
    max_result_bytes: u64,
}

impl Agent {
    pub fn new(provider: Arc<dyn Provider>, registry: Arc<ToolRegistry>) -> Self {
        Self {
            provider,
            registry,
            max_turns: AI_MAX_TURNS_DEFAULT,
            model: AI_MODEL_DEFAULT.to_owned(),
            system: AI_SYSTEM_PROMPT_DEFAULT.to_owned(),
            max_tokens: AI_MAX_OUTPUT_TOKENS_DEFAULT,
            max_result_bytes: AI_MAX_RESULT_BYTES_DEFAULT,
        }
    }

    /// How many turns this run may take, when somebody wants a tighter or looser limit than the
    /// default. The ceiling itself is the host's policy, not the model's.
    pub fn with_max_turns(mut self, max_turns: u32) -> Self {
        self.max_turns = max_turns;
        self
    }

    /// The model every request names. The host reads it from the active provider's configuration;
    /// this crate has no opinion about which model exists.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// The instructions the run starts from. An empty string asks for no system prompt at all,
    /// which is what a provider that has none of its own expects.
    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = system.into();
        self
    }

    /// The output budget one request gets. The provider clamps it to the model's own limit; this is
    /// the number the settings page configured.
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// The most one tool result may contribute to the conversation, in bytes.
    pub fn with_max_result_bytes(mut self, max_result_bytes: u64) -> Self {
        self.max_result_bytes = max_result_bytes;
        self
    }

    pub fn max_turns(&self) -> u32 {
        self.max_turns
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn system(&self) -> &str {
        &self.system
    }

    pub fn max_tokens(&self) -> u32 {
        self.max_tokens
    }

    pub fn max_result_bytes(&self) -> u64 {
        self.max_result_bytes
    }

    pub fn provider(&self) -> &Arc<dyn Provider> {
        &self.provider
    }

    pub fn registry(&self) -> &Arc<ToolRegistry> {
        &self.registry
    }

    /// A fresh id for one run. The window files every event of the run under it.
    pub fn new_run_id() -> String {
        Uuid::new_v4().to_string()
    }

    /// The tools the model is offered under one scope.
    ///
    /// The filter is the registry's own [`scope_allows`], not a second rule: a read-only scope must
    /// not *offer* a write tool at all, so that the model is not asked to call something it will be
    /// refused for. The registry checks again when the call arrives, which is what keeps an old
    /// conversation from reaching a tool the scope no longer offers.
    pub fn tools(&self, scope: McpScope) -> Vec<ToolSpec> {
        self.registry
            .descriptors()
            .into_iter()
            .filter(|descriptor| scope_allows(scope, descriptor.access))
            .map(|descriptor| ToolSpec {
                name: descriptor.name,
                description: descriptor.description,
                input_schema: descriptor.input_schema.as_value().clone(),
            })
            .collect()
    }

    /// Builds the request for one turn.
    fn request(&self, context: &ToolContext, messages: &[ChatMessage]) -> ProviderRequest {
        // A blank system prompt means "none": some local servers reject an empty one, and sending
        // the field with nothing in it would be a request for a provider to refuse.
        let system = if self.system.trim().is_empty() {
            None
        } else {
            Some(self.system.clone())
        };
        ProviderRequest {
            model: self.model.clone(),
            system,
            messages: messages.to_vec(),
            tools: self.tools(context.scope()),
            max_tokens: self.max_tokens,
            temperature: None,
        }
    }

    /// Runs one conversation.
    ///
    /// `history` is the conversation so far, oldest first, and is taken by value: the messages this
    /// run adds come back in [`RunOutcome::history`], and the host is the side that keeps them.
    /// `events` receives everything the run does, and `cancel` is the run's own stop — the host
    /// builds the [`ToolContext`] with the same object when it wants a cancel to reach a tool too.
    ///
    /// Every ending except a closed event sink produces an [`AgentEvent::Done`], and a failure is
    /// announced by an [`AgentEvent::Failed`] before it. A cancellation during a tool call is not
    /// honoured — the tool has its own timeout, and the next check happens before the following
    /// turn — because dropping a call mid-connection would leave the host with a handle it cannot
    /// account for.
    pub async fn run(
        &self,
        context: &ToolContext,
        history: Vec<ChatMessage>,
        events: EventSink,
        cancel: Cancel,
    ) -> Result<RunOutcome, AgentError> {
        let run_id = Self::new_run_id();
        let mut messages = history;
        let mut turns: u32 = 0;
        let mut usage = Usage::default();
        let mut saw_usage = false;

        let stop_reason = loop {
            if cancel.is_cancelled() {
                break RunStopReason::Cancelled;
            }
            if turns >= self.max_turns {
                break RunStopReason::MaxTurns;
            }
            turns += 1;

            // One assistant turn, one message id: the window groups the text, the reasoning and the
            // tool cards of a turn under it, and a later turn must not paint into an earlier one.
            let message_id = format!("{run_id}-{turns}");
            let request = self.request(context, &messages);

            // The connect phase is cancellable too: a request that has not been answered yet is
            // exactly what somebody pressing stop expects to stop.
            let opened = cancel
                .guard(async { Ok::<_, ToolError>(self.provider.stream(request).await) })
                .await;
            let mut stream = match opened {
                Err(_) => break RunStopReason::Cancelled,
                Ok(Err(error)) => {
                    emit_failure(&events, &run_id, &error)?;
                    break RunStopReason::Error;
                }
                Ok(Ok(stream)) => stream,
            };

            let mut slots: BTreeMap<u32, Slot> = BTreeMap::new();
            let mut cancelled = false;
            let mut failure: Option<ProviderError> = None;

            loop {
                let next = cancel.guard(async { Ok::<_, ToolError>(stream.next().await) }).await;
                let item = match next {
                    Err(_) => {
                        cancelled = true;
                        break;
                    }
                    Ok(None) => break,
                    Ok(Some(item)) => item,
                };
                let event = match item {
                    Ok(event) => event,
                    // An error item may follow half an answer. The turn failed: a short answer read
                    // as a complete one would have the model build on a sentence it never finished.
                    Err(error) => {
                        failure = Some(error);
                        break;
                    }
                };

                match event {
                    StreamEvent::TextDelta { index, text } => {
                        emit(
                            &events,
                            AgentEvent::TextDelta {
                                run_id: run_id.clone(),
                                message_id: message_id.clone(),
                                block_index: index,
                                text: text.clone(),
                            },
                        )?;
                        append_text(&mut slots, index, text);
                    }
                    StreamEvent::ThinkingDelta { index, text } => {
                        emit(
                            &events,
                            AgentEvent::ThinkingDelta {
                                run_id: run_id.clone(),
                                message_id: message_id.clone(),
                                block_index: index,
                                text: text.clone(),
                            },
                        )?;
                        append_thinking(&mut slots, index, text);
                    }
                    StreamEvent::ToolUseStart { index, id, name } => {
                        slots.insert(
                            index,
                            Slot::ToolUse {
                                id,
                                name,
                                json: String::new(),
                                input: None,
                            },
                        );
                    }
                    // A fragment is not a document, so it is kept as text and parsed only if the
                    // provider never sends the assembled value. `ToolUseDone` is authoritative when
                    // it arrives.
                    StreamEvent::ToolInputDelta { index, json } => {
                        if let Some(Slot::ToolUse { json: fragments, .. }) = slots.get_mut(&index) {
                            fragments.push_str(&json);
                        }
                    }
                    StreamEvent::ToolUseDone { index, id, name, input } => {
                        slots.insert(
                            index,
                            Slot::ToolUse {
                                id,
                                name,
                                json: String::new(),
                                input: Some(input),
                            },
                        );
                    }
                    StreamEvent::Usage(reported) => {
                        saw_usage = true;
                        usage.input_tokens = usage.input_tokens.saturating_add(reported.input_tokens);
                        usage.output_tokens = usage.output_tokens.saturating_add(reported.output_tokens);
                        usage.cache_read_tokens = match (usage.cache_read_tokens, reported.cache_read_tokens) {
                            (Some(kept), Some(more)) => Some(kept.saturating_add(more)),
                            (kept, more) => kept.or(more),
                        };
                    }
                    // The stop reason is a fact about the answer, not about the loop: `EndTurn` with
                    // tool calls in the blocks is still a turn with tool calls, and the blocks are
                    // what decide whether anything runs.
                    StreamEvent::Stop { .. } => {}
                }
            }

            if cancelled {
                break RunStopReason::Cancelled;
            }
            if let Some(error) = failure {
                emit_failure(&events, &run_id, &error)?;
                break RunStopReason::Error;
            }
            // A person who has just pressed stop does not want a tool to start running. This is the
            // last check before anything acts, and it comes before the turn is kept: a turn whose
            // tool calls never ran is not a turn the next request could carry.
            if cancel.is_cancelled() {
                break RunStopReason::Cancelled;
            }

            let blocks = assemble(slots);
            if blocks.is_empty() {
                break RunStopReason::EndTurn;
            }

            let calls: Vec<PendingCall> = blocks
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::ToolUse { id, name, input } => Some(PendingCall {
                        id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                    }),
                    _ => None,
                })
                .collect();

            messages.push(ChatMessage::assistant(blocks));

            if calls.is_empty() {
                break RunStopReason::EndTurn;
            }

            let mut results = Vec::with_capacity(calls.len());
            for (position, call) in calls.into_iter().enumerate() {
                if position == 0 {
                    emit(
                        &events,
                        AgentEvent::ToolCall {
                            run_id: run_id.clone(),
                            call_id: call.id.clone(),
                            tool: call.name.clone(),
                            input: call.input.clone(),
                            session_id: None,
                        },
                    )?;

                    let started = Instant::now();
                    // The arguments go in as they arrived. The registry refuses a name it does not
                    // have, a tool the scope does not offer and — through the tool's own closed
                    // input — an argument nobody declared. None of that is the loop's to decide.
                    let outcome = self
                        .registry
                        .call(
                            context,
                            ToolCall {
                                name: call.name.clone(),
                                arguments: call.input,
                            },
                        )
                        .await;
                    let duration_ms = millis(started.elapsed());
                    let (is_error, code, raw) = describe(outcome);
                    let (content, truncated) = truncate_to(raw, self.max_result_bytes);

                    emit(
                        &events,
                        AgentEvent::ToolResult {
                            run_id: run_id.clone(),
                            call_id: call.id.clone(),
                            tool: call.name,
                            ok: !is_error,
                            code,
                            preview: preview(&content),
                            truncated,
                            duration_ms,
                        },
                    )?;
                    results.push(ContentBlock::ToolResult {
                        tool_use_id: call.id,
                        content,
                        is_error,
                    });
                } else {
                    emit(
                        &events,
                        AgentEvent::ToolCall {
                            run_id: run_id.clone(),
                            call_id: call.id.clone(),
                            tool: call.name.clone(),
                            input: call.input,
                            session_id: None,
                        },
                    )?;
                    // Nothing ran, so there is nothing to preview: the code is what the window draws
                    // the sentence from, and a duration of zero is the truth.
                    emit(
                        &events,
                        AgentEvent::ToolResult {
                            run_id: run_id.clone(),
                            call_id: call.id.clone(),
                            tool: call.name,
                            ok: false,
                            code: Some(AI_TOOL_SKIPPED_CODE.to_owned()),
                            preview: String::new(),
                            truncated: false,
                            duration_ms: 0,
                        },
                    )?;
                    results.push(ContentBlock::ToolResult {
                        tool_use_id: call.id,
                        content: AI_TOOL_SKIPPED_CONTENT.to_owned(),
                        is_error: true,
                    });
                }
            }

            // One user message carrying every result, because that is the shape a provider expects
            // an assistant turn's tool calls to be answered in, and answering only some of them is
            // a request no provider will take.
            messages.push(ChatMessage {
                role: Role::User,
                blocks: results,
            });
        };

        let usage = saw_usage.then_some(usage);
        emit(
            &events,
            AgentEvent::Done {
                run_id: run_id.clone(),
                stop_reason,
                turns,
                usage,
            },
        )?;

        Ok(RunOutcome {
            run_id,
            stop_reason,
            turns,
            usage,
            history: messages,
        })
    }
}

/// One content block as it is being assembled from streamed fragments.
enum Slot {
    Text(String),
    Thinking(String),
    ToolUse {
        id: String,
        name: String,
        /// The fragments `ToolInputDelta` delivered, joined. Parsed only when the assembled value
        /// never arrives.
        json: String,
        input: Option<Value>,
    },
}

/// One tool call of an assistant turn, ready to run.
struct PendingCall {
    id: String,
    name: String,
    input: Value,
}

fn append_text(slots: &mut BTreeMap<u32, Slot>, index: u32, text: String) {
    match slots.get_mut(&index) {
        Some(Slot::Text(buffer)) => buffer.push_str(&text),
        // A provider that reuses an index for a different kind of block is one this build cannot
        // describe; the later block is what the message keeps, because a partial answer is better
        // than a failed run and there is no way to keep both.
        _ => {
            slots.insert(index, Slot::Text(text));
        }
    }
}

fn append_thinking(slots: &mut BTreeMap<u32, Slot>, index: u32, text: String) {
    match slots.get_mut(&index) {
        Some(Slot::Thinking(buffer)) => buffer.push_str(&text),
        _ => {
            slots.insert(index, Slot::Thinking(text));
        }
    }
}

/// Turns the assembled slots into one assistant message, in the order the provider numbered them.
///
/// Empty text is dropped: a delta of nothing is not a block, and an empty block is one some
/// providers refuse to read back. A tool call whose arguments never arrived is dropped for the same
/// reason — running it with `{}` would be inventing the arguments.
fn assemble(slots: BTreeMap<u32, Slot>) -> Vec<ContentBlock> {
    let mut blocks = Vec::with_capacity(slots.len());
    for slot in slots.into_values() {
        match slot {
            Slot::Text(text) if !text.is_empty() => blocks.push(ContentBlock::Text { text }),
            Slot::Thinking(text) if !text.is_empty() => blocks.push(ContentBlock::Thinking { text }),
            Slot::Text(_) | Slot::Thinking(_) => {}
            Slot::ToolUse { id, name, json, input } => {
                let input = input.or_else(|| serde_json::from_str(&json).ok());
                if let Some(input) = input {
                    blocks.push(ContentBlock::ToolUse { id, name, input });
                }
            }
        }
    }
    blocks
}

/// What one call produced, in the two forms that are needed: for the model and for the window.
fn describe(outcome: ToolOutcome) -> (bool, Option<String>, String) {
    match outcome {
        ToolOutcome::Succeeded { result } => {
            let text = match serde_json::to_string(&result.value) {
                Ok(text) => text,
                Err(error) => format!("the result could not be serialized: {error}"),
            };
            (false, None, text)
        }
        // The error's own rendering — its stable code and its scrubbed sentence — is what the model
        // can act on. A failure is a result the model reads, not a turn that failed.
        ToolOutcome::Failed { error } => (true, Some(error.code().to_owned()), error.to_string()),
    }
}

/// Cuts a result to `limit` bytes, on a character boundary, and says whether it had to.
fn truncate_to(text: String, limit: u64) -> (String, bool) {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    if text.len() <= limit {
        return (text, false);
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut cut = String::with_capacity(end + AI_TRUNCATION_MARKER.len());
    cut.push_str(&text[..end]);
    cut.push_str(AI_TRUNCATION_MARKER);
    (cut, true)
}

/// What the window is given to draw: scrubbed, without control characters, and short.
///
/// A preview is read by a person and may be copied out of the window, so it gets the same treatment
/// an error's detail gets. The control characters go first because a terminal result can be full of
/// escape sequences, and keeping them would spend the budget on text nothing can draw.
fn preview(content: &str) -> String {
    let cleaned: String = content
        .chars()
        .filter(|character| !character.is_control() || *character == '\n' || *character == '\t')
        .collect();
    scrub_bounded(&cleaned, AI_TOOL_PREVIEW_CHARS)
}

/// Announces a provider failure, in the catalog's words rather than a sentence of its own.
fn emit_failure(events: &EventSink, run_id: &str, error: &ProviderError) -> Result<(), AgentError> {
    let code = error.code();
    let message_key = ErrorCode::from_code(code)
        .map(ErrorCode::message_key)
        .unwrap_or(ErrorCode::AiProviderUnavailable.message_key());
    emit(
        events,
        AgentEvent::Failed {
            run_id: run_id.to_owned(),
            code: code.to_owned(),
            message_key: message_key.to_owned(),
            message_params: BTreeMap::new(),
            retryable: error.is_retryable(),
        },
    )
}

/// Sends one event, and stops the run when nobody is left to read it.
///
/// The result is checked rather than discarded: a send that fails means the window is gone, and a
/// loop that kept calling tools for a closed window would be an agent acting unobserved.
fn emit(events: &EventSink, event: AgentEvent) -> Result<(), AgentError> {
    events.send(event).map_err(|_| AgentError::EventsClosed)
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_result_that_fits_is_kept_whole() {
        let (text, truncated) = truncate_to("up 3 days".to_owned(), 32);

        assert_eq!(text, "up 3 days");
        assert!(!truncated);
    }

    #[test]
    fn a_result_over_the_limit_is_cut_at_a_character_boundary_and_says_so() {
        // Three bytes per character: a cut at the byte limit would land inside one.
        let (text, truncated) = truncate_to("数据数据数据".to_owned(), 4);

        assert!(truncated);
        assert_eq!(text, "数".to_owned() + AI_TRUNCATION_MARKER);
    }

    #[test]
    fn a_preview_drops_control_characters_and_scrubs_credentials() {
        let content = "line one\u{7}\u{1b}[0m password=hunter2\nline two";
        let drawn = preview(content);

        assert!(!drawn.contains('\u{7}'), "a bell is not something a card can draw");
        assert!(!drawn.contains('\u{1b}'), "an escape sequence would be drawn as text");
        assert!(!drawn.contains("hunter2"), "a preview may be copied out of the window");
        assert!(drawn.contains("[redacted]"), "{drawn}");
        assert!(drawn.contains("line two"), "a newline is worth keeping: {drawn}");
    }

    #[test]
    fn a_turn_is_assembled_in_the_order_the_provider_numbered_its_blocks() {
        let mut slots = BTreeMap::new();
        append_text(&mut slots, 0, "Checking".to_owned());
        append_text(&mut slots, 0, " the box.".to_owned());
        slots.insert(
            1,
            Slot::ToolUse {
                id: "call-1".to_owned(),
                name: "sshell_exec_run".to_owned(),
                json: "{\"command\":".to_owned() + "\"uptime\"}",
                input: None,
            },
        );

        let blocks = assemble(slots);

        assert_eq!(
            blocks,
            vec![
                ContentBlock::Text {
                    text: "Checking the box.".to_owned()
                },
                ContentBlock::ToolUse {
                    id: "call-1".to_owned(),
                    name: "sshell_exec_run".to_owned(),
                    input: serde_json::json!({ "command": "uptime" }),
                },
            ],
            "fragments of one block join, and a call without its assembled value is parsed from them"
        );
    }

    #[test]
    fn a_tool_call_whose_arguments_never_arrived_is_not_a_call() {
        let mut slots = BTreeMap::new();
        slots.insert(
            0,
            Slot::ToolUse {
                id: "call-1".to_owned(),
                name: "sshell_exec_run".to_owned(),
                json: String::new(),
                input: None,
            },
        );

        assert!(
            assemble(slots).is_empty(),
            "running a call with `{{}}` would be inventing the arguments the model never sent"
        );
    }
}
