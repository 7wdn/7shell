import type { RiskAssessment } from "@/types/batch";

/**
 * Hand-written mirror of the Rust wire types for saved commands and for what sending one reported.
 *
 * `packages/app-tests/contracts.test.ts` compares these field names with their Rust counterparts in
 * both directions, so a rename on one side fails the test suite rather than a release.
 *
 * The one shape worth reading twice is `slotHistory`: it is indexed by slot, so index 0 holds the
 * values used before in `#1`. A command and its slots are one thing — `#1` of a restart command has
 * nothing to do with `#1` of a log command — which is why the values are stored per command rather
 * than in one list of "things typed into boxes".
 */

/** One group of saved commands, in the order the interface lists them. */
export interface SnippetGroup {
  readonly id: string;
  /** Empty for the default group: its label is a translation, not a name somebody typed. */
  readonly name: string;
  readonly order: number;
  readonly commands: readonly Snippet[];
}

/** One saved command. */
export interface Snippet {
  readonly id: string;
  readonly groupId: string;
  readonly name: string;
  /**
   * The short text that brings the line up in the command line, or empty for none: what somebody
   * reaches for is usually shorter than either the name or the line itself.
   */
  readonly prefix: string;
  /** The line, with `#1`…`#5` where values go. */
  readonly command: string;
  /** Whether sending it presses Enter afterwards. */
  readonly appendCr: boolean;
  readonly order: number;
  /** Values used in each slot before, newest first: index 0 belongs to `#1`. */
  readonly slotHistory: readonly (readonly string[])[];
}

/** One command that was sent, and when. */
export interface CommandHistoryEntry {
  readonly command: string;
  /** Seconds since the epoch. */
  readonly at: number;
}

/** Where a command is sent: the terminals open right now, by handle. */
export type SendTarget = { readonly kind: "terminals"; readonly handles: readonly string[] };

/** One terminal a command could not be sent to. */
export interface SendFailure {
  readonly handle: string;
  readonly code: string;
  readonly messageKey: string;
  readonly messageParams: Record<string, string>;
  readonly detail?: string;
}

/** What sending did, per terminal. */
export interface SendReport {
  readonly sent: readonly string[];
  readonly failures: readonly SendFailure[];
}

/** What sending answered. */
export type SendOutcome =
  | { readonly kind: "approvalRequired"; readonly risk: RiskAssessment }
  | { readonly kind: "sent"; readonly report: SendReport };
