/**
 * What the interface decides about a backup, and what it is told afterwards.
 *
 * Counts, a file name, and the switches. Nothing here carries a secret: the logins are read out of
 * the vault, sealed into the file and read back by the shell, so what crosses this boundary is how
 * much travelled and never what it was.
 */

/** What a snapshot is asked to carry. */
export interface SyncSelection {
  /**
   * Session ids, or absent for all of them.
   *
   * The four lists exist so that a file written by something that narrows one day can be read; this
   * build sends none and applies whatever a file holds.
   */
  readonly sessions?: readonly string[];
  readonly groups?: readonly string[];
  readonly snippets?: readonly string[];
  readonly words?: readonly string[];
  /** Whether the stored logins and key material travel, sealed under a passphrase. */
  readonly credentials: boolean;
  /**
   * Whether the commands that were sent travel.
   *
   * Off unless somebody asks: a history is a record of what they typed, host names and paths in it.
   */
  readonly history: boolean;
  /**
   * Whether the trusted host keys travel.
   *
   * Off unless somebody asks: carrying them means a machine accepting keys it never verified.
   */
  readonly knownHosts: boolean;
  readonly settings: boolean;
}

/** What a snapshot holds, as counts. */
export interface SyncContents {
  readonly sessions: number;
  readonly groups: number;
  readonly snippets: number;
  readonly words: number;
  /** Zero when the snapshot does not carry a history at all. */
  readonly history: number;
  readonly knownHosts: number;
  readonly settings: number;
  /** Always zero in the plain form: a file that is not sealed carries no logins. */
  readonly secrets: number;
  readonly keys: number;
}

/** What a file holds, for the dialog that asks what to do with it. */
export interface SyncCatalog {
  readonly contents: SyncContents;
  readonly sealed: boolean;
  readonly exportedAt: number;
  readonly appVersion: string;
}

/** What writing a backup did. */
export interface SyncReceipt {
  readonly contents: SyncContents;
  readonly sealed: boolean;
  /** The file's own name, never a path. */
  readonly fileName: string;
}

/**
 * What restoring a backup did.
 *
 * `replaced` and `copied` are the same event under the two meanings it can have: an arrival whose id
 * was already here — written over when replacement was asked for, and a second copy when it was not.
 */
export interface SyncApplySummary {
  readonly contents: SyncContents;
  readonly replaced: number;
  readonly copied: number;
  /** Restored sessions that name a key this device does not hold. */
  readonly missingKeys: number;
  /** Restored sessions that lost a jump host the file did not carry. */
  readonly droppedHops: number;
}

/** What came of asking to read a backup file. */
export type SyncInspectOutcome =
  | { readonly kind: "read"; readonly catalog: SyncCatalog; readonly fileName: string }
  | { readonly kind: "needsPassphrase"; readonly fileName: string };

/**
 * What this installation has configured for a sync server.
 *
 * Two answers about what is kept and nothing about what it is: the address and the user name come
 * from the settings this side already holds, and neither the password nor the passphrase is ever
 * sent anywhere but a header and a file.
 */
export interface SyncServiceStatus {
  readonly hasPassword: boolean;
  readonly hasPassphrase: boolean;
}

/** What an upload to a server did. */
export interface SyncUploadReceipt {
  readonly contents: SyncContents;
  readonly sealed: boolean;
  /** Where inside the endpoint the file went. Configuration rather than a secret. */
  readonly remotePath: string;
}

/**
 * What comes of asking a server for the file it holds.
 *
 * Three answers rather than two: a server with nothing there yet is not a failure and not a
 * question — it is the ordinary state of a sync that has never run.
 */
export type SyncDownloadOutcome =
  | { readonly kind: "nothingYet" }
  | { readonly kind: "read"; readonly catalog: SyncCatalog }
  | { readonly kind: "needsPassphrase" };
