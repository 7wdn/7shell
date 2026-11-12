//! What the interface may read about the policy matrix's decisions.
//!
//! Read-only, and deliberately narrow: the matrix itself is written through the ordinary settings
//! save, and the trail is evidence, not something the window edits or clears. The answer is the
//! records and nothing about where they live — the interface never learns a local path.

use sevenshell_error::ShellError;
use sevenshell_policy::PolicyAuditEvent;
use tauri::State;

use crate::state::ShellState;

/// The last `limit` policy decisions, oldest first.
///
/// A machine with no configuration directory has nowhere to write a trail; the command says so
/// rather than answering an empty list, because "no trail" and "nothing was decided" are different
/// things and a page that drew both the same way would hide the first.
#[tauri::command]
pub fn policy_audit_tail(state: State<'_, ShellState>, limit: usize) -> Result<Vec<PolicyAuditEvent>, ShellError> {
    let path = state.policy_log().path().ok_or_else(|| {
        ShellError::store_io("the policy audit trail has nowhere to live: this machine has no configuration directory")
    })?;
    Ok(crate::policy::read_tail(path, limit))
}
