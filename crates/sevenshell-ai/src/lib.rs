//! The provider half of the in-application assistant.
//!
//! This crate is a client and nothing else: it builds a request out of the conversation the agent
//! loop holds, sends it to one provider, and turns the streamed answer back into [`StreamEvent`]s.
//! It does not know what a session is, does not open the vault, and does not know that a webview
//! exists — the desktop hands it a key and a base address, and gets events back.
//!
//! Three shapes are frozen here because two adapters and a future one have to agree about them:
//!
//! * [`types::ContentBlock`] and [`types::ChatMessage`] — the conversation, in the vocabulary both
//!   APIs can express. A provider adapter translates to its own wire shape and back; nothing above
//!   this crate sees Anthropic's or OpenAI's JSON.
//! * [`types::StreamEvent`] — one thing that happened while the answer was arriving. The agent loop
//!   turns these into interface events and tool calls.
//! * [`ProviderError`] — the failures every adapter can produce, mapped to the catalog's stable AI
//!   codes in [`types::ProviderError::code`].
//!
//! Where the pieces are:
//!
//! * [`provider`] — the trait the loop holds, and [`provider::build_provider`], the one place a
//!   configuration entry becomes an adapter.
//! * [`anthropic`] and [`openai`] — the two wire formats, each in its own module with its own
//!   quirks documented there.
//! * [`sse`] — the framing both streaming APIs share.
//! * [`stream`], [`http`] — what the adapters do not each get to invent: the two stream deadlines,
//!   the retry rule, and the mapping from a refusal to a failure.
//!
//! A key is never a field of anything that is serialized, logged or sent to the interface. An
//! adapter takes one when it is built and keeps it in a buffer that is wiped when the adapter is
//! dropped; the host is the only side that knows where it came from.

pub mod anthropic;
mod http;
pub mod openai;
pub mod provider;
pub mod sse;
mod stream;
pub mod types;

pub use anthropic::AnthropicProvider;
pub use openai::OpenAiProvider;
pub use provider::{Provider, ProviderStream, build_provider};
pub use types::{
    AiProviderConfig, AiProviderKind, AiProviderView, ChatMessage, ContentBlock, ProviderError, ProviderRequest, Role,
    StopReason, StreamEvent, ToolSpec, Usage,
};
