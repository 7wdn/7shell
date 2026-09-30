//! Wire types for 7Shell.
//!
//! Two halves that have to agree: the hand-written DTOs the interface consumes, and
//! the descriptor tables generated from `descriptors/`. The generated module is
//! included from `OUT_DIR`, so it always reflects the committed manifest.

mod app_info;
mod descriptors;
mod session;
mod settings;

mod generated {
    include!(concat!(env!("OUT_DIR"), "/descriptors.rs"));
}

pub use app_info::{AppInfo, PRODUCT_NAME};
pub use generated::{
    AUTH_METHODS, AuthMethod, AuthMethodSpec, CAPABILITIES, Capability, CapabilityGroup, CapabilitySpec,
    DESCRIPTOR_SCHEMA_VERSION, PROTOCOLS, Protocol, ProtocolSpec, Runtime, SupportLevel,
};
pub use session::{
    KeyInfo, KnownHostEntry, LocalForward, OpenOutcome, OpenTunnel, PresentedHostKey, SecretInput, Session,
    SessionAuth, SessionGroup, TerminalSettings, TunnelFailure, VaultState,
};
pub use settings::{AppSettings, ThemeMode};
