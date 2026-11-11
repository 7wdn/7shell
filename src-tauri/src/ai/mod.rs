//! The in-application assistant, as the desktop owns it.
//!
//! Four things live in this tree and nothing else:
//!
//! * [`runtime`] — the conversation the window mirrors, the run answering it right now, and the
//!   provider built from the settings file plus the key in the vault.
//! * [`sessions`] — the conversation as a stored document: what the list draws, what the transcript
//!   restores, and the bounds that keep one chat from becoming an archive.
//! * [`compact`] — how the older half of a long conversation becomes a summary (or, when the model
//!   cannot write one, a deterministic cut).
//! * [`events`] — the `ai:*` IPC events, and the seam that lets the run be tested without a webview.
//! * [`context`] — what travels with a question: the sanitized terminal excerpt, the system prompt
//!   and the history budget.
//!
//! Between this module and [`sevenshell_agent`] the split is the same one MCP already has: the crate
//! runs the loop, the host owns what only this process can know — the vault, the window, the keys.
//! The loop is handed a [`sevenshell_api::ToolContext`] built here, so an assistant tool call meets
//! exactly the scope, approval and audit an MCP call meets; the only difference a tool can observe
//! is [`sevenshell_types::AgentOrigin::Ai`] on its questions and its audit line.
//!
//! Nothing here holds a provider key beyond the construction of a provider. The key is read from the
//! vault, handed to the adapter and dropped — it never enters a message, an event, a log line or an
//! error detail. A conversation reaches the vault as one opaque string, and the vault never parses it.

pub mod compact;
pub mod context;
pub mod events;
mod runtime;
pub mod sessions;

use sevenshell_error::{ErrorCode, ShellError};

pub use context::AiChatContext;
pub use runtime::{AiChatSend, AiChatStarted, AiProviderTestResult, AiRuntime};

/// The envelope for a catalog code that has no named constructor yet.
///
/// [`ShellError`]'s fields are private and its constructors live in the error crate, which is not
/// this lane's. The assistant's seven codes are in the catalog but have no `ai_no_provider()` and
/// friends yet, so the envelope is rebuilt through its own wire shape — the shape
/// `crates/sevenshell-error/tests/wire_format.rs` pins — with the code *and* the message key taken
/// from [`ErrorCode`], so the pair still cannot drift the way the catalog exists to prevent.
///
/// This is the one place to delete when the error crate grows those constructors, and
/// `an_assistant_failure_carries_its_catalog_code` is what fails if the envelope changes underneath
/// it.
pub(crate) fn catalog_error(code: ErrorCode) -> ShellError {
    let envelope = serde_json::json!({
        "version": sevenshell_error::ENVELOPE_VERSION,
        "code": code.as_str(),
        "messageKey": code.message_key(),
        "messageParams": {},
        // Not a transport failure: nothing left the machine because a provider was not set up.
        "origin": { "subsystem": "app" },
        "outcome": "notSent",
    });

    serde_json::from_value(envelope).unwrap_or_else(|error| {
        // A panic here would turn "the assistant has no provider" into a dead window. The sentence
        // is deliberately free of anything the person typed.
        ShellError::app_internal(format!("an assistant failure envelope could not be built: {error}"))
    })
}

/// The catalog's key for a code that arrived as a string.
///
/// Used for the codes the agent loop reports in its events: an event carries a code and a message
/// key rather than a sentence, and the host is the side that knows the catalog. A code this build
/// does not have — a newer loop, or one of Lane B's placeholders — falls back to the provider
/// sentence rather than leaving the window with a key it cannot resolve.
pub(crate) fn message_key_for(code: &str) -> &'static str {
    ErrorCode::from_code(code)
        .map(ErrorCode::message_key)
        .unwrap_or("errors.ai.providerUnavailable")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_assistant_failure_carries_its_catalog_code() {
        let error = catalog_error(ErrorCode::AiNoProvider);

        assert_eq!(error.code(), ErrorCode::AiNoProvider);
        assert_eq!(error.message_key(), "errors.ai.noProvider");
        assert_eq!(error.message_params().len(), 0);
        assert!(
            error.detail().is_none(),
            "a failure about configuration has nothing to explain beyond its code"
        );
    }

    #[test]
    fn a_code_this_build_does_not_know_still_has_a_sentence() {
        assert_eq!(message_key_for("AI-NO-KEY"), "errors.ai.noKey");
        assert_eq!(
            message_key_for("AI-NOT-IMPLEMENTED"),
            "errors.ai.providerUnavailable",
            "a placeholder code must not leave the window with an unresolvable key"
        );
    }
}
