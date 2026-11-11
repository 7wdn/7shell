/**
 * Hand-written mirror of the wire types the agent interface (MCP) answers with.
 *
 * One rule shapes the whole file: **there is no token here, and no way to add one.** The endpoint
 * file holds the token, the bridge reads it, and the window is never told it — so nothing in this
 * file has a field a token could arrive in. The same holds for the client configuration: it names
 * the bridge and the endpoint file, and deliberately not the secret the file contains.
 *
 * The status is the listener's own answer rather than a reading of the settings file: a switch that
 * says "on" while nothing is bound to a port is the one thing this page must not draw.
 */

import type { McpScope } from "@/types/app";
import type { RiskAssessment } from "@/types/batch";
import type { PresentedHostKey } from "@/types/session";

/** What the listener is doing, as the settings page shows it. */
export interface McpStatus {
  /** Whether anything is listening for a bridge to call. */
  readonly enabled: boolean;
  /** The port it answered with, or null while nothing is listening. Zero in the settings means "let the system choose". */
  readonly port: number | null;
  /** The tier the running listener was started with. */
  readonly scope: McpScope;
  /**
   * The snippet a client's configuration file takes, without the token.
   *
   * Handed over for the copy button rather than to be drawn: it names a program and a file on this
   * machine, and what it is for is being pasted somewhere else.
   */
  readonly clientConfig: string;
  /** Where the audit file lives, for somebody who wants to read it with another tool. */
  readonly auditPath: string;
  /** How many calls are waiting for a person's answer right now. */
  readonly pendingApprovals: number;
}

/** Why one call stopped for an answer. */
export type McpApprovalReason =
  | { readonly kind: "risk"; readonly assessment: RiskAssessment }
  | { readonly kind: "hostKeyUnknown"; readonly key: PresentedHostKey };

/**
 * Who asked for the call.
 *
 * `mcp` is a client driving the tools over the loopback endpoint; `ai` is this window's own
 * assistant. They share one dialog and one answer command, because both are the same question —
 * "may this run" — and a person should not have to learn two.
 */
export type McpApprovalOrigin = "mcp" | "ai";

/** One call waiting for a person to allow or refuse it. */
export interface McpApprovalRequest {
  /** Identifies the call; the answer is filed against it rather than against the dialog. */
  readonly callId: string;
  /** The tool's wire name, as the agent asked for it. */
  readonly tool: string;
  /**
   * Who is waiting. Absent reads as `mcp`: a host written before the assistant existed only ever
   * had one origin, and an old page must not draw "asked by" as something it cannot name.
   */
  readonly origin?: McpApprovalOrigin;
  readonly sessionId?: string;
  readonly sessionName?: string;
  /** What the call is about: a command line, a path, a container. Never a credential. */
  readonly subject?: string;
  readonly reason: McpApprovalReason;
  /** When the listener stops waiting, in milliseconds since the Unix epoch. */
  readonly expiresAt: number;
}

/** The two answers a person can give. */
export type McpApprovalDecision = "approved" | "denied";

/** How one call ended. */
export type McpAuditStatus = "succeeded" | "failed" | "denied";

/**
 * What was decided about a call that stopped to ask.
 *
 * `timedOut` is its own answer rather than a kind of `denied`: nobody refused it, and an audit that
 * could not tell the two apart would report people saying no when they were not there.
 */
export interface McpAuditApproval {
  /** The reason the call stopped, as the backend recorded it. Not interpreted here. */
  readonly reason: unknown;
  readonly decision: McpApprovalDecision | "timedOut";
  readonly waitedMs: number;
}

/** One line of the audit: a call, what it was about, and what came of it. */
export interface McpAuditEntry {
  /** Seconds since the Unix epoch, the same clock the audit file and the other settings use. */
  readonly at: number;
  readonly tool: string;
  readonly scope: McpScope;
  readonly status: McpAuditStatus;
  /** The error code, when the call failed. */
  readonly code?: string;
  readonly durationMs: number;
  readonly sessionId?: string;
  readonly subject?: string;
  readonly risk?: RiskAssessment;
  readonly approval?: McpAuditApproval;
}

/**
 * Whether a value off the wire is one of the tiers this build draws.
 *
 * The contract says an unknown tier reads as the narrowest one; the shell does that before it
 * answers, and this is the same rule for a window that is handed a newer build's answer.
 */
export function isMcpScope(value: unknown): value is McpScope {
  return value === "readOnly" || value === "safeWrite" || value === "highRisk";
}

/**
 * The defaults a fresh installation has, mirroring `sevenshell_types::settings`.
 *
 * Written here as well because the settings page draws what an unset field means, and a page that
 * guessed its own answer would show one number while the shell used another.
 */
export const DEFAULT_MCP_ENABLED = false;
export const DEFAULT_MCP_PORT = 0;
export const DEFAULT_MCP_APPROVAL_TIMEOUT_SECONDS = 120;
export const DEFAULT_MCP_AUDIT_RETENTION_DAYS = 30;
export const DEFAULT_MCP_MAX_RESULT_BYTES = 262_144;

/**
 * The ends of the fields the page offers.
 *
 * The contract fixes the defaults but not the range, and a number typed into a box is a number
 * somebody can mistype: a port above 65535 cannot be bound at all, and a result limit of nothing
 * would truncate every answer to an empty string. These are the generous ends of each — wide enough
 * that nobody's real answer is outside, narrow enough that a typo is corrected instead of accepted.
 */
export const MCP_PORT_MAX = 65_535;
export const MCP_APPROVAL_TIMEOUT_SECONDS_MIN = 10;
export const MCP_APPROVAL_TIMEOUT_SECONDS_MAX = 1_800;
export const MCP_AUDIT_RETENTION_DAYS_MIN = 1;
export const MCP_AUDIT_RETENTION_DAYS_MAX = 365;
/** In bytes: 16 KiB is about the shortest answer worth returning, 16 MiB the longest worth reading. */
export const MCP_MAX_RESULT_BYTES_MIN = 16_384;
export const MCP_MAX_RESULT_BYTES_MAX = 16_777_216;

/** How many audit lines the settings page asks for at a time. */
export const MCP_AUDIT_TAIL_LIMIT = 50;
