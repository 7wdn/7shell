// The local browser service: one loopback HTTP listener that serves the built interface and answers
// the backend calls the interface makes. It is a second host, not a second product: the interface,
// the core and the command contract are the same ones the desktop shell uses. Nothing here knows
// Tauri, and nothing here listens beyond 127.0.0.1.
//
// The token is the whole authentication story: the launcher prints a URL containing it, the client
// sends it with every call and the event socket, and anything else on the loopback is refused.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sevenshell_core::{Core, CoreEvent};
use sevenshell_error::ShellError;
use sevenshell_session::TerminalEvent;
use sevenshell_types::{
    AppInfo, AppSettings, AuthMethod, DownloadFolderView, EntryKind, PresentedHostKey, Protocol, SecretInput, Session,
    SessionGroup, UpdateCheck, VaultState,
};
use sevenshell_vault::{SessionSecret, Vault, VaultStatus};
use tokio::sync::{broadcast, mpsc};
use tower_http::services::{ServeDir, ServeFile};

const HELP: &str = r#"Usage: sshell-web [OPTION]

Serve the 7Shell interface on this machine and let the system browser open it.

Options:
  -h, --help  Show this help message and exit.

Environment variables:
  SEVENSHELL_WEB_PORT    Listen port (default: 4225)
  SEVENSHELL_WEB_TOKEN   Access token (default: one random value per start)
  SEVENSHELL_WEB_STATIC  Built interface directory (default: apps/desktop/dist)
"#;

struct WebState {
    token: String,
    events: broadcast::Sender<String>,
    vault: Arc<Vault>,
    core: Arc<Core>,
}

#[derive(Deserialize)]
struct TokenQuery {
    token: Option<String>,
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

async fn dispatch(state: &WebState, cmd: &str, args: Value) -> Result<Value, ShellError> {
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
        "vault_set_aside" => to_value(state.vault.set_aside()?),
        "vault_archives" => to_value(state.vault.archives()?),

        "sessions_list" => to_value(state.vault.sessions()?),
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
        "terminal_close" => {
            let handle: String = from_args(&args, "handle")?;
            state.core.close_terminal(&handle).await?;
            Ok(Value::Null)
        }
        "terminals_open" => to_value(state.core.open_terminals().await),
        "terminal_local_shells" => Ok(json!([])),
        "terminal_wsl_distros" => Ok(json!([])),
        "terminal_serial_ports" => Ok(json!([])),
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

        "policy_audit_tail" => Ok(json!([])),
        "ai_providers" => Ok(json!([])),
        "ai_sessions_list" => Ok(json!([])),
        "mcp_status" => Ok(json!({
            "enabled": false,
            "port": null,
            "scope": "readOnly",
            "clientConfig": "",
            "auditPath": "",
            "pendingApprovals": 0
        })),
        "window_theme_apply" | "window_quit" | "window_quit_guard" => Ok(Value::Null),

        _ => Err(ShellError::app_internal(format!(
            "the browser backend does not answer `{cmd}` yet"
        ))),
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

async fn pump_core_events(mut receiver: mpsc::Receiver<CoreEvent>, events: broadcast::Sender<String>) {
    while let Some(event) = receiver.recv().await {
        let (name, payload) = match event.event {
            TerminalEvent::Data(bytes) => (
                "terminal:data",
                json!({ "handle": event.handle, "base64": BASE64.encode(bytes) }),
            ),
            TerminalEvent::Closed { reason } => {
                ("terminal:closed", json!({ "handle": event.handle, "reason": reason }))
            }
        };
        let _ = events.send(json!({ "event": name, "payload": payload }).to_string());
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
    tokio::spawn(pump_core_events(core_events, event_sender.clone()));

    let state = Arc::new(WebState {
        token: token.clone(),
        events: event_sender,
        vault,
        core,
    });
    let app = Router::new()
        .route("/invoke", post(invoke))
        .route("/events", get(events))
        .fallback_service(ServeDir::new(static_dir.clone()).not_found_service(ServeFile::new(index)))
        .with_state(state);

    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = tokio::net::TcpListener::bind(address).await?;
    println!("7Shell browser UI: http://127.0.0.1:{port}/?token={token}");
    println!("serving the interface from {}", static_dir.display());
    axum::serve(listener, app).await?;
    Ok(())
}
