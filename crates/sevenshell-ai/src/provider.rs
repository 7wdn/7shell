//! What a provider adapter is, from the agent loop's side.
//!
//! One object per configured provider, built by the host with the base address, the model and the
//! key. The loop hands it a [`ProviderRequest`] and reads a stream of [`StreamEvent`]s; it never
//! learns which HTTP client or wire format is behind the trait.
//!
//! The stream is a boxed [`Stream`] rather than a channel: cancellation is dropping it, which closes
//! the response the same way a dropped future does, and the loop's own cancellation is a `select!`
//! over the next item.

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::Stream;

use crate::types::{AiProviderConfig, AiProviderKind, ProviderError, ProviderRequest, StreamEvent};

/// What a provider answers with: events as they arrive, or the failure that ended the stream.
///
/// An error item may follow some events — a provider can stream half an answer and then drop the
/// connection — and the loop treats that as the turn failing rather than as a short answer.
pub type ProviderStream = Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>;

/// One provider this build can talk to.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Which protocol this adapter speaks. The settings page stores the same word.
    fn kind(&self) -> AiProviderKind;

    /// Sends one request and returns the answer as it arrives.
    ///
    /// The call returns once the response has begun; a provider that refuses the request — a bad key,
    /// a context that is too long — fails here rather than in the stream, because there is nothing
    /// streamed to report it through.
    async fn stream(&self, request: ProviderRequest) -> Result<ProviderStream, ProviderError>;
}

/// Builds the adapter one configuration entry names.
///
/// This is the only place the kind is dispatched on. Everything above it holds an `Arc<dyn Provider>`
/// and learns the protocol's name only when it has to draw one.
///
/// A key is passed separately from the configuration because the configuration has no field for one:
/// the host reads it from the vault, and a provider that is asked to run without a key fails with
/// [`ProviderError::NoKey`] before any request is made — except a local one, which needs no account.
pub fn build_provider(config: &AiProviderConfig, key: Option<&str>) -> Result<Arc<dyn Provider>, ProviderError> {
    match config.kind {
        AiProviderKind::Anthropic => Ok(Arc::new(crate::anthropic::AnthropicProvider::new(config, key)?)),
        AiProviderKind::OpenAi | AiProviderKind::Local => {
            Ok(Arc::new(crate::openai::OpenAiProvider::new(config, key)?))
        }
    }
}

/// Whether a provider address is one this build will send a key to.
///
/// `https` anywhere, and `http` only to this machine. The exception is what makes a model served by
/// Ollama, vLLM or LM Studio reachable without a certificate; the rule is what keeps a key from
/// crossing a network in the clear. An empty address is not a provider.
pub fn is_allowed_base_url(base_url: &str) -> bool {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("https://") {
        return true;
    }
    let Some(rest) = lower.strip_prefix("http://") else {
        return false;
    };

    // The authority ends at the first path, query or fragment. A user name and password are not part
    // of an address this build accepts, but stripping one keeps the host comparison honest.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    if host.starts_with('[') {
        return host.starts_with("[::1]");
    }
    matches!(host.split(':').next().unwrap_or_default(), "127.0.0.1" | "localhost")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_is_always_allowed_and_plain_http_only_to_this_machine() {
        for allowed in [
            "https://api.anthropic.com",
            "https://api.openai.com/v1",
            "https://gateway.example/v1",
            "http://127.0.0.1:11434/v1",
            "http://localhost:1234/v1",
            "http://[::1]:8080/v1",
        ] {
            assert!(is_allowed_base_url(allowed), "`{allowed}` should be reachable");
        }

        for refused in [
            "",
            "   ",
            "http://gateway.example/v1",
            "http://192.168.1.10:11434",
            "ftp://gateway.example",
            "api.anthropic.com",
        ] {
            assert!(
                !is_allowed_base_url(refused),
                "`{refused}` would send a key somewhere this build must not"
            );
        }
    }

    fn config(kind: AiProviderKind, base_url: &str) -> AiProviderConfig {
        AiProviderConfig::new("provider", "Provider", kind, base_url, "a-model")
    }

    #[test]
    fn the_factory_builds_the_adapter_the_kind_names() {
        let cases = [
            (AiProviderKind::Anthropic, "https://api.anthropic.com"),
            (AiProviderKind::OpenAi, "https://api.openai.com/v1"),
            (AiProviderKind::Local, "http://127.0.0.1:11434/v1"),
        ];

        for (kind, base_url) in cases {
            let provider = build_provider(&config(kind, base_url), Some("sk-1")).expect("a provider");
            assert_eq!(provider.kind(), kind, "`{kind:?}` was built as the wrong adapter");
        }
    }

    #[test]
    fn the_factory_refuses_a_keyless_hosted_provider_and_a_plain_http_address() {
        let keyless = build_provider(&config(AiProviderKind::OpenAi, "https://api.openai.com/v1"), None)
            .err()
            .expect("a hosted provider has no anonymous mode");
        assert_eq!(keyless, ProviderError::NoKey);

        let refused = build_provider(
            &config(AiProviderKind::Anthropic, "http://gateway.example"),
            Some("sk-1"),
        )
        .err()
        .expect("a key must not cross a network in the clear");
        assert!(matches!(refused, ProviderError::Protocol(_)), "{refused:?}");

        let local = build_provider(&config(AiProviderKind::Local, "http://localhost:1234/v1"), None)
            .expect("a local server needs no key");
        assert_eq!(local.kind(), AiProviderKind::Local);
    }
}
