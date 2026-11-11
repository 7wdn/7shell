//! A real OpenAI-compatible gateway, when one is configured.
//!
//! Ignored by default: the gateway address, key and model are a machine's own, and a test that
//! needed them would make every other machine fail. Run it with:
//!
//! ```text
//! SEVENSHELL_TEST_AI_BASE_URL=http://127.0.0.1:3425/v1 \
//! SEVENSHELL_TEST_AI_KEY=... \
//! SEVENSHELL_TEST_AI_MODEL=group/auto-deepseek-v4.1-flash \
//!   cargo test -p sevenshell-ai --test live_gateway -- --ignored --test-threads=1
//! ```
//!
//! The key is read from the environment and never printed. What this proves is the adapter, not
//! the model: a real server's SSE frames, its reasoning channel, its finish chunk and its usage.

use futures_util::StreamExt as _;
use sevenshell_ai::{
    AiProviderConfig, AiProviderKind, ChatMessage, ProviderRequest, StreamEvent, ToolSpec, build_provider,
};

fn setting(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

#[tokio::test]
#[ignore = "needs SEVENSHELL_TEST_AI_BASE_URL / _KEY / _MODEL"]
async fn a_real_openai_compatible_gateway_streams_an_answer() {
    let Some(base_url) = setting("SEVENSHELL_TEST_AI_BASE_URL") else {
        eprintln!("skipped: SEVENSHELL_TEST_AI_BASE_URL is not set");
        return;
    };
    let Some(key) = setting("SEVENSHELL_TEST_AI_KEY") else {
        eprintln!("skipped: SEVENSHELL_TEST_AI_KEY is not set");
        return;
    };
    let Some(model) = setting("SEVENSHELL_TEST_AI_MODEL") else {
        eprintln!("skipped: SEVENSHELL_TEST_AI_MODEL is not set");
        return;
    };

    let config = AiProviderConfig {
        id: "gateway".to_owned(),
        name: "Gateway".to_owned(),
        kind: AiProviderKind::OpenAi,
        base_url,
        model: model.clone(),
        max_output_tokens: 64,
    };
    let provider = build_provider(&config, Some(&key)).expect("the gateway configuration is usable");
    let request = ProviderRequest {
        model,
        system: None,
        messages: vec![ChatMessage::user("Reply with the single word: pong")],
        tools: Vec::new(),
        max_tokens: 32,
        temperature: None,
    };

    let mut stream = provider
        .stream(request)
        .await
        .expect("the gateway accepted the request");
    let mut answer = String::new();
    let mut saw_thinking = false;
    let mut saw_stop = false;

    while let Some(event) = stream.next().await {
        match event.expect("the gateway sent a readable event") {
            StreamEvent::TextDelta { text, .. } => answer.push_str(&text),
            StreamEvent::ThinkingDelta { .. } => saw_thinking = true,
            StreamEvent::Stop { .. } => {
                saw_stop = true;
                break;
            }
            StreamEvent::Usage(_)
            | StreamEvent::ToolUseStart { .. }
            | StreamEvent::ToolInputDelta { .. }
            | StreamEvent::ToolUseDone { .. } => {}
        }
    }

    assert!(saw_stop, "the stream has to end with a stop event");
    assert!(
        answer.to_ascii_lowercase().contains("pong"),
        "the model answered `{answer}` instead of the requested word"
    );
    // The reasoning channel is optional per model; the assertion is that a server which sends it
    // does not break the adapter, not that every model has one.
    let _ = saw_thinking;
}

#[tokio::test]
#[ignore = "needs SEVENSHELL_TEST_AI_BASE_URL / _KEY / _MODEL"]
async fn a_real_openai_compatible_gateway_requests_a_tool() {
    let Some(base_url) = setting("SEVENSHELL_TEST_AI_BASE_URL") else {
        eprintln!("skipped: SEVENSHELL_TEST_AI_BASE_URL is not set");
        return;
    };
    let Some(key) = setting("SEVENSHELL_TEST_AI_KEY") else {
        eprintln!("skipped: SEVENSHELL_TEST_AI_KEY is not set");
        return;
    };
    let Some(model) = setting("SEVENSHELL_TEST_AI_MODEL") else {
        eprintln!("skipped: SEVENSHELL_TEST_AI_MODEL is not set");
        return;
    };

    let config = AiProviderConfig {
        id: "gateway".to_owned(),
        name: "Gateway".to_owned(),
        kind: AiProviderKind::OpenAi,
        base_url,
        model: model.clone(),
        max_output_tokens: 256,
    };
    let provider = build_provider(&config, Some(&key)).expect("the gateway configuration is usable");
    let request = ProviderRequest {
        model,
        system: Some("You are a tool-calling test. Use the supplied tool instead of answering in text.".to_owned()),
        messages: vec![ChatMessage::user("Use test_echo to echo the word hello.")],
        tools: vec![ToolSpec {
            name: "test_echo".to_owned(),
            description: "Echo the supplied text back.".to_owned(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"],
                "additionalProperties": false
            }),
        }],
        max_tokens: 256,
        temperature: None,
    };

    let mut stream = provider
        .stream(request)
        .await
        .expect("the gateway accepted the request");
    let mut call = None;
    while let Some(event) = stream.next().await {
        match event.expect("the gateway sent a readable event") {
            StreamEvent::ToolUseDone { name, input, .. } => {
                call = Some((name, input));
                break;
            }
            StreamEvent::Stop { .. } => break,
            _ => {}
        }
    }

    let (name, input) = call.expect("the model should ask for the supplied tool");
    assert_eq!(name, "test_echo");
    assert_eq!(input["text"], "hello", "the arguments were assembled from the stream");
}
