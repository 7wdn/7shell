//! Saved commands, the words somebody adds to what follows one, the history of what was sent, and
//! sending one.
//!
//! The library, the words and the history live in the vault, so every command here goes through the core
//! rather than touching storage directly — and so the same operations are available to a command
//! line or an agent later without going through a window.

use sevenshell_core::Core;
use sevenshell_error::ShellError;
use sevenshell_types::{
    CommandHistoryEntry, CompletionWord, RiskAssessment, SendOutcome, SendTarget, Snippet, SnippetGroup,
};
use tauri::State;

use crate::state::ShellState;

/// The saved commands, grouped.
#[tauri::command]
pub fn snippets_open(state: State<'_, ShellState>) -> Result<Vec<SnippetGroup>, ShellError> {
    state.core().snippet_groups()
}

/// Adds or renames a group. A group with no id is a new one.
#[tauri::command]
pub fn snippet_group_save(state: State<'_, ShellState>, group: SnippetGroup) -> Result<SnippetGroup, ShellError> {
    state.core().save_snippet_group(group)
}

#[tauri::command]
pub fn snippet_group_delete(state: State<'_, ShellState>, id: String) -> Result<(), ShellError> {
    state.core().delete_snippet_group(&id)
}

/// Adds or updates a saved command. A command with no id is a new one.
#[tauri::command]
pub fn snippet_save(state: State<'_, ShellState>, snippet: Snippet) -> Result<Snippet, ShellError> {
    state.core().save_snippet(snippet)
}

#[tauri::command]
pub fn snippet_delete(state: State<'_, ShellState>, id: String) -> Result<(), ShellError> {
    state.core().delete_snippet(&id)
}

/// The words somebody added to what follows a command, in the order they added them.
#[tauri::command]
pub fn completion_words(state: State<'_, ShellState>) -> Result<Vec<CompletionWord>, ShellError> {
    state.core().completion_words()
}

/// Adds or updates one word. A word with no id is a new one.
#[tauri::command]
pub fn completion_word_save(state: State<'_, ShellState>, word: CompletionWord) -> Result<CompletionWord, ShellError> {
    state.core().save_completion_word(word)
}

#[tauri::command]
pub fn completion_word_delete(state: State<'_, ShellState>, id: String) -> Result<(), ShellError> {
    state.core().delete_completion_word(&id)
}

/// What was sent, newest first.
#[tauri::command]
pub fn command_history(state: State<'_, ShellState>) -> Result<Vec<CommandHistoryEntry>, ShellError> {
    state.core().command_history()
}

#[tauri::command]
pub fn command_history_clear(state: State<'_, ShellState>) -> Result<(), ShellError> {
    state.core().clear_command_history()
}

/// What the classifier makes of a command, without sending it.
///
/// Asked for as somebody types, so the warning is on screen before the button is pressed. It is
/// the same classifier the send itself consults, which is what keeps what is shown and what is
/// enforced from disagreeing.
#[tauri::command]
pub fn command_classify(command: String) -> Result<RiskAssessment, ShellError> {
    Ok(sevenshell_core::classify_send(&command))
}

/// Writes a command into the terminals in the target.
///
/// The answer is a report per terminal, or the verdict that stopped it: sending to several shells
/// at once is the one case that waits for a person, and it waits before anything is written.
#[tauri::command]
pub async fn command_send(
    state: State<'_, ShellState>,
    target: SendTarget,
    command: String,
    append_cr: bool,
    acknowledged: bool,
) -> Result<SendOutcome, ShellError> {
    let core: &Core = state.core();
    core.send_command(target, &command, append_cr, acknowledged).await
}
