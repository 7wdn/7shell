//! OpenAI's Responses API — its own endpoint, its own request shape and its own stream vocabulary.
//!
//! A second OpenAI adapter exists because the Responses endpoint is not Chat Completions wearing a
//! different name: the conversation is a flat list of `input` items rather than `messages`, the system
//! prompt is `instructions`, the budget is `max_output_tokens`, a tool call is an item rather than a
//! field of a delta, and the stream is `response.*` events rather than `choices[].delta`. A gateway
//! configured for one answers the other with a 404 or with a body this build cannot read, so routing
//! one kind to the other adapter would turn a plain configuration mistake into an unreadable answer.
//!
//! Four things this adapter has to absorb:
//!
//! * **Blocks are output items.** The wire numbers output items rather than content blocks: reasoning
//!   usually comes first, then the message, then each function call. That wire index becomes the
//!   block index here, so a tool card keeps the place the provider gave it and a delta can be routed
//!   without guessing.
//! * **Thinking has no signature to send back.** A reasoning item carries an encrypted payload this
//!   build does not keep — the same reason Anthropic's signed thinking blocks are not sent. Reasoning
//!   that *arrives* is still reported, so an interface can fold it away.
//! * **The vocabulary keeps growing.** Web search, file search, code interpreter and the content-part
//!   notifications are events this build does not draw; an unknown event is ignored rather than
//!   failing the turn, because a provider is allowed to say more than this build understands. A
//!   payload that is not JSON at all is still a protocol failure.
//! * **A call without its arguments is dropped, not invented.** Running a tool with `{}` because the
//!   document never arrived would run something the model never asked for.
//!
//! | wire event | what this adapter does |
//! |---|---|
//! | `response.output_item.added` | opens a text, thinking or tool slot at `output_index` |
//! | `response.output_text.delta` | `TextDelta` for that slot |
//! | `response.reasoning_summary_text.delta` | `ThinkingDelta` (raw `reasoning_text` too) |
//! | `response.function_call_arguments.delta` | `ToolInputDelta`, buffered for the close |
//! | `response.function_call_arguments.done` | fills the buffer for a gateway that does not stream |
//! | `response.output_item.done` | a function call's arguments are parsed and reported |
//! | `response.completed` | usage, then `toolUse` when a call was answered, else `endTurn` |
//! | `response.incomplete` | the stop reason the provider gave for cutting the answer |
//! | `response.failed`, `error` | the failure its code (or status) names |
//! | anything else | ignored |
//!
//! The one thing that is not tolerated is the body ending without `response.completed`: half an
//! answer reported as a whole one is worse than a failure the person can retry.

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde_json::{Map, Value, json};
use sevenshell_types::AiProviderKind;

use crate::http::{ApiKey, Limits, build_client, endpoint, post_stream};
use crate::provider::{Provider, ProviderStream};
use crate::sse::SseEvent;
use crate::stream::{SseParser, events, provider_stream};
use crate::types::{ContentBlock, ProviderError, ProviderRequest, Role, StopReason, StreamEvent, Usage};

/// Where the Responses API lives under a base address.
const ENDPOINT_PATH: &str = "/responses";

/// An OpenAI Responses provider: one base address, one model and one key.
///
/// `base_url` is the assembled endpoint, built and checked once when the provider is created.
#[derive(Debug, Clone)]
pub struct OpenAiResponsesProvider {
    base_url: reqwest::Url,
    model: String,
    key: ApiKey,
    client: reqwest::Client,
}

impl OpenAiResponsesProvider {
    /// Builds the provider for one configuration entry.
    ///
    /// The key is required even when the address is a local gateway: unlike `Local`, this kind names
    /// a hosted protocol, and a request without credentials would be refused at the far end with a
    /// status that says less than [`ProviderError::NoKey`] does.
    pub fn new(config: &crate::types::AiProviderConfig, key: Option<&str>) -> Result<Self, ProviderError> {
        let base_url = endpoint(&config.base_url, ENDPOINT_PATH)?;
        let key = ApiKey::from_option(key).ok_or(ProviderError::NoKey)?;
        let client = build_client(&base_url)?;

        Ok(Self {
            base_url,
            model: config.model.clone(),
            key,
            client,
        })
    }

    /// The model this provider was configured with.
    pub fn model(&self) -> &str {
        &self.model
    }

    fn request_body(&self, request: &ProviderRequest) -> Result<Vec<u8>, ProviderError> {
        let mut body = Map::new();
        body.insert("model".to_owned(), json!(self.model_for(request)));
        body.insert("stream".to_owned(), json!(true));
        // Nothing is stored on the provider's side: the conversation belongs to this application,
        // and a stored response would outlive the turn the person asked for.
        body.insert("store".to_owned(), json!(false));
        body.insert("max_output_tokens".to_owned(), json!(request.max_tokens));

        if let Some(system) = request.system.as_deref().map(str::trim).filter(|text| !text.is_empty()) {
            body.insert("instructions".to_owned(), json!(system));
        }

        body.insert("input".to_owned(), Value::Array(input(request)));

        if !request.tools.is_empty() {
            body.insert(
                "tools".to_owned(),
                Value::Array(
                    request
                        .tools
                        .iter()
                        .map(|tool| {
                            // Flat here, unlike Chat Completions: there is no `function` wrapper.
                            json!({
                                "type": "function",
                                "name": tool.name,
                                "description": tool.description,
                                "parameters": tool.input_schema,
                            })
                        })
                        .collect(),
                ),
            );
        }
        // Only when it was set: a temperature of our own choosing would quietly change the model's
        // answers for a person who never asked for it.
        if let Some(temperature) = request.temperature {
            body.insert("temperature".to_owned(), json!(temperature));
        }

        serde_json::to_vec(&Value::Object(body))
            .map_err(|error| ProviderError::Protocol(format!("the request could not be written: {error}")))
    }

    fn model_for<'a>(&'a self, request: &'a ProviderRequest) -> &'a str {
        // The configuration names the model; the request may override it, and an empty name means
        // "the one this provider was configured with".
        if request.model.trim().is_empty() {
            &self.model
        } else {
            &request.model
        }
    }
}

#[async_trait]
impl Provider for OpenAiResponsesProvider {
    fn kind(&self) -> AiProviderKind {
        AiProviderKind::OpenAiResponses
    }

    async fn stream(&self, request: ProviderRequest) -> Result<ProviderStream, ProviderError> {
        let body = self.request_body(&request)?;
        let headers = [("authorization", self.key.bearer_header()?)];

        let response = post_stream(&self.client, &self.base_url, &headers, &body, Limits::default()).await?;

        Ok(provider_stream(
            events(response, Limits::default()),
            ResponsesParser::default(),
        ))
    }
}

/// The conversation as this API's `input` array.
///
/// Three shapes differ from Chat Completions and are worth naming:
///
/// * text is a part of a message item, and the part's name says who wrote it (`input_text` for the
///   person, `output_text` for the model);
/// * a tool call is its own item, with `call_id` — the result answers that id, not the item id;
/// * a tool result is its own item too, so it is not glued to the turn that carries it.
///
/// `is_error` has no field here, and is not invented as one: a provider that validates its input
/// strictly would refuse the whole request. The error text is in the result's output, which is what
/// the model reads either way.
fn input(request: &ProviderRequest) -> Vec<Value> {
    let mut out = Vec::new();

    for message in &request.messages {
        match message.role {
            Role::User => {
                let mut parts: Vec<Value> = Vec::new();
                for block in &message.blocks {
                    match block {
                        // An empty text part is refused by the API, and a part that says nothing is
                        // a part nobody wrote.
                        ContentBlock::Text { text } if !text.trim().is_empty() => {
                            parts.push(json!({ "type": "input_text", "text": text }));
                        }
                        ContentBlock::Text { .. } => {}
                        ContentBlock::ToolResult {
                            tool_use_id, content, ..
                        } => {
                            // A result answers the call directly before it, so anything said before
                            // it has to go first.
                            flush_parts(&mut out, "user", &mut parts);
                            out.push(json!({
                                "type": "function_call_output",
                                "call_id": tool_use_id,
                                "output": content,
                            }));
                        }
                        // A user turn cannot ask for a tool, and thinking is not sent back.
                        ContentBlock::ToolUse { .. } | ContentBlock::Thinking { .. } => {}
                    }
                }
                flush_parts(&mut out, "user", &mut parts);
            }
            Role::Assistant => {
                let mut parts: Vec<Value> = Vec::new();
                let mut calls: Vec<Value> = Vec::new();
                for block in &message.blocks {
                    match block {
                        ContentBlock::Text { text } if !text.trim().is_empty() => {
                            parts.push(json!({ "type": "output_text", "text": text }));
                        }
                        ContentBlock::Text { .. } => {}
                        ContentBlock::ToolUse { id, name, input } => calls.push(json!({
                            "type": "function_call",
                            "call_id": id,
                            "name": name,
                            // The arguments are a JSON *string* in this API, not an object.
                            "arguments": serde_json::to_string(input).unwrap_or_else(|_| "{}".to_owned()),
                        })),
                        // See the module note: reasoning items carry a payload this build does not
                        // keep, and inventing one would be refused by the provider.
                        ContentBlock::Thinking { .. } | ContentBlock::ToolResult { .. } => {}
                    }
                }
                flush_parts(&mut out, "assistant", &mut parts);
                out.extend(calls);
            }
        }
    }

    out
}

fn flush_parts(out: &mut Vec<Value>, role: &str, parts: &mut Vec<Value>) {
    if parts.is_empty() {
        return;
    }
    out.push(json!({ "role": role, "content": std::mem::take(parts) }));
}

/// What one output item is, so a delta can be routed and a close can be told apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Text,
    Thinking,
    Tool,
}

/// One function call being assembled from its fragments.
#[derive(Debug, Default)]
struct PendingTool {
    id: String,
    name: String,
    fragments: String,
    started: bool,
}

/// This adapter's reading of the stream.
#[derive(Debug, Default)]
struct ResponsesParser {
    slots: BTreeMap<u32, Slot>,
    tools: BTreeMap<u32, PendingTool>,
    answered_a_call: bool,
    finished: bool,
}

impl ResponsesParser {
    fn item_added(&mut self, value: &Value) -> Result<Vec<StreamEvent>, ProviderError> {
        let index = output_index(value)?;
        let item = value
            .get("item")
            .ok_or_else(|| ProviderError::Protocol("an output item started without one".to_owned()))?;

        match item.get("type").and_then(Value::as_str).unwrap_or_default() {
            "message" => {
                self.slots.insert(index, Slot::Text);
                Ok(Vec::new())
            }
            "reasoning" => {
                self.slots.insert(index, Slot::Thinking);
                Ok(Vec::new())
            }
            "function_call" => {
                let id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .or_else(|| item.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()))
                    .map_or_else(|| synthesised_id(index), str::to_owned);
                let name = item.get("name").and_then(Value::as_str).unwrap_or_default().to_owned();

                self.slots.insert(index, Slot::Tool);
                // A tool card is drawn with its name. A gateway that names the call later gets the
                // start then, when the call closes, instead of a card that says nothing now.
                let started = !name.is_empty();
                self.tools.insert(
                    index,
                    PendingTool {
                        id: id.clone(),
                        name: name.clone(),
                        fragments: String::new(),
                        started,
                    },
                );

                if started {
                    Ok(vec![StreamEvent::ToolUseStart { index, id, name }])
                } else {
                    Ok(Vec::new())
                }
            }
            // `web_search_call`, `file_search_call`, `code_interpreter_call` and whatever comes next:
            // this build draws none of them, and refusing them would break a provider that merely
            // says more than it was asked.
            _ => Ok(Vec::new()),
        }
    }

    /// A text or thinking fragment. `reasoning` picks the slot and the event.
    fn text_delta(&mut self, value: &Value, reasoning: bool) -> Result<Vec<StreamEvent>, ProviderError> {
        let index = output_index(value)?;
        // Some gateways skip `output_item.added` for an item they are about to stream; the delta
        // itself is enough to know what the slot is.
        self.slots
            .entry(index)
            .or_insert(if reasoning { Slot::Thinking } else { Slot::Text });

        let delta = value.get("delta").and_then(Value::as_str).unwrap_or_default();
        if delta.is_empty() {
            return Ok(Vec::new());
        }

        Ok(vec![if reasoning {
            StreamEvent::ThinkingDelta {
                index,
                text: delta.to_owned(),
            }
        } else {
            StreamEvent::TextDelta {
                index,
                text: delta.to_owned(),
            }
        }])
    }

    fn tool_delta(&mut self, value: &Value) -> Result<Vec<StreamEvent>, ProviderError> {
        let index = output_index(value)?;
        let delta = value
            .get("delta")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();

        self.slots.entry(index).or_insert(Slot::Tool);
        self.tools.entry(index).or_default().fragments.push_str(&delta);

        if delta.is_empty() {
            return Ok(Vec::new());
        }
        Ok(vec![StreamEvent::ToolInputDelta { index, json: delta }])
    }

    /// `response.function_call_arguments.done`.
    ///
    /// It repeats a document the fragments already carried, so it usually says nothing new. It is
    /// read anyway because a gateway that does not stream fragments names the whole document only
    /// here, and a call with no document at all is a call that gets dropped.
    fn tool_arguments(&mut self, value: &Value) -> Result<Vec<StreamEvent>, ProviderError> {
        let index = output_index(value)?;
        let missing = self.tools.get(&index).is_none_or(|tool| tool.fragments.is_empty());
        if missing && let Some(arguments) = value.get("arguments").and_then(Value::as_str) {
            self.tools.entry(index).or_default().fragments = arguments.to_owned();
        }
        Ok(Vec::new())
    }

    fn item_done(&mut self, value: &Value) -> Result<Vec<StreamEvent>, ProviderError> {
        let index = output_index(value)?;
        let item = value.get("item");
        let kind = item.and_then(|item| item.get("type")).and_then(Value::as_str);

        if kind == Some("function_call") || self.slots.get(&index) == Some(&Slot::Tool) {
            // A gateway that closes a call it never announced still names it and its arguments
            // here; the entry the deltas would have created is created now instead.
            self.tools.entry(index).or_default();
            return self.close_tool(index, item);
        }

        // A message or a reasoning item closing: the deltas were the content.
        self.slots.remove(&index);
        Ok(Vec::new())
    }

    fn close_tool(&mut self, index: u32, item: Option<&Value>) -> Result<Vec<StreamEvent>, ProviderError> {
        self.slots.remove(&index);
        let Some(mut tool) = self.tools.remove(&index) else {
            return Ok(Vec::new());
        };

        if let Some(item) = item {
            if tool.name.is_empty() {
                tool.name = item.get("name").and_then(Value::as_str).unwrap_or_default().to_owned();
            }
            if tool.id.is_empty()
                && let Some(call_id) = item.get("call_id").and_then(Value::as_str).filter(|id| !id.is_empty())
            {
                tool.id = call_id.to_owned();
            }
        }

        // The `done` item repeats the whole document; the fragments are what is held when the close
        // arrives without it. Neither present means the arguments never arrived at all.
        let arguments = item
            .and_then(|item| item.get("arguments"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| (!tool.fragments.is_empty()).then(|| tool.fragments.clone()));

        let Some(arguments) = arguments else {
            // The call is dropped rather than run with invented arguments: the model asked for
            // something specific, and this build does not know what it was.
            return Ok(Vec::new());
        };

        if tool.name.is_empty() {
            return Err(ProviderError::Protocol(
                "a function call arrived without a tool name".to_owned(),
            ));
        }

        // A call with no parameters produces an empty document, not a parse failure.
        let input = if arguments.trim().is_empty() {
            Value::Object(Map::new())
        } else {
            serde_json::from_str(&arguments)
                .map_err(|_| ProviderError::Protocol("the model's tool arguments were not valid JSON".to_owned()))?
        };

        let mut events = Vec::new();
        if !tool.started {
            // Held back at `added` because the call had no name yet.
            events.push(StreamEvent::ToolUseStart {
                index,
                id: tool.id.clone(),
                name: tool.name.clone(),
            });
        }
        events.push(StreamEvent::ToolUseDone {
            index,
            id: tool.id,
            name: tool.name,
            input,
        });
        self.answered_a_call = true;
        Ok(events)
    }

    fn completed(&mut self, value: &Value) -> Result<Vec<StreamEvent>, ProviderError> {
        let mut events = Vec::new();

        // A gateway may finish the response without closing every call; the fragments that did
        // arrive are still a document this build can assemble.
        let open: Vec<u32> = self.tools.keys().copied().collect();
        for index in open {
            events.extend(self.close_tool(index, None)?);
        }

        if let Some(usage) = value.pointer("/response/usage") {
            events.push(StreamEvent::Usage(usage_of(usage)));
        }

        self.finished = true;
        events.push(StreamEvent::Stop {
            reason: if self.answered_a_call {
                StopReason::ToolUse
            } else {
                StopReason::EndTurn
            },
        });
        Ok(events)
    }

    fn incomplete(&mut self, value: &Value) -> Result<Vec<StreamEvent>, ProviderError> {
        let reason = value
            .pointer("/response/incomplete_details/reason")
            .and_then(Value::as_str)
            .ok_or_else(|| ProviderError::Protocol("the response stopped early without saying why".to_owned()))?;
        let reason = incomplete_reason(reason)?;

        self.finished = true;
        // A call cut off by the output budget has no complete argument document; its fragments are
        // dropped rather than run as half an object, and reported as what they are: a truncated turn.
        self.tools.clear();
        Ok(vec![StreamEvent::Stop { reason }])
    }

    fn failed(&mut self, value: &Value) -> ProviderError {
        self.finished = true;
        let error = value
            .get("response")
            .and_then(|response| response.get("error"))
            .or_else(|| value.get("error"));

        match error {
            Some(error) => classify_failure(error),
            // A failure with no error object is still a failure, and the provider's own side is
            // where it happened even when it did not say what.
            None => ProviderError::Server {
                status: 500,
                detail: "the provider failed the response without saying why".to_owned(),
            },
        }
    }
}

impl SseParser for ResponsesParser {
    fn on_event(&mut self, event: &SseEvent) -> Result<Vec<StreamEvent>, ProviderError> {
        // Anything after the close is a server talking to itself; the turn is over.
        if self.finished {
            return Ok(Vec::new());
        }

        let data = event.data.trim();
        if data.is_empty() {
            // A keep-alive with no payload. Nothing happened.
            return Ok(Vec::new());
        }
        // The sentinel some gateways append after the last event. It says nothing
        // `response.completed` has not already said, and a body that ends right after it is judged by
        // `on_end` exactly like any other end.
        if data == "[DONE]" {
            return Ok(Vec::new());
        }

        let value: Value = serde_json::from_str(data)
            .map_err(|_| ProviderError::Protocol("the provider sent a stream event that is not JSON".to_owned()))?;

        // The `type` field is authoritative; the `event:` line carries the same word and is used only
        // when a gateway has dropped the field.
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .or(event.event.as_deref())
            .unwrap_or_default();

        match kind {
            "response.output_item.added" => self.item_added(&value),
            "response.output_item.done" => self.item_done(&value),
            "response.output_text.delta" => self.text_delta(&value, false),
            // A summary is what this API streams for a reasoning item; some models stream the raw
            // reasoning text instead. Both are thinking, and neither is part of the answer.
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => self.text_delta(&value, true),
            "response.function_call_arguments.delta" => self.tool_delta(&value),
            "response.function_call_arguments.done" => self.tool_arguments(&value),
            "response.completed" => self.completed(&value),
            "response.incomplete" => self.incomplete(&value),
            "response.failed" => Err(self.failed(&value)),
            "error" => {
                let error = value.get("error").unwrap_or(&value);
                Err(classify_failure(error))
            }
            // `response.created`, `response.in_progress`, the content-part and output-text `done`
            // events, and whatever a later version adds: they announce or repeat what the deltas
            // already said. The vocabulary grows (web search, file search, code interpreter), so an
            // unknown event is not a failure — a payload that is not JSON is, and that was refused
            // above.
            _ => Ok(Vec::new()),
        }
    }

    fn on_end(&mut self) -> Result<Vec<StreamEvent>, ProviderError> {
        if self.finished {
            return Ok(Vec::new());
        }

        // This API closes a response with `response.completed` (or `incomplete`/`failed`). A body
        // that ends without one was cut off, and reporting that as a finished answer would present
        // half a turn as a whole one.
        Err(ProviderError::Protocol(
            "the stream ended before the provider finished the response".to_owned(),
        ))
    }
}

fn output_index(value: &Value) -> Result<u32, ProviderError> {
    count(value.get("output_index"))
        .ok_or_else(|| ProviderError::Protocol("an output item event arrived without an output index".to_owned()))
}

fn synthesised_id(index: u32) -> String {
    // A gateway that never names its calls still gets ids a result can answer, stable for the
    // conversation because they are derived from the call's position.
    format!("call_{index}")
}

fn usage_of(usage: &Value) -> Usage {
    Usage {
        input_tokens: count(usage.get("input_tokens")).unwrap_or_default(),
        output_tokens: count(usage.get("output_tokens")).unwrap_or_default(),
        cache_read_tokens: count(usage.pointer("/input_tokens_details/cached_tokens")),
    }
}

fn count(value: Option<&Value>) -> Option<u32> {
    value
        .and_then(Value::as_u64)
        .map(|number| u32::try_from(number).unwrap_or(u32::MAX))
}

fn incomplete_reason(reason: &str) -> Result<StopReason, ProviderError> {
    match reason {
        "max_output_tokens" => Ok(StopReason::MaxTokens),
        "stop_sequence" => Ok(StopReason::StopSequence),
        "content_filter" | "refusal" => Ok(StopReason::Refusal),
        other => Err(ProviderError::Protocol(format!(
            "the response stopped early for a reason this build does not know: {other}"
        ))),
    }
}

/// Turns the `error` object of a failed response into the failure the loop acts on.
///
/// The codes are this API's own (`invalid_api_key`, `rate_limit_exceeded`, …) rather than the
/// Anthropic-style `type` the shared classifier is built around. A code this build does not know is
/// placed by the status the object named, and failing that by the shared classifier, which knows how
/// to recognise "the conversation is too long" from the words of the message.
fn classify_failure(error: &Value) -> ProviderError {
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .or_else(|| error.get("type").and_then(Value::as_str))
        .unwrap_or_default();

    match code {
        "invalid_api_key" | "authentication_error" | "permission_denied" | "insufficient_permissions" => {
            ProviderError::Auth
        }
        "rate_limit_exceeded" | "rate_limit_error" | "insufficient_quota" => {
            ProviderError::RateLimited { retry_after_ms: None }
        }
        "context_length_exceeded" | "request_too_large" => ProviderError::ContextTooLong,
        "server_error" | "internal_error" | "api_error" => crate::http::classify_status(500, &error.to_string(), None),
        _ => match status_of(error) {
            Some(status) => crate::http::classify_status(status, &error.to_string(), None),
            None => crate::http::classify_error_object(error),
        },
    }
}

fn status_of(error: &Value) -> Option<u16> {
    let status = count(error.get("status"))?;
    u16::try_from(status).ok().filter(|status| (100..600).contains(status))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AiProviderConfig, ChatMessage, ToolSpec};

    fn config(base_url: &str) -> AiProviderConfig {
        AiProviderConfig::new(
            "responses",
            "OpenAI Responses",
            AiProviderKind::OpenAiResponses,
            base_url,
            "gpt-4.1",
        )
    }

    fn provider() -> OpenAiResponsesProvider {
        OpenAiResponsesProvider::new(&config("https://api.openai.com/v1"), Some("sk-live-1")).expect("a provider")
    }

    fn parse(parser: &mut ResponsesParser, json: &str) -> Result<Vec<StreamEvent>, ProviderError> {
        parser.on_event(&SseEvent {
            event: None,
            data: json.to_owned(),
        })
    }

    #[test]
    fn a_responses_provider_without_a_key_is_refused_before_anything_is_sent() {
        assert_eq!(
            OpenAiResponsesProvider::new(&config("https://api.openai.com/v1"), None)
                .expect_err("this protocol has no anonymous mode"),
            ProviderError::NoKey
        );
        assert_eq!(
            OpenAiResponsesProvider::new(&config("https://api.openai.com/v1"), Some("  "))
                .expect_err("whitespace is not a key"),
            ProviderError::NoKey
        );
    }

    #[test]
    fn a_provider_debug_line_does_not_contain_the_key() {
        let printed = format!("{:?}", provider());

        assert!(
            !printed.contains("sk-live-1"),
            "a provider formatted into a log must not carry its key: {printed}"
        );
        assert!(printed.contains("gpt-4.1"), "the model is not a secret: {printed}");
    }

    #[test]
    fn the_request_body_flattens_the_conversation_into_input_items() {
        let request = ProviderRequest {
            model: String::new(),
            system: Some("You are an operations assistant.".to_owned()),
            messages: vec![
                ChatMessage::user("how full is the disk?"),
                ChatMessage::assistant(vec![
                    ContentBlock::Text {
                        text: "Let me look.".to_owned(),
                    },
                    ContentBlock::ToolUse {
                        id: "call_1".to_owned(),
                        name: "sshell_exec_run".to_owned(),
                        input: json!({ "command": "df -h" }),
                    },
                ]),
                ChatMessage::tool_result("call_1", "43% used", false),
            ],
            tools: vec![ToolSpec {
                name: "sshell_exec_run".to_owned(),
                description: "Runs a command".to_owned(),
                input_schema: json!({ "type": "object", "properties": { "command": { "type": "string" } } }),
            }],
            max_tokens: 1_024,
            temperature: Some(0.2),
        };

        let body: Value = serde_json::from_slice(&provider().request_body(&request).expect("a body")).expect("JSON");

        assert_eq!(
            body["model"],
            json!("gpt-4.1"),
            "an empty model means the configured one"
        );
        assert_eq!(body["instructions"], json!("You are an operations assistant."));
        assert_eq!(body["max_output_tokens"], json!(1_024));
        assert_eq!(body["stream"], json!(true));
        assert_eq!(
            body["store"],
            json!(false),
            "a stored response would outlive the turn on the provider's side"
        );

        assert_eq!(
            body["input"][0],
            json!({ "role": "user", "content": [{ "type": "input_text", "text": "how full is the disk?" }] }),
            "a user message is an item whose parts are what the person wrote"
        );
        assert_eq!(body["input"][1]["role"], json!("assistant"));
        assert_eq!(
            body["input"][1]["content"][0],
            json!({ "type": "output_text", "text": "Let me look." }),
            "what the model wrote is an output part"
        );
        assert_eq!(body["input"][2]["type"], json!("function_call"));
        assert_eq!(body["input"][2]["call_id"], json!("call_1"));
        assert_eq!(
            body["input"][2]["arguments"],
            json!("{\"command\":\"df -h\"}"),
            "the arguments are a JSON string in this API"
        );
        assert_eq!(
            body["input"][3],
            json!({ "type": "function_call_output", "call_id": "call_1", "output": "43% used" }),
            "a tool result is its own item here"
        );
        assert!(
            body["input"][3].get("is_error").is_none(),
            "this API has no such field; the text carries the error"
        );

        assert_eq!(body["tools"][0]["type"], json!("function"));
        assert_eq!(body["tools"][0]["name"], json!("sshell_exec_run"));
        assert_eq!(
            body["tools"][0]["parameters"]["properties"]["command"]["type"],
            json!("string")
        );
        assert!(
            body["tools"][0].get("function").is_none(),
            "a Responses tool is flat, unlike a Chat Completions one"
        );

        let temperature = body["temperature"].as_f64().expect("a number");
        assert!((temperature - 0.2).abs() < 1e-6, "the temperature was {temperature}");
    }

    #[test]
    fn thinking_is_never_sent_back_and_an_empty_turn_is_dropped() {
        let request = ProviderRequest {
            model: String::new(),
            system: Some("   ".to_owned()),
            messages: vec![
                ChatMessage::assistant(vec![ContentBlock::Thinking {
                    text: "the person probably means df".to_owned(),
                }]),
                ChatMessage::user("   "),
                ChatMessage::user("df?"),
            ],
            tools: Vec::new(),
            max_tokens: 100,
            temperature: None,
        };

        let body: Value = serde_json::from_slice(&provider().request_body(&request).expect("a body")).expect("JSON");
        let input = body["input"].as_array().expect("an array");

        assert_eq!(
            input.len(),
            1,
            "a thinking-only turn and a blank turn have nothing to say: {body}"
        );
        assert_eq!(input[0]["content"][0]["text"], json!("df?"));
        assert!(
            body.get("instructions").is_none(),
            "a blank system prompt is not written"
        );
        assert!(
            body.get("tools").is_none(),
            "no tool list is sent when none was offered"
        );
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn a_reasoning_item_feeds_the_thinking_slot_and_a_message_feeds_text() {
        let mut parser = ResponsesParser::default();

        parse(
            &mut parser,
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"rs_1","type":"reasoning","summary":[]}}"#,
        )
        .expect("a reasoning item");
        parse(
            &mut parser,
            r#"{"type":"response.output_item.added","output_index":1,"item":{"id":"msg_1","type":"message","role":"assistant","content":[]}}"#,
        )
        .expect("a message item");

        assert_eq!(
            parse(
                &mut parser,
                r#"{"type":"response.reasoning_summary_text.delta","item_id":"rs_1","output_index":0,"delta":"the disk"}"#,
            )
            .expect("a thought"),
            vec![StreamEvent::ThinkingDelta {
                index: 0,
                text: "the disk".to_owned(),
            }],
            "reasoning is thinking, not part of the answer"
        );
        assert_eq!(
            parse(
                &mut parser,
                r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":1,"delta":"43%"}"#,
            )
            .expect("text"),
            vec![StreamEvent::TextDelta {
                index: 1,
                text: "43%".to_owned(),
            }],
            "the wire's output index is the block index"
        );
    }

    #[test]
    fn a_closed_response_reports_its_usage_and_the_reason_it_stopped_once() {
        let mut parser = ResponsesParser::default();
        parse(
            &mut parser,
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"fc_1","type":"function_call","call_id":"call_1","name":"sshell_exec_run"}}"#,
        )
        .expect("a call");
        parse(
            &mut parser,
            r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":0,"delta":"{\"command\":\"df -h\"}"}"#,
        )
        .expect("a fragment");

        let done = parse(
            &mut parser,
            r#"{"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":11,"output_tokens":4,"input_tokens_details":{"cached_tokens":3}}}}"#,
        )
        .expect("a close");
        assert_eq!(
            done,
            vec![
                StreamEvent::ToolUseDone {
                    index: 0,
                    id: "call_1".to_owned(),
                    name: "sshell_exec_run".to_owned(),
                    input: json!({ "command": "df -h" }),
                },
                StreamEvent::Usage(Usage {
                    input_tokens: 11,
                    output_tokens: 4,
                    cache_read_tokens: Some(3),
                }),
                StreamEvent::Stop {
                    reason: StopReason::ToolUse
                },
            ],
            "a call the provider never closed is closed here; answering one is what makes this a tool turn"
        );

        assert!(
            parse(
                &mut parser,
                r#"{"type":"response.completed","response":{"status":"completed"}}"#,
            )
            .expect("a repeated close")
            .is_empty(),
            "a second stop would make the loop think the turn ended twice"
        );
    }

    #[test]
    fn a_call_that_was_never_announced_is_still_closed_from_its_done_item() {
        // A gateway may skip `output_item.added` and name the call only at its close. Losing a call
        // the model asked for would leave the answer ending as if no tool were wanted.
        let mut parser = ResponsesParser::default();

        let done = parse(
            &mut parser,
            r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"fc_1","type":"function_call","status":"completed","call_id":"call_7","name":"sshell_sessions_list","arguments":"{}"}}"#,
        )
        .expect("a close");

        assert_eq!(
            done,
            vec![
                StreamEvent::ToolUseStart {
                    index: 0,
                    id: "call_7".to_owned(),
                    name: "sshell_sessions_list".to_owned(),
                },
                StreamEvent::ToolUseDone {
                    index: 0,
                    id: "call_7".to_owned(),
                    name: "sshell_sessions_list".to_owned(),
                    input: json!({}),
                },
            ]
        );
    }

    #[test]
    fn every_documented_incomplete_reason_is_mapped_and_an_unknown_one_is_not_guessed() {
        for (wire, expected) in [
            ("max_output_tokens", StopReason::MaxTokens),
            ("stop_sequence", StopReason::StopSequence),
            ("content_filter", StopReason::Refusal),
        ] {
            assert_eq!(incomplete_reason(wire).expect(wire), expected);
        }

        assert!(matches!(
            incomplete_reason("brand_new_reason"),
            Err(ProviderError::Protocol(_))
        ));
    }

    #[test]
    fn a_failure_is_placed_by_the_code_or_the_status_it_names() {
        let detail = json!({ "message": "something went wrong" });
        let with_code = |code: &str| {
            let mut error = detail.clone();
            error["code"] = json!(code);
            classify_failure(&error)
        };

        assert_eq!(with_code("invalid_api_key"), ProviderError::Auth);
        assert_eq!(
            with_code("rate_limit_exceeded"),
            ProviderError::RateLimited { retry_after_ms: None }
        );
        assert_eq!(with_code("context_length_exceeded"), ProviderError::ContextTooLong);
        assert!(
            matches!(with_code("server_error"), ProviderError::Server { status: 500, .. }),
            "a server error is the provider's own side failing"
        );

        // A code this build does not know is placed by the status it named.
        let by_status = classify_failure(&json!({ "status": 429, "message": "slow down" }));
        assert_eq!(by_status, ProviderError::RateLimited { retry_after_ms: None });

        let unknown = classify_failure(&json!({ "code": "brand_new_error", "message": "?" }));
        assert!(
            matches!(unknown, ProviderError::Protocol(_)),
            "an error this build cannot place must not be guessed at: {unknown:?}"
        );
    }
}
