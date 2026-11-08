//! One completion generator, run on the far side of an open terminal.
//!
//! The validation, the policy decision and the bounded execution live in the core; this is the
//! thin wire adapter the interface calls.

use sevenshell_error::ShellError;
use sevenshell_types::{CompletionExecuteRequest, CompletionExecuteResult};
use tauri::State;

use crate::state::ShellState;

#[tauri::command]
pub async fn completion_execute(
    state: State<'_, ShellState>,
    request: CompletionExecuteRequest,
) -> Result<CompletionExecuteResult, ShellError> {
    state.core_arc().run_completion(request).await
}
