//! Naming and the in-memory shape of a server's secrets.
//!
//! Two things live here rather than in the transport: how an external tool is named, and what the
//! vault's one JSON value for a server decodes into. Both are shared by every transport, and both
//! are small enough to read without a network anywhere in sight.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sevenshell_types::McpServerConfig;

use crate::McpClientError;

/// The prefix every external tool's name carries.
pub const TOOL_NAME_PREFIX: &str = "mcp__";

/// The separator between the parts of an external tool's name.
pub const TOOL_NAME_SEPARATOR: &str = "__";

/// The name an agent calls a server's tool by.
///
/// Built rather than stored: renaming a server in the settings file renames its tools, and there is
/// exactly one place that knows the spelling. The server id is validated before a server is kept, so
/// this never produces a name that cannot be read back.
pub fn tool_name(server_id: &str, tool: &str) -> String {
    format!("{TOOL_NAME_PREFIX}{server_id}{TOOL_NAME_SEPARATOR}{tool}")
}

/// The two parts of an external tool's name, when it has them.
///
/// The id rule forbids `__` inside a server id, which is what makes the first separator the only
/// separator: a tool whose own name contains `__` keeps the rest in its half.
pub fn split_tool_name(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix(TOOL_NAME_PREFIX)?;
    let (server, tool) = rest.split_once(TOOL_NAME_SEPARATOR)?;
    if server.is_empty() || tool.is_empty() {
        return None;
    }
    Some((server, tool))
}

/// Whether a name belongs to an external server rather than to this build.
pub fn is_external_tool_name(name: &str) -> bool {
    split_tool_name(name).is_some()
}

/// Whether an id can name a server, delegated to the one rule in the wire types.
pub fn is_valid_server_id(id: &str) -> bool {
    sevenshell_types::is_valid_mcp_server_id(id)
}

/// How long one call to this server's tools may take, as a duration.
///
/// The stored number is clamped here as well: a settings file read by a build whose page offered a
/// wider range must not be able to hold a call open longer than this build allows.
pub fn tool_timeout(server: &McpServerConfig) -> Duration {
    Duration::from_secs(sevenshell_types::normalize_mcp_server_tool_timeout_seconds(
        server.tool_timeout_seconds,
    ))
}

/// The same entry with everything this crate needs to be true made true, or a refusal.
///
/// Three rules, all of them about a value a hand-edited settings file can get wrong:
///
/// * the id has to be one a tool name can be built from and read back (the types crate's rule);
/// * `stdio` needs a program to start, `http` and `sse` need an address to dial — an entry that
///   names neither cannot be attempted, and one that names both is not ambiguous because the
///   transport decides which one is used;
/// * an address has to parse as `http` or `https` and must not carry a user name or password: a
///   password in a URL ends up in every transport error that mentions the URL.
///
/// The returned entry is [normalized](McpServerConfig::normalized), so the timeout and the
/// surrounding whitespace are this crate's opinion rather than the file's.
pub fn validate_server(server: &McpServerConfig) -> Result<McpServerConfig, McpClientError> {
    let server = server.clone().normalized();

    if !is_valid_server_id(&server.id) {
        return Err(McpClientError::InvalidServerId(server.id));
    }

    if server.transport.uses_command() && server.command.is_none() {
        return Err(McpClientError::InvalidConfig {
            server: server.id,
            detail: "an stdio server needs a command to start".to_owned(),
        });
    }

    if server.transport.uses_url() {
        let Some(url) = server.url.as_deref() else {
            return Err(McpClientError::InvalidConfig {
                server: server.id,
                detail: format!("a {} server needs an address", server.transport.as_str()),
            });
        };
        let parsed = reqwest::Url::parse(url).map_err(|_| McpClientError::InvalidConfig {
            server: server.id.clone(),
            detail: "the address is not a URL".to_owned(),
        })?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(McpClientError::InvalidConfig {
                server: server.id,
                detail: "the address has to be http or https".to_owned(),
            });
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(McpClientError::InvalidConfig {
                server: server.id,
                detail: "the address must not carry a user name or password".to_owned(),
            });
        }
    }

    Ok(server)
}

/// What the vault holds for one server, decoded.
///
/// The vault stores one string per server, which is this object as JSON:
/// `{"env":{...},"headers":{...}}`. Environment values are what an stdio child is started with;
/// headers are what an HTTP or SSE request carries. Both are credentials or credential-adjacent and
/// only ever travel outwards, into the child process or onto the wire — never into an event, a log
/// line or an error.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerSecrets {
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

impl McpServerSecrets {
    /// Whether there is anything to apply at all.
    pub fn is_empty(&self) -> bool {
        self.env.is_empty() && self.headers.is_empty()
    }

    /// Decodes the vault's value. A value this build cannot read is no secrets rather than a
    /// failure: a server without its environment is one that fails to start, which is visible,
    /// while a refused settings path would take the whole feature down with it.
    pub fn from_vault_value(value: &str) -> Self {
        serde_json::from_str(value).unwrap_or_default()
    }
}

/// Prints counts, never values.
///
/// A derived `Debug` would put every credential into the first log line that formatted a session's
/// configuration. This is the same rule the endpoint file's `Debug` follows for its token.
impl fmt::Debug for McpServerSecrets {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpServerSecrets")
            .field("env", &self.env.len())
            .field("headers", &self.headers.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sevenshell_types::McpServerTransport;

    #[test]
    fn a_tool_name_round_trips_through_the_two_parts() {
        let name = tool_name("files", "read_file");
        assert_eq!(name, "mcp__files__read_file");
        assert_eq!(split_tool_name(&name), Some(("files", "read_file")));
        assert!(is_external_tool_name(&name));

        // A tool whose own name has the separator keeps it in its half: only the first one splits.
        assert_eq!(split_tool_name("mcp__files__read__all"), Some(("files", "read__all")));
    }

    #[test]
    fn a_name_outside_the_prefix_or_with_an_empty_part_is_not_an_external_tool() {
        for name in [
            "sshell_sessions_list",
            "mcp__",
            "mcp__files__",
            "mcp____read",
            "mcp_files",
        ] {
            assert_eq!(split_tool_name(name), None, "`{name}` names no external tool");
            assert!(!is_external_tool_name(name));
        }
    }

    #[test]
    fn the_timeout_is_read_through_the_same_ends_the_page_offers() {
        let mut server = McpServerConfig::new("files", "Files", McpServerTransport::Http);
        server.tool_timeout_seconds = 0;
        assert_eq!(
            tool_timeout(&server),
            std::time::Duration::from_secs(sevenshell_types::MCP_SERVER_TOOL_TIMEOUT_SECONDS_MIN)
        );
        server.tool_timeout_seconds = 10_000;
        assert_eq!(
            tool_timeout(&server),
            std::time::Duration::from_secs(sevenshell_types::MCP_SERVER_TOOL_TIMEOUT_SECONDS_MAX)
        );
    }

    #[test]
    fn the_vault_value_decodes_and_its_debug_prints_counts_rather_than_values() {
        let secrets =
            McpServerSecrets::from_vault_value(r#"{"env":{"TOKEN":"hunter2"},"headers":{"Authorization":"Bearer x"}}"#);
        assert_eq!(secrets.env.get("TOKEN").map(String::as_str), Some("hunter2"));
        assert!(!secrets.is_empty());

        let printed = format!("{secrets:?}");
        assert!(
            !printed.contains("hunter2"),
            "a credential reached a Debug line: {printed}"
        );
        assert!(!printed.contains("Bearer x"));
        assert!(printed.contains('1'), "the count is still there to be useful");

        assert!(
            McpServerSecrets::from_vault_value("not json").is_empty(),
            "an unreadable value is no secrets, not a crash"
        );
    }

    #[test]
    fn a_transport_without_the_address_it_needs_is_refused_instead_of_attempted() {
        let stdio = McpServerConfig::new("files", "Files", McpServerTransport::Stdio);
        let error = validate_server(&stdio).expect_err("no command");
        assert_eq!(error.code(), "MCP-INVALID-ARGUMENTS");
        assert!(
            error.detail().is_some_and(|detail| detail.contains("command")),
            "the refusal says what is missing: {error:?}"
        );

        let http = McpServerConfig::new("tickets", "Tickets", McpServerTransport::Http);
        assert!(validate_server(&http).is_err(), "no address");
    }

    #[test]
    fn an_address_that_is_not_http_or_carries_a_password_is_refused_without_quoting_it() {
        let mut server = McpServerConfig::new("tickets", "Tickets", McpServerTransport::Sse);
        for bad in [
            "not a url",
            "ftp://example.test/mcp",
            "https://user:hunter2@example.test/mcp",
        ] {
            server.url = Some(bad.to_owned());
            let error = validate_server(&server).expect_err("this address must be refused");
            assert_eq!(error.code(), "MCP-INVALID-ARGUMENTS", "{bad}");
            assert!(
                !format!("{error:?}").contains("hunter2"),
                "the password leaked into the refusal: {error:?}"
            );
        }

        server.url = Some("https://example.test/mcp".to_owned());
        assert!(validate_server(&server).is_ok());
    }

    #[test]
    fn validating_trims_and_clamps_the_way_the_settings_page_does() {
        let mut server = McpServerConfig::new(" files ", " Files ", McpServerTransport::Stdio);
        server.command = Some("  npx  ".to_owned());
        server.tool_timeout_seconds = 1;

        let valid = validate_server(&server).expect("a command is all stdio needs");
        assert_eq!(valid.id, "files");
        assert_eq!(valid.command.as_deref(), Some("npx"));
        assert_eq!(
            valid.tool_timeout_seconds,
            sevenshell_types::MCP_SERVER_TOOL_TIMEOUT_SECONDS_MIN,
            "a stored number below the range is read as the shortest real budget"
        );
    }
}
