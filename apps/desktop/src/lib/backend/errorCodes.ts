/**
 * Mirror of the Rust error catalog in `crates/sevenshell-error/src/code.rs`.
 *
 * This is the one place where the interface repeats knowledge that lives in Rust, so
 * `packages/app-tests/contracts.test.ts` compares both directions — a code or a message
 * key added on one side and not the other fails the test suite.
 */
export const ERROR_CATALOG = {
  // Vault
  "VAULT-NOT-FOUND": "errors.vault.notFound",
  "VAULT-LOCKED": "errors.vault.locked",
  "VAULT-KDF-FAILED": "errors.vault.kdfFailed",
  "VAULT-CORRUPT": "errors.vault.corrupt",
  "VAULT-UNLOCK-FAILED": "errors.vault.unlockFailed",
  // SSH
  "SSH-HOSTKEY-UNKNOWN": "errors.ssh.hostKeyUnknown",
  "SSH-HOSTKEY-MISMATCH": "errors.ssh.hostKeyMismatch",
  "SSH-AUTH-FAILED": "errors.ssh.authFailed",
  "SSH-UNREACHABLE": "errors.ssh.unreachable",
  "SSH-TIMEOUT": "errors.ssh.timeout",
  "SSH-CHANNEL-FAILED": "errors.ssh.channelFailed",
  "SSH-KEY-INVALID": "errors.ssh.keyInvalid",
  "SSH-KEY-UNSUPPORTED": "errors.ssh.keyUnsupported",
  // RDP
  "RDP-LAUNCH-FAILED": "errors.rdp.launchFailed",
  // Terminal transports that are not SSH
  "TERMINAL-UNREACHABLE": "errors.terminal.unreachable",
  "TERMINAL-SERIAL-FAILED": "errors.terminal.serialFailed",
  "TERMINAL-SHELL-FAILED": "errors.terminal.shellFailed",
  "TERMINAL-INVALID": "errors.terminal.invalid",
  // SFTP
  "SFTP-NOT-FOUND": "errors.sftp.notFound",
  "SFTP-PERMISSION-DENIED": "errors.sftp.permissionDenied",
  "SFTP-EXISTS": "errors.sftp.exists",
  "SFTP-ARCHIVE-FAILED": "errors.sftp.archiveFailed",
  "SFTP-TRANSFER-FAILED": "errors.sftp.transferFailed",
  "SFTP-OPERATION-FAILED": "errors.sftp.operationFailed",
  // Local file grants
  "FS-GRANT-UNKNOWN": "errors.fs.grantUnknown",
  "FS-GRANT-EXPIRED": "errors.fs.grantExpired",
  "FS-OPEN-FAILED": "errors.fs.openFailed",
  // Persistence
  "STORE-IO": "errors.store.io",
  "STORE-SCHEMA-NEWER": "errors.store.schemaNewer",
  "STORE-BACKGROUND-UNSUPPORTED": "errors.store.backgroundUnsupported",
  "STORE-BACKGROUND-TOO-LARGE": "errors.store.backgroundTooLarge",
  "SESSION-NOT-FOUND": "errors.session.notFound",
  "SESSION-OPEN-CANCELLED": "errors.session.openCancelled",
  "SESSION-BUNDLE-INVALID": "errors.session.bundleInvalid",
  "SESSION-BUNDLE-IO": "errors.session.bundleIo",
  "SESSION-BUNDLE-PASSPHRASE": "errors.session.bundlePassphrase",
  "SESSION-BUNDLE-PASSPHRASE-SHORT": "errors.session.bundlePassphraseShort",
  // Sync
  "SYNC-SNAPSHOT-INVALID": "errors.sync.snapshotInvalid",
  "SYNC-IO": "errors.sync.io",
  "SYNC-PASSPHRASE": "errors.sync.passphrase",
  "SYNC-PASSPHRASE-SHORT": "errors.sync.passphraseShort",
  "SYNC-NOT-CONFIGURED": "errors.sync.notConfigured",
  "SYNC-UNREACHABLE": "errors.sync.unreachable",
  "SYNC-AUTH-FAILED": "errors.sync.authFailed",
  "SYNC-HTTP-FAILED": "errors.sync.httpFailed",
  // WebDAV file browsing
  "WEBDAV-INVALID-ADDRESS": "errors.webdav.invalidAddress",
  "WEBDAV-UNREACHABLE": "errors.webdav.unreachable",
  "WEBDAV-AUTH-FAILED": "errors.webdav.authFailed",
  "WEBDAV-PERMISSION-DENIED": "errors.webdav.permissionDenied",
  "WEBDAV-NOT-FOUND": "errors.webdav.notFound",
  "WEBDAV-EXISTS": "errors.webdav.exists",
  "WEBDAV-HTTP-FAILED": "errors.webdav.httpFailed",
  "WEBDAV-TRANSFER-FAILED": "errors.webdav.transferFailed",
  "WEBDAV-MALFORMED": "errors.webdav.malformed",
  // FTP / FTPS
  "FTP-INVALID-PATH": "errors.ftp.invalidPath",
  "FTP-UNREACHABLE": "errors.ftp.unreachable",
  "FTP-AUTH-FAILED": "errors.ftp.authFailed",
  "FTP-TLS-FAILED": "errors.ftp.tlsFailed",
  "FTP-PERMISSION-DENIED": "errors.ftp.permissionDenied",
  "FTP-EXISTS": "errors.ftp.exists",
  "FTP-OPERATION-FAILED": "errors.ftp.operationFailed",
  // SCP fallback
  "SCP-UNAVAILABLE": "errors.scp.unavailable",
  "SCP-TRANSFER-FAILED": "errors.scp.transferFailed",
  "SCP-OPERATION-FAILED": "errors.scp.operationFailed",
  "SCP-EXISTS": "errors.scp.exists",
  // Policy
  "POLICY-DENIED": "errors.policy.denied",
  "POLICY-APPROVAL-REQUIRED": "errors.policy.approvalRequired",
  // Networking
  "NET-PROXY-FAILED": "errors.net.proxyFailed",
  // Updates
  "UPDATE-CHECK-FAILED": "errors.update.checkFailed",
  "UPDATE-NOT-AVAILABLE": "errors.update.notAvailable",
  "UPDATE-INSTALL-FAILED": "errors.update.installFailed",
  "UPDATE-RELEASE-PAGE-FAILED": "errors.update.releasePageFailed",
  // Application
  "APP-NOT-READY": "errors.app.notReady",
  "APP-NO-TRANSPORT": "errors.app.noTransport",
  "APP-UNSUPPORTED": "errors.app.unsupported",
  "APP-INTERNAL": "errors.app.internal",
} as const satisfies Record<string, string>;

export type ErrorCode = keyof typeof ERROR_CATALOG;

/** Raised by the interface itself when there is no backend transport to talk to. */
export const APP_NO_TRANSPORT: ErrorCode = "APP-NO-TRANSPORT";

/** Raised when a failure arrives that is not an envelope, for example a raw string. */
export const APP_INTERNAL: ErrorCode = "APP-INTERNAL";

export function messageKeyFor(code: ErrorCode): string {
  return ERROR_CATALOG[code];
}
