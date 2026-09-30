//! The error contract that crosses every boundary in 7Shell.
//!
//! One structure travels over Tauri IPC, out of the CLI and (later) through the
//! agent interface. Fields are private and can only be filled by the named
//! constructors below, so a `code` and the `message_key` used to localize it can
//! never drift apart.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::code::ErrorCode;
use crate::redact::{self, MASK, MAX_DETAIL_CHARS};

/// Bumped only when the envelope layout changes in a way consumers must adapt to.
pub const ENVELOPE_VERSION: u8 = 1;

/// Where a failure came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Subsystem {
    Vault,
    Ssh,
    Sftp,
    Fs,
    Store,
    Policy,
    Net,
    App,
    Plugin,
}

impl Subsystem {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Vault => "vault",
            Self::Ssh => "ssh",
            Self::Sftp => "sftp",
            Self::Fs => "fs",
            Self::Store => "store",
            Self::Policy => "policy",
            Self::Net => "net",
            Self::App => "app",
            Self::Plugin => "plugin",
        }
    }
}

impl fmt::Display for Subsystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What happened to the sensitive material involved in a failure.
///
/// This answers the question a user actually asks first: "did my password or
/// command already leave this machine?". Callers must set it deliberately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationOutcome {
    /// The subsystem could not tell.
    Unknown,
    /// Nothing sensitive was transmitted.
    NotSent,
    /// Material was transmitted (for example credentials that the server rejected).
    Sent,
    /// Material was withheld because a precondition failed.
    Rejected,
}

impl OperationOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::NotSent => "notSent",
            Self::Sent => "sent",
            Self::Rejected => "rejected",
        }
    }
}

/// Identifies the failing subsystem, and optionally the adapter and the peer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Origin {
    subsystem: Subsystem,
    // Optional fields are `default` as well as skipped when empty: a producer that
    // omits them must still be understood, not rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    adapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    peer: Option<String>,
}

impl Origin {
    pub fn new(subsystem: Subsystem) -> Self {
        Self {
            subsystem,
            adapter: None,
            peer: None,
        }
    }

    /// The transport or library a failure came from, for example `russh`.
    pub fn with_adapter(mut self, adapter: impl AsRef<str>) -> Self {
        self.adapter = Some(redact::scrub_bounded(adapter.as_ref(), MAX_DETAIL_CHARS));
        self
    }

    /// The remote endpoint, for example `db.example.com:22`.
    pub fn with_peer(mut self, peer: impl AsRef<str>) -> Self {
        self.peer = Some(redact::scrub_bounded(peer.as_ref(), MAX_DETAIL_CHARS));
        self
    }

    pub fn subsystem(&self) -> Subsystem {
        self.subsystem
    }

    pub fn adapter(&self) -> Option<&str> {
        self.adapter.as_deref()
    }

    pub fn peer(&self) -> Option<&str> {
        self.peer.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShellErrorFields {
    version: u8,
    code: ErrorCode,
    message_key: String,
    #[serde(default)]
    message_params: BTreeMap<String, String>,
    origin: Origin,
    outcome: OperationOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    diagnostics: BTreeMap<String, String>,
}

/// The single error type that crosses a process or transport boundary.
///
/// The fields sit behind a `Box` deliberately: every command returns
/// `Result<T, ShellError>`, and a 160-byte error variant would be copied on every
/// *successful* call for no reason. This is a newtype, so the JSON is still one flat
/// object — `tests/wire_format.rs` proves it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellError(Box<ShellErrorFields>);

impl ShellError {
    pub(crate) fn new(code: ErrorCode, origin: Origin) -> Self {
        Self(Box::new(ShellErrorFields {
            version: ENVELOPE_VERSION,
            code,
            message_key: code.message_key().to_owned(),
            message_params: BTreeMap::new(),
            origin,
            outcome: OperationOutcome::Unknown,
            detail: None,
            diagnostics: BTreeMap::new(),
        }))
    }

    /// A parameter substituted into the localized message. Values for keys that
    /// look like secrets are replaced with a mask.
    pub fn with_param(mut self, key: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        let key = key.as_ref();
        let value = if redact::is_secret_key(key) {
            MASK.to_owned()
        } else {
            redact::scrub_bounded(value.as_ref(), MAX_DETAIL_CHARS)
        };
        self.0.message_params.insert(key.to_owned(), value);
        self
    }

    /// Free-form human-readable context. Scrubbed and truncated.
    pub fn with_detail(mut self, detail: impl AsRef<str>) -> Self {
        self.0.detail = Some(redact::scrub_bounded(detail.as_ref(), MAX_DETAIL_CHARS));
        self
    }

    /// A named, allow-listed extra such as a key fingerprint or an exit code.
    /// Values for keys that look like secrets are replaced with a mask.
    pub fn with_diagnostic(mut self, key: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        let key = key.as_ref();
        let value = if redact::is_secret_key(key) {
            MASK.to_owned()
        } else {
            redact::scrub_bounded(value.as_ref(), MAX_DETAIL_CHARS)
        };
        self.0.diagnostics.insert(key.to_owned(), value);
        self
    }

    pub fn with_outcome(mut self, outcome: OperationOutcome) -> Self {
        self.0.outcome = outcome;
        self
    }

    pub fn version(&self) -> u8 {
        self.0.version
    }

    pub fn code(&self) -> ErrorCode {
        self.0.code
    }

    pub fn message_key(&self) -> &str {
        &self.0.message_key
    }

    pub fn message_params(&self) -> &BTreeMap<String, String> {
        &self.0.message_params
    }

    pub fn origin(&self) -> &Origin {
        &self.0.origin
    }

    pub fn outcome(&self) -> OperationOutcome {
        self.0.outcome
    }

    pub fn detail(&self) -> Option<&str> {
        self.0.detail.as_deref()
    }

    pub fn diagnostics(&self) -> &BTreeMap<String, String> {
        &self.0.diagnostics
    }

    // -- Vault -------------------------------------------------------------

    pub fn vault_not_found() -> Self {
        Self::new(ErrorCode::VaultNotFound, Origin::new(Subsystem::Vault)).with_outcome(OperationOutcome::Rejected)
    }

    pub fn vault_locked() -> Self {
        Self::new(ErrorCode::VaultLocked, Origin::new(Subsystem::Vault)).with_outcome(OperationOutcome::Rejected)
    }

    pub fn vault_kdf_failed() -> Self {
        Self::new(ErrorCode::VaultKdfFailed, Origin::new(Subsystem::Vault)).with_outcome(OperationOutcome::Rejected)
    }

    /// The vault did not open: the password is wrong, or the file is not the file it was.
    ///
    /// One code and one message for both, on purpose: telling them apart would turn this into an
    /// oracle for anyone who has a copy of the file.
    pub fn vault_unlock_failed() -> Self {
        Self::new(ErrorCode::VaultUnlockFailed, Origin::new(Subsystem::Vault)).with_outcome(OperationOutcome::Rejected)
    }

    pub fn vault_corrupt(detail: impl AsRef<str>) -> Self {
        Self::new(ErrorCode::VaultCorrupt, Origin::new(Subsystem::Vault)).with_detail(detail)
    }

    // -- SSH ---------------------------------------------------------------

    pub fn ssh_host_key_unknown(host: &str, port: u16, key_type: &str) -> Self {
        Self::new(ErrorCode::SshHostKeyUnknown, ssh_origin(host, port))
            .with_param("host", host)
            .with_param("port", port.to_string())
            .with_diagnostic("keyType", key_type)
            .with_outcome(OperationOutcome::NotSent)
    }

    pub fn ssh_host_key_mismatch(host: &str, port: u16, key_type: &str) -> Self {
        Self::new(ErrorCode::SshHostKeyMismatch, ssh_origin(host, port))
            .with_param("host", host)
            .with_param("port", port.to_string())
            .with_diagnostic("keyType", key_type)
            .with_outcome(OperationOutcome::NotSent)
    }

    pub fn ssh_auth_failed(host: &str, port: u16, method: &str) -> Self {
        Self::new(ErrorCode::SshAuthFailed, ssh_origin(host, port))
            .with_param("host", host)
            .with_param("port", port.to_string())
            .with_diagnostic("method", method)
            .with_outcome(OperationOutcome::Sent)
    }

    pub fn ssh_unreachable(host: &str, port: u16, detail: impl AsRef<str>) -> Self {
        Self::new(ErrorCode::SshUnreachable, ssh_origin(host, port))
            .with_param("host", host)
            .with_param("port", port.to_string())
            .with_detail(detail)
            .with_outcome(OperationOutcome::NotSent)
    }

    pub fn ssh_timeout(host: &str, port: u16, seconds: u64) -> Self {
        Self::new(ErrorCode::SshTimeout, ssh_origin(host, port))
            .with_param("host", host)
            .with_param("port", port.to_string())
            .with_param("seconds", seconds.to_string())
            .with_outcome(OperationOutcome::Unknown)
    }

    pub fn ssh_channel_failed(host: &str, port: u16, detail: impl AsRef<str>) -> Self {
        Self::new(ErrorCode::SshChannelFailed, ssh_origin(host, port))
            .with_param("host", host)
            .with_param("port", port.to_string())
            .with_detail(detail)
    }

    pub fn ssh_key_invalid(detail: impl AsRef<str>) -> Self {
        Self::new(ErrorCode::SshKeyInvalid, Origin::new(Subsystem::Ssh))
            .with_detail(detail)
            .with_outcome(OperationOutcome::NotSent)
    }

    pub fn ssh_key_unsupported(key_type: &str) -> Self {
        Self::new(ErrorCode::SshKeyUnsupported, Origin::new(Subsystem::Ssh))
            .with_diagnostic("keyType", key_type)
            .with_outcome(OperationOutcome::NotSent)
    }

    // -- SFTP --------------------------------------------------------------

    pub fn sftp_not_found(remote_path: &str) -> Self {
        Self::new(ErrorCode::SftpNotFound, Origin::new(Subsystem::Sftp))
            .with_param("path", remote_path)
            .with_outcome(OperationOutcome::NotSent)
    }

    pub fn sftp_permission_denied(remote_path: &str) -> Self {
        Self::new(ErrorCode::SftpPermissionDenied, Origin::new(Subsystem::Sftp))
            .with_param("path", remote_path)
            .with_outcome(OperationOutcome::NotSent)
    }

    pub fn sftp_transfer_failed(remote_path: &str, detail: impl AsRef<str>) -> Self {
        Self::new(ErrorCode::SftpTransferFailed, Origin::new(Subsystem::Sftp))
            .with_param("path", remote_path)
            .with_detail(detail)
    }

    // -- Local file grants -------------------------------------------------

    pub fn fs_grant_unknown() -> Self {
        Self::new(ErrorCode::FsGrantUnknown, Origin::new(Subsystem::Fs)).with_outcome(OperationOutcome::Rejected)
    }

    pub fn fs_grant_expired() -> Self {
        Self::new(ErrorCode::FsGrantExpired, Origin::new(Subsystem::Fs)).with_outcome(OperationOutcome::Rejected)
    }

    // -- Persistence -------------------------------------------------------

    pub fn session_not_found(id: &str) -> Self {
        Self::new(ErrorCode::SessionNotFound, Origin::new(Subsystem::Store))
            .with_diagnostic("sessionId", id)
            .with_outcome(OperationOutcome::NotSent)
    }

    pub fn store_io(detail: impl AsRef<str>) -> Self {
        Self::new(ErrorCode::StoreIo, Origin::new(Subsystem::Store)).with_detail(detail)
    }

    pub fn store_schema_newer(found: u32, supported: u32) -> Self {
        Self::new(ErrorCode::StoreSchemaNewer, Origin::new(Subsystem::Store))
            .with_param("found", found.to_string())
            .with_param("supported", supported.to_string())
            .with_outcome(OperationOutcome::Rejected)
    }

    // -- Policy ------------------------------------------------------------

    pub fn policy_denied(action: &str) -> Self {
        Self::new(ErrorCode::PolicyDenied, Origin::new(Subsystem::Policy))
            .with_param("action", action)
            .with_outcome(OperationOutcome::Rejected)
    }

    pub fn policy_approval_required(action: &str) -> Self {
        Self::new(ErrorCode::PolicyApprovalRequired, Origin::new(Subsystem::Policy))
            .with_param("action", action)
            .with_outcome(OperationOutcome::Rejected)
    }

    // -- Networking --------------------------------------------------------

    pub fn net_proxy_failed(detail: impl AsRef<str>) -> Self {
        Self::new(ErrorCode::NetProxyFailed, Origin::new(Subsystem::Net)).with_detail(detail)
    }

    // -- Application -------------------------------------------------------

    pub fn app_not_ready() -> Self {
        Self::new(ErrorCode::AppNotReady, Origin::new(Subsystem::App)).with_outcome(OperationOutcome::Rejected)
    }

    /// Raised by the client when no backend transport is available, for example when
    /// the interface is opened in a plain browser instead of the desktop shell.
    pub fn app_no_transport(operation: &str) -> Self {
        Self::new(ErrorCode::AppNoTransport, Origin::new(Subsystem::App))
            .with_param("operation", operation)
            .with_outcome(OperationOutcome::Rejected)
    }

    /// Something the descriptors declare but this build has not implemented. Saying so beats
    /// failing mysteriously, and the message names what is missing.
    pub fn app_unsupported(what: &str) -> Self {
        Self::new(ErrorCode::AppUnsupported, Origin::new(Subsystem::App))
            .with_param("what", what)
            .with_outcome(OperationOutcome::NotSent)
    }

    pub fn app_internal(detail: impl AsRef<str>) -> Self {
        Self::new(ErrorCode::AppInternal, Origin::new(Subsystem::App)).with_detail(detail)
    }
}

fn ssh_origin(host: &str, port: u16) -> Origin {
    Origin::new(Subsystem::Ssh)
        .with_adapter("russh")
        .with_peer(format!("{host}:{port}"))
}

impl fmt::Display for ShellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.0.code, self.0.message_key)?;
        if let Some(detail) = &self.0.detail {
            write!(f, ": {detail}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ShellError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_carries_version_code_and_key() {
        let error = ShellError::vault_locked();
        assert_eq!(error.version(), ENVELOPE_VERSION);
        assert_eq!(error.code(), ErrorCode::VaultLocked);
        assert_eq!(error.message_key(), ErrorCode::VaultLocked.message_key());
        assert_eq!(error.outcome(), OperationOutcome::Rejected);
    }

    #[test]
    fn host_key_failures_never_claim_material_was_sent() {
        let mismatch = ShellError::ssh_host_key_mismatch("host.example", 22, "ssh-ed25519");
        assert_eq!(mismatch.outcome(), OperationOutcome::NotSent);

        let unknown = ShellError::ssh_host_key_unknown("host.example", 22, "ssh-ed25519");
        assert_eq!(unknown.outcome(), OperationOutcome::NotSent);

        let rejected = ShellError::ssh_auth_failed("host.example", 22, "password");
        assert_eq!(rejected.outcome(), OperationOutcome::Sent);
    }

    #[test]
    fn origin_records_the_peer_and_adapter() {
        let error = ShellError::ssh_timeout("10.0.0.7", 2222, 30);
        assert_eq!(error.origin().subsystem(), Subsystem::Ssh);
        assert_eq!(error.origin().peer(), Some("10.0.0.7:2222"));
        assert_eq!(error.origin().adapter(), Some("russh"));
    }

    #[test]
    fn secret_looking_parameters_are_masked() {
        let error = ShellError::app_internal("boom")
            .with_param("password", "hunter2")
            .with_diagnostic("privateKey", "-----BEGIN");

        assert_eq!(error.message_params().get("password").map(String::as_str), Some(MASK));
        assert_eq!(error.diagnostics().get("privateKey").map(String::as_str), Some(MASK));
    }

    #[test]
    fn detail_is_scrubbed_before_it_is_stored() {
        let error = ShellError::store_io("failed with password=hunter2 while opening");
        let detail = error.detail().unwrap_or_default();
        assert!(!detail.contains("hunter2"), "detail leaked a secret: {detail}");
        assert!(detail.contains(MASK));
    }

    #[test]
    fn display_never_reveals_a_secret() {
        let error = ShellError::sftp_transfer_failed("/remote/file", "auth token=abcdef123 rejected");
        let rendered = error.to_string();
        assert!(rendered.contains("SFTP-TRANSFER-FAILED"));
        assert!(!rendered.contains("abcdef123"));
    }
}
