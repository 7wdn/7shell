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

/** A local port answered from the far side of a connection: the `ssh -L` shape. */
export interface LocalForward {
  readonly id: string;
  /** The address to listen on, `127.0.0.1` unless somebody deliberately says otherwise. */
  readonly bindHost: string;
  /** Zero asks the system for a free port, and the open outcome says which one it got. */
  readonly bindPort: number;
  readonly targetHost: string;
  readonly targetPort: number;
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
  /** Saved sessions to reach this one through, in order: the first is connected to directly. */
  readonly jump?: readonly string[];
  /** Ports to forward while this session is open. */
  readonly forwards?: readonly LocalForward[];
  readonly updatedAt: number;
}

/** A forward that is up right now. */
export interface OpenTunnel {
  readonly id: string;
  /** The terminal it belongs to; closing the terminal closes the forward. */
  readonly handle: string;
  readonly sessionId: string;
  readonly bindHost: string;
  readonly bindPort: number;
  readonly targetHost: string;
  readonly targetPort: number;
  readonly connections: number;
}

/** A forward that was asked for and could not be opened. */
export interface TunnelFailure {
  readonly forwardId: string;
  readonly bindHost: string;
  readonly bindPort: number;
  readonly detail: string;
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
  | {
      readonly kind: "opened";
      readonly handle: string;
      readonly sessionId: string;
      /** The forwards this session brought up. */
      readonly tunnels: readonly OpenTunnel[];
      /** The ones that did not come up, each with its reason. */
      readonly tunnelFailures: readonly TunnelFailure[];
    };

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
