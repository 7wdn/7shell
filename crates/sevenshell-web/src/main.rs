// The local browser service: one loopback HTTP listener that serves the built interface and answers
// the backend calls the interface makes. It is a second host, not a second product: the interface,
// the core and the command contract are the same ones the desktop shell uses. Nothing here knows
// Tauri, and nothing here listens beyond 127.0.0.1.
//
// The token is the whole authentication story: the launcher prints a URL containing it, the client
// sends it with every call and the event socket, and anything else on the loopback is refused.

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Path as UrlPath, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures_util::StreamExt;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sevenshell_core::{BatchEvent, Core, CoreEvent};
use sevenshell_error::ShellError;
use sevenshell_session::TerminalEvent;
use sevenshell_types::{
    AppInfo, AppSettings, AuthMethod, BatchRequest, CompletionExecuteRequest, CompletionWord, ConnectionTestOutcome,
    ContainerCommitRequest, ContainerCreateRequest, ContainerImageTransfer, ContainerRegistryLogin, ContainerStreamEnd,
    ContainerStreamLine, CreatedKey, DownloadFolderView, EntryKind, K8sExecRequest, K8sLogRequest, K8sRequest,
    K8sStreamEnd, K8sStreamLine, K8sWatchEvent, KeyAlgorithm, KeyInfo, PresentedHostKey, Protocol, SecretInput,
    SendTarget, Session, SessionExportSummary, SessionGroup, SessionImportOutcome, Snippet, SnippetGroup,
    SyncSelection, TerminalEncoding, TransferState, UpdateCheck, UploadAnswer, UploadOutcome, VaultKeychainStatus,
    VaultState, VmPowerAction,
};
use sevenshell_vault::{ExportProtection, OpenedBundle, SessionSecret, Vault, VaultStatus};
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::{broadcast, mpsc};
use tokio_util::io::ReaderStream;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

mod agent;
mod ai;
mod approvals;
mod jsonl;
mod mcp;
mod policy;
mod sync;
mod zmodem;

const HELP: &str = r#"Usage: sshell-web [OPTION]

Serve the 7Shell interface on this machine and let the system browser open it.

Options:
  -h, --help  Show this help message and exit.

Environment variables:
  SEVENSHELL_WEB_PORT    Listen port (default: 4225)
  SEVENSHELL_WEB_TOKEN   Access token (default: one random value per start)
  SEVENSHELL_WEB_STATIC  Built interface directory (default: apps/desktop/dist)
"#;

#[derive(Clone)]
pub(crate) struct StagedFile {
    pub(crate) path: PathBuf,
    pub(crate) name: String,
}

struct PendingUpload {
    handle: String,
    directory: String,
    sources: Vec<PathBuf>,
    packed: bool,
    taken: Vec<String>,
}

impl PendingUpload {
    fn missing(&self) -> Vec<PathBuf> {
        self.sources
            .iter()
            .filter(|source| {
                let name = source
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                !self.taken.contains(&name)
            })
            .cloned()
            .collect()
    }
}

struct WebState {
    token: String,
    events: broadcast::Sender<String>,
    vault: Arc<Vault>,
    core: Arc<Core>,
    policy_log: Arc<policy::PolicyLog>,
    tools: Arc<agent::ToolRuntime>,
    ai: Arc<ai::AiRuntime>,
    mcp: Arc<mcp::McpRuntime>,
    staging: Arc<tokio::sync::Mutex<HashMap<String, StagedFile>>>,
    pending_uploads: tokio::sync::Mutex<HashMap<String, PendingUpload>>,
    browser_downloads: tokio::sync::Mutex<HashMap<String, PathBuf>>,
    zmodem: Arc<zmodem::Zmodem>,
}

#[derive(Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

#[derive(Deserialize)]
struct OAuthCallbackQuery {
    state: Option<String>,
    code: Option<String>,
}

/// The loopback landing page of the OAuth authorization. The state is the CSRF check; the page
/// itself says nothing, because the app's server row is where the outcome appears.
async fn oauth_callback(State(state): State<Arc<WebState>>, Query(query): Query<OAuthCallbackQuery>) -> Response {
    let (Some(oauth_state), Some(code)) = (query.state, query.code) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    match state.mcp.complete_oauth(&oauth_state, &code).await {
        Ok(()) => (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            "<script>window.close()</script>",
        )
            .into_response(),
        Err(error) => {
            log::warn!("the OAuth callback could not be completed: {error}");
            StatusCode::BAD_REQUEST.into_response()
        }
    }
}

#[derive(Deserialize)]
struct InvokeRequest {
    cmd: String,
    #[serde(default)]
    args: Value,
}

fn authorized(headers: &HeaderMap, query: &TokenQuery, state: &WebState) -> bool {
    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    header == Some(state.token.as_str()) || query.token.as_deref() == Some(state.token.as_str())
}

fn from_args<T: DeserializeOwned>(args: &Value, key: &str) -> Result<T, ShellError> {
    serde_json::from_value(args.get(key).cloned().unwrap_or(Value::Null))
        .map_err(|error| ShellError::app_internal(format!("the `{key}` argument was not valid: {error}")))
}

fn optional_args<T: DeserializeOwned>(args: &Value, key: &str) -> Result<Option<T>, ShellError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|error| ShellError::app_internal(format!("the `{key}` argument was not valid: {error}"))),
    }
}

fn to_value<T: serde::Serialize>(value: T) -> Result<Value, ShellError> {
    serde_json::to_value(value)
        .map_err(|error| ShellError::app_internal(format!("the answer could not be serialized: {error}")))
}

fn vault_state(vault: &Vault) -> VaultState {
    match vault.status() {
        VaultStatus::Missing => VaultState::Missing,
        VaultStatus::Locked => VaultState::Locked,
        VaultStatus::Unlocked => VaultState::Unlocked,
    }
}

fn app_info() -> AppInfo {
    #[allow(unused_mut)]
    let mut info = AppInfo::current();
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        let value = |key: &str| {
            text.lines()
                .find_map(|line| line.strip_prefix(key))
                .and_then(|rest| rest.strip_prefix('='))
                .map(|value| value.trim().trim_matches('"').to_owned())
                .filter(|value| !value.is_empty())
        };
        info.os_name = value("NAME")
            .or_else(|| value("PRETTY_NAME"))
            .unwrap_or_else(|| "Linux".to_owned());
        info.os_version = value("VERSION_ID");
    }
    info
}

fn mcp_status_disabled(pending_approvals: usize) -> Value {
    json!({
        "enabled": false,
        "port": null,
        "scope": "readOnly",
        "clientConfig": "",
        "auditPath": "",
        "pendingApprovals": pending_approvals
    })
}

async fn start_or_ask(
    state: &WebState,
    handle: String,
    directory: String,
    sources: Vec<PathBuf>,
    packed: bool,
) -> Result<UploadOutcome, ShellError> {
    if sources.is_empty() {
        return Ok(UploadOutcome::Cancelled);
    }

    let taken = state.core.upload_names_taken(&handle, &directory, &sources).await?;
    if taken.is_empty() {
        return start_uploads(state, &handle, &directory, &sources, packed).await;
    }

    let skippable = !packed && sources.len() > 1;
    let token = uuid::Uuid::new_v4().simple().to_string();
    state.pending_uploads.lock().await.insert(
        token.clone(),
        PendingUpload {
            handle,
            directory,
            sources,
            packed,
            taken: taken.clone(),
        },
    );

    Ok(UploadOutcome::Conflict {
        token,
        names: taken,
        skippable,
    })
}

async fn start_uploads(
    state: &WebState,
    handle: &str,
    directory: &str,
    sources: &[PathBuf],
    packed: bool,
) -> Result<UploadOutcome, ShellError> {
    if packed {
        let id = state
            .core
            .start_packed_upload(handle, sources.to_vec(), directory)
            .await?;
        return Ok(UploadOutcome::Started { ids: vec![id] });
    }

    let mut ids = Vec::with_capacity(sources.len());
    for source in sources {
        ids.push(state.core.start_upload(handle, source.clone(), directory).await?);
    }
    Ok(UploadOutcome::Started { ids })
}

fn next_key_name(state: &WebState, algorithm: KeyAlgorithm) -> Result<String, ShellError> {
    let taken: Vec<String> = state.vault.keys()?.into_iter().map(|key| key.display_name).collect();
    let base = format!("id_{}", algorithm.as_str());
    if !taken.contains(&base) {
        return Ok(base);
    }
    let mut number = 2;
    loop {
        let candidate = format!("{base}-{number}");
        if !taken.contains(&candidate) {
            return Ok(candidate);
        }
        number += 1;
    }
}

fn forward_container_stream(
    sender: broadcast::Sender<String>,
    mut lines: UnboundedReceiver<ContainerStreamLine>,
    mut end: UnboundedReceiver<ContainerStreamEnd>,
) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(line) = lines.recv() => {
                    let _ = sender.send(json!({ "event": "container:stream", "payload": line }).to_string());
                }
                Some(finished) = end.recv() => {
                    while let Ok(line) = lines.try_recv() {
                        let _ = sender.send(json!({ "event": "container:stream", "payload": line }).to_string());
                    }
                    let _ = sender.send(json!({ "event": "container:stream-end", "payload": finished }).to_string());
                    break;
                }
            }
        }
    });
}

fn forward_k8s_watch(
    sender: broadcast::Sender<String>,
    mut events: UnboundedReceiver<K8sWatchEvent>,
    mut end: UnboundedReceiver<K8sStreamEnd>,
) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(event) = events.recv() => {
                    let _ = sender.send(json!({ "event": "k8s:watch", "payload": event }).to_string());
                }
                Some(finished) = end.recv() => {
                    while let Ok(event) = events.try_recv() {
                        let _ = sender.send(json!({ "event": "k8s:watch", "payload": event }).to_string());
                    }
                    let _ = sender.send(json!({ "event": "k8s:stream-end", "payload": finished }).to_string());
                    return;
                }
                else => return,
            }
        }
    });
}

fn forward_k8s_log(
    sender: broadcast::Sender<String>,
    mut lines: UnboundedReceiver<K8sStreamLine>,
    mut end: UnboundedReceiver<K8sStreamEnd>,
) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(line) = lines.recv() => {
                    let _ = sender.send(json!({ "event": "k8s:stream", "payload": line }).to_string());
                }
                Some(finished) = end.recv() => {
                    while let Ok(line) = lines.try_recv() {
                        let _ = sender.send(json!({ "event": "k8s:stream", "payload": line }).to_string());
                    }
                    let _ = sender.send(json!({ "event": "k8s:stream-end", "payload": finished }).to_string());
                    return;
                }
                else => return,
            }
        }
    });
}

async fn dispatch(state: &WebState, cmd: &str, args: Value) -> Result<Value, ShellError> {
    if let Some(result) = state.ai.dispatch(cmd, &args).await {
        return result;
    }
    if let Some(result) = state.mcp.dispatch(cmd, &args).await {
        return result;
    }
    match cmd {
        "app_info" => to_value(app_info()),
        "app_ready" => Ok(Value::Null),
        "system_theme" => Ok(Value::Null),
        "system_fonts_list" => Ok(json!([
            "monospace",
            "DejaVu Sans Mono",
            "Noto Sans Mono",
            "Noto Sans Mono CJK SC"
        ])),
        "settings_get" => to_value(sevenshell_platform::load()?),
        "settings_set" => {
            let settings: AppSettings = from_args(&args, "settings")?;
            sevenshell_platform::save(&settings)?;
            state.policy_log.configure(settings.mcp_audit_retention_days);
            let journal: Arc<dyn sevenshell_policy::PolicyJournal> = state.policy_log.clone();
            state.core.set_policy(settings.policy_matrix.clone(), Some(journal));
            Ok(Value::Null)
        }

        "vault_state" => to_value(vault_state(&state.vault)),
        "vault_create" => {
            let password: String = from_args(&args, "password")?;
            state.vault.create(&password)?;
            to_value(vault_state(&state.vault))
        }
        "vault_unlock" => {
            let password: String = from_args(&args, "password")?;
            state.vault.unlock(&password)?;
            to_value(vault_state(&state.vault))
        }
        "vault_lock" => {
            state.vault.lock();
            to_value(vault_state(&state.vault))
        }
        "vault_change_password" => {
            let current: String = from_args(&args, "current")?;
            let next: String = from_args(&args, "next")?;
            state.vault.change_password(&current, &next)?;
            Ok(Value::Null)
        }
        // A browser host keeps no system keychain of its own: the package is used from a desktop
        // session this process cannot ask, and the fallback is the password the vault already has.
        "vault_keychain_status" => to_value(VaultKeychainStatus {
            available: false,
            enrolled: false,
        }),
        "vault_keychain_enroll" | "vault_keychain_forget" => Err(ShellError::vault_keychain_unavailable()),
        "vault_set_aside" => to_value(state.vault.set_aside()?),
        "vault_archives" => to_value(state.vault.archives()?),

        "sessions_list" => to_value(state.vault.sessions()?),
        "ssh_config_hosts" => to_value(sevenshell_core::openssh::hosts()),
        "sessions_export_browser" => {
            let file_name: String =
                optional_args(&args, "fileName")?.unwrap_or_else(|| "7shell-sessions.json".to_owned());
            let passphrase: Option<String> = optional_args(&args, "passphrase")?;
            let protection = match passphrase.as_deref() {
                Some(passphrase) => ExportProtection::Passphrase(passphrase),
                None => ExportProtection::WithoutCredentials,
            };
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs())
                .unwrap_or_default();
            let exported = state.vault.export_bundle(protection, now)?;
            let summary = SessionExportSummary {
                sessions: exported.sessions,
                groups: exported.groups,
                secrets: exported.secrets,
                encrypted: exported.encrypted,
                file_name,
            };
            Ok(json!({ "summary": summary, "contents": exported.text }))
        }
        "sessions_import_browser" => {
            let contents: String = from_args(&args, "contents")?;
            let file_name: String =
                optional_args(&args, "fileName")?.unwrap_or_else(|| "7shell-sessions.json".to_owned());
            let passphrase: Option<String> = optional_args(&args, "passphrase")?;
            if contents.len() as u64 > sevenshell_vault::MAX_BUNDLE_BYTES {
                return Err(ShellError::session_bundle_invalid(
                    "that file is far too large to be a session export",
                ));
            }
            let bundle = match sevenshell_vault::open(&contents)? {
                OpenedBundle::Plain(bundle) => *bundle,
                OpenedBundle::Sealed(envelope) => match passphrase.as_deref() {
                    Some(passphrase) => envelope.unseal(passphrase)?,
                    None => return to_value(SessionImportOutcome::NeedsPassphrase { file_name }),
                },
            };
            let summary = state.vault.import_bundle(bundle)?;
            to_value(SessionImportOutcome::Imported { summary })
        }
        "session_save" => {
            let session: Session = from_args(&args, "session")?;
            let secret: Option<SecretInput> = optional_args(&args, "secret")?;
            let secret = secret.map(|input| match session.protocol {
                Protocol::Kubernetes => SessionSecret::Kubeconfig(input.value),
                _ => match session.auth.method {
                    AuthMethod::PublicKey => SessionSecret::KeyPassphrase(input.value),
                    _ => SessionSecret::Password(input.value),
                },
            });
            to_value(state.vault.save_session(session, secret)?)
        }
        "session_delete" => {
            let id: String = from_args(&args, "id")?;
            state.vault.delete_session(&id)?;
            Ok(Value::Null)
        }
        "groups_list" => to_value(state.vault.groups()?),
        "group_save" => {
            let group: SessionGroup = from_args(&args, "group")?;
            to_value(state.vault.save_group(group)?)
        }
        "group_delete" => {
            let id: String = from_args(&args, "id")?;
            state.vault.delete_group(&id)?;
            Ok(Value::Null)
        }

        "known_hosts_list" => to_value(state.vault.known_hosts()?),
        "known_host_trust" => {
            let key: PresentedHostKey = from_args(&args, "key")?;
            state.core.trust_host_key(&key)?;
            Ok(Value::Null)
        }
        "known_host_forget" => {
            let host: String = from_args(&args, "host")?;
            let port: u16 = from_args(&args, "port")?;
            state.vault.forget_host(&host, port)?;
            Ok(Value::Null)
        }
        "keys_list" => to_value(state.core.keys()?),

        "session_open" => {
            let session_id: String = from_args(&args, "sessionId")?;
            to_value(state.core.open_terminal(&session_id).await?)
        }
        "session_open_cancel" => {
            let session_id: String = from_args(&args, "sessionId")?;
            Ok(json!(state.core.cancel_open(&session_id).await))
        }
        "session_test" => {
            let session: Session = from_args(&args, "session")?;
            let secret: Option<SecretInput> = optional_args(&args, "secret")?;
            let outcome: ConnectionTestOutcome = state.core.test_session(&session, secret).await?;
            to_value(outcome)
        }
        "session_system" => {
            let handle: String = from_args(&args, "handle")?;
            to_value(state.core.host_system(&handle).await?)
        }
        "terminal_write" => {
            let handle: String = from_args(&args, "handle")?;
            let encoded: String = from_args(&args, "base64")?;
            let data = BASE64.decode(encoded.as_bytes()).map_err(|error| {
                ShellError::app_internal(format!("the terminal input was not valid base64: {error}"))
            })?;
            state.core.write(&handle, &data).await?;
            Ok(Value::Null)
        }
        "terminal_resize" => {
            let handle: String = from_args(&args, "handle")?;
            let columns: u32 = from_args(&args, "columns")?;
            let rows: u32 = from_args(&args, "rows")?;
            state.core.resize(&handle, columns, rows).await?;
            Ok(Value::Null)
        }
        "terminal_write_text" => {
            let handle: String = from_args(&args, "handle")?;
            let text: String = from_args(&args, "text")?;
            let encoding: TerminalEncoding = from_args(&args, "encoding")?;
            state.core.write_text(&handle, &text, encoding).await?;
            Ok(Value::Null)
        }
        "terminal_close" => {
            let handle: String = from_args(&args, "handle")?;
            state.core.close_terminal(&handle).await?;
            Ok(Value::Null)
        }
        "zmodem_send_staged" => {
            let handle: String = from_args(&args, "handle")?;
            let token: String = from_args(&args, "token")?;
            let path = state
                .staging
                .lock()
                .await
                .get(&token)
                .map(|file| file.path.clone())
                .ok_or_else(|| ShellError::app_internal("a staged ZMODEM file is no longer available"))?;
            state.zmodem.send_file(&handle, path).await?;
            Ok(Value::Null)
        }
        "zmodem_cancel" => {
            let handle: String = from_args(&args, "handle")?;
            state.zmodem.cancel(&handle).await;
            Ok(Value::Null)
        }
        "terminals_open" => to_value(state.core.open_terminals().await),
        "terminal_local_shells" => to_value(state.core.local_shells()),
        "terminal_wsl_distros" => to_value(state.core.wsl_distros()),
        "terminal_serial_ports" => to_value(state.core.serial_ports()),
        "tunnels_open" => to_value(state.core.tunnels().await),
        "tunnel_close" => {
            let id: String = from_args(&args, "id")?;
            state.core.close_tunnel(&id).await?;
            Ok(Value::Null)
        }

        "files_open" => {
            let session_id: String = from_args(&args, "sessionId")?;
            to_value(state.core.open_files(&session_id).await?)
        }
        "files_open_container" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let container_id: String = from_args(&args, "containerId")?;
            let path: String = from_args(&args, "path")?;
            to_value(
                state
                    .core
                    .open_container_files(&connection_id, &container_id, &path)
                    .await?,
            )
        }
        "files_open_pod" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let namespace: String = from_args(&args, "namespace")?;
            let pod: String = from_args(&args, "pod")?;
            let container: String = from_args(&args, "container")?;
            let path: String = from_args(&args, "path")?;
            to_value(
                state
                    .core
                    .open_pod_files(&connection_id, &namespace, &pod, &container, &path)
                    .await?,
            )
        }
        "files_list" => {
            let handle: String = from_args(&args, "handle")?;
            let path: String = from_args(&args, "path")?;
            to_value(state.core.list_files(&handle, &path).await?)
        }
        "files_create_folder" => {
            let handle: String = from_args(&args, "handle")?;
            let path: String = from_args(&args, "path")?;
            state.core.create_folder(&handle, &path).await?;
            Ok(Value::Null)
        }
        "files_create_file" => {
            let handle: String = from_args(&args, "handle")?;
            let path: String = from_args(&args, "path")?;
            state.core.create_file(&handle, &path).await?;
            Ok(Value::Null)
        }
        "files_rename" => {
            let handle: String = from_args(&args, "handle")?;
            let from: String = from_args(&args, "from")?;
            let to: String = from_args(&args, "to")?;
            state.core.rename_entry(&handle, &from, &to).await?;
            Ok(Value::Null)
        }
        "files_set_permissions" => {
            let handle: String = from_args(&args, "handle")?;
            let path: String = from_args(&args, "path")?;
            let mode: u32 = from_args(&args, "mode")?;
            state.core.set_permissions(&handle, &path, mode).await?;
            Ok(Value::Null)
        }
        "files_delete" => {
            let handle: String = from_args(&args, "handle")?;
            let path: String = from_args(&args, "path")?;
            let kind: EntryKind = from_args(&args, "kind")?;
            state.core.delete_entry(&handle, &path, kind).await?;
            Ok(Value::Null)
        }
        "files_paste" => {
            let handle: String = from_args(&args, "handle")?;
            let sources: Vec<String> = from_args(&args, "sources")?;
            let destination: String = from_args(&args, "destination")?;
            let moving: bool = from_args(&args, "moving")?;
            let overwrite: bool = from_args(&args, "overwrite")?;
            to_value(
                state
                    .core
                    .paste_entries(&handle, &sources, &destination, moving, overwrite)
                    .await?,
            )
        }
        "files_read_text" => {
            let handle: String = from_args(&args, "handle")?;
            let path: String = from_args(&args, "path")?;
            to_value(state.core.read_text_file(&handle, &path).await?)
        }
        "files_write_text" => {
            let handle: String = from_args(&args, "handle")?;
            let path: String = from_args(&args, "path")?;
            let text: String = from_args(&args, "text")?;
            state.core.write_text_file(&handle, &path, &text).await?;
            Ok(Value::Null)
        }
        "files_close" => {
            let handle: String = from_args(&args, "handle")?;
            state.core.close_files(&handle).await?;
            Ok(Value::Null)
        }
        "files_upload_staged" => {
            let handle: String = from_args(&args, "handle")?;
            let directory: String = from_args(&args, "directory")?;
            let tokens: Vec<String> = from_args(&args, "tokens")?;
            let packed: bool = from_args(&args, "packed")?;
            let sources = {
                let staging = state.staging.lock().await;
                tokens
                    .iter()
                    .map(|token| staging.get(token).map(|file| file.path.clone()))
                    .collect::<Option<Vec<_>>>()
            };
            let Some(sources) = sources else {
                return Err(ShellError::app_internal(
                    "a staged upload is no longer available; choose the files again",
                ));
            };
            to_value(start_or_ask(state, handle, directory, sources, packed).await?)
        }
        "files_upload_answer" => {
            let token: String = from_args(&args, "token")?;
            let answer: UploadAnswer = from_args(&args, "answer")?;
            let upload = state.pending_uploads.lock().await.remove(&token);
            let Some(upload) = upload else {
                return Err(ShellError::app_internal(
                    "that upload is no longer waiting for an answer",
                ));
            };
            let sources = match answer {
                UploadAnswer::Overwrite => upload.sources.clone(),
                UploadAnswer::Skip => upload.missing(),
                UploadAnswer::Cancel => return to_value(UploadOutcome::Cancelled),
            };
            if sources.is_empty() {
                return to_value(UploadOutcome::Cancelled);
            }
            to_value(start_uploads(state, &upload.handle, &upload.directory, &sources, upload.packed).await?)
        }
        "files_download_browser" => {
            let handle: String = from_args(&args, "handle")?;
            let paths: Vec<String> = from_args(&args, "paths")?;
            let packed: bool = from_args(&args, "packed")?;
            if paths.is_empty() {
                return Ok(json!({ "transferId": null }));
            }

            let directory = staging_root().join(format!("download-{}", uuid::Uuid::new_v4().simple()));
            tokio::fs::create_dir_all(&directory).await.map_err(|error| {
                ShellError::app_internal(format!("the download folder could not be created: {error}"))
            })?;

            let id = if packed || paths.len() > 1 {
                state
                    .core
                    .start_packed_download(&handle, &paths, directory.clone())
                    .await?
            } else {
                state.core.start_download_into(&handle, &paths[0], &directory).await?
            };
            state.browser_downloads.lock().await.insert(id.clone(), directory);
            Ok(json!({ "transferId": id }))
        }
        "files_download_browser_take" => {
            let transfer_id: String = from_args(&args, "transferId")?;
            let directory = state
                .browser_downloads
                .lock()
                .await
                .get(&transfer_id)
                .cloned()
                .ok_or_else(|| ShellError::app_internal("that browser download is no longer waiting"))?;
            let row = state
                .core
                .transfers()
                .await
                .into_iter()
                .find(|row| row.id == transfer_id)
                .ok_or_else(|| ShellError::app_internal("that browser download is no longer in the list"))?;
            match row.state {
                TransferState::Done => {}
                TransferState::Failed { detail } => {
                    state.browser_downloads.lock().await.remove(&transfer_id);
                    let _ = tokio::fs::remove_dir_all(&directory).await;
                    return Err(ShellError::sftp_transfer_failed(&row.remote_path, detail));
                }
                TransferState::Cancelled => {
                    state.browser_downloads.lock().await.remove(&transfer_id);
                    let _ = tokio::fs::remove_dir_all(&directory).await;
                    return Err(ShellError::app_internal("the download was stopped"));
                }
                _ => return Ok(json!({ "state": "running", "files": [] })),
            }

            state.browser_downloads.lock().await.remove(&transfer_id);
            let mut files = Vec::new();
            let mut entries = tokio::fs::read_dir(&directory)
                .await
                .map_err(|error| ShellError::app_internal(format!("the download folder could not be read: {error}")))?;
            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|error| ShellError::app_internal(format!("the download folder could not be read: {error}")))?
            {
                if !entry.file_type().await.map(|kind| kind.is_file()).unwrap_or(false) {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                let token = uuid::Uuid::new_v4().simple().to_string();
                state.staging.lock().await.insert(
                    token.clone(),
                    StagedFile {
                        path: entry.path(),
                        name: name.clone(),
                    },
                );
                files.push(json!({ "token": token, "name": name }));
            }
            if files.is_empty() {
                return Err(ShellError::app_internal("the download produced no file"));
            }
            Ok(json!({ "state": "done", "files": files }))
        }
        "transfers_open" => to_value(state.core.transfers().await),
        "transfer_stop" => {
            let id: String = from_args(&args, "id")?;
            state.core.stop_transfer(&id).await?;
            Ok(Value::Null)
        }
        "transfer_remove" => {
            let id: String = from_args(&args, "id")?;
            state.core.remove_transfer(&id).await?;
            Ok(Value::Null)
        }
        "transfer_stop_all" => {
            state.core.stop_active_transfers().await?;
            Ok(Value::Null)
        }
        "transfer_clear_settled" => {
            state.core.clear_settled_transfers().await?;
            Ok(Value::Null)
        }
        "transfer_pause" => {
            let id: String = from_args(&args, "id")?;
            state.core.pause_transfer(&id).await?;
            Ok(Value::Null)
        }
        "transfer_resume" => {
            let id: String = from_args(&args, "id")?;
            state.core.resume_transfer(&id).await?;
            Ok(Value::Null)
        }
        "transfer_retry" => {
            let id: String = from_args(&args, "id")?;
            state.core.retry_transfer(&id).await?;
            Ok(Value::Null)
        }
        "transfer_open" | "transfer_reveal" => Ok(Value::Null),
        "download_folder_get" | "download_folder_select" | "download_folder_ask" => to_value(DownloadFolderView {
            current: None,
            history: Vec::new(),
        }),
        "download_folder_choose" => Ok(Value::Null),
        "download_folder_reveal" => Ok(Value::Null),
        "update_check" => to_value(UpdateCheck::NotInstalled {
            current_version: env!("CARGO_PKG_VERSION").to_owned(),
        }),

        "snippets_open" => to_value(state.core.snippet_groups()?),
        "snippet_group_save" => {
            let group: SnippetGroup = from_args(&args, "group")?;
            to_value(state.core.save_snippet_group(group)?)
        }
        "snippet_group_delete" => {
            let id: String = from_args(&args, "id")?;
            state.core.delete_snippet_group(&id)?;
            Ok(Value::Null)
        }
        "snippet_save" => {
            let snippet: Snippet = from_args(&args, "snippet")?;
            to_value(state.core.save_snippet(snippet)?)
        }
        "snippet_delete" => {
            let id: String = from_args(&args, "id")?;
            state.core.delete_snippet(&id)?;
            Ok(Value::Null)
        }
        "completion_words" => to_value(state.core.completion_words()?),
        "completion_word_save" => {
            let word: CompletionWord = from_args(&args, "word")?;
            to_value(state.core.save_completion_word(word)?)
        }
        "completion_word_delete" => {
            let id: String = from_args(&args, "id")?;
            state.core.delete_completion_word(&id)?;
            Ok(Value::Null)
        }
        "completion_words_clear" => {
            state.core.clear_completion_words()?;
            Ok(Value::Null)
        }
        "command_history" => to_value(state.core.command_history()?),
        "command_history_clear" => {
            state.core.clear_command_history()?;
            Ok(Value::Null)
        }
        "command_remember" => {
            let line: String = from_args(&args, "line")?;
            state.core.remember_command(&line)?;
            Ok(Value::Null)
        }
        "command_classify" => {
            let command: String = from_args(&args, "command")?;
            to_value(sevenshell_core::classify_send(&command))
        }
        "command_send" => {
            let target: SendTarget = from_args(&args, "target")?;
            let command: String = from_args(&args, "command")?;
            let append_cr: bool = from_args(&args, "appendCr")?;
            let acknowledged: bool = from_args(&args, "acknowledged")?;
            to_value(
                state
                    .core
                    .send_command(target, &command, append_cr, acknowledged)
                    .await?,
            )
        }
        "completion_execute" => {
            let request: CompletionExecuteRequest = from_args(&args, "request")?;
            to_value(state.core.run_completion(request).await?)
        }
        "batch_run" => {
            let request: BatchRequest = from_args(&args, "request")?;
            let (progress, mut events) = mpsc::channel::<BatchEvent>(64);
            let sender = state.events.clone();
            tokio::spawn(async move {
                while let Some(event) = events.recv().await {
                    let payload = match event {
                        BatchEvent::HostStarted { session_id } => {
                            json!({ "kind": "hostStarted", "sessionId": session_id })
                        }
                        BatchEvent::HostFinished { result } => json!({ "kind": "hostFinished", "result": result }),
                    };
                    let _ = sender.send(json!({ "event": "batch:event", "payload": payload }).to_string());
                }
            });
            to_value(state.core.run_across(request, progress).await?)
        }

        "monitor_sample" => {
            let handle: String = from_args(&args, "handle")?;
            to_value(state.core.sample_host(&handle).await?)
        }
        "monitor_processes" => {
            let handle: String = from_args(&args, "handle")?;
            let limit: usize = from_args(&args, "limit")?;
            to_value(state.core.list_processes(&handle, limit).await?)
        }
        "monitor_process_detail" => {
            let handle: String = from_args(&args, "handle")?;
            let pid: u32 = from_args(&args, "pid")?;
            to_value(state.core.process_detail(&handle, pid).await?)
        }
        "monitor_sockets" => {
            let handle: String = from_args(&args, "handle")?;
            to_value(state.core.list_sockets(&handle).await?)
        }

        "key_generate" => {
            let algorithm: KeyAlgorithm = from_args(&args, "algorithm")?;
            let comment: String = from_args(&args, "comment")?;
            let passphrase: Option<String> = optional_args(&args, "passphrase")?;
            let stored = passphrase.clone();
            let making = passphrase;
            let generated = tokio::task::spawn_blocking(move || {
                sevenshell_ssh::generate(algorithm, comment.trim(), making.as_deref())
            })
            .await
            .map_err(|error| ShellError::app_internal(format!("the key could not be generated: {error}")))??;
            let description = generated.description().clone();
            let info = KeyInfo {
                key_ref: None,
                display_name: next_key_name(state, algorithm)?,
                algorithm: description.algorithm,
                fingerprint: description.fingerprint,
                has_passphrase: description.has_passphrase,
            };
            let info = state
                .vault
                .import_key(generated.private_text().to_owned(), stored, info)?;
            to_value(CreatedKey {
                info,
                public_key: generated.public_line().to_owned(),
            })
        }
        "key_public" => {
            let key_ref: String = from_args(&args, "keyRef")?;
            let material = state
                .vault
                .key_material(&key_ref)?
                .ok_or_else(|| ShellError::ssh_key_invalid("that key is no longer stored"))?;
            to_value(sevenshell_ssh::public_line(
                &material.contents,
                material.passphrase.as_deref(),
            )?)
        }
        "key_forget" => {
            let key_ref: String = from_args(&args, "keyRef")?;
            state.vault.forget_key(&key_ref)?;
            Ok(Value::Null)
        }
        "key_import_text" => {
            let display_name: String = from_args(&args, "displayName")?;
            let contents: String = from_args(&args, "contents")?;
            let passphrase: Option<String> = optional_args(&args, "passphrase")?;
            if contents.len() > 1024 * 1024 {
                return Err(ShellError::ssh_key_invalid(
                    "that file is far too large to be a private key",
                ));
            }
            let description = sevenshell_ssh::describe_key(&contents, passphrase.as_deref())?;
            let info = KeyInfo {
                key_ref: None,
                display_name,
                algorithm: description.algorithm,
                fingerprint: description.fingerprint,
                has_passphrase: passphrase.is_some() || description.has_passphrase,
            };
            to_value(state.vault.import_key(contents, passphrase, info)?)
        }

        "background_image_read" => {
            let directory = sevenshell_platform::config_dir()
                .ok_or_else(|| ShellError::store_io("the configuration directory could not be located"))?;
            let bytes = tokio::task::spawn_blocking(move || sevenshell_platform::background::read(&directory))
                .await
                .map_err(|error| ShellError::store_io(format!("reading the background picture failed: {error}")))??;
            to_value(BASE64.encode(bytes))
        }
        "background_image_clear" => {
            let directory = sevenshell_platform::config_dir()
                .ok_or_else(|| ShellError::store_io("the configuration directory could not be located"))?;
            tokio::task::spawn_blocking(move || sevenshell_platform::background::clear(&directory))
                .await
                .map_err(|error| ShellError::store_io(format!("removing the background picture failed: {error}")))??;
            Ok(Value::Null)
        }
        "background_image_choose_staged" => {
            let token: String = from_args(&args, "token")?;
            let staged = state.staging.lock().await.get(&token).cloned();
            let Some(staged) = staged else {
                return Err(ShellError::store_io("that image is no longer staged; choose it again"));
            };
            let directory = sevenshell_platform::config_dir()
                .ok_or_else(|| ShellError::store_io("the configuration directory could not be located"))?;
            let name = staged.name.clone();
            tokio::task::spawn_blocking(move || sevenshell_platform::background::store(&directory, &staged.path))
                .await
                .map_err(|error| {
                    ShellError::store_io(format!("the background picture could not be stored: {error}"))
                })??;
            to_value(name)
        }

        "vault_archive_verify" => {
            let name: String = from_args(&args, "name")?;
            let password: String = from_args(&args, "password")?;
            state.vault.verify_archive(&name, &password)?;
            Ok(Value::Null)
        }
        "vault_archive_activate" => {
            let name: String = from_args(&args, "name")?;
            to_value(state.vault.activate_archive(&name)?)
        }
        "vault_archive_import" => {
            let name: String = from_args(&args, "name")?;
            let password: String = from_args(&args, "password")?;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs())
                .unwrap_or_default();
            to_value(state.vault.import_archive(&name, &password, now)?)
        }
        "vault_reveal_folder" => Ok(Value::Null),
        "desktop_open" => {
            let session_id: String = from_args(&args, "sessionId")?;
            state.core.open_desktop(&session_id).await?;
            Ok(Value::Null)
        }

        "vmrest_config_set" => {
            let user: String = from_args(&args, "user")?;
            let password: String = from_args(&args, "password")?;
            state.core.set_vmrest_auth(user, password)?;
            Ok(Value::Null)
        }
        "vmrest_status" => {
            state.core.vmrest_probe().await?;
            Ok(Value::Null)
        }
        "vmrest_start" => Err(ShellError::app_unsupported("starting vmrest needs the desktop shell")),
        "host_location_choose" => Err(ShellError::app_unsupported(
            "choosing a host location needs the desktop shell",
        )),
        "host_locations_list" => to_value(state.core.host_locations()?),
        "host_location_forget" => {
            let token: String = from_args(&args, "token")?;
            Ok(json!(state.core.host_location_forget(&token)?))
        }
        "vmware_scan" => to_value(state.core.vmware_scan().await?),
        "vmware_status" => {
            let vm_id: String = from_args(&args, "vmId")?;
            to_value(state.core.vmware_status(&vm_id).await?)
        }
        "vmware_power" => {
            let vm_id: String = from_args(&args, "vmId")?;
            let action: VmPowerAction = from_args(&args, "action")?;
            let acknowledged: bool = from_args(&args, "acknowledged")?;
            state.core.vmware_power(&vm_id, action, acknowledged).await?;
            Ok(Value::Null)
        }
        "vmware_take_ownership" => {
            let vm_id: String = from_args(&args, "vmId")?;
            let acknowledged: bool = from_args(&args, "acknowledged")?;
            state.core.vmware_take_ownership(&vm_id, acknowledged).await?;
            Ok(Value::Null)
        }

        "container_connect" => {
            let session_id: String = from_args(&args, "sessionId")?;
            to_value(state.core.container_connect(&session_id).await?)
        }
        "container_disconnect" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            state.core.container_disconnect(&connection_id).await;
            Ok(Value::Null)
        }
        "container_list" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            to_value(state.core.container_list(&connection_id).await?)
        }
        "container_inspect" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let container_id: String = from_args(&args, "containerId")?;
            to_value(state.core.container_inspect(&connection_id, &container_id).await?)
        }
        "container_action" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let container_id: String = from_args(&args, "containerId")?;
            let action: String = from_args(&args, "action")?;
            let acknowledged: bool = from_args(&args, "acknowledged")?;
            state
                .core
                .container_action(&connection_id, &container_id, &action, acknowledged)
                .await?;
            Ok(Value::Null)
        }
        "container_rename" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let container_id: String = from_args(&args, "containerId")?;
            let name: String = from_args(&args, "name")?;
            state
                .core
                .container_rename(&connection_id, &container_id, &name)
                .await?;
            Ok(Value::Null)
        }
        "container_create" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let request: ContainerCreateRequest = from_args(&args, "request")?;
            state.core.container_create(&connection_id, &request).await?;
            Ok(Value::Null)
        }
        "container_stats" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            to_value(state.core.container_stats(&connection_id).await?)
        }
        "container_namespaces" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            to_value(state.core.container_namespaces(&connection_id).await?)
        }
        "container_set_namespace" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let namespace: String = from_args(&args, "namespace")?;
            state.core.container_set_namespace(&connection_id, &namespace)?;
            Ok(Value::Null)
        }
        "container_runtime_info" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            to_value(state.core.container_runtime_info(&connection_id).await?)
        }
        "container_logs_start" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let container_id: String = from_args(&args, "containerId")?;
            let tail: u32 = from_args(&args, "tail")?;
            let timestamps: bool = from_args(&args, "timestamps")?;
            let (events, lines) = tokio::sync::mpsc::unbounded_channel();
            let (end, ends) = tokio::sync::mpsc::unbounded_channel();
            let stream_id = state
                .core
                .container_logs_start(&connection_id, &container_id, tail, timestamps, events, end)
                .await?;
            forward_container_stream(state.events.clone(), lines, ends);
            Ok(json!(stream_id))
        }
        "container_stream_stop" => {
            let stream_id: String = from_args(&args, "streamId")?;
            state.core.container_stream_stop(&stream_id).await;
            Ok(Value::Null)
        }
        "container_exec_open" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let container_id: String = from_args(&args, "containerId")?;
            let shell: String = from_args(&args, "shell")?;
            to_value(
                state
                    .core
                    .container_exec_open(&connection_id, &container_id, &shell, 80, 24)
                    .await?,
            )
        }
        "container_images" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            to_value(state.core.container_images(&connection_id).await?)
        }
        "container_image_inspect" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let image: String = from_args(&args, "image")?;
            to_value(state.core.container_image_inspect(&connection_id, &image).await?)
        }
        "container_image_history" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let image: String = from_args(&args, "image")?;
            to_value(state.core.container_image_history(&connection_id, &image).await?)
        }
        "container_image_pull" | "container_image_push" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let request: ContainerImageTransfer = from_args(&args, "request")?;
            let push = cmd == "container_image_push";
            let (events, lines) = tokio::sync::mpsc::unbounded_channel();
            let (end, ends) = tokio::sync::mpsc::unbounded_channel();
            let stream_id = state
                .core
                .container_image_transfer_start(&connection_id, &request, push, events, end)
                .await?;
            forward_container_stream(state.events.clone(), lines, ends);
            Ok(json!(stream_id))
        }
        "container_image_remove" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let image: String = from_args(&args, "image")?;
            let acknowledged: bool = from_args(&args, "acknowledged")?;
            state
                .core
                .container_image_remove(&connection_id, &image, acknowledged)
                .await?;
            Ok(Value::Null)
        }
        "container_image_prune" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let acknowledged: bool = from_args(&args, "acknowledged")?;
            state.core.container_image_prune(&connection_id, acknowledged).await?;
            Ok(Value::Null)
        }
        "container_image_tag" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let image: String = from_args(&args, "image")?;
            let tag: String = from_args(&args, "tag")?;
            state.core.container_image_tag(&connection_id, &image, &tag).await?;
            Ok(Value::Null)
        }
        "container_image_load" => Ok(Value::Null),
        "container_image_load_staged" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let token: String = from_args(&args, "token")?;
            let staged = state.staging.lock().await.get(&token).cloned();
            let Some(staged) = staged else {
                return Err(ShellError::app_internal(
                    "that archive is no longer staged; choose it again",
                ));
            };
            to_value(state.core.container_image_load(&connection_id, &staged.path).await?)
        }
        "container_image_commit" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let request: ContainerCommitRequest = from_args(&args, "request")?;
            let acknowledged: bool = from_args(&args, "acknowledged")?;
            to_value(
                state
                    .core
                    .container_image_commit(&connection_id, &request, acknowledged)
                    .await?,
            )
        }
        "container_registry_login" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let request: ContainerRegistryLogin = from_args(&args, "request")?;
            state.core.container_registry_login(&connection_id, &request).await?;
            Ok(Value::Null)
        }

        "k8s_contexts" => {
            let kubeconfig: Option<SecretInput> = optional_args(&args, "kubeconfig")?;
            to_value(state.core.k8s_contexts(kubeconfig).await?)
        }
        "k8s_connect" => {
            let session_id: String = from_args(&args, "sessionId")?;
            to_value(state.core.k8s_connect(&session_id).await?)
        }
        "k8s_disconnect" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            state.core.k8s_disconnect(&connection_id);
            Ok(Value::Null)
        }
        "k8s_request" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let request: K8sRequest = from_args(&args, "request")?;
            to_value(state.core.k8s_request(&connection_id, request).await?)
        }
        "k8s_watch_start" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let path: String = from_args(&args, "path")?;
            let (events, event_receiver) = tokio::sync::mpsc::unbounded_channel();
            let (end, end_receiver) = tokio::sync::mpsc::unbounded_channel();
            let stream_id = state.core.k8s_watch_start(&connection_id, &path, events, end)?;
            forward_k8s_watch(state.events.clone(), event_receiver, end_receiver);
            Ok(json!(stream_id))
        }
        "k8s_logs_start" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let request: K8sLogRequest = from_args(&args, "request")?;
            let (events, lines) = tokio::sync::mpsc::unbounded_channel();
            let (end, ends) = tokio::sync::mpsc::unbounded_channel();
            let stream_id = state.core.k8s_logs_start(&connection_id, &request, events, end).await?;
            forward_k8s_log(state.events.clone(), lines, ends);
            Ok(json!(stream_id))
        }
        "k8s_stream_stop" => {
            let stream_id: String = from_args(&args, "streamId")?;
            state.core.k8s_stream_stop(&stream_id);
            Ok(Value::Null)
        }
        "k8s_exec_open" => {
            let connection_id: String = from_args(&args, "connectionId")?;
            let request: K8sExecRequest = from_args(&args, "request")?;
            to_value(state.core.k8s_exec_open(&connection_id, &request, 80, 24).await?)
        }

        "sync_available" => to_value(sync::available(&state.vault)?),
        "sync_status" => to_value(sync::status(&state.vault)?),
        "sync_password" => {
            let password: Option<String> = optional_args(&args, "password")?;
            to_value(sync::set_password(&state.vault, password)?)
        }
        "sync_passphrase" => {
            let passphrase: Option<String> = optional_args(&args, "passphrase")?;
            to_value(sync::set_passphrase(&state.vault, passphrase)?)
        }
        "sync_test" => {
            sync::test(&state.vault).await?;
            Ok(Value::Null)
        }
        "sync_upload" => {
            let selection: SyncSelection = from_args(&args, "selection")?;
            let passphrase: Option<String> = optional_args(&args, "passphrase")?;
            to_value(sync::upload(&state.vault, selection, passphrase).await?)
        }
        "sync_download" => {
            let passphrase: Option<String> = optional_args(&args, "passphrase")?;
            to_value(sync::download(&state.vault, passphrase).await?)
        }
        "sync_apply" => {
            let replace: bool = from_args(&args, "replace")?;
            let passphrase: Option<String> = optional_args(&args, "passphrase")?;
            to_value(sync::apply_sync(&state.vault, replace, passphrase).await?)
        }
        "sync_backup_export" | "sync_backup_inspect" | "sync_backup_restore" => Err(ShellError::app_internal(
            "backup files use the browser file picker in this edition; that path is not wired yet",
        )),
        "sync_backup_export_browser" => {
            let file_name: String =
                optional_args(&args, "fileName")?.unwrap_or_else(|| "7shell-backup.json".to_owned());
            let selection: SyncSelection = from_args(&args, "selection")?;
            let passphrase: Option<String> = optional_args(&args, "passphrase")?;
            let (receipt, contents) = sync::export_text(&state.vault, selection, passphrase, file_name)?;
            Ok(json!({ "receipt": receipt, "contents": contents }))
        }
        "sync_backup_inspect_browser" => {
            let contents: String = from_args(&args, "contents")?;
            let file_name: String =
                optional_args(&args, "fileName")?.unwrap_or_else(|| "7shell-backup.json".to_owned());
            let passphrase: Option<String> = optional_args(&args, "passphrase")?;
            to_value(sync::inspect_text(&contents, file_name, passphrase.as_deref())?)
        }
        "sync_backup_restore_browser" => {
            let contents: String = from_args(&args, "contents")?;
            let passphrase: Option<String> = optional_args(&args, "passphrase")?;
            let replace: bool = from_args(&args, "replace")?;
            to_value(sync::restore_text(
                &state.vault,
                &contents,
                passphrase.as_deref(),
                replace,
            )?)
        }

        "snippet_sync_status" => to_value(sync::snippet_status(&state.vault)?),
        "snippet_sync_token" => {
            let token: Option<String> = optional_args(&args, "token")?;
            to_value(sync::set_snippet_token(&state.vault, token)?)
        }
        "snippet_sync_test" => {
            sync::snippet_test(&state.vault).await?;
            Ok(Value::Null)
        }
        "snippet_sync_upload" => {
            let selection: SyncSelection = from_args(&args, "selection")?;
            let passphrase: Option<String> = optional_args(&args, "passphrase")?;
            to_value(sync::snippet_upload(&state.vault, selection, passphrase).await?)
        }
        "snippet_sync_download" => {
            let passphrase: Option<String> = optional_args(&args, "passphrase")?;
            to_value(sync::snippet_download(&state.vault, passphrase).await?)
        }
        "snippet_sync_apply" => {
            let replace: bool = from_args(&args, "replace")?;
            let passphrase: Option<String> = optional_args(&args, "passphrase")?;
            to_value(sync::snippet_apply(&state.vault, replace, passphrase).await?)
        }

        "policy_audit_tail" => {
            let limit: usize = from_args(&args, "limit")?;
            let tail = state
                .policy_log
                .path()
                .map(|path| policy::read_tail(path, limit))
                .unwrap_or_default();
            to_value(tail)
        }
        "ai_providers" => Ok(json!([])),
        "ai_sessions_list" => Ok(json!([])),
        "mcp_status" | "mcp_enable" | "mcp_disable" => Ok(mcp_status_disabled(state.tools.pending_approvals())),
        "mcp_catalog" => Ok(json!({ "tools": [], "counts": { "total": 0, "enabled": 0 } })),
        "mcp_servers_list" => Ok(json!([])),
        "mcp_client_config" => Ok(json!("")),
        "mcp_audit_tail" => {
            let limit: usize = optional_args(&args, "limit")?.unwrap_or(100);
            to_value(state.tools.audit_tail(limit))
        }
        "mcp_approval_answer" => {
            let call_id: String = from_args(&args, "callId")?;
            let decision: String = from_args(&args, "decision")?;
            let answer = if decision == "approved" {
                sevenshell_api::ApprovalDecision::Approved
            } else {
                sevenshell_api::ApprovalDecision::Denied
            };
            state.tools.answer_approval(&call_id, answer);
            Ok(Value::Null)
        }
        "window_theme_apply" | "window_quit" | "window_quit_guard" => Ok(Value::Null),

        _ => Err(ShellError::app_internal(format!(
            "the browser backend does not answer `{cmd}` yet"
        ))),
    }
}

pub(crate) fn staging_root() -> PathBuf {
    sevenshell_platform::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("web-staging")
}

async fn upload(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    body: Body,
) -> Response {
    if !authorized(&headers, &query, &state) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let name = headers
        .get("x-sshell-name")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| BASE64.decode(value).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_else(|| "file".to_owned());
    let name = name
        .rsplit(['/', '\\'])
        .next()
        .map(str::trim)
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .unwrap_or("file")
        .to_owned();

    let token = uuid::Uuid::new_v4().simple().to_string();
    let directory = staging_root().join(&token);
    if let Err(error) = tokio::fs::create_dir_all(&directory).await {
        log::warn!("a staged upload directory could not be created: {error}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let path = directory.join(&name);
    let mut file = match tokio::fs::File::create(&path).await {
        Ok(file) => file,
        Err(error) => {
            log::warn!("a staged upload could not be created: {error}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let bytes = match chunk {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("a staged upload was interrupted: {error}");
                return StatusCode::BAD_REQUEST.into_response();
            }
        };
        if let Err(error) = file.write_all(&bytes).await {
            log::warn!("a staged upload could not be written: {error}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    if let Err(error) = file.flush().await {
        log::warn!("a staged upload could not be flushed: {error}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    state.staging.lock().await.insert(
        token.clone(),
        StagedFile {
            path,
            name: name.clone(),
        },
    );
    (StatusCode::OK, Json(json!({ "token": token, "name": name }))).into_response()
}

async fn download(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    UrlPath(token): UrlPath<String>,
) -> Response {
    if !authorized(&headers, &query, &state) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let staged = state.staging.lock().await.get(&token).cloned();
    let Some(staged) = staged else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let file = match tokio::fs::File::open(&staged.path).await {
        Ok(file) => file,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };

    let ascii: String = staged
        .name
        .chars()
        .map(|character| {
            if character.is_ascii_graphic() || character == ' ' {
                character
            } else {
                '_'
            }
        })
        .collect();
    let encoded = utf8_percent_encode(&staged.name, NON_ALPHANUMERIC);
    let disposition = format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}");
    match Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_DISPOSITION, disposition)
        .body(Body::from_stream(ReaderStream::new(file)))
    {
        Ok(response) => response,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn invoke(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    Json(request): Json<InvokeRequest>,
) -> Response {
    if !authorized(&headers, &query, &state) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match dispatch(&state, &request.cmd, request.args).await {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => (StatusCode::OK, Json(error)).into_response(),
    }
}

async fn events(
    ws: WebSocketUpgrade,
    State(state): State<Arc<WebState>>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&headers, &query, &state) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let receiver = state.events.subscribe();
    ws.on_upgrade(move |socket| stream_events(socket, receiver))
}

async fn stream_events(mut socket: WebSocket, mut receiver: broadcast::Receiver<String>) {
    loop {
        tokio::select! {
            event = receiver.recv() => match event {
                Ok(payload) => {
                    if socket.send(Message::Text(payload.into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(_)) => break,
            },
        }
    }
}

async fn pump_core_events(
    mut receiver: mpsc::Receiver<CoreEvent>,
    events: broadcast::Sender<String>,
    zmodem: Arc<zmodem::Zmodem>,
) {
    while let Some(event) = receiver.recv().await {
        match event.event {
            TerminalEvent::Data(bytes) => zmodem.feed(&event.handle, bytes).await,
            TerminalEvent::Closed { reason } => {
                zmodem.forget(&event.handle).await;
                let payload = json!({ "handle": event.handle, "reason": reason });
                let _ = events.send(json!({ "event": "terminal:closed", "payload": payload }).to_string());
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|argument| argument == "-h" || argument == "--help") {
        print!("{HELP}");
        return Ok(());
    }

    let port: u16 = std::env::var("SEVENSHELL_WEB_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(4225);
    let token = std::env::var("SEVENSHELL_WEB_TOKEN")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    let static_dir = std::env::var_os("SEVENSHELL_WEB_STATIC")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("apps/desktop/dist"));
    let index = static_dir.join("index.html");

    let vault_path = sevenshell_vault::default_path().unwrap_or_else(|| PathBuf::from("vault.json"));
    let vault = Arc::new(Vault::open_at(vault_path));
    let (core, core_events) = Core::new(Arc::clone(&vault));
    let (event_sender, _) = broadcast::channel(512);
    let staging = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let zmodem = Arc::new(zmodem::Zmodem::new(core.clone(), event_sender.clone(), staging.clone()));
    zmodem.spawn_ticker();
    tokio::spawn(pump_core_events(core_events, event_sender.clone(), zmodem.clone()));

    let policy_log = Arc::new(match policy::default_path() {
        Ok(path) => policy::PolicyLog::new(path),
        Err(_) => policy::PolicyLog::at(None),
    });
    let settings = sevenshell_platform::load().ok();
    if let Some(settings) = settings.as_ref() {
        policy_log.configure(settings.mcp_audit_retention_days);
    }
    let journal: Arc<dyn sevenshell_policy::PolicyJournal> = policy_log.clone();
    core.set_policy(
        settings.map(|settings| settings.policy_matrix).unwrap_or_default(),
        Some(journal),
    );

    let tools = Arc::new(agent::ToolRuntime::new(
        vault.clone(),
        core.clone(),
        event_sender.clone(),
    )?);
    let ai = Arc::new(ai::AiRuntime::new(
        vault.clone(),
        core.clone(),
        event_sender.clone(),
        tools.clone(),
    )?);
    let mcp = Arc::new(mcp::McpRuntime::new(
        vault.clone(),
        core.clone(),
        event_sender.clone(),
        tools.clone(),
        port,
    )?);
    mcp.restore().await;

    let state = Arc::new(WebState {
        token: token.clone(),
        events: event_sender,
        vault,
        core,
        policy_log,
        tools,
        ai,
        mcp,
        staging,
        pending_uploads: tokio::sync::Mutex::new(HashMap::new()),
        browser_downloads: tokio::sync::Mutex::new(HashMap::new()),
        zmodem,
    });
    let app = Router::new()
        .route("/invoke", post(invoke))
        .route("/events", get(events))
        .route("/oauth/callback", get(oauth_callback))
        .route(
            "/upload",
            post(upload).layer(DefaultBodyLimit::max(8 * 1024 * 1024 * 1024)),
        )
        .route("/download/{token}", get(download))
        .fallback_service(ServeDir::new(static_dir.clone()).not_found_service(ServeFile::new(index)))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .with_state(state);

    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = tokio::net::TcpListener::bind(address).await?;
    println!("7Shell browser UI: http://127.0.0.1:{port}/?token={token}");
    println!("serving the interface from {}", static_dir.display());
    axum::serve(listener, app).await?;
    Ok(())
}
