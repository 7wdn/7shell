//! Wire types for 7Shell.
//!
//! Two halves that have to agree: the hand-written DTOs the interface consumes, and
//! the descriptor tables generated from `descriptors/`. The generated module is
//! included from `OUT_DIR`, so it always reflects the committed manifest.

mod app_info;
mod batch;
mod descriptors;
mod monitor;
mod risk;
mod session;
mod settings;
mod snippet;
mod words;

mod generated {
    include!(concat!(env!("OUT_DIR"), "/descriptors.rs"));
}

pub use app_info::{AppInfo, PRODUCT_NAME};
pub use batch::{BatchHostOutcome, BatchHostRef, BatchHostResult, BatchOutcome, BatchRequest, HostFailure};
pub use generated::{
    AUTH_METHODS, AuthMethod, AuthMethodSpec, CAPABILITIES, Capability, CapabilityGroup, CapabilitySpec,
    DESCRIPTOR_SCHEMA_VERSION, PROTOCOLS, Protocol, ProtocolSpec, Runtime, SupportLevel,
};
pub use monitor::{
    CpuCounters, CpuModel, CpuStates, DiskUsage, EnvironmentEntry, HostMetrics, LoadAverage, MAX_COMMAND_CHARS,
    MAX_DISKS, MAX_ENVIRONMENT_ENTRIES, MAX_PAGE_PROCESSES, MAX_PROCESSES, MAX_SOCKETS, MemoryUsage, NetworkCounters,
    ProcessDetail, ProcessPage, ProcessSample, SocketRow,
};
pub use risk::{RiskAssessment, RiskLevel, RiskReason};
pub use session::{
    EntryKind, ForwardMode, KeyInfo, KnownHostEntry, LocalForward, OpenOutcome, OpenTunnel, PresentedHostKey,
    SecretInput, Session, SessionAuth, SessionExportSummary, SessionGroup, SessionImportOutcome, SessionImportSummary,
    SftpEntry, SftpListing, SftpOutcome, TerminalSettings, TransferDirection, TransferInfo, TransferState,
    TunnelFailure, UploadAnswer, UploadOutcome, VaultState,
};
pub use settings::{
    AppLayout, AppSettings, BACKGROUND_BLUR_MAX, BACKGROUND_TRANSPARENCY_MAX, BackgroundDisplayMode, BackgroundImage,
    CornerStyle, CustomColors, FilesLayout, HomeLayout, SessionLayout, TERMINAL_FONT_SIZE_MAX, TERMINAL_FONT_SIZE_MIN,
    TabGroupMode, TabLayout, TabMaxWidth, TabPlacement, TabSortMode, ThemeMode, ThemePalette, UI_SCALE_PERCENTS,
    background_blur_default, background_transparency_default, command_completion_on, monitor_on,
    normalize_terminal_font_size, normalize_ui_scale, sidebar_on, terminal_font_size_default, tray_icon_on,
    ui_scale_default,
};
pub use snippet::{
    CommandHistoryEntry, DEFAULT_GROUP_ID, SLOT_HISTORY, SNIPPET_SLOTS, SendFailure, SendOutcome, SendReport,
    SendTarget, Snippet, SnippetGroup,
};
pub use words::CompletionWord;
