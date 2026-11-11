//! The project's error vocabulary.
//!
//! The catalog is declared once, as a macro invocation, so the enum, the complete
//! `ALL` list, the wire code and the localization key are produced from a single
//! line per entry. There is no way to add a code without also naming its message
//! key, and `scripts/check-i18n.mjs` verifies that every key exists in every locale.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! error_catalog {
    ($( $(#[doc = $doc:expr])? $variant:ident => $code:literal, $key:literal; )*) => {
        /// Every error 7Shell can report. Codes are stable and are never reused or renamed.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum ErrorCode {
            $( $(#[doc = $doc])? $variant, )*
        }

        impl ErrorCode {
            /// Every code, in declaration order. Consumers iterate this for parity checks.
            pub const ALL: &'static [ErrorCode] = &[$(ErrorCode::$variant),*];

            /// The stable wire code, for example `SSH-HOSTKEY-MISMATCH`.
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $code,)* }
            }

            /// The message key resolved against the locale files.
            pub const fn message_key(self) -> &'static str {
                match self { $(Self::$variant => $key,)* }
            }
        }
    };
}

error_catalog! {
    // -- Vault -------------------------------------------------------------
    #[doc = "No vault file exists on this device yet."]
    VaultNotFound => "VAULT-NOT-FOUND", "errors.vault.notFound";
    #[doc = "The vault is locked; an unlock is required before anything can be read."]
    VaultLocked => "VAULT-LOCKED", "errors.vault.locked";
    #[doc = "The key derivation could not be completed."]
    VaultKdfFailed => "VAULT-KDF-FAILED", "errors.vault.kdfFailed";
    #[doc = "The vault did not open. Deliberately one code for a wrong password and for a file that has been changed."]
    VaultUnlockFailed => "VAULT-UNLOCK-FAILED", "errors.vault.unlockFailed";
    #[doc = "The vault file exists but cannot be decrypted or parsed."]
    VaultCorrupt => "VAULT-CORRUPT", "errors.vault.corrupt";

    // -- SSH ---------------------------------------------------------------
    #[doc = "The server presented a host key that has never been seen for this host."]
    SshHostKeyUnknown => "SSH-HOSTKEY-UNKNOWN", "errors.ssh.hostKeyUnknown";
    #[doc = "The presented host key differs from the trusted one. Credentials were withheld."]
    SshHostKeyMismatch => "SSH-HOSTKEY-MISMATCH", "errors.ssh.hostKeyMismatch";
    #[doc = "The server rejected the credentials or the authentication method."]
    SshAuthFailed => "SSH-AUTH-FAILED", "errors.ssh.authFailed";
    #[doc = "The host could not be reached."]
    SshUnreachable => "SSH-UNREACHABLE", "errors.ssh.unreachable";
    #[doc = "The connection or a step in it exceeded its time budget."]
    SshTimeout => "SSH-TIMEOUT", "errors.ssh.timeout";
    #[doc = "A channel, pty request or subsystem request failed."]
    SshChannelFailed => "SSH-CHANNEL-FAILED", "errors.ssh.channelFailed";
    #[doc = "The key file could not be read, or its passphrase was wrong."]
    SshKeyInvalid => "SSH-KEY-INVALID", "errors.ssh.keyInvalid";
    #[doc = "The key is of a kind this build cannot use."]
    SshKeyUnsupported => "SSH-KEY-UNSUPPORTED", "errors.ssh.keyUnsupported";

    // -- RDP ---------------------------------------------------------------
    #[doc = "The system Remote Desktop client could not be started."]
    RdpLaunchFailed => "RDP-LAUNCH-FAILED", "errors.rdp.launchFailed";

    // -- SFTP --------------------------------------------------------------
    #[doc = "The remote path does not exist."]
    SftpNotFound => "SFTP-NOT-FOUND", "errors.sftp.notFound";
    #[doc = "The server refused the operation for permission reasons."]
    SftpPermissionDenied => "SFTP-PERMISSION-DENIED", "errors.sftp.permissionDenied";
    #[doc = "Something with that name is already there."]
    SftpExists => "SFTP-EXISTS", "errors.sftp.exists";
    #[doc = "The far side could not pack or unpack an archive."]
    SftpArchiveFailed => "SFTP-ARCHIVE-FAILED", "errors.sftp.archiveFailed";
    #[doc = "A transfer failed part-way through."]
    SftpTransferFailed => "SFTP-TRANSFER-FAILED", "errors.sftp.transferFailed";
    #[doc = "A file operation the far side runs itself (copy, move, chmod) failed."]
    SftpOperationFailed => "SFTP-OPERATION-FAILED", "errors.sftp.operationFailed";

    // -- Local file grants -------------------------------------------------
    #[doc = "The file token is not known to this process."]
    FsGrantUnknown => "FS-GRANT-UNKNOWN", "errors.fs.grantUnknown";
    #[doc = "The file token expired or was revoked when the vault locked."]
    FsGrantExpired => "FS-GRANT-EXPIRED", "errors.fs.grantExpired";
    #[doc = "The system's file manager could not be asked to show a location."]
    FsOpenFailed => "FS-OPEN-FAILED", "errors.fs.openFailed";

    // -- Persistence -------------------------------------------------------
    #[doc = "Reading or writing application state failed."]
    StoreIo => "STORE-IO", "errors.store.io";
    #[doc = "The stored data was written by a newer version of the application."]
    StoreSchemaNewer => "STORE-SCHEMA-NEWER", "errors.store.schemaNewer";
    #[doc = "The picture chosen as the window's background is a kind this build cannot draw."]
    StoreBackgroundUnsupported => "STORE-BACKGROUND-UNSUPPORTED", "errors.store.backgroundUnsupported";
    #[doc = "The picture chosen as the window's background is larger than this build will copy."]
    StoreBackgroundTooLarge => "STORE-BACKGROUND-TOO-LARGE", "errors.store.backgroundTooLarge";
    #[doc = "The session being opened is no longer saved."]
    SessionNotFound => "SESSION-NOT-FOUND", "errors.session.notFound";
    #[doc = "Somebody called off a connection attempt before it finished."]
    SessionOpenCancelled => "SESSION-OPEN-CANCELLED", "errors.session.openCancelled";
    #[doc = "The chosen file is not a session export from this application, or it has been damaged."]
    SessionBundleInvalid => "SESSION-BUNDLE-INVALID", "errors.session.bundleInvalid";
    #[doc = "A session export file could not be read or written."]
    SessionBundleIo => "SESSION-BUNDLE-IO", "errors.session.bundleIo";
    #[doc = "The passphrase did not open an encrypted session export."]
    SessionBundlePassphrase => "SESSION-BUNDLE-PASSPHRASE", "errors.session.bundlePassphrase";
    #[doc = "The passphrase offered for an export is below the length this build writes files with."]
    SessionBundlePassphraseShort => "SESSION-BUNDLE-PASSPHRASE-SHORT", "errors.session.bundlePassphraseShort";

    // -- Sync --------------------------------------------------------------
    #[doc = "The chosen file is not a backup from this application, or it has been damaged."]
    SyncSnapshotInvalid => "SYNC-SNAPSHOT-INVALID", "errors.sync.snapshotInvalid";
    #[doc = "A backup file could not be read or written."]
    SyncIo => "SYNC-IO", "errors.sync.io";
    #[doc = "The passphrase did not open a backup's sealed credentials."]
    SyncPassphrase => "SYNC-PASSPHRASE", "errors.sync.passphrase";
    #[doc = "The passphrase offered for a backup is below the length this build writes files with."]
    SyncPassphraseShort => "SYNC-PASSPHRASE-SHORT", "errors.sync.passphraseShort";
    #[doc = "No sync server has been configured on this device yet."]
    SyncNotConfigured => "SYNC-NOT-CONFIGURED", "errors.sync.notConfigured";
    #[doc = "The sync server could not be reached at all."]
    SyncUnreachable => "SYNC-UNREACHABLE", "errors.sync.unreachable";
    #[doc = "The sync server refused the credentials."]
    SyncAuthFailed => "SYNC-AUTH-FAILED", "errors.sync.authFailed";
    #[doc = "The sync server answered with a status that is not success."]
    SyncHttpFailed => "SYNC-HTTP-FAILED", "errors.sync.httpFailed";

    // -- WebDAV file browsing ----------------------------------------------
    #[doc = "The WebDAV address is not one this build can use."]
    WebDavInvalidAddress => "WEBDAV-INVALID-ADDRESS", "errors.webdav.invalidAddress";
    #[doc = "The WebDAV server could not be reached at all."]
    WebDavUnreachable => "WEBDAV-UNREACHABLE", "errors.webdav.unreachable";
    #[doc = "The WebDAV server refused the credentials."]
    WebDavAuthFailed => "WEBDAV-AUTH-FAILED", "errors.webdav.authFailed";
    #[doc = "The WebDAV server refused the operation for permission reasons."]
    WebDavPermissionDenied => "WEBDAV-PERMISSION-DENIED", "errors.webdav.permissionDenied";
    #[doc = "The remote path does not exist."]
    WebDavNotFound => "WEBDAV-NOT-FOUND", "errors.webdav.notFound";
    #[doc = "Something with that name is already there."]
    WebDavExists => "WEBDAV-EXISTS", "errors.webdav.exists";
    #[doc = "The WebDAV server answered with a status that is not success."]
    WebDavHttpFailed => "WEBDAV-HTTP-FAILED", "errors.webdav.httpFailed";
    #[doc = "A file transfer over WebDAV failed part-way through."]
    WebDavTransferFailed => "WEBDAV-TRANSFER-FAILED", "errors.webdav.transferFailed";
    #[doc = "A listing answer from the WebDAV server could not be read."]
    WebDavMalformed => "WEBDAV-MALFORMED", "errors.webdav.malformed";

    // -- FTP file browsing -------------------------------------------------
    #[doc = "A path the FTP client refused before it became a command."]
    FtpInvalidPath => "FTP-INVALID-PATH", "errors.ftp.invalidPath";
    #[doc = "The FTP server could not be reached."]
    FtpUnreachable => "FTP-UNREACHABLE", "errors.ftp.unreachable";
    #[doc = "The FTP server refused the credentials."]
    FtpAuthFailed => "FTP-AUTH-FAILED", "errors.ftp.authFailed";
    #[doc = "The FTPS connection could not be secured."]
    FtpTlsFailed => "FTP-TLS-FAILED", "errors.ftp.tlsFailed";
    #[doc = "The FTP server refused the operation for permission reasons."]
    FtpPermissionDenied => "FTP-PERMISSION-DENIED", "errors.ftp.permissionDenied";
    #[doc = "Something with that name is already there."]
    FtpExists => "FTP-EXISTS", "errors.ftp.exists";
    #[doc = "An FTP command or transfer failed."]
    FtpOperationFailed => "FTP-OPERATION-FAILED", "errors.ftp.operationFailed";

    // -- SCP fallback ------------------------------------------------------
    #[doc = "The server cannot be browsed this way: no SCP or no shell listing exists."]
    ScpUnavailable => "SCP-UNAVAILABLE", "errors.scp.unavailable";
    #[doc = "A file transfer over SCP failed part-way through."]
    ScpTransferFailed => "SCP-TRANSFER-FAILED", "errors.scp.transferFailed";
    #[doc = "A shell-side file operation failed."]
    ScpOperationFailed => "SCP-OPERATION-FAILED", "errors.scp.operationFailed";
    #[doc = "Something with that name is already there."]
    ScpExists => "SCP-EXISTS", "errors.scp.exists";

    // -- Terminal transports (Telnet, serial, local shells) ----------------
    #[doc = "A Telnet console could not be reached."]
    TerminalUnreachable => "TERMINAL-UNREACHABLE", "errors.terminal.unreachable";
    #[doc = "A serial port could not be opened or stopped answering."]
    TerminalSerialFailed => "TERMINAL-SERIAL-FAILED", "errors.terminal.serialFailed";
    #[doc = "A shell on this machine could not be started."]
    TerminalShellFailed => "TERMINAL-SHELL-FAILED", "errors.terminal.shellFailed";
    #[doc = "A terminal was asked for with settings no transport can attempt."]
    TerminalInvalid => "TERMINAL-INVALID", "errors.terminal.invalid";

    // -- Policy ------------------------------------------------------------
    #[doc = "The active policy forbids this action."]
    PolicyDenied => "POLICY-DENIED", "errors.policy.denied";
    #[doc = "The action requires an explicit approval or an unlocked window."]
    PolicyApprovalRequired => "POLICY-APPROVAL-REQUIRED", "errors.policy.approvalRequired";

    // -- Networking --------------------------------------------------------
    #[doc = "The proxy could not be used to reach the target."]
    NetProxyFailed => "NET-PROXY-FAILED", "errors.net.proxyFailed";

    // -- Updates -----------------------------------------------------------
    #[doc = "The update channel could not be read, so whether a newer release exists is unknown."]
    UpdateCheckFailed => "UPDATE-CHECK-FAILED", "errors.update.checkFailed";
    #[doc = "An installation was asked for while no checked release was waiting."]
    UpdateNotPending => "UPDATE-NOT-AVAILABLE", "errors.update.notAvailable";
    #[doc = "A checked release could not be downloaded or installed."]
    UpdateInstallFailed => "UPDATE-INSTALL-FAILED", "errors.update.installFailed";
    #[doc = "The page a release is published on could not be opened."]
    UpdateReleasePageFailed => "UPDATE-RELEASE-PAGE-FAILED", "errors.update.releasePageFailed";

    // -- Containers --------------------------------------------------------
    #[doc = "The container runtime could not be reached, or its command line is not installed."]
    ContainerUnavailable => "CONTAINER-UNAVAILABLE", "errors.container.unavailable";
    #[doc = "A container runtime command ran but did not succeed."]
    ContainerCommandFailed => "CONTAINER-COMMAND-FAILED", "errors.container.commandFailed";
    #[doc = "The container connection this call names is no longer open."]
    ContainerNotConnected => "CONTAINER-NOT-CONNECTED", "errors.container.notConnected";

    // -- Kubernetes --------------------------------------------------------
    #[doc = "The Kubernetes API server could not be reached."]
    K8sUnavailable => "K8S-UNAVAILABLE", "errors.k8s.unavailable";
    #[doc = "The kubeconfig document could not be read or understood."]
    K8sKubeconfigInvalid => "K8S-KUBECONFIG-INVALID", "errors.k8s.kubeconfigInvalid";
    #[doc = "The API server answered a request with an error status."]
    K8sRequestFailed => "K8S-REQUEST-FAILED", "errors.k8s.requestFailed";
    #[doc = "The cluster connection this call names is no longer open."]
    K8sNotConnected => "K8S-NOT-CONNECTED", "errors.k8s.notConnected";

    // -- Assistant ---------------------------------------------------------
    #[doc = "The assistant has no provider configured yet."]
    AiNoProvider => "AI-NO-PROVIDER", "errors.ai.noProvider";
    #[doc = "No API key is stored for the chosen provider."]
    AiNoKey => "AI-NO-KEY", "errors.ai.noKey";
    #[doc = "The provider could not be reached, refused the request, or answered with an error."]
    AiProviderUnavailable => "AI-PROVIDER-UNAVAILABLE", "errors.ai.providerUnavailable";
    #[doc = "The conversation is larger than the model's context window."]
    AiContextTooLong => "AI-CONTEXT-TOO-LONG", "errors.ai.contextTooLong";
    #[doc = "The assistant stopped after reaching its tool-call limit."]
    AiMaxTurns => "AI-MAX-TURNS", "errors.ai.maxTurns";
    #[doc = "The request was called off before it finished."]
    AiCancelled => "AI-CANCELLED", "errors.ai.cancelled";
    #[doc = "A tool call was skipped because only one may run per turn."]
    AiToolSkipped => "AI-TOOL-SKIPPED", "errors.ai.toolSkipped";
    #[doc = "A stored conversation is larger than this build will keep."]
    AiSessionTooLarge => "AI-SESSION-TOO-LARGE", "errors.ai.sessionTooLarge";
    // Deliberately no catalog entry for a failed summary: when the model cannot summarise, the host
    // falls back to a deterministic truncation and carries on, so there is nothing a person could
    // do with a sentence about it. `AI-SUMMARY-FAILED` is a log and event token only.

    // -- Application -------------------------------------------------------
    #[doc = "A required service is not ready yet, for example the vault during startup."]
    AppNotReady => "APP-NOT-READY", "errors.app.notReady";
    #[doc = "No backend transport is available, for example a browser without the desktop shell."]
    AppNoTransport => "APP-NO-TRANSPORT", "errors.app.noTransport";
    #[doc = "The requested capability is declared but not implemented in this build."]
    AppUnsupported => "APP-UNSUPPORTED", "errors.app.unsupported";
    #[doc = "An unexpected internal failure."]
    AppInternal => "APP-INTERNAL", "errors.app.internal";
}

impl ErrorCode {
    /// Looks a code up by its wire string. Unknown codes are rejected rather than
    /// guessed at, so a newer producer surfaces loudly instead of silently.
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|candidate| candidate.as_str() == code)
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for ErrorCode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::from_code(&raw).ok_or_else(|| serde::de::Error::custom(format!("unknown error code: {raw}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_message_keys_are_unique() {
        let mut codes: Vec<&str> = ErrorCode::ALL.iter().map(|code| code.as_str()).collect();
        let mut keys: Vec<&str> = ErrorCode::ALL.iter().map(|code| code.message_key()).collect();
        let code_count = codes.len();
        let key_count = keys.len();

        codes.sort_unstable();
        codes.dedup();
        keys.sort_unstable();
        keys.dedup();

        assert_eq!(codes.len(), code_count, "two error codes share a wire string");
        assert_eq!(keys.len(), key_count, "two error codes share a message key");
    }

    #[test]
    fn every_code_round_trips_through_its_wire_string() {
        for code in ErrorCode::ALL {
            assert_eq!(
                ErrorCode::from_code(code.as_str()),
                Some(*code),
                "{code} did not round-trip"
            );
        }
    }

    #[test]
    fn message_keys_are_namespaced() {
        for code in ErrorCode::ALL {
            assert!(
                code.message_key().starts_with("errors."),
                "{code} has an unnamespaced key"
            );
        }
    }

    #[test]
    fn codes_are_namespaced_and_upper_case() {
        for code in ErrorCode::ALL {
            let raw = code.as_str();
            assert!(raw.contains('-'), "{raw} has no namespace separator");
            assert_eq!(raw, raw.to_ascii_uppercase(), "{raw} is not upper case");
        }
    }

    #[test]
    fn unknown_codes_are_rejected() {
        let parsed: Result<ErrorCode, _> = serde_json::from_str("\"SSH-NOT-A-REAL-CODE\"");
        assert!(parsed.is_err());
    }
}
