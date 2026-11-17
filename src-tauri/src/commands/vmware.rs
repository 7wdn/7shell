//! The VMware command surface.
//!
//! Each command is a thin adapter over the core's discovery and power actions. Scanning is a
//! question about this machine, so it takes no session; a power action takes the identity the scan
//! answered with and the acknowledgement a destructive action needs.

use sevenshell_error::ShellError;
use sevenshell_types::{VmInfo, VmPowerAction, VmStatus};
use tauri::State;

use crate::state::ShellState;

#[tauri::command]
pub async fn vmware_scan(state: State<'_, ShellState>) -> Result<Vec<VmInfo>, ShellError> {
    state.core().vmware_scan().await
}

#[tauri::command]
pub async fn vmware_status(state: State<'_, ShellState>, vm_id: String) -> Result<VmStatus, ShellError> {
    state.core().vmware_status(&vm_id).await
}

#[tauri::command]
pub async fn vmware_power(
    state: State<'_, ShellState>,
    vm_id: String,
    action: VmPowerAction,
    acknowledged: bool,
) -> Result<(), ShellError> {
    state.core().vmware_power(&vm_id, action, acknowledged).await
}

#[tauri::command]
pub async fn vmware_take_ownership(
    state: State<'_, ShellState>,
    vm_id: String,
    acknowledged: bool,
) -> Result<(), ShellError> {
    state.core().vmware_take_ownership(&vm_id, acknowledged).await
}
