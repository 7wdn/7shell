//! The Workstation REST service: its credentials, whether it answers, and the one action of
//! starting it that a person asks for.
//!
//! Credentials stay in memory for the session. `vmrest -C` is a one-time setup somebody runs in a
//! terminal; this surface does not pretend to replace it.

use std::process::Stdio;
use std::time::Duration;

use sevenshell_error::ShellError;
use tauri::State;
use tokio::time::sleep;

use crate::state::ShellState;

#[tauri::command]
pub fn vmrest_config_set(state: State<'_, ShellState>, user: String, password: String) -> Result<(), ShellError> {
    state.core().set_vmrest_auth(user, password)
}

#[tauri::command]
pub async fn vmrest_status(state: State<'_, ShellState>) -> Result<(), ShellError> {
    state.core().vmrest_probe().await
}

/// Starts the service beside the Workstation this build found, then asks whether it answers.
#[tauri::command]
pub async fn vmrest_start(state: State<'_, ShellState>) -> Result<(), ShellError> {
    let Some(program) = state.core().vmrest_program() else {
        return Err(ShellError::vmrest_unavailable(
            "the vmrest command was not found beside Workstation or on PATH",
        ));
    };
    let mut command = std::process::Command::new(program);
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
        .spawn()
        .map_err(|error| ShellError::vmrest_unavailable(format!("vmrest could not be started: {error}")))?;
    // The service is a console program that takes a moment to bind its port.
    sleep(Duration::from_millis(1_200)).await;
    state.core().vmrest_probe().await
}
