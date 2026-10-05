/**
 * Hand-written mirror of the Rust wire types for batches and for the risk verdict.
 *
 * `packages/app-tests/contracts.test.ts` compares these field names with their Rust counterparts
 * in both directions, so a rename on one side fails the test suite rather than a release.
 *
 * The shape worth noticing is the one an outcome has: a batch answers *per host*, so a failure is
 * data inside a result rather than an error that replaces it. The one exception is a host key
 * decision, which is about one host but stops the whole run — nothing has executed anywhere when it
 * comes back — and carries which host raised it so the prompt can say so.
 */

import type { KnownHostEntry, PresentedHostKey } from "@/types/session";

/** How much damage a command could do, worst last. */
export type RiskLevel = "safe" | "caution" | "destructive" | "catastrophic";

/** One rule that fired, and the message key that explains it. */
export interface RiskReason {
  /** Stable identifier, so a person can quote the rule they disagree with. */
  readonly ruleId: string;
  /** A key into the translations under `risk.*`. */
  readonly labelKey: string;
  readonly level: RiskLevel;
}

/** What the classifier concluded about a command line. */
export interface RiskAssessment {
  /** The worst level any fired rule reached. */
  readonly level: RiskLevel;
  readonly reasons: readonly RiskReason[];
}

/** What to run, and where. */
export interface BatchRequest {
  /** The saved sessions to run on, in the order the results come back. */
  readonly sessionIds: readonly string[];
  readonly command: string;
  readonly timeoutMs: number;
  readonly parallel: number;
  /**
   * Set once somebody has seen what the command would do. Without it, a command whose level needs
   * approval comes back with its verdict instead of running; the flag records that the reasons
   * were shown, not that the command became safe.
   */
  readonly acknowledged: boolean;
}

/** Which host in the request something is about. */
export interface BatchHostRef {
  readonly sessionId: string;
  readonly sessionName: string;
}

/**
 * Why one host has nothing to show.
 *
 * The parts of the error envelope a person reads, carried across deliberately: the code, the
 * message key and its parameters, and the scrubbed detail.
 */
export interface HostFailure {
  readonly code: string;
  readonly messageKey: string;
  readonly messageParams: Record<string, string>;
  readonly detail?: string;
}

/** One host's answer to the command. */
export type BatchHostOutcome =
  | {
      readonly kind: "ran";
      /** The status the server reported; null when it reported none. */
      readonly exitCode: number | null;
      readonly stdout: string;
      readonly stderr: string;
    }
  | { readonly kind: "failed"; readonly error: HostFailure };

/** What one host has to say. */
export interface BatchHostResult {
  readonly sessionId: string;
  readonly sessionName: string;
  readonly outcome: BatchHostOutcome;
  readonly durationMs: number;
}

/** What a batch did, or what it is waiting for. */
export type BatchOutcome =
  | { readonly kind: "hostKeyUnknown"; readonly host: BatchHostRef; readonly key: PresentedHostKey }
  | {
      readonly kind: "hostKeyMismatch";
      readonly host: BatchHostRef;
      readonly key: PresentedHostKey;
      readonly trusted: KnownHostEntry;
    }
  | { readonly kind: "approvalRequired"; readonly risk: RiskAssessment }
  | {
      readonly kind: "finished";
      readonly command: string;
      readonly risk: RiskAssessment;
      readonly results: readonly BatchHostResult[];
      readonly durationMs: number;
    };

/** One host starting or finishing, as it arrives while a run is going. */
export type BatchProgress =
  | { readonly kind: "hostStarted"; readonly sessionId: string }
  | { readonly kind: "hostFinished"; readonly result: BatchHostResult };
