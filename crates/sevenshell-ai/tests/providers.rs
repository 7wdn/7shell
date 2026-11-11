//! The two adapters against a provider that lives on this machine.
//!
//! Every test here starts a real listener and speaks real HTTP to it, because the parts that go wrong
//! between an adapter and a provider are the wire parts: a header spelled the wrong way, an event that
//! straddles two chunks, a retry that replays a body, a stream that is supposed to stop reading when
//! the caller walks away. Nothing here reaches the internet, and nothing here needs a key that works.

mod support;

use std::time::Duration;

use futures_util::StreamExt;
use serde_json::json;
use sevenshell_ai::{
    AiProviderConfig, AiProviderKind, AnthropicProvider, ChatMessage, ContentBlock, OpenAiProvider, Provider,
    ProviderError, ProviderRequest, ProviderStream, StopReason, StreamEvent, ToolSpec, Usage,
};
use support::{FakeProvider, Reply};

const ANTHROPIC_TEXT: &str = include_str!("fixtures/anthropic_text.sse");
const ANTHROPIC_TOOL: &str = include_str!("fixtures/anthropic_tool.sse");
const ANTHROPIC_THINKING: &str = include_str!("fixtures/anthropic_thinking.sse");
const OPENAI_TEXT: &str = include_str!("fixtures/openai_text.sse");
const OPENAI_TOOL: &str = include_str!("fixtures/openai_tool.sse");

fn anthropic_config(base_url: &str) -> AiProviderConfig {
    AiProviderConfig::new(
        "claude",
        "Claude",
        AiProviderKind::Anthropic,
        base_url,
        "claude-sonnet-4-5",
    )
}

fn openai_config(base_url: &str) -> AiProviderConfig {
    AiProviderConfig::new("openai", "OpenAI", AiProviderKind::OpenAi, base_url, "gpt-4o")
}

fn local_config(base_url: &str) -> AiProviderConfig {
    AiProviderConfig::new("ollama", "Ollama", AiProviderKind::Local, base_url, "llama3.1")
}

/// One request that exercises every part of the conversation the two APIs spell differently.
fn request() -> ProviderRequest {
    ProviderRequest {
        model: String::new(),
        system: Some("You are an operations assistant.".to_owned()),
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
            input_schema: json!({ "type": "object", "properties": { "command": { "type": "string" } } }),
        }],
        max_tokens: 1_024,
        temperature: None,
    }
}

/// Splits a fixture at arbitrary byte boundaries, so events straddle chunks the way a real network
/// splits them.
fn in_parts(text: &str, size: usize) -> Vec<String> {
    text.as_bytes()
        .chunks(size)
        .map(|chunk| String::from_utf8(chunk.to_vec()).expect("the fixtures are ASCII"))
        .collect()
}

async fn events_of(stream: ProviderStream) -> Vec<StreamEvent> {
    let mut stream = stream;
    let mut events = Vec::new();
    while let Some(item) = stream.next().await {
        events.push(item.expect("the stream should not have failed"));
    }
    events
}

fn text_of(events: &[StreamEvent]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::TextDelta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn usage_of(events: &[StreamEvent]) -> Option<Usage> {
    events.iter().find_map(|event| match event {
        StreamEvent::Usage(usage) => Some(*usage),
        _ => None,
    })
}

fn last_stop(events: &[StreamEvent]) -> StopReason {
    match events.last() {
        Some(StreamEvent::Stop { reason }) => *reason,
        other => panic!("a stream ends with a stop, not {other:?}"),
    }
}

#[tokio::test]
async fn anthropic_sends_the_message_the_api_expects_and_reads_the_answer_back() {
    let server = FakeProvider::start(vec![Reply::stream(in_parts(ANTHROPIC_TEXT, 37))]).await;
    let provider =
        AnthropicProvider::new(&anthropic_config(&server.base_url()), Some("sk-ant-test")).expect("a provider");

    let events = events_of(provider.stream(request()).await.expect("a stream")).await;

    assert_eq!(text_of(&events), "The disk is 43% full.");
    assert_eq!(
        usage_of(&events),
        Some(Usage {
            input_tokens: 25,
            output_tokens: 9,
            cache_read_tokens: Some(12),
        }),
        "the prompt usage from the start and the answer usage from the end are one number"
    );
    assert_eq!(last_stop(&events), StopReason::EndTurn);

    let requests = server.requests();
    assert_eq!(requests.len(), 1, "one request, no retry");
    let sent = &requests[0];

    assert_eq!(sent.method(), "POST");
    assert_eq!(sent.path(), "/v1/messages");
    assert_eq!(
        sent.header("x-api-key"),
        Some("sk-ant-test"),
        "the key travels in its own header"
    );
    assert_eq!(sent.header("anthropic-version"), Some("2023-06-01"));
    assert_eq!(sent.header("content-type"), Some("application/json"));
    assert_eq!(sent.header("accept"), Some("text/event-stream"));
    assert!(
        sent.header("authorization").is_none(),
        "this API does not take a bearer token"
    );

    let body = sent.body_json();
    assert_eq!(
        body["model"],
        json!("claude-sonnet-4-5"),
        "an empty model means the configured one"
    );
    assert_eq!(body["max_tokens"], json!(1_024));
    assert_eq!(body["stream"], json!(true));
    assert_eq!(body["system"], json!("You are an operations assistant."));
    assert!(
        body.get("temperature").is_none(),
        "no temperature was asked for, so none is sent"
    );
    assert_eq!(body["messages"][0]["role"], json!("user"));
    assert_eq!(
        body["messages"][0]["content"][0],
        json!({ "type": "text", "text": "how full is the disk?" })
    );
    assert_eq!(body["messages"][1]["content"][0]["type"], json!("tool_use"));
    assert_eq!(body["messages"][2]["content"][0]["type"], json!("tool_result"));
    assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], json!("call_1"));
    assert_eq!(body["tools"][0]["name"], json!("sshell_exec_run"));
    assert_eq!(
        body["tools"][0]["input_schema"]["properties"]["command"]["type"],
        json!("string")
    );
}

#[tokio::test]
async fn anthropic_assembles_a_tool_call_from_fragments_that_straddle_chunks() {
    let server = FakeProvider::start(vec![Reply::stream(in_parts(ANTHROPIC_TOOL, 13))]).await;
    let provider =
        AnthropicProvider::new(&anthropic_config(&server.base_url()), Some("sk-ant-test")).expect("a provider");

    let events = events_of(provider.stream(request()).await.expect("a stream")).await;

    assert_eq!(text_of(&events), "Let me look.");

    let starts: Vec<_> = events
        .iter()
        .filter(|event| matches!(event, StreamEvent::ToolUseStart { .. }))
        .collect();
    assert_eq!(
        starts,
        vec![&StreamEvent::ToolUseStart {
            index: 1,
            id: "toolu_01".to_owned(),
            name: "sshell_exec_run".to_owned(),
        }]
    );

    let fragments: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::ToolInputDelta { json, .. } => Some(json.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        fragments.len(),
        3,
        "each fragment is reported as it arrived: {fragments:?}"
    );

    let done: Vec<_> = events
        .iter()
        .filter(|event| matches!(event, StreamEvent::ToolUseDone { .. }))
        .collect();
    assert_eq!(
        done,
        vec![&StreamEvent::ToolUseDone {
            index: 1,
            id: "toolu_01".to_owned(),
            name: "sshell_exec_run".to_owned(),
            input: json!({ "command": "df -h", "timeoutMs": 5_000 }),
        }],
        "the arguments are parsed once the block closes, not before"
    );
    assert_eq!(last_stop(&events), StopReason::ToolUse);
}

#[tokio::test]
async fn anthropic_reports_thinking_separately_from_the_answer() {
    let server = FakeProvider::start(vec![Reply::whole(ANTHROPIC_THINKING)]).await;
    let provider =
        AnthropicProvider::new(&anthropic_config(&server.base_url()), Some("sk-ant-test")).expect("a provider");

    let events = events_of(provider.stream(request()).await.expect("a stream")).await;

    let thought: String = events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::ThinkingDelta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(thought, "The person asks about the disk. df -h answers that.");
    assert_eq!(text_of(&events), "It is 43% full.");
    assert_eq!(last_stop(&events), StopReason::EndTurn);
}

#[tokio::test]
async fn a_refused_key_never_gets_as_far_as_a_stream() {
    let server = FakeProvider::start(vec![Reply::status(
        401,
        "{\"type\":\"error\",\"error\":{\"type\":\"authentication_error\",\"message\":\"invalid x-api-key\"}}",
    )])
    .await;
    let provider =
        AnthropicProvider::new(&anthropic_config(&server.base_url()), Some("sk-ant-wrong")).expect("a provider");

    let error = provider
        .stream(request())
        .await
        .err()
        .expect("a refused key is a failure");

    assert_eq!(error, ProviderError::Auth);
    assert_eq!(error.code(), "AI-PROVIDER-UNAVAILABLE");
    assert!(
        !format!("{error:?}").contains("sk-ant-wrong"),
        "the failure must not echo the key it was given"
    );
    assert_eq!(server.request_count(), 1, "a refused key is not retried");
}

#[tokio::test]
async fn a_rate_limited_request_is_tried_again_once_and_then_streams() {
    let server = FakeProvider::start(vec![
        Reply::status(
            429,
            "{\"type\":\"error\",\"error\":{\"type\":\"rate_limit_error\",\"message\":\"slow down\"}}",
        )
        .with_header("retry-after", "0"),
        Reply::whole(ANTHROPIC_TEXT),
    ])
    .await;
    let provider =
        AnthropicProvider::new(&anthropic_config(&server.base_url()), Some("sk-ant-test")).expect("a provider");

    let events = events_of(provider.stream(request()).await.expect("a stream")).await;

    assert_eq!(
        text_of(&events),
        "The disk is 43% full.",
        "the retry carried the same request"
    );
    assert_eq!(server.request_count(), 2, "one refusal and one successful attempt");

    let requests = server.requests();
    assert_eq!(
        requests[0].body, requests[1].body,
        "a retried request is the same request, not a rebuilt one"
    );
    assert_eq!(requests[1].header("x-api-key"), Some("sk-ant-test"));
}

#[tokio::test]
async fn a_provider_that_stays_overloaded_stops_after_two_retries() {
    let overloaded = || Reply::status(529, "{\"error\":{\"type\":\"overloaded_error\",\"message\":\"busy\"}}");
    let server = FakeProvider::start(vec![overloaded(), overloaded(), overloaded(), overloaded()]).await;
    let provider =
        AnthropicProvider::new(&anthropic_config(&server.base_url()), Some("sk-ant-test")).expect("a provider");

    let error = provider.stream(request()).await.err().expect("it never answers");

    assert_eq!(error, ProviderError::Overloaded);
    assert_eq!(server.request_count(), 3, "the first try and two retries, and no more");
}

#[tokio::test]
async fn an_over_long_conversation_is_recognised_from_the_body_not_the_status() {
    let server = FakeProvider::start(vec![Reply::status(
        400,
        "{\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"prompt is too long: 250000 tokens > 200000 maximum\"}}",
    )])
    .await;
    let provider =
        AnthropicProvider::new(&anthropic_config(&server.base_url()), Some("sk-ant-test")).expect("a provider");

    let error = provider.stream(request()).await.err().expect("the request is refused");

    assert_eq!(error, ProviderError::ContextTooLong);
    assert_eq!(error.code(), "AI-CONTEXT-TOO-LONG");
    assert_eq!(
        server.request_count(),
        1,
        "shortening the conversation is the person's move, not a retry"
    );
}

#[tokio::test]
async fn tool_arguments_that_are_not_json_end_the_stream() {
    let broken = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":5}}}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"t1\",\"name\":\"sshell_exec_run\",\"input\":{}}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"command\\\":\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
    );
    let server = FakeProvider::start(vec![Reply::whole(broken)]).await;
    let provider =
        AnthropicProvider::new(&anthropic_config(&server.base_url()), Some("sk-ant-test")).expect("a provider");

    let mut stream = provider.stream(request()).await.expect("a stream");
    let mut failure = None;
    while let Some(item) = stream.next().await {
        if let Err(error) = item {
            failure = Some(error);
        }
    }

    assert!(
        matches!(failure, Some(ProviderError::Protocol(_))),
        "half an argument object is not a call: {failure:?}"
    );
}

#[tokio::test]
async fn an_acknowledged_request_that_never_says_anything_is_not_an_answer() {
    // A provider that accepts the request and closes the body without an event at all: the message
    // never finished, and reporting an empty turn would look like a model with nothing to say.
    let server = FakeProvider::start(vec![Reply::whole("")]).await;
    let provider =
        AnthropicProvider::new(&anthropic_config(&server.base_url()), Some("sk-ant-test")).expect("a provider");

    let mut stream = provider.stream(request()).await.expect("a stream");
    let mut failure = None;
    while let Some(item) = stream.next().await {
        if let Err(error) = item {
            failure = Some(error);
        }
    }

    assert!(matches!(failure, Some(ProviderError::Protocol(_))), "{failure:?}");
}

#[tokio::test]
async fn a_provider_that_cannot_be_trusted_with_a_key_is_refused_before_a_request() {
    for base_url in [
        "http://gateway.example/v1",
        "http://192.168.1.10:8080",
        "ftp://api.example",
    ] {
        let error = AnthropicProvider::new(&anthropic_config(base_url), Some("sk-ant-test"))
            .err()
            .unwrap_or_else(|| panic!("`{base_url}` must be refused"));

        assert!(matches!(error, ProviderError::Protocol(_)), "`{base_url}` -> {error:?}");
    }

    let error =
        AnthropicProvider::new(&anthropic_config("https://api.anthropic.com"), None).expect_err("a key is required");
    assert_eq!(error, ProviderError::NoKey);
    assert_eq!(error.code(), "AI-NO-KEY");
}

#[tokio::test]
async fn openai_sends_the_system_message_the_tools_and_the_usage_request() {
    let server = FakeProvider::start(vec![Reply::stream(in_parts(OPENAI_TEXT, 29))]).await;
    let provider = OpenAiProvider::new(
        &openai_config(&format!("{}/v1", server.base_url())),
        Some("sk-live-test"),
    )
    .expect("a provider");

    let events = events_of(provider.stream(request()).await.expect("a stream")).await;

    assert_eq!(text_of(&events), "The disk is 43% full.");
    assert_eq!(
        usage_of(&events),
        Some(Usage {
            input_tokens: 31,
            output_tokens: 8,
            cache_read_tokens: Some(16),
        })
    );
    assert_eq!(last_stop(&events), StopReason::EndTurn);

    let sent = &server.requests()[0];
    assert_eq!(sent.path(), "/v1/chat/completions");
    assert_eq!(sent.header("authorization"), Some("Bearer sk-live-test"));
    assert_eq!(sent.header("content-type"), Some("application/json"));
    assert_eq!(sent.header("accept"), Some("text/event-stream"));

    let body = sent.body_json();
    assert_eq!(body["model"], json!("gpt-4o"));
    assert_eq!(body["stream"], json!(true));
    assert_eq!(body["stream_options"]["include_usage"], json!(true));
    assert_eq!(
        body["messages"][0],
        json!({ "role": "system", "content": "You are an operations assistant." })
    );
    assert_eq!(
        body["messages"][1],
        json!({ "role": "user", "content": "how full is the disk?" })
    );
    assert_eq!(
        body["messages"][3],
        json!({ "role": "tool", "tool_call_id": "call_1", "content": "43% used" }),
        "a tool result is its own message here"
    );
    assert_eq!(body["tools"][0]["type"], json!("function"));
    assert_eq!(body["tools"][0]["function"]["name"], json!("sshell_exec_run"));
}

#[tokio::test]
async fn openai_assembles_tool_calls_and_stops_for_the_tool() {
    let server = FakeProvider::start(vec![Reply::stream(in_parts(OPENAI_TOOL, 17))]).await;
    let provider = OpenAiProvider::new(
        &openai_config(&format!("{}/v1", server.base_url())),
        Some("sk-live-test"),
    )
    .expect("a provider");

    let events = events_of(provider.stream(request()).await.expect("a stream")).await;

    assert_eq!(text_of(&events), "Let me look.");
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StreamEvent::ToolUseStart { .. }))
            .count(),
        1,
        "a call is announced once, not once per fragment: {events:?}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StreamEvent::ToolInputDelta { .. }))
            .count(),
        2,
        "two argument fragments arrived"
    );

    let done: Vec<_> = events
        .iter()
        .filter(|event| matches!(event, StreamEvent::ToolUseDone { .. }))
        .collect();
    assert_eq!(
        done,
        vec![&StreamEvent::ToolUseDone {
            index: 1,
            id: "call_01".to_owned(),
            name: "sshell_exec_run".to_owned(),
            input: json!({ "command": "df -h" }),
        }]
    );
    assert_eq!(last_stop(&events), StopReason::ToolUse);
}

#[tokio::test]
async fn a_local_server_that_says_nothing_after_the_last_chunk_still_finishes_the_turn() {
    // No `[DONE]`, no usage, and a `finish_reason` of null throughout: what a compatible server that
    // simply hangs up looks like. The answer is complete, so the turn is.
    let sse = concat!(
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"43% full\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\".\"},\"finish_reason\":null}]}\n\n",
    );
    let server = FakeProvider::start(vec![Reply::stream(in_parts(sse, 23))]).await;
    let provider = OpenAiProvider::new(&local_config(&format!("{}/v1", server.base_url())), None)
        .expect("a local server needs no key");

    let events = events_of(provider.stream(request()).await.expect("a stream")).await;

    assert_eq!(text_of(&events), "43% full.");
    assert_eq!(last_stop(&events), StopReason::EndTurn);
    assert_eq!(
        usage_of(&events),
        None,
        "a server that reports no usage is not a failure"
    );
    assert!(
        server.requests()[0].header("authorization").is_none(),
        "a keyless provider sends no bearer header at all"
    );
    assert_eq!(provider.kind(), AiProviderKind::Local);
}

#[tokio::test]
async fn an_openai_error_body_is_classified_from_what_it_says() {
    let server = FakeProvider::start(vec![Reply::status(
        400,
        "{\"error\":{\"message\":\"This model's maximum context length is 8192 tokens.\",\"type\":\"invalid_request_error\",\"code\":\"context_length_exceeded\"}}",
    )])
    .await;
    let provider = OpenAiProvider::new(
        &openai_config(&format!("{}/v1", server.base_url())),
        Some("sk-live-test"),
    )
    .expect("a provider");

    let error = provider.stream(request()).await.err().expect("the request is refused");

    assert_eq!(error, ProviderError::ContextTooLong);
}

#[tokio::test]
async fn a_stream_that_is_not_json_ends_in_a_protocol_failure() {
    let server = FakeProvider::start(vec![Reply::whole("data: this is not JSON\n\ndata: [DONE]\n\n")]).await;
    let provider = OpenAiProvider::new(
        &openai_config(&format!("{}/v1", server.base_url())),
        Some("sk-live-test"),
    )
    .expect("a provider");

    let mut stream = provider.stream(request()).await.expect("a stream");
    let mut failure = None;
    while let Some(item) = stream.next().await {
        if let Err(error) = item {
            failure = Some(error);
        }
    }

    assert!(matches!(failure, Some(ProviderError::Protocol(_))), "{failure:?}");
}

#[tokio::test]
async fn dropping_the_stream_closes_the_connection_instead_of_reading_on() {
    let server = FakeProvider::start(vec![Reply::held(
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"one\"},\"finish_reason\":null}]}\n\n",
    )])
    .await;
    let provider = OpenAiProvider::new(
        &openai_config(&format!("{}/v1", server.base_url())),
        Some("sk-live-test"),
    )
    .expect("a provider");

    let mut stream = provider.stream(request()).await.expect("a stream");
    let first = stream.next().await.expect("one event").expect("an event");
    assert_eq!(
        first,
        StreamEvent::TextDelta {
            index: 0,
            text: "one".to_owned(),
        }
    );

    drop(stream);

    // Nothing else will ever be written, so the only way this resolves is the client's side of the
    // connection going away: cancellation is the drop, and it really does close the socket.
    tokio::time::timeout(Duration::from_secs(5), server.wait_for_close())
        .await
        .expect("dropping the stream must close the connection");
}
