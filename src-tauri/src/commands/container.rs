//! The container command surface.
//!
//! Each command is a thin adapter over `sevenshell-container` and the core's container
//! orchestration. The implementations land one operation at a time; until then a call answers with
//! a real envelope instead of an undefined-function error, which is what lets the interface be
//! built against this surface in parallel.

use sevenshell_error::ShellError;
use sevenshell_types::{
    ContainerCommitRequest, ContainerCreateRequest, ContainerDetail, ContainerImage, ContainerImageTransfer,
    ContainerRegistryLogin, ContainerRuntimeInfo, ContainerStats, ContainerSummary, ImageLayer,
};
use tauri::State;

use crate::state::ShellState;

fn pending(what: &str) -> ShellError {
    ShellError::app_unsupported(what)
}

#[tauri::command]
pub async fn container_connect(_state: State<'_, ShellState>, _session_id: String) -> Result<String, ShellError> {
    Err(pending("container connections"))
}

#[tauri::command]
pub async fn container_disconnect(_state: State<'_, ShellState>, _connection_id: String) -> Result<(), ShellError> {
    Err(pending("container connections"))
}

#[tauri::command]
pub async fn container_list(
    _state: State<'_, ShellState>,
    _connection_id: String,
) -> Result<Vec<ContainerSummary>, ShellError> {
    Err(pending("container listing"))
}

#[tauri::command]
pub async fn container_inspect(
    _state: State<'_, ShellState>,
    _connection_id: String,
    _container_id: String,
) -> Result<ContainerDetail, ShellError> {
    Err(pending("container inspection"))
}

#[tauri::command]
pub async fn container_action(
    _state: State<'_, ShellState>,
    _connection_id: String,
    _container_id: String,
    _action: String,
) -> Result<(), ShellError> {
    Err(pending("container actions"))
}

#[tauri::command]
pub async fn container_rename(
    _state: State<'_, ShellState>,
    _connection_id: String,
    _container_id: String,
    _name: String,
) -> Result<(), ShellError> {
    Err(pending("container rename"))
}

#[tauri::command]
pub async fn container_create(
    _state: State<'_, ShellState>,
    _connection_id: String,
    _request: ContainerCreateRequest,
) -> Result<(), ShellError> {
    Err(pending("container creation"))
}

#[tauri::command]
pub async fn container_stats(
    _state: State<'_, ShellState>,
    _connection_id: String,
) -> Result<Vec<ContainerStats>, ShellError> {
    Err(pending("container statistics"))
}

#[tauri::command]
pub async fn container_namespaces(
    _state: State<'_, ShellState>,
    _connection_id: String,
) -> Result<Vec<String>, ShellError> {
    Err(pending("container namespaces"))
}

#[tauri::command]
pub async fn container_set_namespace(
    _state: State<'_, ShellState>,
    _connection_id: String,
    _namespace: String,
) -> Result<(), ShellError> {
    Err(pending("container namespaces"))
}

#[tauri::command]
pub async fn container_runtime_info(
    _state: State<'_, ShellState>,
    _connection_id: String,
) -> Result<ContainerRuntimeInfo, ShellError> {
    Err(pending("container runtime information"))
}

#[tauri::command]
pub async fn container_logs_start(
    _state: State<'_, ShellState>,
    _connection_id: String,
    _container_id: String,
    _tail: u32,
    _timestamps: bool,
) -> Result<String, ShellError> {
    Err(pending("container logs"))
}

#[tauri::command]
pub async fn container_stream_stop(_state: State<'_, ShellState>, _stream_id: String) -> Result<(), ShellError> {
    Err(pending("container streams"))
}

#[tauri::command]
pub async fn container_exec_open(
    _state: State<'_, ShellState>,
    _connection_id: String,
    _container_id: String,
    _shell: String,
) -> Result<String, ShellError> {
    Err(pending("container terminals"))
}

#[tauri::command]
pub async fn container_images(
    _state: State<'_, ShellState>,
    _connection_id: String,
) -> Result<Vec<ContainerImage>, ShellError> {
    Err(pending("container images"))
}

#[tauri::command]
pub async fn container_image_inspect(
    _state: State<'_, ShellState>,
    _connection_id: String,
    _image: String,
) -> Result<String, ShellError> {
    Err(pending("container image inspection"))
}

#[tauri::command]
pub async fn container_image_history(
    _state: State<'_, ShellState>,
    _connection_id: String,
    _image: String,
) -> Result<Vec<ImageLayer>, ShellError> {
    Err(pending("container image history"))
}

#[tauri::command]
pub async fn container_image_pull(
    _state: State<'_, ShellState>,
    _connection_id: String,
    _request: ContainerImageTransfer,
) -> Result<String, ShellError> {
    Err(pending("container image transfers"))
}

#[tauri::command]
pub async fn container_image_push(
    _state: State<'_, ShellState>,
    _connection_id: String,
    _request: ContainerImageTransfer,
) -> Result<String, ShellError> {
    Err(pending("container image transfers"))
}

#[tauri::command]
pub async fn container_image_remove(
    _state: State<'_, ShellState>,
    _connection_id: String,
    _image: String,
) -> Result<(), ShellError> {
    Err(pending("container image removal"))
}

#[tauri::command]
pub async fn container_image_prune(_state: State<'_, ShellState>, _connection_id: String) -> Result<(), ShellError> {
    Err(pending("container image pruning"))
}

#[tauri::command]
pub async fn container_image_tag(
    _state: State<'_, ShellState>,
    _connection_id: String,
    _image: String,
    _tag: String,
) -> Result<(), ShellError> {
    Err(pending("container image tagging"))
}

#[tauri::command]
pub async fn container_image_load(
    _state: State<'_, ShellState>,
    _connection_id: String,
    _grant: String,
) -> Result<String, ShellError> {
    Err(pending("container image archives"))
}

#[tauri::command]
pub async fn container_image_commit(
    _state: State<'_, ShellState>,
    _connection_id: String,
    _request: ContainerCommitRequest,
) -> Result<String, ShellError> {
    Err(pending("container image commits"))
}

#[tauri::command]
pub async fn container_registry_login(
    _state: State<'_, ShellState>,
    _connection_id: String,
    _request: ContainerRegistryLogin,
) -> Result<(), ShellError> {
    Err(pending("container registry login"))
}
