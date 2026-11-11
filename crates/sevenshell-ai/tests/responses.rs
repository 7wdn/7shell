//! The Responses adapter against a provider that lives on this machine.
//!
//! Every test here starts a real listener and speaks real HTTP to it, because the parts that go
//! wrong between an adapter and a provider are the wire parts: the endpoint path, the header the key
//! travels in, the `input` items a conversation becomes, events that straddle chunk boundaries, the
//! usage the provider only reports at the very end, and a stream that must stop reading when the
//! caller walks away. Nothing here reaches the internet, and nothing here needs a key that works.

mod support;

use std::time::Duration;

use futures_util::StreamExt;
use serde_json::json;
use sevenshell_ai::{
    AiProviderConfig, AiProviderKind, ChatMessage, ContentBlock, OpenAiResponsesProvider, Provider, ProviderError,
    ProviderRequest, ProviderStream, StopReason, StreamEvent, ToolSpec, Usage,
};
use support::{FakeProvider, Reply};

const RESPONSES_TEXT: &str = include_str!("fixtures/responses/text.sse");
const RESPONSES_TOOL: &str = include_str!("fixtures/responses/tool.sse");
const RESPONSES_USAGE: &str = include_str!("fixtures/responses/usage.sse");
const RESPONSES_INCOMPLETE: &str = include_str!("fixtures/responses/incomplete.sse");
const RESPONSES_ERROR: &str = include_str!("fixtures/responses/error.sse");

fn responses_config(base_url: &str) -> AiProviderConfig {
    AiProviderConfig::new(
        "responses",
        "OpenAI Responses",
        AiProviderKind::OpenAiResponses,
        base_url,
        "gpt-4.1",
    )
}

fn provider(server: &FakeProvider) -> OpenAiResponsesProvider {
    OpenAiResponsesProvider::new(
        &responses_config(&format!("{}/v1", server.base_url())),
        Some("sk-live-test"),
    )
    .expect("a provider")
}

/// One request that exercises every part of the conversation this protocol spells its own way.
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

async fn failure_of(stream: ProviderStream) -> Option<ProviderError> {
    let mut stream = stream;
    let mut failure = None;
    while let Some(item) = stream.next().await {
        if let Err(error) = item {
            failure = Some(error);
        }
    }
    failure
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

fn thinking_of(events: &[StreamEvent]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::ThinkingDelta { text, .. } => Some(text.as_str()),
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
async fn responses_sends_the_instructions_input_and_tools_the_api_expects_and_reads_the_answer_back() {
    let server = FakeProvider::start(vec![Reply::stream(in_parts(RESPONSES_TEXT, 37))]).await;
    let provider = provider(&server);

    let events = events_of(provider.stream(request()).await.expect("a stream")).await;

    assert_eq!(text_of(&events), "The disk is 43% full.");
    assert_eq!(
        usage_of(&events),
        Some(Usage {
            input_tokens: 25,
            output_tokens: 9,
            cache_read_tokens: Some(12),
        })
    );
    assert_eq!(last_stop(&events), StopReason::EndTurn);

    let requests = server.requests();
    assert_eq!(requests.len(), 1, "one request, no retry");
    let sent = &requests[0];

    assert_eq!(sent.method(), "POST");
    assert_eq!(
        sent.path(),
        "/v1/responses",
        "the Responses endpoint, not chat/completions"
    );
    assert_eq!(sent.header("authorization"), Some("Bearer sk-live-test"));
    assert_eq!(sent.header("content-type"), Some("application/json"));
    assert_eq!(sent.header("accept"), Some("text/event-stream"));

    let body = sent.body_json();
    assert_eq!(
        body["model"],
        json!("gpt-4.1"),
        "an empty model means the configured one"
    );
    assert_eq!(
        body["instructions"],
        json!("You are an operations assistant."),
        "the system prompt is its own field here"
    );
    assert_eq!(body["max_output_tokens"], json!(1_024));
    assert_eq!(body["stream"], json!(true));
    assert_eq!(
        body["store"],
        json!(false),
        "the conversation belongs to this application"
    );
    assert!(
        body.get("messages").is_none(),
        "this API takes `input` items, not messages"
    );
    assert!(
        body.get("max_tokens").is_none(),
        "the budget field is named `max_output_tokens` here"
    );
    assert_eq!(
        body["input"][0],
        json!({ "role": "user", "content": [{ "type": "input_text", "text": "how full is the disk?" }] })
    );
    assert_eq!(body["input"][1]["type"], json!("function_call"));
    assert_eq!(body["input"][1]["call_id"], json!("call_1"));
    assert_eq!(body["input"][1]["name"], json!("sshell_exec_run"));
    assert_eq!(
        body["input"][2],
        json!({ "type": "function_call_output", "call_id": "call_1", "output": "43% used" }),
        "a tool result is its own item, not a message of its own"
    );
    assert_eq!(body["tools"][0]["type"], json!("function"));
    assert_eq!(body["tools"][0]["name"], json!("sshell_exec_run"));
    assert_eq!(
        body["tools"][0]["parameters"]["properties"]["command"]["type"],
        json!("string")
    );
    assert!(
        body.get("temperature").is_none(),
        "no temperature was asked for, so none is sent"
    );
}

#[tokio::test]
async fn responses_reports_thinking_and_assembles_a_function_call_from_fragments() {
    let server = FakeProvider::start(vec![Reply::stream(in_parts(RESPONSES_TOOL, 29))]).await;
    let provider = provider(&server);

    let events = events_of(provider.stream(request()).await.expect("a stream")).await;

    assert_eq!(
        thinking_of(&events),
        "The person asks about the disk. df -h answers that.",
        "reasoning is thinking, not part of the answer"
    );
    assert_eq!(text_of(&events), "Let me look.");

    let starts: Vec<_> = events
        .iter()
        .filter(|event| matches!(event, StreamEvent::ToolUseStart { .. }))
        .collect();
    assert_eq!(
        starts,
        vec![&StreamEvent::ToolUseStart {
            index: 2,
            id: "call_1".to_owned(),
            name: "sshell_exec_run".to_owned(),
        }],
        "a call is announced once, at the output index the provider gave it"
    );

    let fragments: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::ToolInputDelta { json, .. } => Some(json.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        fragments,
        vec!["{\"command\":", " \"df -h\"}"],
        "each fragment is reported as it arrived"
    );

    let done: Vec<_> = events
        .iter()
        .filter(|event| matches!(event, StreamEvent::ToolUseDone { .. }))
        .collect();
    assert_eq!(
        done,
        vec![&StreamEvent::ToolUseDone {
            index: 2,
            id: "call_1".to_owned(),
            name: "sshell_exec_run".to_owned(),
            input: json!({ "command": "df -h" }),
        }],
        "the arguments are parsed once the call closes, not before"
    );
    assert_eq!(last_stop(&events), StopReason::ToolUse);
}

#[tokio::test]
async fn responses_reports_the_cached_prompt_tokens_it_was_given() {
    let server = FakeProvider::start(vec![Reply::whole(RESPONSES_USAGE)]).await;
    let provider = provider(&server);

    let events = events_of(provider.stream(request()).await.expect("a stream")).await;

    assert_eq!(text_of(&events), "It is 43% full.");
    assert_eq!(
        usage_of(&events),
        Some(Usage {
            input_tokens: 31,
            output_tokens: 8,
            cache_read_tokens: Some(16),
        }),
        "the cache read is under input_tokens_details, not at the top level"
    );
    assert_eq!(last_stop(&events), StopReason::EndTurn);
}

#[tokio::test]
async fn an_incomplete_response_stops_for_the_reason_it_names() {
    let server = FakeProvider::start(vec![Reply::stream(in_parts(RESPONSES_INCOMPLETE, 23))]).await;
    let provider = provider(&server);

    let events = events_of(provider.stream(request()).await.expect("a stream")).await;

    assert_eq!(
        text_of(&events),
        "The disk is 43",
        "what arrived before the cut is still reported"
    );
    assert_eq!(
        last_stop(&events),
        StopReason::MaxTokens,
        "a response cut by its output budget is a truncated answer, not a finished one"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StreamEvent::Stop { .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn a_rate_limited_responses_request_is_tried_again_once_and_then_streams() {
    // The retry rule is the shared one: nothing has been answered yet, so trying again is safe and
    // the request that is repeated is the same body, not a rebuilt one.
    let server = FakeProvider::start(vec![
        Reply::status(
            429,
            "{\"error\":{\"code\":\"rate_limit_exceeded\",\"message\":\"slow down\"}}",
        )
        .with_header("retry-after", "0"),
        Reply::whole(RESPONSES_TEXT),
    ])
    .await;
    let provider = provider(&server);

    let events = events_of(provider.stream(request()).await.expect("a stream")).await;

    assert_eq!(text_of(&events), "The disk is 43% full.");
    assert_eq!(server.request_count(), 2, "one refusal and one successful attempt");

    let requests = server.requests();
    assert_eq!(
        requests[0].body, requests[1].body,
        "a retried request is the same request"
    );
    assert_eq!(requests[1].header("authorization"), Some("Bearer sk-live-test"));
}

#[tokio::test]
async fn a_failed_response_is_the_failure_it_names() {
    let server = FakeProvider::start(vec![Reply::stream(in_parts(RESPONSES_ERROR, 31))]).await;
    let provider = provider(&server);

    let failure = failure_of(provider.stream(request()).await.expect("a stream")).await;

    assert_eq!(
        failure,
        Some(ProviderError::ContextTooLong),
        "the error code is what separates an over-long conversation from a broken request"
    );
    assert_eq!(failure.expect("a failure").code(), "AI-CONTEXT-TOO-LONG");
}

#[tokio::test]
async fn an_error_event_mid_stream_is_the_failure_it_names() {
    let sse = concat!(
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"status\":\"in_progress\"}}\n\n",
        "event: error\n",
        "data: {\"type\":\"error\",\"code\":\"rate_limit_exceeded\",\"message\":\"slow down\"}\n\n",
    );
    let server = FakeProvider::start(vec![Reply::whole(sse)]).await;
    let provider = provider(&server);

    let failure = failure_of(provider.stream(request()).await.expect("a stream")).await;

    assert_eq!(
        failure,
        Some(ProviderError::RateLimited { retry_after_ms: None }),
        "an error event is a failure reported through the stream, not an end of the answer"
    );
}

#[tokio::test]
async fn a_body_that_ends_without_a_completed_response_is_not_a_finished_answer() {
    // The provider accepts the request, streams half a sentence and hangs up. Presenting that as a
    // finished turn would hide the truncation behind an answer that just stops.
    let sse = concat!(
        "event: response.output_item.added\n",
        "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[]}}\n\n",
        "event: response.output_text.delta\n",
        "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg_1\",\"output_index\":0,\"delta\":\"The disk is 43%\"}\n\n",
    );
    let server = FakeProvider::start(vec![Reply::whole(sse)]).await;
    let provider = provider(&server);

    let failure = failure_of(provider.stream(request()).await.expect("a stream")).await;

    assert!(
        matches!(failure, Some(ProviderError::Protocol(_))),
        "a stream that never said `response.completed` was cut off: {failure:?}"
    );
}

#[tokio::test]
async fn the_done_sentinel_some_gateways_append_is_tolerated() {
    let with_sentinel = format!("{RESPONSES_TEXT}\ndata: [DONE]\n\n");
    let server = FakeProvider::start(vec![Reply::stream(in_parts(&with_sentinel, 41))]).await;
    let provider = provider(&server);

    let events = events_of(provider.stream(request()).await.expect("a stream")).await;

    assert_eq!(
        text_of(&events),
        "The disk is 43% full.",
        "the sentinel says nothing the completed event did not already say"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StreamEvent::Stop { .. }))
            .count(),
        1,
        "the turn ends once: {events:?}"
    );
}

#[tokio::test]
async fn a_function_call_without_its_arguments_is_dropped_rather_than_run() {
    // The call is announced, and no argument document ever arrives — neither fragments nor a
    // complete one on the close. Running it with invented `{}` would run something the model never
    // asked for, so the call is dropped and the turn ends the way a text turn does.
    let sse = concat!(
        "event: response.output_item.added\n",
        "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"call_id\":\"call_9\",\"name\":\"sshell_sessions_list\"}}\n\n",
        "event: response.output_item.done\n",
        "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"call_id\":\"call_9\",\"name\":\"sshell_sessions_list\"}}\n\n",
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":3,\"output_tokens\":1}}}\n\n",
    );
    let server = FakeProvider::start(vec![Reply::whole(sse)]).await;
    let provider = provider(&server);

    let events = events_of(provider.stream(request()).await.expect("a stream")).await;

    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::ToolUseStart { .. })),
        "the call was announced before it could be judged: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, StreamEvent::ToolUseDone { .. })),
        "a call with no arguments cannot be run: {events:?}"
    );
    assert_eq!(
        last_stop(&events),
        StopReason::EndTurn,
        "with the call dropped there is no tool to answer"
    );
}

#[tokio::test]
async fn dropping_the_stream_closes_the_connection_instead_of_reading_on() {
    let server = FakeProvider::start(vec![Reply::held(
        "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"delta\":\"one\"}\n\n",
    )])
    .await;
    let provider = provider(&server);

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
