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

/**
 * The kinds of key this build can make.
 *
 * Short and lower case — `ed25519`, not `ssh-ed25519` — because these are what the dropdown stores
 * and what the request carries. The *key type* shown beside a key in the list is the OpenSSH name,
 * and it comes back from reading the key itself rather than from this list.
 */
export type KeyAlgorithm = "ed25519" | "ecdsa" | "rsa";

/** The order the dropdown offers them in: the one to pick when in doubt first. */
export const KEY_ALGORITHMS: readonly [KeyAlgorithm, ...KeyAlgorithm[]] = ["ed25519", "ecdsa", "rsa"];

/**
 * A key that was just made, and the half of it that goes on a server.
 *
 * The public line is the one key-shaped thing that crosses this boundary, and it is meant to: a
 * public key is not a secret, it is the thing somebody pastes into `authorized_keys`.
 */
export interface CreatedKey {
  readonly info: KeyInfo;
  readonly publicKey: string;
}

export interface SessionAuth {
  readonly method: AuthMethod;
  readonly key?: KeyInfo;
  readonly hasSecret: boolean;
}

export interface TerminalSettings {
  readonly terminalType: string;
  readonly keepaliveSeconds: number;
  /**
   * Whether the far side may use the key agent on this machine while the session lasts.
   *
   * Off by default, and off is the safe answer: while it is on, anything running on that server can
   * ask this machine's agent to sign. It is a deliberate lending of the keys.
   */
  readonly forwardAgent: boolean;
  /**
   * Whether this session is written down as it happens.
   *
   * A recording is a plain file on disk holding everything the terminal showed, so it is off unless
   * somebody says so.
   */
  readonly record: boolean;
}

/**
 * What kind of forward a saved forward is.
 *
 * `local` is the `ssh -L` shape: one port, one destination. `dynamic` is `ssh -D`: one port that
 * speaks SOCKS5, and whoever connects says where it wants to go.
 */
export type ForwardMode = "local" | "dynamic";

/** A local port answered from the far side of a connection: the `ssh -L` shape. */
export interface LocalForward {
  readonly id: string;
  /** The address to listen on, `127.0.0.1` unless somebody deliberately says otherwise. */
  readonly bindHost: string;
  /** Zero asks the system for a free port, and the open outcome says which one it got. */
  readonly bindPort: number;
  /** Local or dynamic. A document written before dynamic forwarding existed carries no mode. */
  readonly mode: ForwardMode;
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
  /** What the listener is for: a dynamic one draws "SOCKS5" where a local one draws an address. */
  readonly mode: ForwardMode;
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
  /**
   * Whether the group is drawn above the groups it was made beside.
   *
   * Laid over `order` rather than written into it: `order` is the arrangement somebody dragged the
   * list into, and unpinning has to put the group back in it.
   */
  readonly pinned: boolean;
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

/** What an entry in a remote directory is. */
export type EntryKind = "directory" | "file" | "symlink" | "other";

export interface SftpEntry {
  readonly name: string;
  /** The full path, so the interface never joins strings and gets it wrong. */
  readonly path: string;
  readonly kind: EntryKind;
  readonly size: number;
  readonly modifiedAt?: number;
  readonly mode?: number;
  /** The owner's name, or the number the server sent when its accounts do not name it. */
  readonly owner?: string;
  readonly group?: string;
  readonly linkTarget?: string;
}

export interface SftpListing {
  /** The path the server resolved, which is what the breadcrumbs show. */
  readonly path: string;
  readonly entries: readonly SftpEntry[];
}

/** What opening a file browser answered. */
export type SftpOutcome =
  | { readonly kind: "hostKeyUnknown"; readonly key: PresentedHostKey }
  | { readonly kind: "hostKeyMismatch"; readonly key: PresentedHostKey; readonly trusted: KnownHostEntry }
  | { readonly kind: "opened"; readonly handle: string; readonly sessionId: string; readonly listing: SftpListing };

export type TransferDirection = "download" | "upload";

export type TransferState =
  | { readonly kind: "running" }
  | { readonly kind: "done" }
  | { readonly kind: "failed"; readonly detail: string }
  | { readonly kind: "cancelled" };

export interface TransferInfo {
  readonly id: string;
  readonly direction: TransferDirection;
  /** The file's own name, not a path on either side. */
  readonly name: string;
  readonly remotePath: string;
  readonly size: number;
  readonly transferred: number;
  readonly state: TransferState;
  /**
   * Whether the files travelled as one archive, packed on the side they came from.
   *
   * Told apart from the name because the two answer different questions — the name says what moved,
   * this says what the transfer did — and because it decides which directory is read again after an
   * upload: a packed one lands its files *in* the directory it was aimed at.
   */
  readonly packed: boolean;
}

/**
 * What an upload did: sent everything, asked about a name, or nothing at all.
 *
 * An upload is the one transfer that can replace something without being asked to, so a name that
 * is already there comes back as a question instead of as a transfer.
 */
export type UploadOutcome =
  | { readonly kind: "started"; readonly ids: readonly string[] }
  | {
      readonly kind: "conflict";
      /** What stands for this upload while the question is open. */
      readonly token: string;
      /** The names already on the far side, which is what the question is about. */
      readonly names: readonly string[];
      /** Whether "send only what is missing" is a real answer — it is not, for one archive. */
      readonly skippable: boolean;
    }
  | { readonly kind: "cancelled" };

/** The answer to that question. */
export type UploadAnswer = "overwrite" | "skip" | "cancel";

/**
 * What an export of the session library wrote.
 *
 * A count and a file name: the file stays where Rust put it, and the interface is never told where
 * that is.
 */
export interface SessionExportSummary {
  readonly sessions: number;
  readonly groups: number;
  /** How many stored credentials travelled. Zero unless a passphrase sealed them into the file. */
  readonly secrets: number;
  /** Whether a passphrase stands between the file and what is in it. */
  readonly encrypted: boolean;
  readonly fileName: string;
}

/**
 * What an import added.
 *
 * The last two are warnings rather than failures: what they count was imported anyway, and they are
 * the only sign that a session may not connect the way it did where the file came from.
 */
export interface SessionImportSummary {
  readonly sessions: number;
  readonly groups: number;
  readonly secrets: number;
  /** Imported sessions that name a key this device does not hold. */
  readonly missingKeys: number;
  /** Imported sessions that jumped through a session the file did not carry. */
  readonly droppedHops: number;
}

/**
 * What came of asking to import a file.
 *
 * Two answers, and neither is a failure: the library was written, or the file is sealed and the
 * passphrase is the next question. A dismissed file dialog is the absence of this value — nothing
 * happened, so there is nothing to say.
 */
export type SessionImportOutcome =
  | { readonly kind: "imported"; readonly summary: SessionImportSummary }
  | { readonly kind: "needsPassphrase"; readonly fileName: string };
