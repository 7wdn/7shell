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

mod generated {
    include!(concat!(env!("OUT_DIR"), "/descriptors.rs"));
}

pub use app_info::{AppInfo, PRODUCT_NAME};
pub use batch::{BatchHostOutcome, BatchHostResult, BatchOutcome, BatchRequest, HostFailure};
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
    EntryKind, KeyInfo, KnownHostEntry, LocalForward, OpenOutcome, OpenTunnel, PresentedHostKey, SecretInput, Session,
    SessionAuth, SessionGroup, SftpEntry, SftpListing, SftpOutcome, TerminalSettings, TransferDirection, TransferInfo,
    TransferState, TunnelFailure, VaultState,
};
pub use settings::{AppSettings, FilesLayout, SessionLayout, ThemeMode, monitor_on};
