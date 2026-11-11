//! OpenAI's Chat Completions API, and every server that speaks the same shape.
//!
//! One adapter serves both [`AiProviderKind::OpenAi`] and [`AiProviderKind::Local`]: the only
//! difference between them is the base address and whether a key is needed, which the host decides.
//! A local server that speaks this shape — Ollama, vLLM, LM Studio — is the reason the third kind
//! exists at all.
//!
//! What this adapter has to absorb, because the servers behind it do not agree:
//!
//! * **Usage may be absent.** `stream_options.include_usage` is what asks for it, and a server that
//!   predates the option ignores it. A stream with no usage is a complete answer.
//! * **`[DONE]` may be absent.** Some servers close the connection after the last chunk. The end of
//!   the body is treated as the end of the answer, and `finish_reason` decides the stop reason.
//! * **`finish_reason` may be absent too** on a server that only ever closes. The answer then ends
//!   the way an assistant turn usually ends, and the tool calls that did arrive are still run.
//!
//! The one thing that is not tolerated is a tool call whose arguments are not JSON: a call the tool
//! layer cannot read is a call nobody can answer.
//!
//! Block indices are this adapter's own. The wire has no content blocks — text and tool calls are
//! separate fields of one delta — so the text is block 0 and the *n*-th tool call is block *n* + 1,
//! which is what lets a conversation that contains both be drawn in the order it was said.

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

/// Where Chat Completions lives under a base address.
const ENDPOINT_PATH: &str = "/chat/completions";

/// An OpenAI-compatible provider: one base address, one model and, usually, one key.
///
/// `base_url` is the assembled endpoint, built and checked once when the provider is created.
#[derive(Debug, Clone)]
pub struct OpenAiProvider {
    base_url: reqwest::Url,
    model: String,
    key: Option<ApiKey>,
    kind: AiProviderKind,
    client: reqwest::Client,
}

impl OpenAiProvider {
    /// Builds the provider for one configuration entry.
    ///
    /// A hosted provider needs a key; a local one does not, because a model served on this machine
    /// usually has no notion of accounts — and sending an empty `Authorization` header would make
    /// some of those servers reject an otherwise unauthenticated request.
    pub fn new(config: &crate::types::AiProviderConfig, key: Option<&str>) -> Result<Self, ProviderError> {
        let kind = match config.kind {
            AiProviderKind::Local => AiProviderKind::Local,
            _ => AiProviderKind::OpenAi,
        };

        let base_url = endpoint(&config.base_url, ENDPOINT_PATH)?;
        let key = ApiKey::from_option(key);
        if key.is_none() && kind == AiProviderKind::OpenAi {
            return Err(ProviderError::NoKey);
        }
        let client = build_client(&base_url)?;

        Ok(Self {
            base_url,
            model: config.model.clone(),
            key,
            kind,
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
        body.insert("messages".to_owned(), Value::Array(messages(request)));
        body.insert("stream".to_owned(), json!(true));
        // Ask for the usage on the last chunk. A server that does not know the option ignores it, and
        // the stream is read without it either way.
        body.insert("stream_options".to_owned(), json!({ "include_usage": true }));
        body.insert("max_tokens".to_owned(), json!(request.max_tokens));

        if !request.tools.is_empty() {
            body.insert(
                "tools".to_owned(),
                Value::Array(
                    request
                        .tools
                        .iter()
                        .map(|tool| {
                            json!({
                                "type": "function",
                                "function": {
                                    "name": tool.name,
                                    "description": tool.description,
                                    "parameters": tool.input_schema,
                                },
                            })
                        })
                        .collect(),
                ),
            );
        }
        if let Some(temperature) = request.temperature {
            body.insert("temperature".to_owned(), json!(temperature));
        }

        serde_json::to_vec(&Value::Object(body))
            .map_err(|error| ProviderError::Protocol(format!("the request could not be written: {error}")))
    }

    fn model_for<'a>(&'a self, request: &'a ProviderRequest) -> &'a str {
        if request.model.trim().is_empty() {
            &self.model
        } else {
            &request.model
        }
    }
}

#[async_trait]
impl Provider for OpenAiProvider {
    fn kind(&self) -> AiProviderKind {
        self.kind
    }

    async fn stream(&self, request: ProviderRequest) -> Result<ProviderStream, ProviderError> {
        let body = self.request_body(&request)?;
        let mut headers = Vec::new();
        if let Some(key) = &self.key {
            let mut value = HeaderValue::from_str(&format!("Bearer {}", key.expose())).map_err(|_| {
                ProviderError::Protocol("the stored API key is not a value an HTTP header can carry".to_owned())
            })?;
            // The bearer value is built from the key and lives only as long as this request; the
            // zeroizing buffer in `key` is still what holds it between requests.
            value.set_sensitive(true);
            headers.push(("authorization", value));
        }

        let response = post_stream(&self.client, &self.base_url, &headers, &body, Limits::default()).await?;

        Ok(provider_stream(
            events(response, Limits::default()),
            OpenAiParser::default(),
        ))
    }
}

/// The conversation as this API's `messages` array.
///
/// Three shapes here that the Anthropic side does not have:
///
/// * the system prompt is a message rather than a field;
/// * a tool result is a message of its own (`role: "tool"`), not a block of the next user turn;
/// * a turn's text is one string, so several text blocks are joined.
///
/// `is_error` has no field in this API, and is not invented as one: a server that validates its input
/// strictly would refuse the whole request. The error text is in the result's content, which is what
/// the model reads either way.
fn messages(request: &ProviderRequest) -> Vec<Value> {
    let mut out = Vec::new();

    if let Some(system) = request.system.as_deref().map(str::trim).filter(|text| !text.is_empty()) {
        out.push(json!({ "role": "system", "content": system }));
    }

    for message in &request.messages {
        match message.role {
            Role::User => {
                let mut text = String::new();
                for block in &message.blocks {
                    match block {
                        ContentBlock::Text { text: said } => {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(said);
                        }
                        ContentBlock::ToolResult {
                            tool_use_id, content, ..
                        } => {
                            // A tool result must follow the assistant message that asked for it
                            // directly, so anything said before it goes first.
                            if !text.trim().is_empty() {
                                out.push(json!({ "role": "user", "content": text }));
                                text.clear();
                            }
                            out.push(json!({
                                "role": "tool",
                                "tool_call_id": tool_use_id,
                                "content": content,
                            }));
                        }
                        // A user turn cannot ask for a tool, and thinking is not this API's.
                        ContentBlock::ToolUse { .. } | ContentBlock::Thinking { .. } => {}
                    }
                }
                if !text.trim().is_empty() {
                    out.push(json!({ "role": "user", "content": text }));
                }
            }
            Role::Assistant => {
                let mut content = String::new();
                let mut calls = Vec::new();
                for block in &message.blocks {
                    match block {
                        ContentBlock::Text { text: said } => {
                            if !content.is_empty() {
                                content.push('\n');
                            }
                            content.push_str(said);
                        }
                        ContentBlock::ToolUse { id, name, input } => calls.push(json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": name,
                                // The arguments are a JSON *string* in this API, not an object.
                                "arguments": serde_json::to_string(input).unwrap_or_else(|_| "{}".to_owned()),
                            },
                        })),
                        ContentBlock::Thinking { .. } | ContentBlock::ToolResult { .. } => {}
                    }
                }

                if content.trim().is_empty() && calls.is_empty() {
                    continue;
                }

                let mut message = Map::new();
                message.insert("role".to_owned(), json!("assistant"));
                if !content.trim().is_empty() {
                    message.insert("content".to_owned(), json!(content));
                }
                if !calls.is_empty() {
                    message.insert("tool_calls".to_owned(), Value::Array(calls));
                }
                out.push(Value::Object(message));
            }
        }
    }

    out
}

/// One tool call being assembled from its fragments.
#[derive(Debug, Default)]
struct PendingTool {
    id: Option<String>,
    name: String,
    arguments: String,
    announced: bool,
}

/// This adapter's reading of the stream.
#[derive(Debug, Default)]
struct OpenAiParser {
    tools: BTreeMap<u32, PendingTool>,
    usage: Option<Usage>,
    usage_sent: bool,
    stop_reason: Option<StopReason>,
    finished: bool,
}

impl OpenAiParser {
    /// Folds one `tool_calls` entry into the call it belongs to.
    ///
    /// The name and the arguments are not guaranteed to arrive in the same chunk, and a call is only
    /// announced once it has a name: a card with no tool name is worse than a card that appears one
    /// chunk later. Arguments that arrive before the name are held and reported with the start.
    fn tool_delta(&mut self, call: &Value, events: &mut Vec<StreamEvent>) -> Result<(), ProviderError> {
        let Some(index) = count(call.get("index")) else {
            return Err(ProviderError::Protocol(
                "a tool call arrived without an index".to_owned(),
            ));
        };
        let block = index + 1;

        let entry = self.tools.entry(index).or_default();
        if let Some(id) = call.get("id").and_then(Value::as_str)
            && !id.is_empty()
        {
            entry.id = Some(id.to_owned());
        }
        if let Some(function) = call.get("function") {
            if let Some(name) = function.get("name").and_then(Value::as_str)
                && entry.name.is_empty()
                && !name.is_empty()
            {
                entry.name = name.to_owned();
            }
            if let Some(fragment) = function.get("arguments").and_then(Value::as_str) {
                entry.arguments.push_str(fragment);

                // Emptiness first: a server that opens a call with `"arguments": ""` is announcing it,
                // not sending a fragment. An unannounced call's fragments are buffered until the name
                // is known, and reported in full with the start.
                if !fragment.is_empty() && entry.announced {
                    events.push(StreamEvent::ToolInputDelta {
                        index: block,
                        json: fragment.to_owned(),
                    });
                }
            }
        }

        if !entry.announced && !entry.name.is_empty() {
            entry.announced = true;
            events.push(StreamEvent::ToolUseStart {
                index: block,
                id: entry.id.clone().unwrap_or_else(|| synthesised_id(index)),
                name: entry.name.clone(),
            });
            if !entry.arguments.is_empty() {
                events.push(StreamEvent::ToolInputDelta {
                    index: block,
                    json: entry.arguments.clone(),
                });
            }
        }

        Ok(())
    }

    /// Closes every call that has not been closed yet.
    ///
    /// Called at `finish_reason` and, for a server that never sends one, at the end of the body.
    fn finish_tools(&mut self) -> Result<Vec<StreamEvent>, ProviderError> {
        let mut events = Vec::new();

        for (index, mut entry) in std::mem::take(&mut self.tools) {
            let block = index + 1;

            if entry.name.is_empty() {
                return Err(ProviderError::Protocol(
                    "a tool call arrived without a tool name".to_owned(),
                ));
            }

            if !entry.announced {
                entry.announced = true;
                events.push(StreamEvent::ToolUseStart {
                    index: block,
                    id: entry.id.clone().unwrap_or_else(|| synthesised_id(index)),
                    name: entry.name.clone(),
                });
                if !entry.arguments.is_empty() {
                    events.push(StreamEvent::ToolInputDelta {
                        index: block,
                        json: entry.arguments.clone(),
                    });
                }
            }

            let input = if entry.arguments.trim().is_empty() {
                Value::Object(Map::new())
            } else {
                serde_json::from_str(&entry.arguments)
                    .map_err(|_| ProviderError::Protocol("the model's tool arguments were not valid JSON".to_owned()))?
            };

            events.push(StreamEvent::ToolUseDone {
                index: block,
                id: entry.id.unwrap_or_else(|| synthesised_id(index)),
                name: entry.name,
                input,
            });
        }

        Ok(events)
    }

    fn stop(&mut self) -> Vec<StreamEvent> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;
        vec![StreamEvent::Stop {
            reason: self.stop_reason.unwrap_or(StopReason::EndTurn),
        }]
    }
}

impl SseParser for OpenAiParser {
    fn on_event(&mut self, event: &SseEvent) -> Result<Vec<StreamEvent>, ProviderError> {
        // Anything after the sentinel is a server talking to itself; the turn is over.
        if self.finished {
            return Ok(Vec::new());
        }

        let data = event.data.trim();
        if data.is_empty() {
            // A keep-alive with no payload. Nothing happened.
            return Ok(Vec::new());
        }

        if data == "[DONE]" {
            let mut events = self.finish_tools()?;
            events.extend(self.stop());
            return Ok(events);
        }

        let value: Value = serde_json::from_str(data)
            .map_err(|_| ProviderError::Protocol("the provider sent a stream event that is not JSON".to_owned()))?;

        if let Some(error) = value.get("error") {
            return Err(crate::http::classify_error_object(error));
        }

        let mut events = Vec::new();

        if let Some(usage) = value.get("usage").filter(|usage| usage.is_object())
            && !self.usage_sent
        {
            self.usage = Some(usage_of(usage));
        }

        // The array is normally one choice. Some servers send none at all on the usage chunk, which
        // is why this is a lookup rather than an index.
        if let Some(choice) = value.get("choices").and_then(|choices| choices.get(0)) {
            if let Some(delta) = choice.get("delta") {
                if let Some(text) = delta.get("content").and_then(Value::as_str)
                    && !text.is_empty()
                {
                    events.push(StreamEvent::TextDelta {
                        index: 0,
                        text: text.to_owned(),
                    });
                }
                // A reasoning model served through this shape may stream its thinking separately.
                if let Some(thought) = delta.get("reasoning_content").and_then(Value::as_str)
                    && !thought.is_empty()
                {
                    events.push(StreamEvent::ThinkingDelta {
                        index: 0,
                        text: thought.to_owned(),
                    });
                }
                if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                    for call in calls {
                        self.tool_delta(call, &mut events)?;
                    }
                }
            }

            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                self.stop_reason = Some(finish_reason(reason)?);
                // The calls are complete once the choice is finished, whichever chunk carried the
                // last fragment.
                events.extend(self.finish_tools()?);
            }
        }

        if let Some(usage) = self.usage.filter(|_| !self.usage_sent) {
            self.usage_sent = true;
            events.push(StreamEvent::Usage(usage));
        }

        Ok(events)
    }

    fn on_end(&mut self) -> Result<Vec<StreamEvent>, ProviderError> {
        if self.finished {
            return Ok(Vec::new());
        }

        // A server that closed without saying `[DONE]` still finished its answer; one that closed
        // before its `finish_reason` did too, as far as this build can tell. Reporting a protocol
        // failure here would break every compatible server that simply hangs up.
        let mut events = self.finish_tools()?;
        events.extend(self.stop());
        Ok(events)
    }
}

fn synthesised_id(index: u32) -> String {
    // A server that never names its calls still gets ids that a result can answer. They are stable
    // for the conversation, because they are derived from the call's position.
    format!("call_{index}")
}

fn usage_of(usage: &Value) -> Usage {
    Usage {
        input_tokens: count(usage.get("prompt_tokens")).unwrap_or_default(),
        output_tokens: count(usage.get("completion_tokens")).unwrap_or_default(),
        cache_read_tokens: count(usage.pointer("/prompt_tokens_details/cached_tokens")),
    }
}

fn count(value: Option<&Value>) -> Option<u32> {
    value
        .and_then(Value::as_u64)
        .map(|number| u32::try_from(number).unwrap_or(u32::MAX))
}

fn finish_reason(reason: &str) -> Result<StopReason, ProviderError> {
    match reason {
        "stop" => Ok(StopReason::EndTurn),
        "tool_calls" | "function_call" => Ok(StopReason::ToolUse),
        "length" => Ok(StopReason::MaxTokens),
        "content_filter" => Ok(StopReason::Refusal),
        other => Err(ProviderError::Protocol(format!(
            "the model stopped for a reason this build does not know: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AiProviderConfig, ChatMessage, ToolSpec};

    fn config(kind: AiProviderKind, base_url: &str) -> AiProviderConfig {
        AiProviderConfig::new("test", "Test", kind, base_url, "gpt-4o")
    }

    fn parse(parser: &mut OpenAiParser, data: &str) -> Result<Vec<StreamEvent>, ProviderError> {
        parser.on_event(&SseEvent {
            event: None,
            data: data.to_owned(),
        })
    }

    #[test]
    fn a_hosted_provider_needs_a_key_and_a_local_one_does_not() {
        assert_eq!(
            OpenAiProvider::new(&config(AiProviderKind::OpenAi, "https://api.openai.com/v1"), None)
                .expect_err("a hosted provider has no anonymous mode"),
            ProviderError::NoKey
        );

        let local = OpenAiProvider::new(&config(AiProviderKind::Local, "http://127.0.0.1:11434/v1"), None)
            .expect("a model on this machine needs no account");
        assert_eq!(local.kind(), AiProviderKind::Local);
    }

    #[test]
    fn a_provider_debug_line_does_not_contain_the_key() {
        let provider = OpenAiProvider::new(
            &config(AiProviderKind::OpenAi, "https://api.openai.com/v1"),
            Some("sk-live-1"),
        )
        .expect("a provider");
        let printed = format!("{provider:?}");

        assert!(
            !printed.contains("sk-live-1"),
            "a provider formatted into a log must not carry its key: {printed}"
        );
    }

    #[test]
    fn the_request_body_uses_a_system_message_and_function_tools() {
        let provider = OpenAiProvider::new(&config(AiProviderKind::OpenAi, "https://api.openai.com/v1"), Some("k"))
            .expect("a provider");
        let request = ProviderRequest {
            model: "gpt-4.1".to_owned(),
            system: Some("Be brief.".to_owned()),
            messages: vec![
                ChatMessage::user("how full is the disk?"),
                ChatMessage::assistant(vec![ContentBlock::ToolUse {
                    id: "call_1".to_owned(),
                    name: "sshell_exec_run".to_owned(),
                    input: json!({ "command": "df -h" }),
                }]),
                ChatMessage::tool_result("call_1", "43% used", false),
            ],
            tools: vec![ToolSpec {
                name: "sshell_exec_run".to_owned(),
                description: "Runs a command".to_owned(),
                input_schema: json!({ "type": "object" }),
            }],
            max_tokens: 512,
            temperature: None,
        };

        let body: Value = serde_json::from_slice(&provider.request_body(&request).expect("a body")).expect("JSON");

        assert_eq!(body["model"], json!("gpt-4.1"));
        assert_eq!(body["stream"], json!(true));
        assert_eq!(body["stream_options"]["include_usage"], json!(true));
        assert_eq!(body["messages"][0], json!({ "role": "system", "content": "Be brief." }));
        assert_eq!(
            body["messages"][1],
            json!({ "role": "user", "content": "how full is the disk?" })
        );
        assert_eq!(body["messages"][2]["role"], json!("assistant"));
        assert_eq!(
            body["messages"][2]["tool_calls"][0]["function"]["name"],
            json!("sshell_exec_run")
        );
        assert_eq!(
            body["messages"][2]["tool_calls"][0]["function"]["arguments"],
            json!("{\"command\":\"df -h\"}"),
            "the arguments are a JSON string in this API"
        );
        assert_eq!(
            body["messages"][3],
            json!({ "role": "tool", "tool_call_id": "call_1", "content": "43% used" }),
            "a tool result is its own message here"
        );
        assert!(
            body["messages"][3].get("is_error").is_none(),
            "this API has no such field"
        );
        assert_eq!(body["tools"][0]["type"], json!("function"));
        assert_eq!(body["tools"][0]["function"]["parameters"]["type"], json!("object"));
    }

    #[test]
    fn tool_calls_are_assembled_by_index_across_chunks() {
        let mut parser = OpenAiParser::default();

        let first = parse(
            &mut parser,
            r#"{"choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_9","type":"function","function":{"name":"sshell_exec_run","arguments":""}}]},"finish_reason":null}]}"#,
        )
        .expect("a chunk");
        assert_eq!(
            first,
            vec![StreamEvent::ToolUseStart {
                index: 1,
                id: "call_9".to_owned(),
                name: "sshell_exec_run".to_owned(),
            }],
            "a call is announced as soon as it has a name"
        );

        let fragment = parse(
            &mut parser,
            r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"command\":"}}]},"finish_reason":null}]}"#,
        )
        .expect("a fragment");
        assert_eq!(
            fragment,
            vec![StreamEvent::ToolInputDelta {
                index: 1,
                json: "{\"command\":".to_owned(),
            }]
        );

        parse(
            &mut parser,
            r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"df -h\"}"}}]},"finish_reason":null}]}"#,
        )
        .expect("a fragment");

        let done = parse(
            &mut parser,
            r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
        )
        .expect("a finish");
        assert_eq!(
            done,
            vec![StreamEvent::ToolUseDone {
                index: 1,
                id: "call_9".to_owned(),
                name: "sshell_exec_run".to_owned(),
                input: json!({ "command": "df -h" }),
            }],
            "the call closes when the choice finishes; the turn itself ends at the sentinel"
        );

        let end = parse(&mut parser, "[DONE]").expect("the sentinel");
        assert_eq!(
            end,
            vec![StreamEvent::Stop {
                reason: StopReason::ToolUse,
            }],
            "the stop reason is the one the choice reported"
        );
    }

    #[test]
    fn an_unnamed_call_gets_an_id_a_result_can_answer() {
        let mut parser = OpenAiParser::default();
        parse(
            &mut parser,
            r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":2,"type":"function","function":{"name":"sshell_sessions_list","arguments":"{}"}}]},"finish_reason":null}]}"#,
        )
        .expect("a chunk");

        let done = parse(&mut parser, "[DONE]").expect("the end");
        let [
            StreamEvent::ToolUseDone { index, id, input, .. },
            StreamEvent::Stop { .. },
        ] = done.as_slice()
        else {
            panic!("a call that never closed is closed at the end: {done:?}");
        };
        assert_eq!(*index, 3, "the third call is block three");
        assert_eq!(id, "call_2");
        assert_eq!(*input, json!({}));
    }

    #[test]
    fn usage_is_reported_once_even_though_it_arrives_after_the_answer() {
        let mut parser = OpenAiParser::default();
        parse(
            &mut parser,
            r#"{"choices":[{"index":0,"delta":{"content":"43%"},"finish_reason":null}]}"#,
        )
        .expect("a chunk");
        parse(
            &mut parser,
            r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
        )
        .expect("a finish");

        let usage = parse(
            &mut parser,
            r#"{"choices":[],"usage":{"prompt_tokens":31,"completion_tokens":8,"prompt_tokens_details":{"cached_tokens":16}}}"#,
        )
        .expect("a usage chunk");
        assert_eq!(
            usage,
            vec![StreamEvent::Usage(Usage {
                input_tokens: 31,
                output_tokens: 8,
                cache_read_tokens: Some(16),
            })]
        );

        let end = parse(&mut parser, "[DONE]").expect("the end");
        assert_eq!(
            end,
            vec![StreamEvent::Stop {
                reason: StopReason::EndTurn
            }]
        );
    }

    #[test]
    fn a_server_that_never_says_done_still_finishes_the_answer() {
        let mut parser = OpenAiParser::default();
        parse(
            &mut parser,
            r#"{"choices":[{"index":0,"delta":{"content":"43% used"},"finish_reason":null}]}"#,
        )
        .expect("a chunk");

        let end = parser.on_end().expect("the body ended");
        assert_eq!(
            end,
            vec![StreamEvent::Stop {
                reason: StopReason::EndTurn
            }],
            "hanging up without a sentinel is how some compatible servers end"
        );
        assert!(parser.on_end().expect("again").is_empty(), "the end is reported once");
    }

    #[test]
    fn every_documented_finish_reason_is_mapped_and_an_unknown_one_is_not_guessed() {
        for (wire, expected) in [
            ("stop", StopReason::EndTurn),
            ("tool_calls", StopReason::ToolUse),
            ("length", StopReason::MaxTokens),
            ("content_filter", StopReason::Refusal),
        ] {
            assert_eq!(finish_reason(wire).expect(wire), expected);
        }

        assert!(matches!(
            finish_reason("brand_new_reason"),
            Err(ProviderError::Protocol(_))
        ));
    }

    #[test]
    fn an_error_chunk_is_reported_as_the_failure_it_names() {
        let mut parser = OpenAiParser::default();
        let error = parse(
            &mut parser,
            r#"{"error":{"message":"This model's maximum context length is 8192 tokens","type":"invalid_request_error","code":"context_length_exceeded"}}"#,
        )
        .expect_err("an error chunk is a failure");

        assert_eq!(error, ProviderError::ContextTooLong);
    }

    #[test]
    fn a_chunk_that_is_not_json_is_a_protocol_failure() {
        let mut parser = OpenAiParser::default();
        let error = parse(&mut parser, "data: not json").expect_err("this is not JSON");

        assert!(matches!(error, ProviderError::Protocol(_)), "{error:?}");
    }
}
