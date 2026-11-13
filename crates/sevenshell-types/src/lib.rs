//! Wire types for 7Shell.
//!
//! Two halves that have to agree: the hand-written DTOs the interface consumes, and
//! the descriptor tables generated from `descriptors/`. The generated module is
//! included from `OUT_DIR`, so it always reflects the committed manifest.

mod agent;
mod ai;
mod ai_conversation;
mod app_info;
mod batch;
mod completion;
mod container;
mod descriptors;
mod k8s;
mod mcp_client;
mod monitor;
mod policy_matrix;
mod risk;
mod session;
mod settings;
mod snippet;
mod sync;
mod system;
mod update;
mod words;

mod generated {
    include!(concat!(env!("OUT_DIR"), "/descriptors.rs"));
}

pub use agent::AgentOrigin;
pub use ai::{
    AI_CONTEXT_LINES_DEFAULT, AI_CONTEXT_LINES_MAX, AI_CONTEXT_LINES_MIN, AI_MAX_OUTPUT_TOKENS_DEFAULT,
    AiProviderConfig, AiProviderKind, AiProviderView, ai_context_enabled_default, ai_context_lines_default,
    ai_max_output_tokens_default, normalize_ai_context_lines,
};
pub use ai_conversation::{
    AI_CONTEXT_SUMMARY_VERSION, AiContextSummary, AiSessionDetail, AiSessionSummary, AiToolOutcome,
    ai_context_summary_version_default,
};
pub use app_info::{AppInfo, PRODUCT_NAME};
pub use batch::{BatchHostOutcome, BatchHostRef, BatchHostResult, BatchOutcome, BatchRequest, HostFailure};
pub use completion::{
    COMPLETION_DEFAULT_TIMEOUT_MS, COMPLETION_MAX_ARGS, COMPLETION_MAX_OUTPUT_BYTES, COMPLETION_MAX_TEXT_BYTES,
    COMPLETION_TIMEOUT_MAX_MS, COMPLETION_TIMEOUT_MIN_MS, CompletionExecuteRequest, CompletionExecuteResult,
};
pub use container::{
    ContainerCommitRequest, ContainerCreateRequest, ContainerDetail, ContainerImage, ContainerImageTransfer,
    ContainerMount, ContainerPort, ContainerPortMapping, ContainerRegistryLogin, ContainerRuntime,
    ContainerRuntimeInfo, ContainerSettings, ContainerStateCount, ContainerStats, ContainerStreamEnd,
    ContainerStreamLine, ContainerSummary, ContainerTransport, ContainerVolume, ImageLayer,
};
pub use generated::{
    AUTH_METHODS, AuthMethod, AuthMethodSpec, CAPABILITIES, Capability, CapabilityGroup, CapabilitySpec,
    DESCRIPTOR_SCHEMA_VERSION, PROTOCOLS, Protocol, ProtocolSpec, Runtime, SupportLevel,
};
pub use k8s::{
    K8sExecRequest, K8sLogRequest, K8sMethod, K8sRequest, K8sResponse, K8sStreamEnd, K8sStreamLine, K8sWatchEvent,
    KubeContextInfo, KubeconfigSource, KubernetesSettings,
};
pub use mcp_client::{
    MCP_SERVER_ID_MAX_CHARS, MCP_SERVER_TOOL_SEPARATOR, MCP_SERVER_TOOL_TIMEOUT_SECONDS_DEFAULT,
    MCP_SERVER_TOOL_TIMEOUT_SECONDS_MAX, MCP_SERVER_TOOL_TIMEOUT_SECONDS_MIN, McpPrompt, McpPromptArgument,
    McpPromptMessage, McpPromptRole, McpPromptText, McpResource, McpResourceContent, McpResourceRead, McpServerConfig,
    McpServerContent, McpServerState, McpServerStatus, McpServerTransport, is_valid_mcp_server_id,
    mcp_server_enabled_default, mcp_server_tool_timeout_seconds_default, normalize_mcp_server_tool_timeout_seconds,
};
pub use monitor::{
    CpuCounters, CpuModel, CpuStates, DiskUsage, EnvironmentEntry, HostMetrics, LoadAverage, MAX_COMMAND_CHARS,
    MAX_DISKS, MAX_ENVIRONMENT_ENTRIES, MAX_PAGE_PROCESSES, MAX_PROCESSES, MAX_SOCKETS, MemoryUsage, NetworkCounters,
    ProcessDetail, ProcessPage, ProcessSample, SocketRow,
};
pub use policy_matrix::{PolicyAction, PolicyMatrix, PolicyRule};
pub use risk::{RiskAssessment, RiskLevel, RiskReason};
pub use session::{
    BackspaceMode, CreatedKey, DesktopSettings, DownloadFolderView, EnterMode, EntryKind, ForwardMode, FtpSettings,
    KeyAlgorithm, KeyInfo, KnownHostEntry, LocalForward, LocalSettings, LocalShell, OpenOutcome, OpenTunnel,
    PresentedHostKey, SecretInput, SerialFlowControl, SerialParity, SerialPortInfo, SerialSettings, SerialStopBits,
    Session, SessionAuth, SessionExportSummary, SessionGroup, SessionImportOutcome, SessionImportSummary, SftpEntry,
    SftpListing, SftpOutcome, SftpPasteItem, SftpPasteOutcome, TelnetSettings, TerminalSettings, TransferDirection,
    TransferInfo, TransferState, TunnelFailure, UploadAnswer, UploadOutcome, VaultArchive, VaultState, WebDavSettings,
    WslSettings,
};
pub use settings::{
    AI_SESSION_RETENTION_DAYS_DEFAULT, AI_SESSION_RETENTION_DAYS_MAX, AI_SESSION_RETENTION_DAYS_MIN, AppLayout,
    AppSettings, BACKGROUND_BLUR_MAX, BACKGROUND_TRANSPARENCY_MAX, BackgroundDisplayMode, BackgroundImage, CornerStyle,
    CustomColors, FilesLayout, HomeLayout, MCP_APPROVAL_TIMEOUT_SECONDS_DEFAULT, MCP_APPROVAL_TIMEOUT_SECONDS_MAX,
    MCP_APPROVAL_TIMEOUT_SECONDS_MIN, MCP_AUDIT_RETENTION_DAYS_DEFAULT, MCP_AUDIT_RETENTION_DAYS_MAX,
    MCP_AUDIT_RETENTION_DAYS_MIN, MCP_MAX_RESULT_BYTES_DEFAULT, MCP_MAX_RESULT_BYTES_MAX, MCP_MAX_RESULT_BYTES_MIN,
    McpAccessPolicy, McpScope, McpToolPolicy, SYNC_AUTO_UPLOAD_MINUTES_DEFAULT, SYNC_AUTO_UPLOAD_MINUTES_MAX,
    SYNC_AUTO_UPLOAD_MINUTES_MIN, SessionLayout, SessionReopen, TERMINAL_CUSTOM_SCHEME_LIMIT, TERMINAL_FONT_SIZE_MAX,
    TERMINAL_FONT_SIZE_MIN, TERMINAL_FONT_WEIGHT_MAX, TERMINAL_FONT_WEIGHT_MIN, TERMINAL_LETTER_SPACING_MAX,
    TERMINAL_LETTER_SPACING_MIN, TERMINAL_LINE_HEIGHT_MAX, TERMINAL_LINE_HEIGHT_MIN, TERMINAL_MIN_CONTRAST_MAX,
    TERMINAL_MIN_CONTRAST_MIN, TERMINAL_PADDING_MAX, TERMINAL_PADDING_MIN, TERMINAL_SCHEME_PROFILE_LIMIT, TabGroupMode,
    TabLayout, TabMaxWidth, TabPlacement, TabSortMode, TerminalCursorInactiveStyle, TerminalCursorStyle,
    TerminalCustomScheme, TerminalSchemeBackground, TerminalSchemeProfile, TerminalSchemeSource, ThemeMode,
    ThemePalette, UI_SCALE_PERCENTS, ai_session_retention_days_default, background_blur_default,
    background_transparency_default, command_completion_on, mcp_approval_timeout_seconds_default,
    mcp_audit_retention_days_default, mcp_max_result_bytes_default, monitor_on, normalize_ai_session_retention_days,
    normalize_mcp_approval_timeout_seconds, normalize_mcp_audit_retention_days, normalize_mcp_max_result_bytes,
    normalize_sync_auto_upload_minutes, normalize_terminal_bold_font_weight, normalize_terminal_font_size,
    normalize_terminal_font_weight, normalize_terminal_letter_spacing, normalize_terminal_line_height,
    normalize_terminal_min_contrast, normalize_terminal_padding, normalize_ui_scale, sidebar_on,
    sync_auto_upload_minutes_default, terminal_bold_bright_on, terminal_bold_font_weight_default,
    terminal_cursor_blink_on, terminal_font_size_default, terminal_font_weight_default,
    terminal_letter_spacing_default, terminal_line_height_default, terminal_min_contrast_default,
    terminal_padding_x_default, terminal_padding_y_default, tray_icon_on, ui_scale_default, update_check_on,
};
pub use snippet::{
    CommandHistoryEntry, DEFAULT_GROUP_ID, SLOT_HISTORY, SNIPPET_SLOTS, SendFailure, SendOutcome, SendReport,
    SendTarget, Snippet, SnippetGroup,
};
pub use sync::{
    SnippetProvider, SnippetStatus, SnippetUploadReceipt, SyncApplySummary, SyncCatalog, SyncContents,
    SyncDownloadOutcome, SyncInspectOutcome, SyncReceipt, SyncSelection, SyncServiceStatus, SyncUploadReceipt,
};
pub use system::HostSystem;
pub use update::{AvailableRelease, UpdateCheck, UpdateProgress};
pub use words::CompletionWord;
