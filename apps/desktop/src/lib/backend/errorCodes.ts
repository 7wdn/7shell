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
  // SFTP
  "SFTP-NOT-FOUND": "errors.sftp.notFound",
  "SFTP-PERMISSION-DENIED": "errors.sftp.permissionDenied",
  "SFTP-EXISTS": "errors.sftp.exists",
  "SFTP-ARCHIVE-FAILED": "errors.sftp.archiveFailed",
  "SFTP-TRANSFER-FAILED": "errors.sftp.transferFailed",
  // Local file grants
  "FS-GRANT-UNKNOWN": "errors.fs.grantUnknown",
  "FS-GRANT-EXPIRED": "errors.fs.grantExpired",
  // Persistence
  "STORE-IO": "errors.store.io",
  "STORE-SCHEMA-NEWER": "errors.store.schemaNewer",
  "SESSION-NOT-FOUND": "errors.session.notFound",
  // Policy
  "POLICY-DENIED": "errors.policy.denied",
  "POLICY-APPROVAL-REQUIRED": "errors.policy.approvalRequired",
  // Networking
  "NET-PROXY-FAILED": "errors.net.proxyFailed",
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
