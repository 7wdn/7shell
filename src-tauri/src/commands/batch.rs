//! Batches: grading a command, running it across hosts, and reporting each host as it lands.
//!
//! The progress events are window events rather than a stream of command results: a run on ten
//! hosts outlives the call that started it as far as the person watching is concerned, and a row
//! that only appears when the slowest host answers is a row nobody can act on.
//!
//! What is emitted is the core's own vocabulary, translated here and nowhere else — everything
//! below this file deals in hosts and exit codes, not in windows.

use serde::Serialize;
use sevenshell_core::BatchEvent;
use sevenshell_error::ShellError;
use sevenshell_types::{BatchHostResult, BatchOutcome, BatchRequest};
use tauri::{AppHandle, Emitter as _, State};

use crate::state::ShellState;

/// One host starting or finishing, as the interface hears it.
///
/// Flattened rather than nested so the listener can switch on `kind` alone: a row that is
/// `hostStarted` has no result yet, and one that is `hostFinished` will not change again.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum BatchProgress {
    HostStarted { session_id: String },
    HostFinished { result: Box<BatchHostResult> },
}

/// Runs one command across the hosts in the request.
///
/// Progress arrives on the `batch:event` window event while this is still running; the answer is
/// the finished run, or the decision or approval that stopped it. Nothing is run on any host until
/// every host has been connected to, so an answer that comes back instead of a run means the
/// command has not executed anywhere.
#[tauri::command]
pub async fn batch_run(
    app: AppHandle,
    state: State<'_, ShellState>,
    request: BatchRequest,
) -> Result<BatchOutcome, ShellError> {
    let (progress, mut events) = tokio::sync::mpsc::channel::<BatchEvent>(64);
    let emit = app.clone();

    // The forwarder is a task of its own because the run is still going while these arrive, and it
    // ends when the run does: the sender is owned by the call below, and dropping it closes the
    // channel.
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            let payload = match event {
                BatchEvent::HostStarted { session_id } => BatchProgress::HostStarted { session_id },
                BatchEvent::HostFinished { result } => BatchProgress::HostFinished { result },
            };
            if let Err(error) = emit.emit("batch:event", payload) {
                log::warn!("batch progress could not be delivered: {error}");
            }
        }
    });

    state.core_arc().run_across(request, progress).await
}
