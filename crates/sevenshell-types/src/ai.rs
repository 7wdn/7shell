//! The assistant's provider configuration, as the interface may see it.
//!
//! Nothing here can hold a key. The non-secret half of a provider — where it is, which model it
//! serves, which protocol it speaks — lives in the settings file; the key lives behind the vault's
//! lock and reaches the interface only as [`AiProviderView::has_key`], which is `None` while the
//! vault is locked because "cannot tell right now" is not "there is none".
//!
//! The kinds are the protocols this build speaks: Anthropic's Messages API, OpenAI's Chat
//! Completions, anything that serves the same shape locally (Ollama, vLLM, LM Studio), and OpenAI's
//! Responses API. The last one is named after its own address rather than as "another OpenAI",
//! because the two are different endpoints with different streaming events — a settings file that
//! said only `openai` would send a Responses model to the wrong one.

use serde::{Deserialize, Serialize};

/// How many lines of terminal output travel with a question when nobody has chosen a number.
///
/// Eighty is about a screen and a half: enough for a command and its error, little enough that a
/// conversation is not mostly a terminal transcript.
pub const AI_CONTEXT_LINES_DEFAULT: u32 = 80;

/// The ends of the number of lines the assistant may send.
///
/// Zero is a real answer — it means "do not attach the terminal at all" — and the ceiling is what
/// keeps a question from arriving with an arbitrary amount of somebody's screen in it.
pub const AI_CONTEXT_LINES_MIN: u32 = 0;
pub const AI_CONTEXT_LINES_MAX: u32 = 200;

/// The output budget a provider entry gets when the file does not name one.
///
/// A generous but finite default: the provider still clamps it to the model's own limit, and a
/// request with no limit at all is one that can run until the account is empty.
pub const AI_MAX_OUTPUT_TOKENS_DEFAULT: u32 = 4_096;

pub fn ai_context_lines_default() -> u32 {
    AI_CONTEXT_LINES_DEFAULT
}

/// Whether the terminal's recent lines travel with a question.
///
/// On unless somebody turns it off: an assistant that cannot see what the person is looking at is
/// answering in the dark, and the preview in the chat shows the exact text before it is sent.
pub fn ai_context_enabled_default() -> bool {
    true
}

pub fn ai_max_output_tokens_default() -> u32 {
    AI_MAX_OUTPUT_TOKENS_DEFAULT
}

/// Keeps a line count inside the range this build offers.
///
/// Zero stays zero: it is not a missing value but the answer "attach nothing", which is what a
/// person chooses when they want a question with no terminal in it.
pub fn normalize_ai_context_lines(lines: u32) -> u32 {
    lines.clamp(AI_CONTEXT_LINES_MIN, AI_CONTEXT_LINES_MAX)
}

/// Which API a provider speaks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiProviderKind {
    /// Anthropic's Messages API.
    #[default]
    Anthropic,
    /// OpenAI's Chat Completions API.
    OpenAi,
    /// A server on this machine or the network that speaks the OpenAI shape.
    Local,
    /// OpenAI's Responses API (`/v1/responses`).
    ///
    /// Its own word rather than a fourth spelling of `openai`, and the one variant whose wire name
    /// is not lower case: the address it lives at is `/responses`, and a value the settings page
    /// compares against has to say which of the two OpenAI-shaped APIs is meant. An older build
    /// reads it as a name it does not know, which is a provider it does not offer — never a request
    /// sent to the other endpoint.
    #[serde(rename = "openaiResponses")]
    OpenAiResponses,
}

impl AiProviderKind {
    pub const ALL: &'static [AiProviderKind] = &[
        AiProviderKind::Anthropic,
        AiProviderKind::OpenAi,
        AiProviderKind::Local,
        AiProviderKind::OpenAiResponses,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::Local => "local",
            Self::OpenAiResponses => "openaiResponses",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.as_str() == id)
    }
}

/// One configured provider, without its key.
///
/// `id` is what the vault files the key under and what the chat names when it pins a provider; it is
/// deliberately opaque. `base_url` is where requests go, so an OpenAI-compatible server and the
/// hosted one are the same kind with two addresses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderConfig {
    pub id: String,
    pub name: String,
    pub kind: AiProviderKind,
    pub base_url: String,
    pub model: String,
    #[serde(default = "ai_max_output_tokens_default")]
    pub max_output_tokens: u32,
}

impl AiProviderConfig {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        kind: AiProviderKind,
        base_url: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind,
            base_url: base_url.into(),
            model: model.into(),
            max_output_tokens: ai_max_output_tokens_default(),
        }
    }
}

/// One provider as the settings page draws it: the configuration, plus whether a key is held.
///
/// Flattened rather than nested so the view is the configuration with one more fact. `has_key` is
/// `None`, and serializes as `null`, while the vault is locked: the interface must not draw "no key"
/// for a question it could not answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderView {
    #[serde(flatten)]
    pub config: AiProviderConfig,
    #[serde(default)]
    pub has_key: Option<bool>,
}

impl AiProviderView {
    pub fn new(config: AiProviderConfig, has_key: Option<bool>) -> Self {
        Self { config, has_key }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AiProviderConfig {
        AiProviderConfig::new(
            "ollama",
            "Ollama",
            AiProviderKind::Local,
            "http://127.0.0.1:11434/v1",
            "llama3.1",
        )
    }

    #[test]
    fn every_provider_kind_round_trips_through_its_own_name() {
        for kind in AiProviderKind::ALL {
            assert_eq!(
                AiProviderKind::from_id(kind.as_str()),
                Some(*kind),
                "a kind whose name does not read back is one the settings file cannot hold"
            );
        }
    }

    #[test]
    fn the_responses_kind_serializes_as_the_address_it_lives_at() {
        // The one kind whose wire name is not lower case, and the one a lower-case rule would get
        // wrong: `openairesponses` is a value the settings page cannot match against anything, and
        // the provider would read as one this build does not offer.
        assert_eq!(
            serde_json::to_value(AiProviderKind::OpenAiResponses).expect("a kind is JSON"),
            serde_json::json!("openaiResponses")
        );
        assert_eq!(
            serde_json::from_value::<AiProviderKind>(serde_json::json!("openaiResponses")).expect("it reads back"),
            AiProviderKind::OpenAiResponses
        );
        assert_eq!(
            AiProviderKind::OpenAiResponses.as_str(),
            "openaiResponses",
            "the stored name and the wire name are one name"
        );
    }

    #[test]
    fn a_provider_round_trips_without_a_field_for_a_key() {
        let config = sample();
        let json = serde_json::to_value(&config).expect("a provider is JSON");

        assert_eq!(json["baseUrl"], serde_json::json!("http://127.0.0.1:11434/v1"));
        assert_eq!(json["maxOutputTokens"], serde_json::json!(AI_MAX_OUTPUT_TOKENS_DEFAULT));
        for forbidden in ["key", "apiKey", "hasKey", "token", "secret"] {
            assert!(
                json.get(forbidden).is_none(),
                "`{forbidden}` is a field a provider configuration must not have: {json}"
            );
        }

        let back: AiProviderConfig = serde_json::from_value(json.clone()).expect("round trip");
        assert_eq!(back, config);
        assert_eq!(serde_json::to_value(&back).expect("again"), json);
    }

    #[test]
    fn a_view_that_could_not_ask_says_null_rather_than_none() {
        let view = AiProviderView::new(sample(), None);
        let json = serde_json::to_value(&view).expect("a view is JSON");

        assert!(
            json["hasKey"].is_null(),
            "a locked vault must not draw as `no key`: {json}"
        );
        assert_eq!(
            json["id"],
            serde_json::json!("ollama"),
            "the configuration is flattened"
        );

        let held = serde_json::to_value(AiProviderView::new(sample(), Some(true))).expect("JSON");
        assert_eq!(held["hasKey"], serde_json::json!(true));
    }

    #[test]
    fn context_lines_are_clamped_but_zero_still_means_nothing_is_attached() {
        assert_eq!(normalize_ai_context_lines(0), 0);
        assert_eq!(normalize_ai_context_lines(900), AI_CONTEXT_LINES_MAX);
        assert_eq!(normalize_ai_context_lines(120), 120);
        assert_eq!(ai_context_lines_default(), AI_CONTEXT_LINES_DEFAULT);
        assert!(
            ai_context_enabled_default(),
            "the terminal travels until somebody says otherwise"
        );
    }
}
