//! External MCP servers, as the settings file and the interface hold them.
//!
//! Nothing here can hold a credential. Where a server is and whether it is switched on is ordinary
//! configuration; the environment an stdio server is started with and the headers an HTTP server
//! expects are secrets and live in the vault, under the server's id. What crosses to the interface
//! is therefore always the same shape, with no field an env value or a bearer token could arrive in.
//!
//! The id is the half of the tool name an agent calls. A tool from these servers is named
//! `mcp__{id}__{tool}`, so the id is restricted to `[A-Za-z0-9_-]` and may not contain `__`: a
//! separator inside a component would make one name mean two tools, and the name is what an agent
//! remembers long after this build is gone.

use serde::{Deserialize, Serialize};

/// The longest id a server may have, in characters.
///
/// The id is typed into a tool name and read back out of it. Thirty-two is long enough for a name
/// somebody recognises and short enough that the prefix plus two separators still leaves most of a
/// tool name for the tool itself.
pub const MCP_SERVER_ID_MAX_CHARS: usize = 32;

/// The separator between the parts of an external tool's name.
pub const MCP_SERVER_TOOL_SEPARATOR: &str = "__";

/// How long one tool call to an external server may take, before a person says otherwise.
///
/// Sixty seconds: a remote tool may be doing real work, unlike the built-in listings, and the
/// ceiling is what keeps a hung server from holding an agent's turn open until somebody notices.
pub const MCP_SERVER_TOOL_TIMEOUT_SECONDS_DEFAULT: u64 = 60;

/// The ends of that number.
///
/// Five seconds is the shortest a network round trip plus real work can be given without turning
/// every slow server into a failure; ten minutes is the longest a single tool call may hold an
/// agent, because beyond that the call is not the thing the person is waiting for any more.
pub const MCP_SERVER_TOOL_TIMEOUT_SECONDS_MIN: u64 = 5;
pub const MCP_SERVER_TOOL_TIMEOUT_SECONDS_MAX: u64 = 600;

pub fn mcp_server_tool_timeout_seconds_default() -> u64 {
    MCP_SERVER_TOOL_TIMEOUT_SECONDS_DEFAULT
}

/// Whether a server added by hand is offered until somebody unticks it.
///
/// On: a server somebody typed in and saved is a server they meant to use. The switch exists for
/// the other direction — keeping a server's configuration while its tools are out of the list.
pub fn mcp_server_enabled_default() -> bool {
    true
}

/// How a server is reached.
///
/// The three transports are distinct protocols with distinct failure modes, not three spellings of
/// one: stdio is a child process this machine starts, HTTP is a request per message, and SSE is a
/// long-lived event stream with requests posted back to it. A server's transport therefore decides
/// which of `command` and `url` is the one that is used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpServerTransport {
    /// A child process spoken to over stdin and stdout.
    #[default]
    Stdio,
    /// Streamable HTTP: a request per message, with a session header the server hands back.
    Http,
    /// A server-sent event stream for what the server says, with messages posted back to it.
    Sse,
}

impl McpServerTransport {
    pub const ALL: &'static [McpServerTransport] = &[
        McpServerTransport::Stdio,
        McpServerTransport::Http,
        McpServerTransport::Sse,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Http => "http",
            Self::Sse => "sse",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|transport| transport.as_str() == id)
    }

    /// Whether this transport starts a program rather than dials an address.
    pub const fn uses_command(self) -> bool {
        matches!(self, Self::Stdio)
    }

    /// Whether this transport dials an address rather than starting a program.
    pub const fn uses_url(self) -> bool {
        matches!(self, Self::Http | Self::Sse)
    }
}

/// One external MCP server, without its secrets.
///
/// `command` and `url` are both optional in the type and exactly one is used, chosen by
/// `transport`. They are kept apart rather than collapsed into one "address" field because the two
/// are not addresses of the same kind: a command is resolved on this machine's `PATH`, a URL is
/// dialled — and a settings page that had one box would accept either and be wrong about one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerConfig {
    /// The opaque name the vault files this server's secrets under and the tool name carries.
    pub id: String,
    /// What a person typed as the server's name. Empty falls back to `id` when something is drawn.
    pub name: String,
    pub transport: McpServerTransport,
    /// The program an stdio server is started from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The arguments that program is started with, in order, as they were typed.
    #[serde(default)]
    pub args: Vec<String>,
    /// The address an HTTP or SSE server is reached at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Whether its tools are offered at all.
    #[serde(default = "mcp_server_enabled_default")]
    pub enabled: bool,
    /// How long one call to one of its tools may take.
    #[serde(default = "mcp_server_tool_timeout_seconds_default")]
    pub tool_timeout_seconds: u64,
}

impl McpServerConfig {
    pub fn new(id: impl Into<String>, name: impl Into<String>, transport: McpServerTransport) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            transport,
            command: None,
            args: Vec::new(),
            url: None,
            enabled: mcp_server_enabled_default(),
            tool_timeout_seconds: mcp_server_tool_timeout_seconds_default(),
        }
    }

    /// The same entry with the parts a hand-edited file can get wrong put right.
    ///
    /// Trimming the id is not cosmetic: ` files ` and `files` would otherwise be two ids for one
    /// server, two vault entries, and a tool name with a space in it that no agent can call back.
    /// Arguments are deliberately left untouched — a leading space inside one is a value, not a
    /// typo — and so are `command` and `url` beyond their own surrounding whitespace.
    pub fn normalized(mut self) -> Self {
        self.id = self.id.trim().to_owned();
        self.name = self.name.trim().to_owned();
        self.command = trimmed_non_empty(self.command);
        self.url = trimmed_non_empty(self.url);
        self.tool_timeout_seconds = normalize_mcp_server_tool_timeout_seconds(self.tool_timeout_seconds);
        self
    }

    /// What a list draws for this server, which is the name somebody typed or else its id.
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() { &self.id } else { &self.name }
    }
}

fn trimmed_non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Whether an id can name a server.
///
/// One to 32 characters from `[A-Za-z0-9_-]`, and no `__` inside: the last rule is what makes the
/// `mcp__{server}__{tool}` name parse back into exactly two parts.
pub fn is_valid_mcp_server_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MCP_SERVER_ID_MAX_CHARS
        && !id.contains(MCP_SERVER_TOOL_SEPARATOR)
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// Keeps a timeout inside the range this build offers.
///
/// Zero is not a value here: it would mean "fail before trying", so a hand-edited zero reads as
/// the shortest budget that is still a budget rather than as a call nobody allows.
pub fn normalize_mcp_server_tool_timeout_seconds(value: u64) -> u64 {
    value.clamp(MCP_SERVER_TOOL_TIMEOUT_SECONDS_MIN, MCP_SERVER_TOOL_TIMEOUT_SECONDS_MAX)
}

/// Where one external server's supervision has got to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpServerState {
    /// A session is being opened, or reopened after a failure.
    #[default]
    Connecting,
    /// The session is open and its tool list is current.
    Ready,
    /// The last attempt failed. The server stays configured; the next attempt is on its own timer.
    Failed,
    /// Somebody switched it off, so nothing is attempted.
    Disabled,
}

impl McpServerState {
    pub const ALL: &'static [McpServerState] = &[
        McpServerState::Connecting,
        McpServerState::Ready,
        McpServerState::Failed,
        McpServerState::Disabled,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Ready => "ready",
            Self::Failed => "failed",
            Self::Disabled => "disabled",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|state| state.as_str() == id)
    }
}

/// One server's state, as the settings page and the status event carry it.
///
/// `code` and `message` are the failure's own words when it failed, and absent otherwise. The
/// message is a plain sentence for the person reading the row; it never contains a secret, and no
/// field here can: what went wrong is named by a code, and the detail stays in the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerStatus {
    pub server_id: String,
    pub state: McpServerState,
    /// How many tools the server last announced. Zero before the first successful handshake.
    #[serde(default)]
    pub tools: u32,
    /// The failure's code, when the state is `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl McpServerStatus {
    pub fn new(server_id: impl Into<String>, state: McpServerState) -> Self {
        Self {
            server_id: server_id.into(),
            state,
            tools: 0,
            code: None,
            message: None,
        }
    }

    pub fn with_tools(mut self, tools: usize) -> Self {
        self.tools = u32::try_from(tools).unwrap_or(u32::MAX);
        self
    }

    pub fn with_failure(mut self, code: impl Into<String>, message: impl Into<String>) -> Self {
        self.state = McpServerState::Failed;
        self.code = Some(code.into());
        self.message = Some(message.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transport_round_trips_through_its_own_name() {
        for transport in McpServerTransport::ALL {
            assert_eq!(
                McpServerTransport::from_id(transport.as_str()),
                Some(*transport),
                "a transport whose name does not read back is one the settings file cannot hold"
            );
        }
        assert_eq!(
            serde_json::to_value(McpServerTransport::Http).expect("a transport is JSON"),
            serde_json::json!("http"),
            "the wire name is the lower-case word the settings page matches"
        );
        assert!(McpServerTransport::Stdio.uses_command() && !McpServerTransport::Stdio.uses_url());
        assert!(McpServerTransport::Sse.uses_url() && !McpServerTransport::Sse.uses_command());
    }

    #[test]
    fn an_id_is_accepted_only_when_it_can_be_read_back_out_of_a_tool_name() {
        for valid in ["files", "github-1", "my_server", "A1", "a", "_leading"] {
            assert!(is_valid_mcp_server_id(valid), "`{valid}` is a name an agent can call");
        }
        for invalid in [
            "",
            "with space",
            "with.slash",
            "two__parts",
            "abcdefghijklmnopqrstuvwxyz0123456",
            "中文",
        ] {
            assert!(
                !is_valid_mcp_server_id(invalid),
                "`{invalid}` would not survive the round trip through a tool name"
            );
        }
        // Exactly the ceiling is a legal id, one past it is not.
        assert!(is_valid_mcp_server_id(&"a".repeat(MCP_SERVER_ID_MAX_CHARS)));
        assert!(!is_valid_mcp_server_id(&"a".repeat(MCP_SERVER_ID_MAX_CHARS + 1)));
    }

    #[test]
    fn normalizing_trims_the_parts_a_hand_edited_file_can_pad() {
        let config = McpServerConfig {
            id: "  files ".to_owned(),
            name: "\tFiles \n".to_owned(),
            transport: McpServerTransport::Stdio,
            command: Some(" npx ".to_owned()),
            args: vec![" -y ".to_owned(), String::new()],
            url: Some("  ".to_owned()),
            enabled: true,
            tool_timeout_seconds: 0,
        }
        .normalized();

        assert_eq!(config.id, "files");
        assert_eq!(config.name, "Files");
        assert_eq!(config.command.as_deref(), Some("npx"));
        assert_eq!(
            config.args,
            vec![" -y ".to_owned(), String::new()],
            "an argument is a value, so its spacing is not trimmed away"
        );
        assert_eq!(config.url, None, "a blank address is no address");
        assert_eq!(
            config.tool_timeout_seconds, MCP_SERVER_TOOL_TIMEOUT_SECONDS_MIN,
            "zero would be a call nobody allows, so it reads as the shortest real budget"
        );
        assert_eq!(
            McpServerConfig::new("x", "", McpServerTransport::Stdio).display_name(),
            "x"
        );
    }

    #[test]
    fn a_new_server_is_enabled_with_the_timeout_the_page_offers() {
        let config = McpServerConfig::new("files", "Files", McpServerTransport::Http);
        assert!(config.enabled, "a server somebody saved is one they meant to use");
        assert_eq!(config.tool_timeout_seconds, MCP_SERVER_TOOL_TIMEOUT_SECONDS_DEFAULT);
        assert!(config.command.is_none() && config.url.is_none());
    }

    #[test]
    fn a_status_names_its_server_and_omits_what_it_does_not_know() {
        let ready = McpServerStatus::new("files", McpServerState::Ready).with_tools(4);
        let json = serde_json::to_value(&ready).expect("a status is JSON");

        assert_eq!(json["serverId"], serde_json::json!("files"));
        assert_eq!(json["state"], serde_json::json!("ready"));
        assert_eq!(json["tools"], serde_json::json!(4));
        assert!(json.get("code").is_none(), "a ready server has no failure to report");
        assert!(json.get("message").is_none());

        let failed = McpServerStatus::new("files", McpServerState::Connecting)
            .with_failure("MCP-SERVER-CONNECT-FAILED", "the server did not answer");
        assert_eq!(failed.state, McpServerState::Failed);
        assert_eq!(failed.code.as_deref(), Some("MCP-SERVER-CONNECT-FAILED"));
    }

    #[test]
    fn every_state_round_trips_through_its_own_name() {
        for state in McpServerState::ALL {
            assert_eq!(
                McpServerState::from_id(state.as_str()),
                Some(*state),
                "a state whose name does not read back is one the interface cannot draw"
            );
            assert_eq!(
                serde_json::to_value(state).expect("a state is JSON"),
                serde_json::json!(state.as_str())
            );
        }
    }
}
