//! The Kubernetes command surface.
//!
//! One generic request command carries the resource API; the typed commands are the two things a
//! generic request cannot express — a stream that outlives the call (logs) and a terminal (exec).
//! A write request carries the person's acknowledgement, because the policy verdict is made on
//! this side of the wire.

use sevenshell_error::ShellError;
use sevenshell_types::{K8sExecRequest, K8sLogRequest, K8sRequest, K8sResponse, KubeContextInfo, SecretInput};
use tauri::State;

use crate::state::ShellState;

fn pending(what: &str) -> ShellError {
    ShellError::app_unsupported(what)
}

#[tauri::command]
pub async fn k8s_contexts(
    _state: State<'_, ShellState>,
    _kubeconfig: Option<SecretInput>,
) -> Result<Vec<KubeContextInfo>, ShellError> {
    Err(pending("Kubernetes contexts"))
}

#[tauri::command]
pub async fn k8s_connect(_state: State<'_, ShellState>, _session_id: String) -> Result<String, ShellError> {
    Err(pending("Kubernetes connections"))
}

#[tauri::command]
pub async fn k8s_disconnect(_state: State<'_, ShellState>, _connection_id: String) -> Result<(), ShellError> {
    Err(pending("Kubernetes connections"))
}

#[tauri::command]
pub async fn k8s_request(
    _state: State<'_, ShellState>,
    _connection_id: String,
    _request: K8sRequest,
) -> Result<K8sResponse, ShellError> {
    Err(pending("Kubernetes requests"))
}

#[tauri::command]
pub async fn k8s_watch_start(
    _state: State<'_, ShellState>,
    _connection_id: String,
    _path: String,
) -> Result<String, ShellError> {
    Err(pending("Kubernetes watches"))
}

#[tauri::command]
pub async fn k8s_logs_start(
    _state: State<'_, ShellState>,
    _connection_id: String,
    _request: K8sLogRequest,
) -> Result<String, ShellError> {
    Err(pending("Kubernetes logs"))
}

#[tauri::command]
pub async fn k8s_stream_stop(_state: State<'_, ShellState>, _stream_id: String) -> Result<(), ShellError> {
    Err(pending("Kubernetes streams"))
}

#[tauri::command]
pub async fn k8s_exec_open(
    _state: State<'_, ShellState>,
    _connection_id: String,
    _request: K8sExecRequest,
) -> Result<String, ShellError> {
    Err(pending("Kubernetes terminals"))
}
