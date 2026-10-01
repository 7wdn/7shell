//! One sample of what a host is doing: the panel beside the workspace, asked for by the interface.
//!
//! The panel polls, so this command is called on a timer while it is on screen. Everything about
//! what is asked of the far side — one command, read-only, on the connection that is already open
//! — is in `sevenshell_core::monitor`; all this adds is the wire type and the error envelope.

use sevenshell_error::ShellError;
use sevenshell_types::{HostMetrics, ProcessDetail, ProcessPage, SocketRow};
use tauri::State;

use crate::state::ShellState;

/// Takes one sample of the host behind an open terminal.
///
/// The handle is a terminal's, not a session id: a sample rides a connection that is up, and the
/// backend refuses a handle that is not — which is the honest answer for a session whose terminal
/// has already ended, and the one the panel draws as "not connected".
#[tauri::command]
pub async fn monitor_sample(state: State<'_, ShellState>, handle: String) -> Result<HostMetrics, ShellError> {
    state.core().sample_host(&handle).await
}

/// The process list for the page of that name.
///
/// Cut on the far side, because the cut happens where the cost is: the table with its command lines
/// is tens of kilobytes, and the page has the whole of it below a limit that only a very large host
/// reaches. The order of what arrives is the host's own — busiest first — and the page sorts what it
/// has, so changing the order costs a redraw rather than another command on somebody's server.
#[tauri::command]
pub async fn monitor_processes(
    state: State<'_, ShellState>,
    handle: String,
    limit: usize,
) -> Result<ProcessPage, ShellError> {
    state.core().list_processes(&handle, limit).await
}

/// One process in full: how it was started, from where, and with what in its environment.
///
/// The environment is read for this one process because somebody opened it — it is the one read in
/// this build that can return credentials, and it is never part of a sample or of a poll.
#[tauri::command]
pub async fn monitor_process_detail(
    state: State<'_, ShellState>,
    handle: String,
    pid: u32,
) -> Result<ProcessDetail, ShellError> {
    state.core().process_detail(&handle, pid).await
}

/// The host's sockets, for the network page.
///
/// Listening and connected in one list, because they are one list on the far side and the page
/// shows them as two views of it: what is waiting on a port, and — under the row somebody picks —
/// who is connected to it. Splitting them here would mean asking the host twice for facts it
/// already answered together.
#[tauri::command]
pub async fn monitor_sockets(state: State<'_, ShellState>, handle: String) -> Result<Vec<SocketRow>, ShellError> {
    state.core().list_sockets(&handle).await
}
