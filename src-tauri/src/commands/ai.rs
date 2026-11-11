//! The assistant's command surface.
//!
//! Seven commands, and the split between them is the split this application keeps everywhere:
//!
//! * **Configuration** is two halves that never meet. `ai_providers` reads the settings file and the
//!   vault together and answers a view with `hasKey` — `null` while the vault is locked, because
//!   "could not tell" is not "there is none". `ai_provider_key_set` writes the key to the vault and
//!   *nothing else*; the non-secret half of a provider is written by the ordinary `settings_set`.
//! * **A test** is a question about a configuration, answered as a result: a refused key is
//!   `ok: false` with the catalog code, not a rejected command.
//! * **A chat** answers with a run id before it has an answer, and everything after that arrives as
//!   `ai:*` events carrying the same id. `ai_chat_cancel` names a run (or, with nothing, whatever is
//!   running); `ai_chat_reset` forgets the conversation and the run in flight.
//!
//! Nothing here returns a key, and no error carries one: the vault holds it, the adapter receives it
//! for the length of one request, and the interface is told `hasKey` and nothing more.

use std::sync::Arc;

use sevenshell_ai::AiProviderView;
use sevenshell_error::{ErrorCode, ShellError};
use tauri::{AppHandle, State};

use crate::ai::events::{AiEventPresenter, WindowEvents};
use crate::ai::{AiChatContext, AiChatSend, AiChatStarted, AiProviderTestResult, catalog_error};
use crate::mcp::approval::{ApprovalPresenter, TauriPresenter};
use crate::state::ShellState;

/// Every configured provider, with whether a key is held.
#[tauri::command]
pub fn ai_providers(state: State<'_, ShellState>) -> Result<Vec<AiProviderView>, ShellError> {
    let settings = sevenshell_platform::load()?;
    Ok(state.ai().providers(&settings))
}

/// Keeps an API key for one provider, or forgets the one that was kept.
///
/// The key crosses this command once and goes into the vault; the answer is the provider's view with
/// `hasKey` set, which is all the page needs to draw. A blank or absent value is a deletion, the
/// same rule the vault's own token commands follow.
#[tauri::command]
pub fn ai_provider_key_set(
    state: State<'_, ShellState>,
    provider_id: String,
    key: Option<String>,
) -> Result<AiProviderView, ShellError> {
    let settings = sevenshell_platform::load()?;
    let config = settings
        .ai_providers
        .iter()
        .find(|config| config.id == provider_id)
        .cloned()
        .ok_or_else(|| catalog_error(ErrorCode::AiNoProvider))?;

    state.ai().set_key(&provider_id, key)?;
    Ok(AiProviderView::new(config, state.ai().key_state(&provider_id)))
}

/// Removes a provider: its key from the vault, then its configuration from the settings file.
///
/// The order is the runtime's, and it is why a locked vault leaves both halves alone: the key
/// deletion refuses before the list has changed. Both writes happen here, so the file is written
/// once — with the entry gone and the default cleared — rather than twice.
#[tauri::command]
pub fn ai_provider_remove(state: State<'_, ShellState>, provider_id: String) -> Result<(), ShellError> {
    let mut settings = sevenshell_platform::load()?;
    state.ai().remove_provider(&mut settings, &provider_id)?;
    sevenshell_platform::save(&settings)
}

/// Sends the smallest possible request to one provider.
///
/// A provider refusing the key is the answer to this question, so it arrives as `ok: false` with the
/// catalog's code and key; only a settings file that cannot be read rejects the command.
#[tauri::command]
pub async fn ai_provider_test(
    state: State<'_, ShellState>,
    provider_id: String,
) -> Result<AiProviderTestResult, ShellError> {
    let settings = sevenshell_platform::load()?;
    Ok(state.ai().test_provider(&settings, &provider_id).await)
}

/// Asks the assistant a question. Answers with the run everything after it is filed under.
#[tauri::command]
pub async fn ai_chat_send(
    app: AppHandle,
    state: State<'_, ShellState>,
    text: String,
    provider_id: Option<String>,
    context: Option<AiChatContext>,
) -> Result<AiChatStarted, ShellError> {
    let settings = sevenshell_platform::load()?;

    // Two presenters, one window: the run's events and the questions its tool calls ask. The
    // approval one is the same object the MCP listener hands its handler, so the dialog is the one
    // the application already draws.
    let events: Arc<dyn AiEventPresenter> = Arc::new(WindowEvents::new(app.clone()));
    let approvals: Arc<dyn ApprovalPresenter> = Arc::new(TauriPresenter::new(app));

    state.ai_arc().start(
        &settings,
        events,
        approvals,
        AiChatSend {
            text,
            provider_id,
            context,
        },
    )
}

/// Calls off the named run, or whatever is running when none is named.
#[tauri::command]
pub fn ai_chat_cancel(state: State<'_, ShellState>, run_id: Option<String>) {
    state.ai().cancel(run_id.as_deref());
}

/// Forgets the conversation, the run in flight and any question it was waiting on.
#[tauri::command]
pub fn ai_chat_reset(state: State<'_, ShellState>) {
    state.ai().reset();
}
