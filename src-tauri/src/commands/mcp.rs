//! What the interface is told and allowed to do about the agent interface.
//!
//! The window can read the status, turn the listener on and off, ask for the snippet it would paste
//! into an MCP client, read the tail of the audit trail and answer a pending approval. It cannot see
//! the token: that lives in the endpoint file beside the token's only reader, the bridge.
//!
//! Turning it on writes the setting *before* the socket is opened, and a start that then fails leaves
//! the setting on and the answer an error. That is deliberate: the switch must not claim a listener
//! that is not there, and the next launch (or the next click) tries again rather than silently
//! forgetting what the person asked for.
//!
//! The second half of the file is the *other* direction: the external servers this installation
//! calls. Those commands keep their logic in [`crate::mcp_client`] — what is saved, what is refused
//! and when a server is reopened is testable there without a webview — and only load the file, hand
//! it over and write it back here. A server's environment and headers go in through
//! `mcp_server_secrets_set` and never come out: every answer carries `hasSecrets`, never a value.

use std::collections::{BTreeMap, BTreeSet};

use sevenshell_api::{
    ApprovalDecision, AuditRecord, BUILTIN_CATALOG, CatalogCategory, ToolAccess, ToolDescriptor, ToolRegistry,
    ToolRisk, catalog_entry,
};
use sevenshell_error::ShellError;
use sevenshell_mcp_client::{McpServerSecrets, split_tool_name};
use sevenshell_types::{AppSettings, McpPromptText, McpResourceRead, McpServerConfig, McpServerContent};
use tauri::{AppHandle, State};

use crate::mcp::{McpStatus, audit};
use crate::mcp_client::{McpServerView, shell_error};
use crate::state::ShellState;

/// What is listening, if anything.
#[tauri::command]
pub async fn mcp_status(state: State<'_, ShellState>) -> Result<McpStatus, ShellError> {
    let settings = match sevenshell_platform::load() {
        Ok(settings) => settings,
        Err(error) => {
            // A settings file that cannot be read is not a reason to refuse to report a listener
            // that is running; the listener's own snapshot supplies the scope, and the defaults are
            // the narrowest tier.
            log::warn!("settings could not be read, so the agent interface reports its defaults: {error}");
            sevenshell_types::AppSettings::default()
        }
    };
    Ok(state.mcp().status(&settings).await)
}

/// Turns the listener on and reports what it bound.
#[tauri::command]
pub async fn mcp_enable(app: AppHandle, state: State<'_, ShellState>) -> Result<McpStatus, ShellError> {
    let mut settings = sevenshell_platform::load()?;
    settings.mcp_enabled = true;
    sevenshell_platform::save(&settings)?;
    crate::mcp::enable(&app, state.mcp(), &settings).await
}

/// Turns the listener off, removes its endpoint file and reports the state afterwards.
#[tauri::command]
pub async fn mcp_disable(app: AppHandle, state: State<'_, ShellState>) -> Result<McpStatus, ShellError> {
    // Stopped before the preference is written, because "turn it off" has to take effect even when
    // the settings file is the thing that is broken. A failed save is reported after the listener is
    // already gone: the next launch may try again, but nothing is listening now.
    let mut settings = match sevenshell_platform::load() {
        Ok(settings) => settings,
        Err(error) => {
            // Still off, still reported; the corrupt file is left alone rather than replaced by
            // defaults, because the rest of somebody's preferences are in it.
            crate::mcp::disable(&app, state.mcp(), &sevenshell_types::AppSettings::default()).await;
            return Err(error);
        }
    };
    settings.mcp_enabled = false;

    let status = crate::mcp::disable(&app, state.mcp(), &settings).await;
    sevenshell_platform::save(&settings)?;
    Ok(status)
}

/// The snippet an MCP client's configuration wants, without the token.
#[tauri::command]
pub fn mcp_client_config() -> Result<String, ShellError> {
    crate::mcp::client_config()
}

/// The last `limit` audit records, oldest first.
#[tauri::command]
pub fn mcp_audit_tail(state: State<'_, ShellState>, limit: usize) -> Result<Vec<AuditRecord>, ShellError> {
    let path = state.mcp().audit_path()?;
    Ok(audit::read_tail(&path, limit))
}

/// Answers one pending approval.
#[tauri::command]
pub fn mcp_approval_answer(state: State<'_, ShellState>, call_id: String, decision: String) -> Result<(), ShellError> {
    let decision = match decision.as_str() {
        "approved" => ApprovalDecision::Approved,
        "denied" => ApprovalDecision::Denied,
        other => {
            return Err(ShellError::app_internal(format!(
                "`{other}` is not an approval decision; expected `approved` or `denied`"
            )));
        }
    };
    // No error for an id that is not waiting: the window can answer a call that timed out a moment
    // before the click, and that is a race, not a mistake worth an error box.
    state.mcp().answer(&call_id, decision);
    Ok(())
}

/// The preferences, or the defaults when the file cannot be read.
///
/// The same rule the listener's status follows: a settings file that cannot be read is not a reason
/// to refuse to say what is configured. The defaults are an empty server list, which is honest.
fn settings_or_default() -> AppSettings {
    match sevenshell_platform::load() {
        Ok(settings) => settings,
        Err(error) => {
            log::warn!("settings could not be read, so the external servers report none: {error}");
            AppSettings::default()
        }
    }
}

/// An id no configured server answers to, as a sentence a page can show.
fn unknown_server(server_id: &str) -> ShellError {
    // The client's own code for this has no catalogue entry yet; the sentence names it so a log line
    // and the page still agree about what happened, and it quotes only what the caller sent.
    ShellError::app_internal(format!(
        "MCP-SERVER-NOT-FOUND: no external MCP server is configured with the id `{server_id}`"
    ))
}

/// Drops the entries a transport could not use, and trims the names.
///
/// A key with nothing in it is not an environment variable or a header. Values are left exactly as
/// typed: surrounding spaces can be part of a token, and a token this build trims is one the server
/// refuses.
fn key_values(values: BTreeMap<String, String>) -> BTreeMap<String, String> {
    values
        .into_iter()
        .filter_map(|(key, value)| {
            let key = key.trim().to_owned();
            (!key.is_empty()).then_some((key, value))
        })
        .collect()
}

/// Every external server, with where its supervision has got to and whether the vault holds secrets.
#[tauri::command]
pub async fn mcp_servers_list(state: State<'_, ShellState>) -> Result<Vec<McpServerView>, ShellError> {
    Ok(state.mcp_client().list(&settings_or_default()))
}

/// Writes one server's non-secret configuration and answers the row as the page draws it.
///
/// The order is deliberate: the manager accepts the list *first*, and only then is the file written.
/// A configuration the manager refuses is a page's answer, not a saved setting, and a list the
/// manager holds but the file does not would be reverted by the next save from any other page.
#[tauri::command]
pub async fn mcp_server_save(
    state: State<'_, ShellState>,
    config: McpServerConfig,
) -> Result<McpServerView, ShellError> {
    let mut settings = sevenshell_platform::load()?;
    let view = state
        .mcp_client()
        .save_server(&mut settings, config)
        .map_err(|error| shell_error(&error))?;

    sevenshell_platform::save(&settings)?;
    // The handshake is somebody else's server: the page must not wait on it. A server whose settings
    // did not change kept its session, so `refresh_if_needed` costs a connection only when the save
    // changed something.
    state.mcp_client().refresh_if_needed(&view.config.id);
    Ok(view)
}

/// Removes one server, with the secrets the vault files under it.
#[tauri::command]
pub async fn mcp_server_remove(state: State<'_, ShellState>, server_id: String) -> Result<(), ShellError> {
    let mut settings = sevenshell_platform::load()?;
    let removed = state
        .mcp_client()
        .remove_server(&mut settings, &server_id)
        .map_err(|error| shell_error(&error))?;

    if removed {
        sevenshell_platform::save(&settings)?;
    }
    // Nothing is emitted for a row that is gone: there is nothing left to draw, and the page learns
    // the server is not there any more from the list it asks for.
    Ok(())
}

/// Keeps the environment and headers one server is reached with, or forgets them.
///
/// Both objects travel whole and are written whole: `{}` in both is how the page clears what is
/// stored. The values go inwards only — the answer carries `hasSecrets`, never an entry.
#[tauri::command]
pub async fn mcp_server_secrets_set(
    state: State<'_, ShellState>,
    server_id: String,
    env: BTreeMap<String, String>,
    headers: BTreeMap<String, String>,
) -> Result<McpServerView, ShellError> {
    let settings = sevenshell_platform::load()?;
    let config = state
        .mcp_client()
        .server(&settings, &server_id)
        .ok_or_else(|| unknown_server(&server_id))?;

    let secrets = McpServerSecrets {
        env: key_values(env),
        headers: key_values(headers),
    };
    if secrets.is_empty() {
        state.vault().set_mcp_server_secrets(&server_id, None)?;
    } else {
        let encoded = serde_json::to_string(&secrets)
            .map_err(|error| ShellError::app_internal(format!("a server's secrets could not be encoded: {error}")))?;
        // An empty string is how the vault deletes; this is never empty here, so `Some` is a save.
        state.vault().set_mcp_server_secrets(&server_id, Some(encoded))?;
    }

    // The manager is handed the whole set again rather than the one entry: it reads the vault, so
    // there is one place secrets come from and no second copy to keep in step.
    state
        .mcp_client()
        .apply(&settings)
        .map_err(|error| shell_error(&error))?;
    // A server that failed for want of a token is retried now, which is what the person typing one
    // is asking for.
    state.mcp_client().refresh_in_background(config.id.clone());

    Ok(state
        .mcp_client()
        .view(&settings, &server_id)
        .expect("the server was checked to be configured above"))
}

/// Reconnects one server now, instead of waiting for its own backoff.
///
/// A failed attempt answers an error *and* leaves the row saying `failed` — the manager recorded it
/// and the event was sent before this returns, so the page draws the same thing whichever of the two
/// it looks at.
#[tauri::command]
pub async fn mcp_server_refresh(state: State<'_, ShellState>, server_id: String) -> Result<McpServerView, ShellError> {
    let settings = sevenshell_platform::load()?;
    if state.mcp_client().server(&settings, &server_id).is_none() {
        return Err(unknown_server(&server_id));
    }

    let attempt = state.mcp_client().refresh(&server_id).await;
    let view = state
        .mcp_client()
        .view(&settings, &server_id)
        .expect("the server was checked to be configured above");

    match attempt {
        Ok(_) => Ok(view),
        Err(error) => Err(shell_error(&error)),
    }
}

/// Every ready external server's resources and prompts, for the assistant's picker.
///
/// A server that offers neither is left out here: an empty heading is not a choice, and the picker
/// draws what it was given. The catalogue is cached by each session, so opening the dialog does not
/// send a request to every configured server.
#[tauri::command]
pub fn mcp_content_catalog(state: State<'_, ShellState>) -> Vec<McpServerContent> {
    state.mcp_client().content_catalog()
}

/// Reads one resource from one external server.
///
/// Content, not an action: no approval and no audit line, because nothing on the far side changes.
/// The answer is bounded by the client, and `truncated` says whether it was cut or a binary piece
/// was left out rather than cut in half.
#[tauri::command]
pub async fn mcp_resource_read(
    state: State<'_, ShellState>,
    server_id: String,
    uri: String,
) -> Result<McpResourceRead, ShellError> {
    state
        .mcp_client()
        .read_resource(&server_id, &uri)
        .await
        .map_err(|error| shell_error(&error))
}

/// Renders one prompt with the arguments a person typed.
///
/// The answer is text the person can edit before sending; rendering is not a tool call and is not
/// audited. The client bounds the text and marks it when it was cut.
#[tauri::command]
pub async fn mcp_prompt_get(
    state: State<'_, ShellState>,
    server_id: String,
    name: String,
    arguments: BTreeMap<String, String>,
) -> Result<McpPromptText, ShellError> {
    state
        .mcp_client()
        .get_prompt(&server_id, &name, arguments)
        .await
        .map_err(|error| shell_error(&error))
}

// -------------------------------------------------------------------------------------------
// The tool catalogue
// -------------------------------------------------------------------------------------------

/// One tool as the capabilities page draws it.
///
/// Presentation only: the name is the key everything else uses, and `enabled` is the page's one
/// decision — written back through the ordinary settings save, never through this command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCatalogTool {
    pub name: String,
    pub title: String,
    pub description: String,
    pub access: ToolAccess,
    pub risk: ToolRisk,
    pub category: CatalogCategory,
    pub enabled: bool,
    /// `null` for a built-in tool; otherwise the id of the external server that offers it.
    pub source: Option<String>,
}

/// How many tools there are and how many are switched on, for the page's summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCatalogCounts {
    pub total: usize,
    pub enabled: usize,
}

/// Everything the capabilities page has to draw, in one answer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCatalog {
    pub tools: Vec<McpCatalogTool>,
    pub counts: McpCatalogCounts,
}

/// Every tool this installation can switch on and off.
///
/// Two lists answer two different questions. The catalogue says a tool *exists* even when it is
/// switched off — a page that drew only the registry's list could not offer the switch back on —
/// while the registry says whether it is offered right now. External servers' tools join from the
/// manager, which is the only side that knows what a server announced; their switch is the server's
/// own `enabled` flag together with the per-tool set.
#[tauri::command]
pub fn mcp_catalog(state: State<'_, ShellState>) -> Result<McpCatalog, ShellError> {
    let settings = settings_or_default();
    let shared = state.shared();
    Ok(build_catalog(
        shared.registry(),
        state.mcp_client().tools(),
        &settings.mcp_client_servers,
    ))
}

fn build_catalog(registry: &ToolRegistry, external: Vec<ToolDescriptor>, servers: &[McpServerConfig]) -> McpCatalog {
    // A pristine built-in registry is where a switched-off tool's title and description still live:
    // the live registry leaves a disabled tool out entirely, and a row with no words beside its
    // switch would make the page unreadable exactly where somebody is looking for something.
    let reference = sevenshell_api::local::default_registry();
    let offered: BTreeSet<String> = registry
        .descriptors()
        .into_iter()
        .map(|descriptor| descriptor.name)
        .collect();
    let disabled = registry.disabled_tools();

    let mut tools = Vec::with_capacity(BUILTIN_CATALOG.len() + external.len());
    for entry in BUILTIN_CATALOG {
        let Some(descriptor) = reference.descriptor(entry.name) else {
            log::error!(
                "the tool catalogue names `{}` but this build registers no such tool; the page leaves it out",
                entry.name
            );
            continue;
        };
        tools.push(McpCatalogTool {
            name: entry.name.to_owned(),
            title: descriptor.title,
            description: descriptor.description,
            access: descriptor.access,
            risk: entry.risk,
            category: entry.category,
            // Present in the live registry unless the settings page switched it off.
            enabled: offered.contains(entry.name),
            source: None,
        });
    }

    // A registry tool the page cannot place is a build mistake rather than a page state. It is said
    // out loud and left out: one un-catalogued name must not blank the nine a person came to see.
    for name in &offered {
        if catalog_entry(name).is_none() && split_tool_name(name).is_none() {
            log::error!("the tool registry offers `{name}`, which the tool catalogue does not name");
        }
    }

    for descriptor in external {
        let Some((server, _tool)) = split_tool_name(&descriptor.name) else {
            log::error!("an external tool's name does not name a server: `{}`", descriptor.name);
            continue;
        };
        let server_enabled = servers
            .iter()
            .find(|config| config.id == server)
            .is_some_and(|config| config.enabled);
        tools.push(McpCatalogTool {
            name: descriptor.name.clone(),
            title: descriptor.title,
            description: descriptor.description,
            access: descriptor.access,
            // An external server says nothing about how dangerous its tool is beyond the protocol's
            // own read-only hint, so the access class is the whole of what can be promised here.
            risk: match descriptor.access {
                ToolAccess::Read => ToolRisk::Read,
                ToolAccess::Write => ToolRisk::Write,
            },
            category: CatalogCategory::External,
            enabled: server_enabled && !disabled.contains(&descriptor.name),
            source: Some(server.to_owned()),
        });
    }

    tools.sort_by(|left, right| {
        category_rank(left.category)
            .cmp(&category_rank(right.category))
            .then_with(|| left.name.cmp(&right.name))
    });

    let enabled = tools.iter().filter(|tool| tool.enabled).count();
    McpCatalog {
        counts: McpCatalogCounts {
            total: tools.len(),
            enabled,
        },
        tools,
    }
}

/// The order the page lists categories in. Presentation, so it lives beside the answer rather than
/// in the catalogue itself.
fn category_rank(category: CatalogCategory) -> u8 {
    match category {
        CatalogCategory::Sessions => 0,
        CatalogCategory::Exec => 1,
        CatalogCategory::Files => 2,
        CatalogCategory::Containers => 3,
        CatalogCategory::Kubernetes => 4,
        CatalogCategory::External => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sevenshell_api::local;
    use sevenshell_types::McpServerTransport;

    fn external_tool(name: &str, access: ToolAccess) -> ToolDescriptor {
        ToolDescriptor {
            name: name.to_owned(),
            title: name.to_owned(),
            description: "a tool of somebody else's server".to_owned(),
            input_schema: sevenshell_api::ToolInputSchema::object(serde_json::json!({}), &[]),
            access,
        }
    }

    fn server(id: &str, enabled: bool) -> McpServerConfig {
        let mut server = McpServerConfig::new(id, id, McpServerTransport::Stdio);
        server.command = Some("unused".to_owned());
        server.enabled = enabled;
        server
    }

    fn find<'a>(catalog: &'a McpCatalog, name: &str) -> &'a McpCatalogTool {
        catalog
            .tools
            .iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("`{name}` is in the catalogue"))
    }

    #[test]
    fn a_switched_off_built_in_is_still_listed_and_counted_but_not_enabled() {
        // The page has to be able to switch it back on, so it must be drawn; the registry is where
        // the answer "not offered right now" comes from.
        let registry = local::default_registry();
        registry.set_disabled_tools(BTreeSet::from(["sshell_exec_run".to_owned()]));

        let catalog = build_catalog(&registry, Vec::new(), &[]);

        let exec = find(&catalog, "sshell_exec_run");
        assert!(!exec.enabled, "the settings file switched it off");
        assert_eq!(exec.category, CatalogCategory::Exec);
        assert_eq!(exec.risk, ToolRisk::CommandGraded);
        assert!(!exec.title.is_empty(), "a disabled row still has words");
        assert!(!exec.description.is_empty());
        assert_eq!(exec.source, None, "a built-in tool belongs to no server");
        assert_eq!(
            exec.access,
            ToolAccess::Write,
            "from the descriptor, not from the catalogue"
        );

        assert_eq!(catalog.counts.total, 10, "every built-in is drawn");
        assert_eq!(catalog.counts.enabled, 9);
        assert!(find(&catalog, "sshell_sessions_list").enabled);
    }

    #[test]
    fn an_external_tool_names_the_server_that_offers_it() {
        let catalog = build_catalog(
            &local::default_registry(),
            vec![
                external_tool("mcp__files__read", ToolAccess::Read),
                external_tool("mcp__files__write", ToolAccess::Write),
                external_tool("mcp__tickets__read", ToolAccess::Read),
            ],
            &[server("files", true), server("tickets", false)],
        );

        let read = find(&catalog, "mcp__files__read");
        assert_eq!(read.source.as_deref(), Some("files"));
        assert_eq!(read.category, CatalogCategory::External);
        assert_eq!(read.risk, ToolRisk::Read);
        assert!(read.enabled);

        let write = find(&catalog, "mcp__files__write");
        assert_eq!(write.source.as_deref(), Some("files"));
        assert_eq!(write.risk, ToolRisk::Write, "the protocol's read-only hint is the risk");
        assert!(write.enabled);

        let switched_off = find(&catalog, "mcp__tickets__read");
        assert_eq!(switched_off.source.as_deref(), Some("tickets"));
        assert!(!switched_off.enabled, "a switched-off server offers nothing");

        assert_eq!(catalog.counts.total, 13, "ten built-ins and three external tools");
        assert_eq!(catalog.counts.enabled, 12);
    }

    #[test]
    fn an_external_tool_in_the_disabled_set_is_not_enabled() {
        let registry = local::default_registry();
        registry.set_disabled_tools(BTreeSet::from(["mcp__files__read".to_owned()]));

        let catalog = build_catalog(
            &registry,
            vec![external_tool("mcp__files__read", ToolAccess::Read)],
            &[server("files", true)],
        );

        assert!(!find(&catalog, "mcp__files__read").enabled);
        assert_eq!(catalog.counts.total, 11);
        assert_eq!(
            catalog.counts.enabled, 10,
            "the ten built-ins are untouched by an external tool's switch"
        );
    }

    #[test]
    fn the_list_runs_by_category_then_name() {
        let catalog = build_catalog(
            &local::default_registry(),
            vec![
                external_tool("mcp__files__read", ToolAccess::Read),
                external_tool("mcp__alpha__read", ToolAccess::Read),
            ],
            &[server("files", true), server("alpha", true)],
        );

        let keys: Vec<(u8, &str)> = catalog
            .tools
            .iter()
            .map(|tool| (category_rank(tool.category), tool.name.as_str()))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "the page draws the list in a stable order");
        assert_eq!(
            keys.last().map(|(rank, _)| *rank),
            Some(category_rank(CatalogCategory::External)),
            "other people's tools come after this build's own"
        );
    }

    #[test]
    fn the_json_is_camel_case_and_a_built_in_source_is_null() {
        let catalog = build_catalog(&local::default_registry(), Vec::new(), &[]);
        let rendered = serde_json::to_value(&catalog).expect("the answer is JSON");

        let first = &rendered["tools"][0];
        for key in [
            "name",
            "title",
            "description",
            "access",
            "risk",
            "category",
            "enabled",
            "source",
        ] {
            assert!(first.get(key).is_some(), "the page matches on `{key}`: {rendered}");
        }
        assert!(
            first["source"].is_null(),
            "a built-in tool names no server, and the page matches on null rather than a missing field"
        );
        assert_eq!(rendered["counts"]["total"], serde_json::json!(10));
        assert_eq!(rendered["counts"]["enabled"], serde_json::json!(10));
    }
}
