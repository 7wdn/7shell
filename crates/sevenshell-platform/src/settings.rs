//! The settings file: reading and writing the preferences described by
//! [`sevenshell_types::AppSettings`].
//!
//! Three rules, all of them about not losing data:
//!
//! 1. **A missing file is not an error.** It means nobody has chosen anything yet.
//! 2. **A file from a newer version is refused.** Dropping fields a future version needs is
//!    how settings get silently destroyed; refusing to start quietly would be worse, so it
//!    reports `STORE-SCHEMA-NEWER` and the interface says so.
//! 3. **Writes are atomic.** Temporary file, flush, rename. An interrupted write leaves the
//!    previous settings readable, which is invariant 5 applied to something less scary than
//!    the vault but no less annoying to lose.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sevenshell_error::ShellError;
use sevenshell_types::{
    AiProviderConfig, AiProviderKind, AppLayout, AppSettings, BACKGROUND_BLUR_MAX, BACKGROUND_TRANSPARENCY_MAX,
    BackgroundDisplayMode, BackgroundImage, CornerStyle, CustomColors, FilesLayout, HomeLayout, McpScope,
    McpServerConfig, McpServerTransport, SessionLayout, SessionReopen, SnippetProvider, TabGroupMode, TabLayout,
    TabMaxWidth, TabPlacement, TabSortMode, ThemeMode, ThemePalette, ai_context_enabled_default,
    ai_context_lines_default, ai_max_output_tokens_default, ai_session_retention_days_default, background_blur_default,
    background_transparency_default, command_completion_on, is_valid_mcp_server_id,
    mcp_approval_timeout_seconds_default, mcp_audit_retention_days_default, mcp_max_result_bytes_default,
    mcp_server_enabled_default, mcp_server_tool_timeout_seconds_default, monitor_on, normalize_ai_context_lines,
    normalize_ai_session_retention_days, normalize_mcp_approval_timeout_seconds, normalize_mcp_audit_retention_days,
    normalize_mcp_max_result_bytes, normalize_sync_auto_upload_minutes, normalize_terminal_font_size,
    normalize_ui_scale, sidebar_on, sync_auto_upload_minutes_default, terminal_font_size_default, tray_icon_on,
    ui_scale_default, update_check_on,
};

/// The picture behind the window, as the file holds it.
///
/// Its own struct rather than a field of the settings, because it is a group of four values that only
/// mean anything together: a file name with a mode and two numbers. Stored as written, and normalised
/// on the way back in — a hand-edited file naming another mode, or a transparency past the end of its
/// slider, reads as the default rather than as a window nobody can work on.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredBackgroundImage {
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    display_mode: String,
    /// The two the picture is drawn with, defaulting to the same numbers the wire type does: a file
    /// that names a picture but not how much of it shows through reads as 15% rather than as an opaque
    /// window nobody chose.
    #[serde(default = "background_transparency_default")]
    transparency: u8,
    #[serde(default = "background_blur_default")]
    blur: u8,
}

impl StoredBackgroundImage {
    fn from_settings(image: &BackgroundImage) -> Self {
        Self {
            file_name: image.file_name.clone(),
            display_mode: image.display_mode.as_str().to_owned(),
            transparency: image.transparency,
            blur: image.blur,
        }
    }

    fn to_settings(&self) -> BackgroundImage {
        BackgroundImage {
            // A name the machine cannot use is dropped rather than kept: the interface would otherwise
            // draw a picture that is not there, which is worse than drawing none.
            file_name: self
                .file_name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
            display_mode: BackgroundDisplayMode::from_id(&self.display_mode).unwrap_or_default(),
            transparency: self.transparency.min(BACKGROUND_TRANSPARENCY_MAX),
            blur: self.blur.min(BACKGROUND_BLUR_MAX),
        }
    }
}

/// The kind is a string here for the reason the theme is: a file written by a build with a fourth
/// protocol in it has to open in this one, and an entry this build cannot speak is not offered
/// rather than guessed at.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredAiProviderConfig {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    model: String,
    #[serde(default = "ai_max_output_tokens_default")]
    max_output_tokens: u32,
}

impl StoredAiProviderConfig {
    fn from_settings(config: &AiProviderConfig) -> Self {
        Self {
            id: config.id.clone(),
            name: config.name.clone(),
            kind: config.kind.as_str().to_owned(),
            base_url: config.base_url.clone(),
            model: config.model.clone(),
            max_output_tokens: config.max_output_tokens,
        }
    }

    /// One entry this build can offer, or nothing.
    ///
    /// An entry with no id cannot be looked up or have a key filed under it, and one naming a
    /// protocol this build does not speak is dropped rather than guessed at: sending a key to the
    /// wrong kind of API is worse than not offering the provider at all.
    fn to_settings(&self) -> Option<AiProviderConfig> {
        let id = self.id.trim();
        if id.is_empty() {
            return None;
        }
        Some(AiProviderConfig {
            id: id.to_owned(),
            name: self.name.trim().to_owned(),
            kind: AiProviderKind::from_id(self.kind.trim())?,
            base_url: self.base_url.trim().to_owned(),
            model: self.model.trim().to_owned(),
            // Zero is not a budget, and a hand-edited file must not leave the provider to decide.
            max_output_tokens: if self.max_output_tokens == 0 {
                ai_max_output_tokens_default()
            } else {
                self.max_output_tokens
            },
        })
    }
}

/// One external MCP server as the file holds it.
///
/// The transport is a string here for the reason the provider's kind is: a file written by a build
/// with a fourth transport in it has to open in this one, and a server this build cannot reach is
/// not offered rather than guessed at. Nothing in this struct has a field an environment value or a
/// header could be written to — those are in the vault.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredMcpServerConfig {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    transport: String,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default = "mcp_server_enabled_default")]
    enabled: bool,
    #[serde(default = "mcp_server_tool_timeout_seconds_default")]
    tool_timeout_seconds: u64,
}

impl StoredMcpServerConfig {
    fn from_settings(config: &McpServerConfig) -> Self {
        Self {
            id: config.id.clone(),
            name: config.name.clone(),
            transport: config.transport.as_str().to_owned(),
            command: config.command.clone(),
            args: config.args.clone(),
            url: config.url.clone(),
            enabled: config.enabled,
            tool_timeout_seconds: config.tool_timeout_seconds,
        }
    }

    /// One entry this build can offer, or nothing.
    ///
    /// An id that cannot be read back out of a tool name is dropped, because every call to this
    /// server would otherwise be made under a name no agent can hold; a transport this build does
    /// not speak is dropped for the same reason a provider of an unknown kind is.
    fn to_settings(&self) -> Option<McpServerConfig> {
        let id = self.id.trim();
        if !is_valid_mcp_server_id(id) {
            return None;
        }
        Some(
            McpServerConfig {
                id: id.to_owned(),
                name: self.name.clone(),
                transport: McpServerTransport::from_id(self.transport.trim())?,
                command: self.command.clone(),
                args: self.args.clone(),
                url: self.url.clone(),
                enabled: self.enabled,
                tool_timeout_seconds: self.tool_timeout_seconds,
            }
            .normalized(),
        )
    }
}

/// Bumped only when the stored layout changes in a way older builds cannot read.
pub const SETTINGS_SCHEMA_VERSION: u32 = 1;

/// What is actually on disk.
///
/// Strings rather than enums for the values the interface chooses: a file written by a newer
/// version may name a theme this build has never heard of, and falling back to the default is
/// better than refusing to open.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSettings {
    schema_version: u32,
    #[serde(default)]
    theme_mode: String,
    /// Absent in a file written before the interface had more than one palette, which reads as the
    /// one it has always drawn in.
    #[serde(default)]
    theme_palette: String,
    /// Absent in a file written before the terminal had a palette of its own, which reads as
    /// "follow the interface" — the honest answer for somebody who never chose.
    #[serde(default)]
    terminal_theme: String,
    #[serde(default)]
    files_layout: String,
    /// Absent in a file written before a session could open as two halves, which reads as the
    /// default — the split — rather than as the terminal alone.
    #[serde(default)]
    session_layout: String,
    /// Absent in a file written before the session list's menu remembered an answer about a session
    /// that is already open, which reads as the default — asking.
    #[serde(default)]
    session_reopen: String,
    /// Absent in a file written before the terminal had a file panel, which reads as "the divider
    /// has never been moved".
    #[serde(default)]
    terminal_panel_height: Option<u32>,
    /// Absent in a file written before there was a monitor, which reads as the default the feature
    /// ships with — on — rather than as a monitor somebody switched off before it existed.
    #[serde(default = "monitor_on")]
    monitor_enabled: bool,
    /// Absent in a file written before there was a picture behind the window.
    #[serde(default)]
    background_image: StoredBackgroundImage,
    /// The two the shell itself obeys, absent in a file written before the shell had a tray. The
    /// first is on for the same reason it is on in the wire type: a window that puts itself away
    /// needs a way back, and an absent value means "nobody has chosen" rather than "chosen off".
    #[serde(default = "tray_icon_on")]
    show_tray_icon: bool,
    #[serde(default)]
    quit_on_close: bool,
    /// Absent in a file written before the update channel existed, which reads as on: the answer only
    /// arrives if somebody asks, and a build that shipped with the check off would be a build nobody
    /// hears about a new release from.
    #[serde(default = "update_check_on")]
    update_check_on_startup: bool,
    /// A version somebody has said they do not want to hear about, which the channel keeps offering.
    ///
    /// A version rather than a switch, because "not this one" is the decision people make: turning
    /// the check off entirely would stop news of everything after it. Absent, and a blank string, both
    /// read as nobody having said anything.
    #[serde(default)]
    ignored_update_version: Option<String>,
    /// Absent in a file written before there was a packed transfer, which reads as off — the default
    /// the feature ships with. A file written by a build that has it cannot be read back by one that
    /// does not, but that is what the schema version is for.
    #[serde(default)]
    packed_transfer: bool,
    /// Absent in a file written before the command line completed, which reads as the default it ships
    /// with — on — rather than as a list somebody switched off.
    #[serde(default = "command_completion_on")]
    command_completion: bool,
    /// Changed keys, by action. Absent actions read as the default the build ships, which is what
    /// makes adding an action in a later build nothing a file has to be migrated for.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    keybindings: BTreeMap<String, String>,
    /// Absent in a file written before the sidebar could be resized, which reads as "the divider
    /// has never been moved".
    #[serde(default)]
    sidebar_width: Option<u32>,
    /// The two panels split down the middle, absent in a file written before either could be
    /// resized, which reads the same way: the divider has never been moved.
    #[serde(default)]
    files_tree_width: Option<u32>,
    #[serde(default)]
    commands_library_width: Option<u32>,
    /// Absent in a file written before the sidebar could be put away, which reads as the default it
    /// ships with — on — rather than as a sidebar somebody closed before it could be closed.
    #[serde(default = "sidebar_on")]
    sidebar_open: bool,
    /// Absent in a file written before the interface could be scaled, which reads as the size it
    /// has always been drawn at rather than as somebody having asked for something else.
    #[serde(default = "ui_scale_default")]
    ui_scale: u32,
    /// Absent in a file written before the corners were a choice, which reads as the roundness this
    /// build has always drawn rather than as somebody having asked for squares.
    #[serde(default)]
    corner_style: String,
    /// Absent in a file written before there were two home pages, which reads as the one this build
    /// has always drawn.
    #[serde(default)]
    home_layout: String,
    /// Absent in a file written before the chrome was a choice, which reads as the shape this build
    /// has always drawn.
    #[serde(default)]
    app_layout: String,
    /// Absent in a file written before the tab bar could wrap, which reads as the one row this build
    /// has always drawn.
    #[serde(default)]
    tab_layout: String,
    /// The four below are absent in the same way: a file written before the bar could be moved,
    /// grouped, ordered or narrowed reads as the bar this build has always drawn.
    #[serde(default)]
    tab_placement: String,
    #[serde(default)]
    tab_group_mode: String,
    #[serde(default)]
    tab_sort_mode: String,
    #[serde(default)]
    tab_max_width: String,
    /// Absent in a file written before a font could be chosen, which reads as the one this build
    /// ships with rather than as somebody having picked something.
    #[serde(default)]
    ui_font: Option<String>,
    #[serde(default)]
    terminal_font: Option<String>,
    /// Absent in a file written before the terminal's size was a choice, and clamped on the way back in:
    /// a hand-edited file asking for a two-point terminal reads as the default.
    #[serde(default = "terminal_font_size_default")]
    terminal_font_size: u8,
    /// Absent in a file written before the interface could be coloured by hand, which reads as
    /// "nobody has picked any" — the defaults a fresh install starts from.
    #[serde(default)]
    custom_colors: Option<CustomColors>,
    #[serde(default)]
    custom_colors_dark: Option<CustomColors>,
    #[serde(default)]
    locale: Option<String>,
    /// The sync server's address, user name and remote path. Absent in a file written before this
    /// application could talk to one, and read back as "no server configured".
    #[serde(default)]
    sync_endpoint: Option<String>,
    #[serde(default)]
    sync_username: Option<String>,
    #[serde(default)]
    sync_remote_path: Option<String>,
    #[serde(default)]
    sync_include_secrets: bool,
    #[serde(default)]
    sync_auto_upload: bool,
    #[serde(default = "sync_auto_upload_minutes_default")]
    sync_auto_upload_minutes: u32,
    /// A snippet backup's service, its GitLab address, and the file each account writes to. Absent in
    /// a file written before there was anywhere to park a snippet, and read back as "nothing chosen".
    ///
    /// The service is a string here for the same reason the theme is: a file written by a build with
    /// a fourth provider in it has to open in this one, and a name this build does not know reads as
    /// nobody having chosen rather than as a refusal to open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sync_snippet_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sync_snippet_instance: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    sync_snippet_ids: BTreeMap<String, String>,
    /// The agent-facing endpoint. Absent in a file written before there was one, which reads as
    /// "nothing is listening" — the state a fresh install is meant to be in, since a listener is
    /// something somebody turns on rather than something they discover later.
    ///
    /// The scope is a string here for the reason the theme is: a file written by a build with a
    /// fourth tier in it has to open in this one, and an unknown name reads as the narrowest tier
    /// rather than as the widest.
    #[serde(default)]
    mcp_enabled: bool,
    #[serde(default)]
    mcp_port: u16,
    #[serde(default)]
    mcp_scope: String,
    #[serde(default = "mcp_approval_timeout_seconds_default")]
    mcp_approval_timeout_seconds: u64,
    #[serde(default = "mcp_audit_retention_days_default")]
    mcp_audit_retention_days: u32,
    #[serde(default = "mcp_max_result_bytes_default")]
    mcp_max_result_bytes: u64,
    /// Absent in a file written before the settings page could switch tools off, which reads as
    /// every tool on.
    #[serde(default)]
    mcp_tool_policy: sevenshell_types::McpToolPolicy,
    /// Absent in a file written before per-session scopes existed, which reads as the global scope
    /// for every session and every session visible.
    #[serde(default)]
    mcp_access_policy: sevenshell_types::McpAccessPolicy,
    /// Absent in a file written before the policy matrix existed, which reads as the behaviour
    /// every build had: safe and caution run, destructive asks.
    #[serde(default)]
    policy_matrix: sevenshell_types::PolicyMatrix,
    /// The assistant's providers. Absent in a file written before there was an assistant, which
    /// reads as "nothing configured" — there is nowhere to send a question yet.
    ///
    /// This is the non-secret half of each provider. Nothing in this struct has a field a key could
    /// be written to: the key lives in the vault, under the provider's id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    ai_providers: Vec<StoredAiProviderConfig>,
    /// Which provider a new conversation starts with, absent until one has been chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ai_default_provider_id: Option<String>,
    /// Whether the terminal's recent lines travel with a question. Absent reads as on, which is what
    /// the assistant was designed around rather than a feature somebody turned on.
    #[serde(default = "ai_context_enabled_default")]
    ai_context_enabled: bool,
    /// How many lines travel. Absent reads as the default the build ships.
    #[serde(default = "ai_context_lines_default")]
    ai_context_lines: u32,
    /// How many days a stored conversation is kept. Absent in a file written before the assistant
    /// remembered anything, which reads as the default the build ships rather than as "keep none".
    #[serde(default = "ai_session_retention_days_default")]
    ai_session_retention_days: u32,
    /// External MCP servers. Absent in a file written before there were any, which reads as an
    /// empty list rather than as a refusal to open. Secrets are not here: they are in the vault.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    mcp_client_servers: Vec<StoredMcpServerConfig>,
}

impl StoredSettings {
    fn from_settings(settings: &AppSettings) -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            theme_mode: settings.theme_mode.as_str().to_owned(),
            theme_palette: settings.theme_palette.as_str().to_owned(),
            terminal_theme: settings.terminal_theme.as_str().to_owned(),
            files_layout: settings.files_layout.as_str().to_owned(),
            session_layout: settings.session_layout.as_str().to_owned(),
            session_reopen: settings.session_reopen.as_str().to_owned(),
            terminal_panel_height: settings.terminal_panel_height,
            sidebar_width: settings.sidebar_width,
            files_tree_width: settings.files_tree_width,
            commands_library_width: settings.commands_library_width,
            sidebar_open: settings.sidebar_open,
            monitor_enabled: settings.monitor_enabled,
            background_image: StoredBackgroundImage::from_settings(&settings.background_image),
            show_tray_icon: settings.show_tray_icon,
            quit_on_close: settings.quit_on_close,
            update_check_on_startup: settings.update_check_on_startup,
            ignored_update_version: settings.ignored_update_version.clone(),
            packed_transfer: settings.packed_transfer,
            command_completion: settings.command_completion,
            keybindings: settings.keybindings.clone(),
            ui_scale: settings.ui_scale,
            corner_style: settings.corner_style.as_str().to_owned(),
            home_layout: settings.home_layout.as_str().to_owned(),
            app_layout: settings.app_layout.as_str().to_owned(),
            tab_layout: settings.tab_layout.as_str().to_owned(),
            tab_placement: settings.tab_placement.as_str().to_owned(),
            tab_group_mode: settings.tab_group_mode.as_str().to_owned(),
            tab_sort_mode: settings.tab_sort_mode.as_str().to_owned(),
            tab_max_width: settings.tab_max_width.as_str().to_owned(),
            ui_font: settings.ui_font.clone(),
            terminal_font: settings.terminal_font.clone(),
            terminal_font_size: settings.terminal_font_size,
            custom_colors: settings.custom_colors.clone(),
            custom_colors_dark: settings.custom_colors_dark.clone(),
            locale: settings.locale.clone(),
            sync_endpoint: settings.sync_endpoint.clone(),
            sync_username: settings.sync_username.clone(),
            sync_remote_path: settings.sync_remote_path.clone(),
            sync_include_secrets: settings.sync_include_secrets,
            sync_auto_upload: settings.sync_auto_upload,
            sync_auto_upload_minutes: settings.sync_auto_upload_minutes,
            sync_snippet_provider: settings
                .sync_snippet_provider
                .map(|provider| provider.as_str().to_owned()),
            sync_snippet_instance: settings.sync_snippet_instance.clone(),
            sync_snippet_ids: settings.sync_snippet_ids.clone(),
            mcp_enabled: settings.mcp_enabled,
            mcp_port: settings.mcp_port,
            mcp_scope: settings.mcp_scope.as_str().to_owned(),
            mcp_approval_timeout_seconds: settings.mcp_approval_timeout_seconds,
            mcp_audit_retention_days: settings.mcp_audit_retention_days,
            mcp_max_result_bytes: settings.mcp_max_result_bytes,
            mcp_tool_policy: settings.mcp_tool_policy.clone(),
            mcp_access_policy: settings.mcp_access_policy.clone(),
            policy_matrix: settings.policy_matrix.clone(),
            ai_providers: settings
                .ai_providers
                .iter()
                .map(StoredAiProviderConfig::from_settings)
                .collect(),
            ai_default_provider_id: settings.ai_default_provider_id.clone(),
            ai_context_enabled: settings.ai_context_enabled,
            ai_context_lines: settings.ai_context_lines,
            ai_session_retention_days: settings.ai_session_retention_days,
            mcp_client_servers: settings
                .mcp_client_servers
                .iter()
                .map(StoredMcpServerConfig::from_settings)
                .collect(),
        }
    }

    fn into_settings(self) -> AppSettings {
        // Worked out before the struct literal because each answer depends on the others: a provider
        // this build cannot offer must take its id with it, and a default naming one that is gone is
        // no default at all. A hand-edited file is still a boundary.
        let mut seen = std::collections::BTreeSet::new();
        let ai_providers: Vec<AiProviderConfig> = self
            .ai_providers
            .iter()
            .filter_map(StoredAiProviderConfig::to_settings)
            .filter(|provider| seen.insert(provider.id.clone()))
            .collect();
        let ai_default_provider_id = self
            .ai_default_provider_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .filter(|id| ai_providers.iter().any(|provider| provider.id == *id))
            .map(str::to_owned);

        // Worked out the same way, and for the same reason: two servers may not share an id, since
        // the id is the vault key for their secrets and the half of every tool name that says which
        // server answers. The first entry with an id wins, so a hand-edited duplicate cannot make
        // which secrets a call uses depend on file order.
        let mut seen_servers = std::collections::BTreeSet::new();
        let mcp_client_servers: Vec<McpServerConfig> = self
            .mcp_client_servers
            .iter()
            .filter_map(StoredMcpServerConfig::to_settings)
            .filter(|server| seen_servers.insert(server.id.clone()))
            .collect();

        AppSettings {
            theme_mode: ThemeMode::from_id(&self.theme_mode).unwrap_or_default(),
            theme_palette: ThemePalette::from_id(&self.theme_palette).unwrap_or_default(),
            terminal_theme: ThemeMode::from_id(&self.terminal_theme).unwrap_or_default(),
            files_layout: FilesLayout::from_id(&self.files_layout).unwrap_or_default(),
            session_layout: SessionLayout::from_id(&self.session_layout).unwrap_or_default(),
            session_reopen: SessionReopen::from_id(&self.session_reopen).unwrap_or_default(),
            terminal_panel_height: self.terminal_panel_height,
            sidebar_width: self.sidebar_width,
            files_tree_width: self.files_tree_width,
            commands_library_width: self.commands_library_width,
            sidebar_open: self.sidebar_open,
            monitor_enabled: self.monitor_enabled,
            background_image: self.background_image.to_settings(),
            show_tray_icon: self.show_tray_icon,
            quit_on_close: self.quit_on_close,
            update_check_on_startup: self.update_check_on_startup,
            ignored_update_version: self.ignored_update_version.filter(|version| !version.trim().is_empty()),
            packed_transfer: self.packed_transfer,
            command_completion: self.command_completion,
            keybindings: self.keybindings,
            ui_scale: normalize_ui_scale(self.ui_scale),
            corner_style: CornerStyle::from_id(&self.corner_style).unwrap_or_default(),
            home_layout: HomeLayout::from_id(&self.home_layout).unwrap_or_default(),
            app_layout: AppLayout::from_id(&self.app_layout).unwrap_or_default(),
            tab_layout: TabLayout::from_id(&self.tab_layout).unwrap_or_default(),
            tab_placement: TabPlacement::from_id(&self.tab_placement).unwrap_or_default(),
            tab_group_mode: TabGroupMode::from_id(&self.tab_group_mode).unwrap_or_default(),
            tab_sort_mode: TabSortMode::from_id(&self.tab_sort_mode).unwrap_or_default(),
            tab_max_width: TabMaxWidth::from_id(&self.tab_max_width).unwrap_or_default(),
            // A name the machine does not have is not an error and not worth a message: the stack
            // falls through to what follows it, which is exactly why the whole stack is stored.
            ui_font: self.ui_font.filter(|font| !font.trim().is_empty()),
            terminal_font: self.terminal_font.filter(|font| !font.trim().is_empty()),
            terminal_font_size: normalize_terminal_font_size(self.terminal_font_size),
            // A hand-coloured set that cannot be drawn is dropped whole rather than per colour: half
            // of somebody's palette with defaults filled in for the rest is a window nobody asked
            // for, and there is nothing in it they could recognise to fix.
            custom_colors: self.custom_colors.filter(CustomColors::is_drawable),
            custom_colors_dark: self.custom_colors_dark.filter(CustomColors::is_drawable),
            locale: self.locale.filter(|locale| !locale.is_empty()),
            sync_endpoint: self.sync_endpoint.filter(|endpoint| !endpoint.trim().is_empty()),
            sync_username: self.sync_username.filter(|name| !name.trim().is_empty()),
            sync_remote_path: self.sync_remote_path.filter(|path| !path.trim().is_empty()),
            sync_include_secrets: self.sync_include_secrets,
            sync_auto_upload: self.sync_auto_upload,
            sync_auto_upload_minutes: normalize_sync_auto_upload_minutes(self.sync_auto_upload_minutes),
            sync_snippet_provider: self.sync_snippet_provider.as_deref().and_then(SnippetProvider::from_id),
            sync_snippet_instance: self.sync_snippet_instance.filter(|value| !value.trim().is_empty()),
            sync_snippet_ids: self.sync_snippet_ids,
            mcp_enabled: self.mcp_enabled,
            mcp_port: self.mcp_port,
            // A tier this build does not know reads as the narrowest one, never as the widest: a file
            // written by a newer build must not widen what an agent may do in this one.
            mcp_scope: McpScope::from_id(&self.mcp_scope).unwrap_or_default(),
            // Read through the same ends the page offers: a file edited by hand is still a file this
            // build has to survive.
            mcp_approval_timeout_seconds: normalize_mcp_approval_timeout_seconds(self.mcp_approval_timeout_seconds),
            mcp_audit_retention_days: normalize_mcp_audit_retention_days(self.mcp_audit_retention_days),
            mcp_max_result_bytes: normalize_mcp_max_result_bytes(self.mcp_max_result_bytes),
            mcp_tool_policy: self.mcp_tool_policy.clone(),
            mcp_access_policy: self.mcp_access_policy.clone(),
            policy_matrix: self.policy_matrix.clone(),
            ai_providers,
            ai_default_provider_id,
            ai_context_enabled: self.ai_context_enabled,
            // The same ends the page offers, and zero survives as "attach nothing".
            ai_context_lines: normalize_ai_context_lines(self.ai_context_lines),
            // A retention, so zero is not a choice: read through the same ends the page offers.
            ai_session_retention_days: normalize_ai_session_retention_days(self.ai_session_retention_days),
            mcp_client_servers,
        }
    }
}

/// Reads settings from `path`. An absent file yields the defaults.
pub fn load_from(path: &Path) -> Result<AppSettings, ShellError> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(AppSettings::default()),
        Err(error) => return Err(ShellError::store_io(format!("cannot read {}: {error}", path.display()))),
    };

    let stored: StoredSettings = serde_json::from_str(&raw)
        .map_err(|error| ShellError::store_io(format!("{} is not valid settings JSON: {error}", path.display())))?;

    if stored.schema_version > SETTINGS_SCHEMA_VERSION {
        return Err(
            ShellError::store_schema_newer(stored.schema_version, SETTINGS_SCHEMA_VERSION)
                .with_detail(format!("{} was written by a newer version of 7Shell", path.display())),
        );
    }

    Ok(stored.into_settings())
}

/// Writes settings to `path`, atomically.
pub fn save_to(path: &Path, settings: &AppSettings) -> Result<(), ShellError> {
    let stored = StoredSettings::from_settings(settings);
    let json = serde_json::to_string_pretty(&stored)
        .map_err(|error| ShellError::app_internal(format!("settings could not be serialized: {error}")))?;

    write_atomically(path, json.as_bytes())
}

/// The settings of the running application, from the platform configuration directory.
pub fn load() -> Result<AppSettings, ShellError> {
    match crate::paths::settings_file() {
        Some(path) => load_from(&path),
        None => Ok(AppSettings::default()),
    }
}

/// Writes the settings of the running application, creating the directory if needed.
pub fn save(settings: &AppSettings) -> Result<(), ShellError> {
    let path = crate::paths::settings_file()
        .ok_or_else(|| ShellError::store_io("the configuration directory could not be located on this system"))?;

    save_to(&path, settings)
}

/// Writes bytes to `path`, atomically.
///
/// Shared with the other file this crate keeps, so that "an interrupted write leaves the previous
/// file readable" is one rule rather than one per file.
pub(crate) fn write_atomically(path: &Path, contents: &[u8]) -> Result<(), ShellError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| ShellError::store_io(format!("cannot create {}: {error}", parent.display())))?;
    }

    let temporary = path.with_extension("json.tmp");
    let write = || -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        Ok(())
    };

    if let Err(error) = write() {
        // Leave nothing half-written behind for the next read to trip over.
        let _ = std::fs::remove_file(&temporary);
        return Err(ShellError::store_io(format!(
            "cannot write {}: {error}",
            temporary.display()
        )));
    }

    std::fs::rename(&temporary, path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        ShellError::store_io(format!("cannot replace {}: {error}", path.display()))
    })
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    fn settings(theme_mode: ThemeMode, terminal_theme: ThemeMode, locale: Option<&str>) -> AppSettings {
        AppSettings {
            theme_mode,
            terminal_theme,
            files_layout: FilesLayout::default(),
            session_layout: SessionLayout::default(),
            session_reopen: SessionReopen::default(),
            terminal_panel_height: None,
            sidebar_width: None,
            files_tree_width: None,
            commands_library_width: None,
            sidebar_open: sidebar_on(),
            monitor_enabled: monitor_on(),
            background_image: BackgroundImage::default(),
            show_tray_icon: tray_icon_on(),
            quit_on_close: false,
            update_check_on_startup: update_check_on(),
            ignored_update_version: None,
            packed_transfer: false,
            command_completion: command_completion_on(),
            keybindings: BTreeMap::new(),
            theme_palette: ThemePalette::default(),
            ui_scale: ui_scale_default(),
            corner_style: CornerStyle::default(),
            home_layout: HomeLayout::default(),
            app_layout: AppLayout::default(),
            tab_layout: TabLayout::default(),
            tab_placement: TabPlacement::default(),
            tab_group_mode: TabGroupMode::default(),
            tab_sort_mode: TabSortMode::default(),
            tab_max_width: TabMaxWidth::default(),
            ui_font: None,
            terminal_font: None,
            terminal_font_size: terminal_font_size_default(),
            custom_colors: None,
            custom_colors_dark: None,
            locale: locale.map(str::to_owned),
            sync_endpoint: None,
            sync_username: None,
            sync_remote_path: None,
            sync_include_secrets: false,
            sync_auto_upload: false,
            sync_auto_upload_minutes: sync_auto_upload_minutes_default(),
            sync_snippet_provider: None,
            sync_snippet_instance: None,
            sync_snippet_ids: BTreeMap::new(),
            mcp_enabled: false,
            mcp_port: 0,
            mcp_scope: McpScope::default(),
            mcp_approval_timeout_seconds: mcp_approval_timeout_seconds_default(),
            mcp_audit_retention_days: mcp_audit_retention_days_default(),
            mcp_max_result_bytes: mcp_max_result_bytes_default(),
            mcp_tool_policy: Default::default(),
            mcp_access_policy: Default::default(),
            policy_matrix: Default::default(),
            ai_providers: Vec::new(),
            ai_default_provider_id: None,
            ai_context_enabled: ai_context_enabled_default(),
            ai_context_lines: ai_context_lines_default(),
            ai_session_retention_days: ai_session_retention_days_default(),
            mcp_client_servers: Vec::new(),
        }
    }

    #[test]
    fn the_agent_endpoint_survives_a_round_trip_and_an_unknown_scope_reads_narrow() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");

        let mut written = settings(ThemeMode::Dark, ThemeMode::System, None);
        written.mcp_enabled = true;
        written.mcp_port = 4711;
        written.mcp_scope = McpScope::SafeWrite;
        written.mcp_approval_timeout_seconds = 90;
        save_to(&path, &written).expect("write");

        assert_eq!(load_from(&path).expect("read"), written);

        // A tier written by a build that has one this build does not must not widen the scope: the
        // narrowest tier is the one whose answer cannot be wrong about what an agent may do.
        std::fs::write(&path, r#"{"schemaVersion":1,"mcpScope":"godmode"}"#).expect("write");
        assert_eq!(load_from(&path).expect("read").mcp_scope, McpScope::ReadOnly);
    }

    #[test]
    fn an_agent_number_written_outside_its_range_reads_at_the_nearest_end() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"schemaVersion":1,"mcpApprovalTimeoutSeconds":0,"mcpAuditRetentionDays":0,"mcpMaxResultBytes":1}"#,
        )
        .expect("write");

        let loaded = load_from(&path).expect("read");

        // A hand-edited file is still a boundary: zero would hold every approval until the tool's own
        // timeout, keep no audit at all, and truncate every answer to a single byte.
        assert_eq!(
            loaded.mcp_approval_timeout_seconds,
            sevenshell_types::MCP_APPROVAL_TIMEOUT_SECONDS_MIN
        );
        assert_eq!(
            loaded.mcp_audit_retention_days,
            sevenshell_types::MCP_AUDIT_RETENTION_DAYS_MIN
        );
        assert_eq!(loaded.mcp_max_result_bytes, sevenshell_types::MCP_MAX_RESULT_BYTES_MIN);
    }

    #[test]
    fn an_assistant_provider_survives_a_round_trip_and_no_key_is_ever_written() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");

        let mut written = settings(ThemeMode::Dark, ThemeMode::System, None);
        written.ai_providers = vec![AiProviderConfig {
            id: "ollama".to_owned(),
            name: "Ollama".to_owned(),
            kind: AiProviderKind::Local,
            base_url: "http://127.0.0.1:11434/v1".to_owned(),
            model: "llama3.1".to_owned(),
            max_output_tokens: 2_048,
        }];
        written.ai_default_provider_id = Some("ollama".to_owned());
        written.ai_context_enabled = false;
        written.ai_context_lines = 40;
        save_to(&path, &written).expect("write");

        assert_eq!(load_from(&path).expect("read"), written);

        let stored: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("json");
        let provider = stored["aiProviders"][0].as_object().expect("one provider");
        assert_eq!(provider.get("kind").and_then(|kind| kind.as_str()), Some("local"));
        assert_eq!(
            provider.len(),
            6,
            "a provider is id, name, kind, address, model and a budget — nothing a key could be written to"
        );
        for forbidden in ["key", "apiKey", "hasKey", "token", "secret"] {
            assert!(
                !provider.contains_key(forbidden),
                "`{forbidden}` reached the settings file, which anything can read"
            );
        }
    }

    #[test]
    fn a_file_written_before_the_assistant_reads_as_nothing_configured() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        std::fs::write(&path, r#"{"schemaVersion":1}"#).expect("write");

        let loaded = load_from(&path).expect("an older file is not an error");

        assert!(loaded.ai_providers.is_empty(), "there is nowhere to send a question");
        assert_eq!(loaded.ai_default_provider_id, None);
        assert!(loaded.ai_context_enabled, "the terminal travels by default");
        assert_eq!(loaded.ai_context_lines, ai_context_lines_default());
        assert_eq!(
            loaded.ai_session_retention_days,
            ai_session_retention_days_default(),
            "a file written before the assistant remembered anything keeps the month this build ships"
        );
    }

    #[test]
    fn the_session_retention_survives_a_round_trip_and_is_clamped_on_the_way_back_in() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");

        let mut written = settings(ThemeMode::Dark, ThemeMode::System, None);
        written.ai_session_retention_days = 90;
        save_to(&path, &written).expect("write");
        assert_eq!(
            load_from(&path).expect("read").ai_session_retention_days,
            90,
            "a retention somebody chose did not round-trip"
        );

        // A hand-edited file is still a boundary: zero would delete every conversation the moment it
        // was written, and a decade is longer than any of them would be read.
        std::fs::write(&path, r#"{"schemaVersion":1,"aiSessionRetentionDays":0}"#).expect("write");
        assert_eq!(
            load_from(&path).expect("read").ai_session_retention_days,
            sevenshell_types::AI_SESSION_RETENTION_DAYS_MIN
        );

        std::fs::write(&path, r#"{"schemaVersion":1,"aiSessionRetentionDays":10000}"#).expect("write");
        assert_eq!(
            load_from(&path).expect("read").ai_session_retention_days,
            sevenshell_types::AI_SESSION_RETENTION_DAYS_MAX
        );
    }

    #[test]
    fn the_assistant_context_is_clamped_to_the_range_the_page_offers() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");

        std::fs::write(&path, r#"{"schemaVersion":1,"aiContextLines":900}"#).expect("write");
        assert_eq!(
            load_from(&path).expect("read").ai_context_lines,
            sevenshell_types::AI_CONTEXT_LINES_MAX
        );

        // Zero is a choice — attach nothing — rather than a missing value, so it survives.
        std::fs::write(
            &path,
            r#"{"schemaVersion":1,"aiContextLines":0,"aiContextEnabled":false}"#,
        )
        .expect("write");
        let loaded = load_from(&path).expect("read");
        assert_eq!(loaded.ai_context_lines, 0);
        assert!(!loaded.ai_context_enabled);
    }

    #[test]
    fn a_provider_this_build_cannot_speak_is_dropped_with_its_default() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"schemaVersion":1,"aiProviders":[{"id":"gemini","name":"Gemini","kind":"gemini","baseUrl":"https://example.test","model":"x","maxOutputTokens":100}],"aiDefaultProviderId":"gemini"}"#,
        )
        .expect("write");

        let loaded = load_from(&path).expect("an unknown protocol is not a refusal to open");

        assert!(
            loaded.ai_providers.is_empty(),
            "a provider whose protocol this build does not speak is not offered"
        );
        assert_eq!(
            loaded.ai_default_provider_id, None,
            "and a default naming it is no default at all"
        );
    }

    #[test]
    fn a_sync_server_survives_a_round_trip_and_a_cleared_field_is_no_server() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");

        let mut written = settings(ThemeMode::Dark, ThemeMode::System, None);
        written.sync_endpoint = Some("https://dav.example.com/remote.php/dav/files/alice/".to_owned());
        written.sync_username = Some("alice".to_owned());
        written.sync_remote_path = Some("7shell/sync/backup.json".to_owned());
        written.sync_include_secrets = true;
        save_to(&path, &written).expect("write");

        let loaded = load_from(&path).expect("read");

        assert_eq!(
            loaded.sync_endpoint.as_deref(),
            Some("https://dav.example.com/remote.php/dav/files/alice/")
        );
        assert_eq!(loaded.sync_username.as_deref(), Some("alice"));
        assert_eq!(loaded.sync_remote_path.as_deref(), Some("7shell/sync/backup.json"));
        assert!(loaded.sync_include_secrets);

        // A field somebody cleared is nobody having a server, not a server at the empty address: the
        // panel's fields are filled in together or not at all, and a half-cleared form must not read
        // as a configuration to dial. Whitespace counts as cleared for the same reason a font name
        // does — it is what an empty text box leaves behind.
        std::fs::write(
            &path,
            r#"{"schemaVersion":1,"syncEndpoint":"   ","syncUsername":"","syncRemotePath":"x"}"#,
        )
        .expect("write");

        let loaded = load_from(&path).expect("read");

        assert_eq!(loaded.sync_endpoint, None);
        assert_eq!(loaded.sync_username, None);
        assert_eq!(
            loaded.sync_remote_path.as_deref(),
            Some("x"),
            "the fields are read one by one"
        );
        assert!(
            !loaded.sync_include_secrets,
            "a file written before this existed is not a person asking for it"
        );
    }

    #[test]
    fn a_snippet_setup_survives_a_round_trip_and_a_service_this_build_does_not_know_is_no_choice() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");

        let mut written = settings(ThemeMode::Dark, ThemeMode::System, None);
        written.sync_snippet_provider = Some(SnippetProvider::GitLab);
        written.sync_snippet_instance = Some("https://gitlab.example.com".to_owned());
        written.sync_snippet_ids = BTreeMap::from([
            ("gitlab:https://gitlab.example.com".to_owned(), "42".to_owned()),
            ("github".to_owned(), "abc123".to_owned()),
        ]);
        save_to(&path, &written).expect("write");

        let loaded = load_from(&path).expect("read");

        assert_eq!(loaded.sync_snippet_provider, Some(SnippetProvider::GitLab));
        assert_eq!(
            loaded.sync_snippet_instance.as_deref(),
            Some("https://gitlab.example.com")
        );
        assert_eq!(
            loaded.sync_snippet_ids.get("github").map(String::as_str),
            Some("abc123"),
            "the id of a service somebody is not currently on is not thrown away"
        );

        // A file naming a service this build has never heard of is not a refusal to open — the same
        // rule the theme follows. What it reads as is "nobody has chosen", which is a state the panel
        // can draw and a person can act on.
        std::fs::write(
            &path,
            r#"{"schemaVersion":1,"syncSnippetProvider":"bitbucket","syncSnippetInstance":"  "}"#,
        )
        .expect("write");

        let loaded = load_from(&path).expect("read");
        assert_eq!(loaded.sync_snippet_provider, None);
        assert_eq!(loaded.sync_snippet_instance, None);
    }

    #[test]
    fn a_file_written_before_the_terminal_had_a_palette_still_loads() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        std::fs::write(&path, r#"{"schemaVersion":1,"themeMode":"dark","locale":"zh-CN"}"#).expect("write");

        let loaded = load_from(&path).expect("an older file is not an error");

        assert_eq!(loaded.theme_mode, ThemeMode::Dark);
        assert_eq!(
            loaded.terminal_theme,
            ThemeMode::System,
            "absent means follow the interface"
        );
        assert_eq!(
            loaded.session_layout,
            SessionLayout::Split,
            "absent means the default, and the default opens both halves"
        );
        assert_eq!(
            loaded.session_reopen,
            SessionReopen::Ask,
            "a file written before the menu could remember anything is not a person who answered"
        );
        assert_eq!(
            loaded.terminal_panel_height, None,
            "absent means the divider has never been moved"
        );
        assert!(
            loaded.monitor_enabled,
            "a file written before the monitor existed is not a person asking for it to be off"
        );
        assert!(
            !loaded.packed_transfer,
            "a file written before packed transfers existed is not a person asking for one"
        );
        assert!(
            loaded.keybindings.is_empty(),
            "a file written before keys could be changed has none changed"
        );
        assert!(
            loaded.command_completion,
            "a file written before the command line completed is not a person asking for it to be off"
        );
        assert_eq!(
            loaded.theme_palette,
            ThemePalette::Default,
            "a file written before there was a choice of palette reads as the one that was always there"
        );
        assert_eq!(
            loaded.ui_scale,
            ui_scale_default(),
            "and the interface is the size it has always been"
        );
        assert_eq!(
            loaded.custom_colors, None,
            "a file written before colours could be picked by hand has picked none"
        );
        assert_eq!(loaded.locale.as_deref(), Some("zh-CN"));
    }

    #[test]
    fn a_palette_this_build_does_not_know_falls_back_instead_of_failing() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"schemaVersion":1,"themeMode":"dark","themePalette":"aubergine","uiScale":125}"#,
        )
        .expect("write");

        let loaded = load_from(&path).expect("a palette from a newer build is not a hard failure");

        assert_eq!(loaded.theme_palette, ThemePalette::Default);
        assert_eq!(
            loaded.ui_scale, 125,
            "the fields either side of the unknown one are still read"
        );
    }

    #[test]
    fn a_hand_coloured_set_is_read_back_and_a_broken_one_is_dropped() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        let chosen = CustomColors {
            background: "#101820".to_owned(),
            foreground: "#e8e8e8".to_owned(),
            primary: "#4f9cf0".to_owned(),
            border: "#2a3340".to_owned(),
            card: "#161f2b".to_owned(),
        };
        let written = AppSettings {
            theme_palette: ThemePalette::Custom,
            custom_colors: Some(chosen.clone()),
            ..AppSettings::default()
        };
        save_to(&path, &written).expect("save");
        assert_eq!(
            load_from(&path).expect("load").custom_colors,
            Some(chosen),
            "a colour somebody picked comes back as it was picked"
        );

        // One colour that is not a colour takes the whole set with it: half a palette with defaults
        // filled in is a window nobody chose and nothing in it to recognise.
        std::fs::write(
            &path,
            // `r##` rather than `r#`: every colour in this JSON opens with `"#`, which is where the
            // shorter delimiter ends.
            r##"{"schemaVersion":1,"themePalette":"custom","customColors":{"background":"rebeccapurple","foreground":"#e8e8e8","primary":"#4f9cf0","border":"#2a3340","card":"#161f2b"}}"##,
        )
        .expect("write");
        assert_eq!(load_from(&path).expect("load").custom_colors, None);
    }

    #[test]
    fn a_scale_no_menu_offers_reads_as_the_default() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        // The interface cannot reach a scale outside the list, so one in the file was either typed
        // by hand or written by a build with a different list — and either way it is not a choice
        // this build can show or take back.
        std::fs::write(&path, r#"{"schemaVersion":1,"uiScale":400}"#).expect("write");

        let loaded = load_from(&path).expect("an unusable scale is not a hard failure");

        assert_eq!(loaded.ui_scale, ui_scale_default());
    }

    #[test]
    fn an_absent_file_means_nothing_has_been_chosen_yet() {
        let directory = tempdir().expect("temp dir");
        let loaded = load_from(&directory.path().join("settings.json")).expect("defaults");

        assert_eq!(loaded, AppSettings::default());
        assert!(loaded.is_untouched());
    }

    #[test]
    fn settings_survive_a_round_trip() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        let written = AppSettings {
            session_layout: SessionLayout::Terminal,
            // Not the default, so the round trip proves a remembered answer survives rather than the
            // question coming back.
            session_reopen: SessionReopen::NewTab,
            terminal_panel_height: Some(320),
            // Not the defaults, so the round trip proves that the two widths a divider was left at
            // come back rather than the file forgetting half of what it was given.
            files_tree_width: Some(300),
            commands_library_width: Some(200),
            // Off, so the round trip proves a choice survives rather than the default coming back.
            monitor_enabled: false,
            // On, for the same reason in the other direction: the default is off.
            packed_transfer: true,
            // Off, for the same reason again: this one's default is on.
            command_completion: false,
            // A key somebody changed, so the round trip proves a changed one survives — and that an
            // action nobody touched is still the default rather than the default written down.
            keybindings: BTreeMap::from([("terminal.find".to_owned(), "Mod+G".to_owned())]),
            // Neither is the default, for the same reason again: a palette and a size both have to
            // come back as the ones that were chosen.
            theme_palette: ThemePalette::Claude,
            ui_scale: 150,
            // Not the default, so the round trip proves a choice survives.
            corner_style: CornerStyle::None,
            // Not the default, so the round trip proves a choice survives.
            home_layout: HomeLayout::Intro,
            // Not the default, so the round trip proves a choice survives.
            app_layout: AppLayout::Classic,
            // The same, for the two the shell reads: the icon off and the window quitting when it is
            // closed, which are both the opposite of what a fresh install has.
            show_tray_icon: false,
            quit_on_close: true,
            // Not the default, so the round trip proves a choice survives.
            terminal_font_size: 20,
            // And one with a picture behind it, written at the far end of every slider.
            background_image: BackgroundImage {
                file_name: Some("wallpaper.png".to_owned()),
                display_mode: BackgroundDisplayMode::Tile,
                transparency: BACKGROUND_TRANSPARENCY_MAX,
                blur: BACKGROUND_BLUR_MAX,
            },
            // The same, for the rest of the words the bar is stored as.
            tab_layout: TabLayout::Wrap,
            tab_placement: TabPlacement::Right,
            tab_group_mode: TabGroupMode::Session,
            tab_sort_mode: TabSortMode::TitleAsc,
            tab_max_width: TabMaxWidth::Compact,
            ui_font: Some("'JetBrains Mono', monospace".to_owned()),
            terminal_font: Some("'Fira Code', monospace".to_owned()),
            custom_colors: Some(CustomColors::light_default()),
            custom_colors_dark: Some(CustomColors::dark_default()),
            ..settings(ThemeMode::Light, ThemeMode::System, Some("zh-CN"))
        };

        save_to(&path, &written).expect("save");
        assert_eq!(load_from(&path).expect("load"), written);

        // Nothing is left over from writing.
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn a_theme_this_build_does_not_know_falls_back_instead_of_failing() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"schemaVersion":1,"themeMode":"midnight-aurora","locale":"en"}"#,
        )
        .expect("write");

        let loaded = load_from(&path).expect("a future theme is not a hard failure");

        assert_eq!(loaded.theme_mode, ThemeMode::System);
        assert_eq!(loaded.locale.as_deref(), Some("en"));
    }

    #[test]
    fn a_newer_schema_is_refused_rather_than_downgraded() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        std::fs::write(&path, r#"{"schemaVersion":2,"themeMode":"dark","locale":"en"}"#).expect("write");

        let error = load_from(&path).expect_err("a newer file must not be read as if it were ours");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::StoreSchemaNewer);
        assert_eq!(error.message_params().get("found").map(String::as_str), Some("2"));
    }

    #[test]
    fn broken_json_is_reported_and_not_swallowed() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        std::fs::write(&path, "{ not json").expect("write");

        let error = load_from(&path).expect_err("broken settings must be visible");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::StoreIo);
    }

    #[test]
    fn a_failed_write_leaves_the_previous_file_alone() {
        let directory = tempdir().expect("temp dir");
        let blocker = directory.path().join("blocker");
        std::fs::write(&blocker, "do not touch").expect("write");

        // The parent of the settings path is a file, so creating the directory fails.
        let error =
            save_to(&blocker.join("settings.json"), &AppSettings::default()).expect_err("cannot write through a file");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::StoreIo);
        assert_eq!(
            std::fs::read_to_string(&blocker).expect("still readable"),
            "do not touch"
        );
    }

    #[test]
    fn an_external_server_round_trips_and_no_secret_is_ever_written() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");

        let mut written = settings(ThemeMode::Dark, ThemeMode::System, None);
        written.mcp_client_servers = vec![McpServerConfig {
            id: "files".to_owned(),
            name: "Files".to_owned(),
            transport: McpServerTransport::Stdio,
            command: Some("npx".to_owned()),
            args: vec!["-y".to_owned(), "server-filesystem".to_owned()],
            url: None,
            enabled: false,
            tool_timeout_seconds: 30,
        }];
        save_to(&path, &written).expect("write");

        assert_eq!(load_from(&path).expect("read"), written);

        let stored: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("json");
        let server = stored["mcpClientServers"][0].as_object().expect("one server");
        assert_eq!(server["transport"], serde_json::json!("stdio"));
        assert_eq!(server["toolTimeoutSeconds"], serde_json::json!(30));
        assert_eq!(server["enabled"], serde_json::json!(false));
        for forbidden in ["env", "headers", "token", "secret", "password"] {
            assert!(
                !server.contains_key(forbidden),
                "`{forbidden}` reached the settings file, where the vault is the only place it belongs"
            );
        }
    }

    #[test]
    fn a_server_this_build_cannot_reach_is_dropped_and_a_timeout_is_clamped() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"schemaVersion":1,"mcpClientServers":[
                {"id":"bad id","transport":"stdio","command":"npx"},
                {"id":"mystery","transport":"websocket","url":"https://example.test/mcp"},
                {"id":" slow ","name":" Slow ","transport":"http","url":"https://example.test/mcp","toolTimeoutSeconds":0},
                {"id":"slow","transport":"http","url":"https://example.test/other"}
            ]}"#,
        )
        .expect("write");

        let loaded = load_from(&path).expect("a file with an unusable server still opens");

        assert_eq!(
            loaded.mcp_client_servers.len(),
            1,
            "an id that cannot name a tool, an unknown transport and a duplicate are all not servers"
        );
        let server = &loaded.mcp_client_servers[0];
        assert_eq!(server.id, "slow", "the id is trimmed before it is compared");
        assert_eq!(server.name, "Slow");
        assert_eq!(
            server.tool_timeout_seconds,
            sevenshell_types::MCP_SERVER_TOOL_TIMEOUT_SECONDS_MIN,
            "zero would fail before trying, so it reads as the shortest real budget"
        );

        std::fs::write(&path, r#"{"schemaVersion":1}"#).expect("write");
        assert!(
            load_from(&path).expect("read").mcp_client_servers.is_empty(),
            "a file written before there were external servers has none"
        );
    }
}
