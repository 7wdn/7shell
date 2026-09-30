/**
 * Hand-written mirror of the Rust wire types for sessions, keys and host keys.
 *
 * `packages/app-tests/contracts.test.ts` compares these field names with their Rust counterparts
 * in both directions, so a rename on one side fails the test suite rather than a release. Note
 * what is *not* here: no password, no passphrase, no key material. The interface is never sent one.
 */

import type { AuthMethod, Protocol } from "@/types/generated/protocols";

/** What the interface may know about an imported key. */
export interface KeyInfo {
  /** The handle the vault knows the material by. */
  readonly keyRef?: string;
  readonly displayName: string;
  readonly algorithm: string;
  readonly fingerprint: string;
  readonly hasPassphrase: boolean;
}

export interface SessionAuth {
  readonly method: AuthMethod;
  readonly key?: KeyInfo;
  readonly hasSecret: boolean;
}

export interface TerminalSettings {
  readonly terminalType: string;
  readonly keepaliveSeconds: number;
}

/** A saved way to reach a machine. */
export interface Session {
  readonly id: string;
  readonly name: string;
  readonly groupId?: string;
  readonly protocol: Protocol;
  readonly host: string;
  readonly port: number;
  readonly user: string;
  readonly auth: SessionAuth;
  readonly terminal: TerminalSettings;
  readonly updatedAt: number;
}

export interface SessionGroup {
  readonly id: string;
  readonly name: string;
  readonly parentId?: string;
  readonly order: number;
}

export interface KnownHostEntry {
  readonly host: string;
  readonly port: number;
  readonly keyType: string;
  readonly publicKey: string;
  readonly fingerprint: string;
  readonly addedAt: number;
}

/** A host key as presented during a handshake, before anyone has decided to trust it. */
export interface PresentedHostKey {
  readonly host: string;
  readonly port: number;
  readonly keyType: string;
  readonly publicKey: string;
  readonly fingerprint: string;
}

/** What opening a session answered. */
export type OpenOutcome =
  | { readonly kind: "hostKeyUnknown"; readonly key: PresentedHostKey }
  | { readonly kind: "hostKeyMismatch"; readonly key: PresentedHostKey; readonly trusted: KnownHostEntry }
  | { readonly kind: "opened"; readonly handle: string; readonly sessionId: string };

export type VaultState = "missing" | "locked" | "unlocked";

/** A secret travelling inwards. There is no type for one travelling out. */
export interface SecretInput {
  readonly value: string;
}

/** A terminal that is open. */
export interface OpenTerminal {
  readonly handle: string;
  readonly sessionId: string;
}

/** Output from a terminal: bytes, base64-encoded, because a terminal stream is not always text. */
export interface TerminalData {
  readonly handle: string;
  readonly base64: string;
}

export interface TerminalClosed {
  readonly handle: string;
  readonly reason?: string;
}
