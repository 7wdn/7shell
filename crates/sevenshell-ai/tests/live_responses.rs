//! A real Responses endpoint, when one is configured.
//!
//! Ignored by default: the address, key and model are a machine's own, and a test that needed them
//! would make every other machine fail. Many gateways speak Chat Completions and not the Responses
//! API, so the three settings being absent is the normal case rather than a broken checkout. Run it
//! with:
//!
//! ```text
//! SEVENSHELL_TEST_AI_RESPONSES_BASE_URL=https://api.openai.com/v1 \
//! SEVENSHELL_TEST_AI_RESPONSES_KEY=... \
//! SEVENSHELL_TEST_AI_RESPONSES_MODEL=gpt-4.1 \
//!   cargo test -p sevenshell-ai --test live_responses -- --ignored --test-threads=1
//! ```
//!
//! The key is read from the environment and never printed. What this proves is the adapter — the
//! endpoint path, the `input` items, the `response.*` frames and the usage block — not the model.

use futures_util::StreamExt as _;
use sevenshell_ai::{
    AiProviderConfig, AiProviderKind, ChatMessage, ProviderRequest, StopReason, StreamEvent, build_provider,
};

fn setting(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

#[tokio::test]
#[ignore = "needs SEVENSHELL_TEST_AI_RESPONSES_BASE_URL / _KEY / _MODEL"]
async fn a_real_responses_endpoint_streams_an_answer() {
    let Some(base_url) = setting("SEVENSHELL_TEST_AI_RESPONSES_BASE_URL") else {
        eprintln!("skipped: SEVENSHELL_TEST_AI_RESPONSES_BASE_URL is not set");
        return;
    };
    let Some(key) = setting("SEVENSHELL_TEST_AI_RESPONSES_KEY") else {
        eprintln!("skipped: SEVENSHELL_TEST_AI_RESPONSES_KEY is not set");
        return;
    };
    let Some(model) = setting("SEVENSHELL_TEST_AI_RESPONSES_MODEL") else {
        eprintln!("skipped: SEVENSHELL_TEST_AI_RESPONSES_MODEL is not set");
        return;
    };

    let config = AiProviderConfig {
        id: "responses".to_owned(),
        name: "Responses".to_owned(),
        kind: AiProviderKind::OpenAiResponses,
        base_url,
        model: model.clone(),
        max_output_tokens: 64,
    };
    let provider = build_provider(&config, Some(&key)).expect("the endpoint configuration is usable");
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
        .expect("the endpoint accepted the request");
    let mut answer = String::new();
    let mut stop = None;

    while let Some(event) = stream.next().await {
        match event.expect("the endpoint sent a readable event") {
            StreamEvent::TextDelta { text, .. } => answer.push_str(&text),
            StreamEvent::Stop { reason } => {
                stop = Some(reason);
                break;
            }
            StreamEvent::ThinkingDelta { .. }
            | StreamEvent::Usage(_)
            | StreamEvent::ToolUseStart { .. }
            | StreamEvent::ToolInputDelta { .. }
            | StreamEvent::ToolUseDone { .. } => {}
        }
    }

    assert_eq!(
        stop,
        Some(StopReason::EndTurn),
        "the stream has to end with the stop the completed response named"
    );
    assert!(
        answer.to_ascii_lowercase().contains("pong"),
        "the model answered `{answer}` instead of the requested word"
    );
}
