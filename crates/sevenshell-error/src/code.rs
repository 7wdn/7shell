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
