//! External MCP servers in the browser host: configuration, secrets, connections and content.
//!
//! The loopback tool listener stays a desktop feature for now; this module owns the other
//! direction — the servers this installation calls. The manager is the same one the desktop host
//! uses, so validation, transport and the tool catalogue do not get a second implementation here.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{Value, json};
use sevenshell_api::{EndpointFile, ToolAccess};
use sevenshell_core::Core;
use sevenshell_error::ShellError;
use sevenshell_mcp_client::{McpClientError, McpClientManager, McpServerSecrets, split_tool_name};
use sevenshell_types::{AppSettings, McpServerConfig, McpServerState, McpServerStatus};
use sevenshell_vault::Vault;
use tokio::sync::{Mutex as AsyncMutex, broadcast, oneshot};

// The audit trail lives beside this module.
pub mod audit;
pub mod auth;
pub mod endpoint;
pub mod server;

pub struct McpRuntime {
    vault: Arc<Vault>,
    events: broadcast::Sender<String>,
    manager: Arc<McpClientManager>,
    tools: Arc<crate::agent::ToolRuntime>,
    listener: AsyncMutex<Option<Running>>,
    calls: Arc<server::CallRegistry>,
}

struct Running {
    port: u16,
    shutdown: oneshot::Sender<()>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerView {
    config: McpServerConfig,
    status: McpServerStatus,
    has_secrets: Option<bool>,
}

fn shell_error(error: McpClientError) -> ShellError {
    ShellError::app_internal(error.to_string())
}

impl McpRuntime {
    pub fn new(
        vault: Arc<Vault>,
        _core: Arc<Core>,
        events: broadcast::Sender<String>,
        tools: Arc<crate::agent::ToolRuntime>,
    ) -> Result<Self, ShellError> {
        let servers = sevenshell_platform::load()
            .map(|settings| settings.mcp_client_servers)
            .unwrap_or_default();
        let manager =
            Arc::new(McpClientManager::from_servers(servers.clone()).unwrap_or_else(|_| McpClientManager::new()));
        apply_secrets(&vault, &manager, &manager.configured());
        tools.add_source(manager.clone().source());
        let refresh = manager.clone();
        tokio::spawn(async move {
            refresh.refresh_all().await;
        });
        Ok(Self {
            vault,
            events,
            manager,
            tools,
            listener: AsyncMutex::new(None),
            calls: Arc::new(server::CallRegistry::new()),
        })
    }

    fn emit_status(&self, status: &McpServerStatus) {
        let _ = self
            .events
            .send(json!({ "event": "mcp:server_status", "payload": status }).to_string());
    }

    fn view(&self, id: &str) -> Option<ServerView> {
        let config = self.manager.configured().into_iter().find(|config| config.id == id)?;
        let status = self
            .manager
            .status_of(id)
            .unwrap_or_else(|| McpServerStatus::new(id, McpServerState::Connecting));
        let has_secrets = match self.vault.mcp_server_secrets(id) {
            Ok(Some(_)) => Some(true),
            Ok(None) => Some(false),
            Err(_) => None,
        };
        Some(ServerView {
            config,
            status,
            has_secrets,
        })
    }

    fn views(&self) -> Vec<ServerView> {
        self.manager
            .configured()
            .into_iter()
            .filter_map(|config| self.view(&config.id))
            .collect()
    }

    /// Starts the listener again when the settings say it was on before the process restarted.
    pub async fn restore(&self) {
        if let Ok(settings) = sevenshell_platform::load()
            && settings.mcp_enabled
            && let Err(error) = self.start(settings.mcp_port, &settings).await
        {
            log::warn!("the agent listener could not be restored: {error}");
        }
    }

    async fn status(&self) -> Value {
        let running = self.listener.lock().await;
        let settings = sevenshell_platform::load().ok();
        let scope = settings
            .as_ref()
            .map(|settings| settings.mcp_scope.as_str())
            .unwrap_or("readOnly");
        let audit_path = audit::default_path()
            .ok()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        json!({
            "enabled": running.is_some(),
            "port": running.as_ref().map(|running| running.port),
            "scope": scope,
            "clientConfig": self.client_config(),
            "auditPath": audit_path,
            "pendingApprovals": self.tools.pending_approvals(),
        })
    }

    async fn announce(&self) {
        let status = self.status().await;
        let _ = self
            .events
            .send(json!({ "event": "mcp:status", "payload": status }).to_string());
    }

    async fn enable(&self) -> Result<Value, ShellError> {
        let mut settings = sevenshell_platform::load()?;
        let stopped = self.listener.lock().await.is_none();
        if stopped {
            self.start(settings.mcp_port, &settings).await?;
        }
        settings.mcp_enabled = true;
        sevenshell_platform::save(&settings)?;
        self.announce().await;
        Ok(self.status().await)
    }

    async fn disable(&self) -> Result<Value, ShellError> {
        if let Some(running) = self.listener.lock().await.take() {
            let _ = running.shutdown.send(());
        }
        if let Ok(path) = endpoint::path() {
            endpoint::remove(&path);
        }
        self.calls.cancel_all();
        let mut settings = sevenshell_platform::load()?;
        settings.mcp_enabled = false;
        sevenshell_platform::save(&settings)?;
        self.announce().await;
        Ok(self.status().await)
    }

    async fn start(&self, port: u16, settings: &AppSettings) -> Result<u16, ShellError> {
        let token = auth::generate_token()?;
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|error| ShellError::app_internal(format!("the agent listener could not bind: {error}")))?;
        let bound = listener
            .local_addr()
            .map_err(|error| ShellError::app_internal(format!("the agent listener has no address: {error}")))?
            .port();

        let started_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        let published = EndpointFile::new(bound, token.clone(), std::process::id(), started_at)
            .with_max_result_bytes(settings.mcp_max_result_bytes);
        endpoint::write_to(&endpoint::path()?, &published)?;

        let state = Arc::new(server::ServerState::new(token, self.tools.clone(), self.calls.clone()));
        let service = server::router(state);
        let (shutdown, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let served = axum::serve(listener, service)
                .with_graceful_shutdown(async move {
                    let _ = receiver.await;
                })
                .await;
            if let Err(error) = served {
                log::warn!("the agent listener stopped answering: {error}");
            }
        });
        *self.listener.lock().await = Some(Running { port: bound, shutdown });
        Ok(bound)
    }

    fn client_config(&self) -> String {
        let mut candidates = Vec::new();
        if let Ok(executable) = std::env::current_exe()
            && let Some(directory) = executable.parent()
        {
            candidates.push(directory.join("sshell-mcp"));
            candidates.push(directory.join("sshell-mcp.exe"));
        }
        if let Some(directory) = sevenshell_platform::bridge_dir() {
            candidates.push(directory.join("sshell-mcp"));
            candidates.push(directory.join("sshell-mcp.exe"));
        }

        let Some(bridge) = candidates.into_iter().find(|path| path.is_file()) else {
            return String::new();
        };
        let mut server = json!({
            "command": bridge,
            "args": []
        });
        if let Ok(endpoint) = endpoint::path() {
            server["env"] = json!({ "SEVENSHELL_ENDPOINT_FILE": endpoint });
        }
        json!({ "mcpServers": { "7shell": server } }).to_string()
    }

    pub async fn dispatch(&self, cmd: &str, args: &Value) -> Option<Result<Value, ShellError>> {
        match cmd {
            "mcp_status" => Some(Ok(self.status().await)),
            "mcp_enable" => Some(self.enable().await),
            "mcp_disable" => Some(self.disable().await),
            "mcp_client_config" => Some(Ok(json!(self.client_config()))),
            "mcp_servers_list" => Some(to_value(self.views())),
            "mcp_server_save" => Some(self.save(args).await),
            "mcp_server_remove" => Some(self.remove(args)),
            "mcp_server_secrets_set" => Some(self.set_secrets(args).await),
            "mcp_server_refresh" => Some(self.refresh(args).await),
            "mcp_content_catalog" => Some(to_value(self.manager.content_catalog())),
            "mcp_resource_read" => Some(self.read_resource(args).await),
            "mcp_prompt_get" => Some(self.get_prompt(args).await),
            "mcp_catalog" => Some(self.catalog()),
            _ => None,
        }
    }

    async fn save(&self, args: &Value) -> Result<Value, ShellError> {
        let config: McpServerConfig = from_args(args, "config")?;
        let mut settings = sevenshell_platform::load()?;
        match settings
            .mcp_client_servers
            .iter_mut()
            .find(|server| server.id == config.id)
        {
            Some(existing) => *existing = config.clone(),
            None => settings.mcp_client_servers.push(config.clone()),
        }
        self.manager
            .replace(settings.mcp_client_servers.clone())
            .map_err(shell_error)?;
        sevenshell_platform::save(&settings)?;
        apply_secrets(&self.vault, &self.manager, &settings.mcp_client_servers);

        if let Ok(status) = self.manager.refresh(&config.id).await {
            self.emit_status(&status);
        }
        to_value(self.view(&config.id))
    }

    fn remove(&self, args: &Value) -> Result<Value, ShellError> {
        let server_id: String = from_args(args, "serverId")?;
        let mut settings = sevenshell_platform::load()?;
        settings.mcp_client_servers.retain(|server| server.id != server_id);
        self.manager
            .replace(settings.mcp_client_servers.clone())
            .map_err(shell_error)?;
        self.vault.set_mcp_server_secrets(&server_id, None)?;
        sevenshell_platform::save(&settings)?;
        Ok(Value::Null)
    }

    async fn set_secrets(&self, args: &Value) -> Result<Value, ShellError> {
        let server_id: String = from_args(args, "serverId")?;
        let env: BTreeMap<String, String> = from_args(args, "env")?;
        let headers: BTreeMap<String, String> = from_args(args, "headers")?;
        let secrets = McpServerSecrets { env, headers };
        let rendered = if secrets.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&secrets).map_err(|e| ShellError::app_internal(e.to_string()))?)
        };
        self.vault.set_mcp_server_secrets(&server_id, rendered)?;
        apply_secrets(&self.vault, &self.manager, &self.manager.configured());
        if let Ok(status) = self.manager.refresh(&server_id).await {
            self.emit_status(&status);
        }
        to_value(self.view(&server_id))
    }

    async fn refresh(&self, args: &Value) -> Result<Value, ShellError> {
        let server_id: String = from_args(args, "serverId")?;
        match self.manager.refresh(&server_id).await {
            Ok(status) => {
                self.emit_status(&status);
                to_value(self.view(&server_id))
            }
            Err(error) => Err(shell_error(error)),
        }
    }

    async fn read_resource(&self, args: &Value) -> Result<Value, ShellError> {
        let server_id: String = from_args(args, "serverId")?;
        let uri: String = from_args(args, "uri")?;
        to_value(
            self.manager
                .read_resource(&server_id, &uri)
                .await
                .map_err(shell_error)?,
        )
    }

    async fn get_prompt(&self, args: &Value) -> Result<Value, ShellError> {
        let server_id: String = from_args(args, "serverId")?;
        let name: String = from_args(args, "name")?;
        let arguments: BTreeMap<String, String> = from_args(args, "arguments")?;
        to_value(
            self.manager
                .get_prompt(&server_id, &name, arguments)
                .await
                .map_err(shell_error)?,
        )
    }

    fn catalog(&self) -> Result<Value, ShellError> {
        let settings = sevenshell_platform::load()?;
        let disabled = settings.mcp_tool_policy.disabled_tools;
        let offered: std::collections::BTreeSet<String> = self
            .tools
            .descriptors()
            .into_iter()
            .map(|descriptor| descriptor.name)
            .collect();
        let pristine = sevenshell_api::local::default_registry();
        let mut tools = Vec::new();

        for entry in sevenshell_api::BUILTIN_CATALOG {
            let descriptor = pristine
                .descriptors()
                .into_iter()
                .find(|descriptor| descriptor.name == entry.name);
            let Some(descriptor) = descriptor else {
                continue;
            };
            tools.push(json!({
                "name": entry.name,
                "title": descriptor.title,
                "description": descriptor.description,
                "access": descriptor.access,
                "risk": entry.risk,
                "category": entry.category,
                "enabled": offered.contains(entry.name),
                "source": Value::Null,
            }));
        }

        for descriptor in self.tools.descriptors() {
            if sevenshell_api::catalog_entry(&descriptor.name).is_some() {
                continue;
            }
            let Some((server, _tool)) = split_tool_name(&descriptor.name) else {
                continue;
            };
            let risk = match descriptor.access {
                ToolAccess::Read => "read",
                ToolAccess::Write => "write",
            };
            tools.push(json!({
                "name": descriptor.name,
                "title": descriptor.title,
                "description": descriptor.description,
                "access": descriptor.access,
                "risk": risk,
                "category": "external",
                "enabled": !disabled.contains(&descriptor.name),
                "source": server,
            }));
        }

        tools.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
        let enabled = tools.iter().filter(|tool| tool["enabled"] == true).count();
        Ok(json!({
            "tools": tools,
            "counts": { "total": tools.len(), "enabled": enabled },
        }))
    }
}

fn apply_secrets(vault: &Vault, manager: &McpClientManager, servers: &[McpServerConfig]) {
    let mut secrets = BTreeMap::new();
    for server in servers {
        if let Ok(Some(secret)) = vault.mcp_server_secrets(&server.id) {
            secrets.insert(server.id.clone(), McpServerSecrets::from_vault_value(secret.expose()));
        }
    }
    manager.set_secrets(secrets);
}

fn from_args<T: serde::de::DeserializeOwned>(args: &Value, key: &str) -> Result<T, ShellError> {
    serde_json::from_value(args.get(key).cloned().unwrap_or(Value::Null))
        .map_err(|error| ShellError::app_internal(format!("`{key}` is not the expected shape: {error}")))
}

fn to_value<T: Serialize>(value: T) -> Result<Value, ShellError> {
    serde_json::to_value(value)
        .map_err(|error| ShellError::app_internal(format!("an answer could not be built: {error}")))
}
