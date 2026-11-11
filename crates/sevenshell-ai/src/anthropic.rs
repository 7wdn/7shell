//! Anthropic's Messages API.
//!
//! The request is assembled here rather than written as a fixed JSON string: `system` is its own
//! field, the conversation is `content` blocks, and `max_tokens` is required by this API on every
//! call. The stream is Anthropic's own event vocabulary, read one event at a time:
//!
//! | wire event | what this adapter does |
//! |---|---|
//! | `message_start` | records the prompt's usage; the message id is not needed here |
//! | `content_block_start` | announces a tool call; text and thinking blocks start empty |
//! | `content_block_delta` | text, thinking or a fragment of a tool call's JSON |
//! | `content_block_stop` | parses the assembled tool arguments and reports `ToolUseDone` |
//! | `message_delta` | the stop reason and the answer's usage, reported once and complete |
//! | `message_stop` | the end of the answer |
//! | `error` | a failure the provider reported mid-stream |
//!
//! Two choices are deliberate:
//!
//! * **Thinking is not requested.** Sending `thinking` back to this API requires the signature that
//!   arrived with it, and this build does not keep block signatures. A conversation that contains
//!   thinking blocks therefore sends their text nowhere — dropping it is the only honest option,
//!   because inventing a signature would be rejected by the provider. Thinking *from* a stream is
//!   still reported, so an interface can fold it away.
//! * **Usage is reported once.** `message_start` carries the prompt, `message_delta` the answer; the
//!   loop should not have to add two halves together, so this adapter accumulates and emits a single
//!   `Usage` at the end, with whatever the provider actually said.

use std::collections::BTreeMap;

use async_trait::async_trait;
use reqwest::header::HeaderValue;
use serde_json::{Map, Value, json};
use sevenshell_types::AiProviderKind;

use crate::http::{ApiKey, Limits, build_client, endpoint, post_stream};
use crate::provider::{Provider, ProviderStream};
use crate::sse::SseEvent;
use crate::stream::{SseParser, events, provider_stream};
use crate::types::{ContentBlock, ProviderError, ProviderRequest, Role, StopReason, StreamEvent, Usage};

/// Where the Messages API lives under a base address.
const ENDPOINT_PATH: &str = "/v1/messages";

/// The API version this adapter writes.
///
/// Anthropic requires it on every request and dates it rather than numbering it; this is the version
/// the event vocabulary above belongs to.
const API_VERSION: &str = "2023-06-01";

/// An Anthropic provider: one base address, one model and one key.
///
/// `base_url` is the assembled endpoint, not the configured prefix: it is built and checked once when
/// the provider is created, so a mistyped address fails in the settings page rather than halfway
/// through a conversation.
#[derive(Debug, Clone)]
pub struct AnthropicProvider {
    base_url: reqwest::Url,
    model: String,
    key: ApiKey,
    client: reqwest::Client,
}

impl AnthropicProvider {
    /// Builds the provider for one configuration entry.
    ///
    /// The key is required: this API has no unauthenticated mode, so a configuration without one is
    /// reported as [`ProviderError::NoKey`] here rather than as a 401 later.
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

    /// Whether this address is one the Anthropic protocol is spoken at.
    ///
    /// The contract allows `http` only for a loopback address, so a local gateway that forwards the
    /// same protocol is still a supported configuration; a plain `http` elsewhere is not.
    pub fn accepts_base_url(base_url: &str) -> bool {
        crate::provider::is_allowed_base_url(base_url)
    }

    /// The model this provider was configured with.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// The request body for one turn.
    fn request_body(&self, request: &ProviderRequest) -> Result<Vec<u8>, ProviderError> {
        let mut body = Map::new();
        body.insert("model".to_owned(), json!(self.model_for(request)));
        body.insert("max_tokens".to_owned(), json!(request.max_tokens));
        body.insert("stream".to_owned(), json!(true));

        if let Some(system) = request.system.as_deref().map(str::trim).filter(|text| !text.is_empty()) {
            body.insert("system".to_owned(), json!(system));
        }
        body.insert("messages".to_owned(), Value::Array(messages(request)));
        if !request.tools.is_empty() {
            body.insert(
                "tools".to_owned(),
                Value::Array(
                    request
                        .tools
                        .iter()
                        .map(|tool| {
                            json!({
                                "name": tool.name,
                                "description": tool.description,
                                "input_schema": tool.input_schema,
                            })
                        })
                        .collect(),
                ),
            );
        }
        // Only when it was set: sending a temperature of our own choosing would quietly change the
        // model's answers for a person who never asked for it.
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
impl Provider for AnthropicProvider {
    fn kind(&self) -> AiProviderKind {
        AiProviderKind::Anthropic
    }

    async fn stream(&self, request: ProviderRequest) -> Result<ProviderStream, ProviderError> {
        let body = self.request_body(&request)?;
        let headers = [
            ("x-api-key", self.key.header_value()?),
            ("anthropic-version", HeaderValue::from_static(API_VERSION)),
        ];

        let response = post_stream(&self.client, &self.base_url, &headers, &body, Limits::default()).await?;

        Ok(provider_stream(
            events(response, Limits::default()),
            AnthropicParser::default(),
        ))
    }
}

/// The conversation as this API's `messages` array.
fn messages(request: &ProviderRequest) -> Vec<Value> {
    request
        .messages
        .iter()
        .filter_map(|message| {
            let blocks: Vec<Value> = message
                .blocks
                .iter()
                .filter_map(|block| match block {
                    // An empty text block is refused by the API, and a turn that ended up with one is
                    // a turn with nothing in it.
                    ContentBlock::Text { text } if text.trim().is_empty() => None,
                    ContentBlock::Text { text } => Some(json!({ "type": "text", "text": text })),
                    // See the module note: a thinking block without the signature that came with it
                    // cannot be sent back, and this build never asks for thinking in the first place.
                    ContentBlock::Thinking { .. } => None,
                    ContentBlock::ToolUse { id, name, input } => {
                        Some(json!({ "type": "tool_use", "id": id, "name": name, "input": input }))
                    }
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => Some(json!({
                        "type": "tool_result",
                        "tool_use_id": tool_use_id,
                        "content": content,
                        "is_error": is_error,
                    })),
                })
                .collect();

            if blocks.is_empty() {
                return None;
            }

            let role = match message.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            Some(json!({ "role": role, "content": blocks }))
        })
        .collect()
}

/// One tool call being assembled from its fragments.
#[derive(Debug, Default)]
struct PendingTool {
    id: String,
    name: String,
    json: String,
}

/// This adapter's reading of the stream.
#[derive(Debug, Default)]
struct AnthropicParser {
    tools: BTreeMap<u32, PendingTool>,
    usage: Usage,
    usage_seen: bool,
    usage_sent: bool,
    stop_reason: Option<StopReason>,
    finished: bool,
}

impl AnthropicParser {
    fn start_block(&mut self, value: &Value) -> Result<Vec<StreamEvent>, ProviderError> {
        let index = block_index(value)?;
        let block = value
            .get("content_block")
            .ok_or_else(|| ProviderError::Protocol("a content block started without one".to_owned()))?;

        match block.get("type").and_then(Value::as_str).unwrap_or_default() {
            "tool_use" => {
                let id = text_field(block, "id");
                let name = text_field(block, "name");
                // The arguments arrive as fragments; the `input` here is always the empty object.
                self.tools.insert(
                    index,
                    PendingTool {
                        id: id.clone(),
                        name: name.clone(),
                        json: String::new(),
                    },
                );
                Ok(vec![StreamEvent::ToolUseStart { index, id, name }])
            }
            "text" => {
                // A text block starts empty, so this is normally nothing; a server that opens it with
                // content is still honoured.
                let text = text_field(block, "text");
                if text.is_empty() {
                    Ok(Vec::new())
                } else {
                    Ok(vec![StreamEvent::TextDelta { index, text }])
                }
            }
            "thinking" => {
                let text = text_field(block, "thinking");
                if text.is_empty() {
                    Ok(Vec::new())
                } else {
                    Ok(vec![StreamEvent::ThinkingDelta { index, text }])
                }
            }
            // `redacted_thinking` and whatever a later version adds are blocks this build does not
            // draw. Refusing them would make a working provider look broken; ignoring them is what
            // "the interface does not show this" has to mean.
            _ => Ok(Vec::new()),
        }
    }

    fn delta(&mut self, value: &Value) -> Result<Vec<StreamEvent>, ProviderError> {
        let index = block_index(value)?;
        let delta = value
            .get("delta")
            .ok_or_else(|| ProviderError::Protocol("a content block changed without a delta".to_owned()))?;

        match delta.get("type").and_then(Value::as_str).unwrap_or_default() {
            "text_delta" => {
                let text = text_field(delta, "text");
                if text.is_empty() {
                    Ok(Vec::new())
                } else {
                    Ok(vec![StreamEvent::TextDelta { index, text }])
                }
            }
            "thinking_delta" => {
                let text = text_field(delta, "thinking");
                if text.is_empty() {
                    Ok(Vec::new())
                } else {
                    Ok(vec![StreamEvent::ThinkingDelta { index, text }])
                }
            }
            "input_json_delta" => {
                let fragment = text_field(delta, "partial_json");
                let tool = self.tools.get_mut(&index).ok_or_else(|| {
                    ProviderError::Protocol("a tool's arguments arrived for a call that never started".to_owned())
                })?;
                tool.json.push_str(&fragment);
                if fragment.is_empty() {
                    Ok(Vec::new())
                } else {
                    Ok(vec![StreamEvent::ToolInputDelta { index, json: fragment }])
                }
            }
            // The signature of a thinking block, which this build does not keep.
            "signature_delta" => Ok(Vec::new()),
            other => Err(ProviderError::Protocol(format!(
                "the stream carried a content change this build does not know: {other}"
            ))),
        }
    }

    fn stop_block(&mut self, value: &Value) -> Result<Vec<StreamEvent>, ProviderError> {
        let index = block_index(value)?;
        let Some(tool) = self.tools.remove(&index) else {
            // A text or thinking block closing: nothing to report, the deltas were the content.
            return Ok(Vec::new());
        };

        // A tool called with no arguments produces no fragments at all, which is an empty object
        // rather than a parse failure.
        let input = if tool.json.trim().is_empty() {
            Value::Object(Map::new())
        } else {
            serde_json::from_str(&tool.json)
                .map_err(|_| ProviderError::Protocol("the model's tool arguments were not valid JSON".to_owned()))?
        };

        Ok(vec![StreamEvent::ToolUseDone {
            index,
            id: tool.id,
            name: tool.name,
            input,
        }])
    }

    fn message_delta(&mut self, value: &Value) -> Result<Vec<StreamEvent>, ProviderError> {
        if let Some(reason) = value.pointer("/delta/stop_reason").and_then(Value::as_str) {
            self.stop_reason = Some(stop_reason(reason)?);
        }

        if let Some(usage) = value.get("usage") {
            if let Some(output) = count(usage.get("output_tokens")) {
                self.usage.output_tokens = output;
                self.usage_seen = true;
            }
            if let Some(cache) = count(usage.get("cache_read_input_tokens")) {
                self.usage.cache_read_tokens = Some(cache);
                self.usage_seen = true;
            }
            if let Some(input) = count(usage.get("input_tokens")) {
                self.usage.input_tokens = input;
                self.usage_seen = true;
            }
        }

        if self.usage_seen && !self.usage_sent {
            self.usage_sent = true;
            Ok(vec![StreamEvent::Usage(self.usage)])
        } else {
            Ok(Vec::new())
        }
    }
}

impl SseParser for AnthropicParser {
    fn on_event(&mut self, event: &SseEvent) -> Result<Vec<StreamEvent>, ProviderError> {
        // Anything after the stop is a server talking to itself; the turn is over.
        if self.finished {
            return Ok(Vec::new());
        }
        if event.data.trim().is_empty() {
            // A keep-alive with no payload. Nothing happened.
            return Ok(Vec::new());
        }

        let value: Value = serde_json::from_str(&event.data)
            .map_err(|_| ProviderError::Protocol("the provider sent a stream event that is not JSON".to_owned()))?;

        // The `type` field is authoritative; the `event:` line is the same word, and is used only when
        // a gateway has dropped the field.
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .or(event.event.as_deref())
            .unwrap_or_default();

        match kind {
            "message_start" => {
                if let Some(usage) = value.pointer("/message/usage") {
                    if let Some(input) = count(usage.get("input_tokens")) {
                        self.usage.input_tokens = input;
                        self.usage_seen = true;
                    }
                    if let Some(cache) = count(usage.get("cache_read_input_tokens")) {
                        self.usage.cache_read_tokens = Some(cache);
                        self.usage_seen = true;
                    }
                }
                Ok(Vec::new())
            }
            "content_block_start" => self.start_block(&value),
            "content_block_delta" => self.delta(&value),
            "content_block_stop" => self.stop_block(&value),
            "message_delta" => self.message_delta(&value),
            "message_stop" => {
                self.finished = true;
                Ok(vec![StreamEvent::Stop {
                    reason: self.stop_reason.unwrap_or(StopReason::EndTurn),
                }])
            }
            // A keep-alive. Nothing happened, and nothing should be drawn.
            "ping" => Ok(Vec::new()),
            "error" => {
                let error = value.get("error").cloned().unwrap_or_else(|| value.clone());
                Err(crate::http::classify_error_object(&error))
            }
            other => Err(ProviderError::Protocol(format!(
                "the stream carried an event this build does not know: {other}"
            ))),
        }
    }

    fn on_end(&mut self) -> Result<Vec<StreamEvent>, ProviderError> {
        if self.finished {
            return Ok(Vec::new());
        }

        // This API always closes a message with `message_stop`. A stream that ends without one was cut
        // off, and reporting that as a finished answer would present half a turn as a whole one.
        Err(ProviderError::Protocol(
            "the stream ended before the provider finished the message".to_owned(),
        ))
    }
}

fn block_index(value: &Value) -> Result<u32, ProviderError> {
    count(value.get("index"))
        .ok_or_else(|| ProviderError::Protocol("a content block event arrived without an index".to_owned()))
}

fn text_field(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn count(value: Option<&Value>) -> Option<u32> {
    value
        .and_then(Value::as_u64)
        .map(|number| u32::try_from(number).unwrap_or(u32::MAX))
}

fn stop_reason(reason: &str) -> Result<StopReason, ProviderError> {
    match reason {
        "end_turn" => Ok(StopReason::EndTurn),
        "tool_use" => Ok(StopReason::ToolUse),
        "max_tokens" => Ok(StopReason::MaxTokens),
        "stop_sequence" => Ok(StopReason::StopSequence),
        "refusal" => Ok(StopReason::Refusal),
        other => Err(ProviderError::Protocol(format!(
            "the model stopped for a reason this build does not know: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AiProviderConfig;

    fn config() -> AiProviderConfig {
        AiProviderConfig::new(
            "claude",
            "Claude",
            AiProviderKind::Anthropic,
            "https://api.anthropic.com",
            "claude-sonnet-4-5",
        )
    }

    fn parse(parser: &mut AnthropicParser, json: &str) -> Result<Vec<StreamEvent>, ProviderError> {
        parser.on_event(&SseEvent {
            event: None,
            data: json.to_owned(),
        })
    }

    #[test]
    fn a_provider_without_a_key_is_refused_before_anything_is_sent() {
        assert_eq!(
            AnthropicProvider::new(&config(), None).expect_err("a key is required"),
            ProviderError::NoKey
        );
        assert_eq!(
            AnthropicProvider::new(&config(), Some("  ")).expect_err("whitespace is not a key"),
            ProviderError::NoKey
        );
    }

    #[test]
    fn a_provider_debug_line_does_not_contain_the_key() {
        let provider = AnthropicProvider::new(&config(), Some("sk-ant-secret-value")).expect("a provider");
        let printed = format!("{provider:?}");

        assert!(
            !printed.contains("secret-value"),
            "a provider formatted into a log must not carry its key: {printed}"
        );
        assert!(
            printed.contains("claude-sonnet-4-5"),
            "the model is not a secret: {printed}"
        );
    }

    #[test]
    fn the_request_body_puts_each_block_where_this_api_expects_it() {
        let provider = AnthropicProvider::new(&config(), Some("sk-ant-1")).expect("a provider");
        let request = ProviderRequest {
            model: String::new(),
            system: Some("You are an operations assistant.".to_owned()),
            messages: vec![
                crate::types::ChatMessage::user("how full is the disk?"),
                crate::types::ChatMessage::assistant(vec![ContentBlock::ToolUse {
                    id: "toolu_1".to_owned(),
                    name: "sshell_exec_run".to_owned(),
                    input: json!({ "command": "df -h" }),
                }]),
                crate::types::ChatMessage::tool_result("toolu_1", "43% used", false),
            ],
            tools: vec![crate::types::ToolSpec {
                name: "sshell_exec_run".to_owned(),
                description: "Runs a command".to_owned(),
                input_schema: json!({ "type": "object" }),
            }],
            max_tokens: 1_024,
            temperature: Some(0.2),
        };

        let body: Value = serde_json::from_slice(&provider.request_body(&request).expect("a body")).expect("JSON");

        assert_eq!(
            body["model"],
            json!("claude-sonnet-4-5"),
            "an empty model means the configured one"
        );
        assert_eq!(body["max_tokens"], json!(1_024));
        assert_eq!(body["stream"], json!(true));
        assert_eq!(body["system"], json!("You are an operations assistant."));
        assert_eq!(body["messages"][0]["role"], json!("user"));
        assert_eq!(body["messages"][1]["content"][0]["type"], json!("tool_use"));
        assert_eq!(body["messages"][1]["content"][0]["input"]["command"], json!("df -h"));
        assert_eq!(body["messages"][2]["content"][0]["type"], json!("tool_result"));
        assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], json!("toolu_1"));
        assert_eq!(body["tools"][0]["input_schema"]["type"], json!("object"));
        // The contract's temperature is an `f32`, and JSON has no such number: the value that reaches
        // the provider is 0.2 to within a rounding error, which is what any model is asked for.
        let temperature = body["temperature"].as_f64().expect("a number");
        assert!((temperature - 0.2).abs() < 1e-6, "the temperature was {temperature}");
    }

    #[test]
    fn thinking_is_never_sent_back_and_an_empty_turn_is_dropped() {
        let provider = AnthropicProvider::new(&config(), Some("sk-ant-1")).expect("a provider");
        let request = ProviderRequest {
            model: String::new(),
            system: None,
            messages: vec![
                crate::types::ChatMessage::assistant(vec![ContentBlock::Thinking {
                    text: "the person probably means df".to_owned(),
                }]),
                crate::types::ChatMessage::user("   "),
                crate::types::ChatMessage::user("df?"),
            ],
            tools: Vec::new(),
            max_tokens: 100,
            temperature: None,
        };

        let body: Value = serde_json::from_slice(&provider.request_body(&request).expect("a body")).expect("JSON");
        let messages = body["messages"].as_array().expect("an array");

        assert_eq!(
            messages.len(),
            1,
            "a thinking-only turn and a blank turn have nothing to say: {body}"
        );
        assert_eq!(messages[0]["content"][0]["text"], json!("df?"));
        assert!(
            body.get("tools").is_none(),
            "no tool list is sent when none was offered"
        );
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn a_tool_call_is_assembled_from_fragments_and_parsed_at_its_close() {
        let mut parser = AnthropicParser::default();

        let start = parse(
            &mut parser,
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_9","name":"sshell_exec_run","input":{}}}"#,
        )
        .expect("a start");
        assert_eq!(
            start,
            vec![StreamEvent::ToolUseStart {
                index: 1,
                id: "toolu_9".to_owned(),
                name: "sshell_exec_run".to_owned(),
            }]
        );

        for fragment in [
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"com"}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"mand\": \"df -h\""}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"}"}}"#,
        ] {
            let events = parse(&mut parser, fragment).expect("a fragment");
            assert_eq!(events.len(), 1, "a fragment is reported as it arrived");
        }

        let done = parse(&mut parser, r#"{"type":"content_block_stop","index":1}"#).expect("a close");
        assert_eq!(
            done,
            vec![StreamEvent::ToolUseDone {
                index: 1,
                id: "toolu_9".to_owned(),
                name: "sshell_exec_run".to_owned(),
                input: json!({ "command": "df -h" }),
            }]
        );
    }

    #[test]
    fn a_tool_call_with_no_arguments_is_an_empty_object() {
        let mut parser = AnthropicParser::default();
        parse(
            &mut parser,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"sshell_sessions_list","input":{}}}"#,
        )
        .expect("a start");

        let done = parse(&mut parser, r#"{"type":"content_block_stop","index":0}"#).expect("a close");
        let [StreamEvent::ToolUseDone { input, .. }] = done.as_slice() else {
            panic!("a tool call closes with its arguments: {done:?}");
        };
        assert_eq!(*input, json!({}));
    }

    #[test]
    fn arguments_that_are_not_json_are_a_protocol_failure() {
        let mut parser = AnthropicParser::default();
        parse(
            &mut parser,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t","name":"n","input":{}}}"#,
        )
        .expect("a start");
        parse(
            &mut parser,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\":"}}"#,
        )
        .expect("a fragment");

        let error = parse(&mut parser, r#"{"type":"content_block_stop","index":0}"#).expect_err("not JSON");
        assert!(matches!(error, ProviderError::Protocol(_)), "{error:?}");
    }

    #[test]
    fn usage_arrives_once_even_though_it_is_reported_twice() {
        let mut parser = AnthropicParser::default();
        parse(
            &mut parser,
            r#"{"type":"message_start","message":{"id":"msg_1","usage":{"input_tokens":25,"output_tokens":1,"cache_read_input_tokens":12}}}"#,
        )
        .expect("a start");

        let usage = parse(
            &mut parser,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":9}}"#,
        )
        .expect("a delta");

        assert_eq!(
            usage,
            vec![StreamEvent::Usage(Usage {
                input_tokens: 25,
                output_tokens: 9,
                cache_read_tokens: Some(12),
            })],
            "the loop gets one complete number rather than two halves to add"
        );
        assert!(
            parse(&mut parser, r#"{"type":"message_stop"}"#).expect("a stop").len() == 1,
            "the message ends once"
        );
        assert!(
            parse(&mut parser, r#"{"type":"message_stop"}"#)
                .expect("a repeated stop")
                .is_empty(),
            "a second stop would make the loop think the turn ended twice"
        );
    }

    #[test]
    fn every_documented_stop_reason_is_mapped_and_an_unknown_one_is_not_guessed() {
        for (wire, expected) in [
            ("end_turn", StopReason::EndTurn),
            ("tool_use", StopReason::ToolUse),
            ("max_tokens", StopReason::MaxTokens),
            ("stop_sequence", StopReason::StopSequence),
            ("refusal", StopReason::Refusal),
        ] {
            assert_eq!(stop_reason(wire).expect(wire), expected);
        }

        assert!(matches!(
            stop_reason("brand_new_reason"),
            Err(ProviderError::Protocol(_))
        ));
    }

    #[test]
    fn a_message_that_ends_without_its_last_event_is_not_a_finished_answer() {
        let mut parser = AnthropicParser::default();
        parse(
            &mut parser,
            r#"{"type":"message_start","message":{"usage":{"input_tokens":3}}}"#,
        )
        .expect("a start");

        let error = parser.on_end().expect_err("half an answer is not an answer");
        assert!(matches!(error, ProviderError::Protocol(_)), "{error:?}");
    }
}
