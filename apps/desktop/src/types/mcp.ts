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

/** How an external MCP server is reached, from `sevenshell_types::McpServerTransport`. */
export type McpServerTransport = "stdio" | "http" | "sse";

/** The transports, in the order a settings row offers them. */
export const MCP_SERVER_TRANSPORTS: readonly McpServerTransport[] = ["stdio", "http", "sse"];

/**
 * One external MCP server, from `sevenshell_types::McpServerConfig`.
 *
 * There is no `env` and no `headers` here, and there is no way to add them: the environment an
 * stdio server is started with and the headers an HTTP server expects are secrets, and they live in
 * the vault under `id`. This interface is only ever told where a server is and whether it is on.
 */
export interface McpServerConfig {
  /** One to 32 characters from `[A-Za-z0-9_-]`, and never containing `__`. */
  readonly id: string;
  /** What a person called it. Empty means the list draws `id`. */
  readonly name: string;
  readonly transport: McpServerTransport;
  /** The program an stdio server starts, when `transport` is `stdio`. */
  readonly command?: string;
  readonly args: readonly string[];
  /** The address an HTTP or SSE server is reached at. */
  readonly url?: string;
  readonly enabled: boolean;
  /** Seconds one call to one of its tools may take, between 5 and 600. */
  readonly toolTimeoutSeconds: number;
}

/** The longest id a server may have, from `sevenshell_types::MCP_SERVER_ID_MAX_CHARS`. */
export const MCP_SERVER_ID_MAX_CHARS = 32;

/** How long a call to an external tool may take: the default, and the ends the page offers. */
export const MCP_SERVER_TOOL_TIMEOUT_SECONDS_DEFAULT = 60;
export const MCP_SERVER_TOOL_TIMEOUT_SECONDS_MIN = 5;
export const MCP_SERVER_TOOL_TIMEOUT_SECONDS_MAX = 600;

/**
 * Where one external server's supervision has got to.
 *
 * `failed` is not the same as `disabled`: the first is a server that is meant to be reachable and
 * is not, the second is one somebody switched off. A page draws them differently because only one
 * of them is something to act on.
 */
export type McpServerState = "connecting" | "ready" | "failed" | "disabled";

/** One server's state, from `sevenshell_types::McpServerStatus`. */
export interface McpServerStatus {
  readonly serverId: string;
  readonly state: McpServerState;
  /** How many tools it last announced. Zero before the first successful handshake. */
  readonly tools: number;
  /** The failure's code, when the state is `failed`. */
  readonly code?: string;
  /** A plain sentence for the row, never a secret. */
  readonly message?: string;
}

/**
 * One external server as the settings page draws it: its configuration, where its supervision has
 * got to, and whether the vault holds secrets for it.
 *
 * `hasSecrets` is `null`, not `false`, while the vault is locked: "cannot tell" and "none stored"
 * are different answers, and a page that drew the second over the first would invite somebody to
 * type a secret they had already saved. Nothing here can carry the secret itself — the values go
 * in through `mcp_server_secrets_set` and are never read back.
 */
export interface McpServerView {
  readonly config: McpServerConfig;
  readonly status: McpServerStatus;
  readonly hasSecrets: boolean | null;
}

/**
 * Whether an id can name a server, mirroring `is_valid_mcp_server_id` in Rust.
 *
 * The shell enforces this too. The form repeats it so a typo is caught before a command that would
 * reject it, and so the rule is testable without a backend: the double underscore is what lets
 * `mcp__{server}__{tool}` parse back into exactly two parts.
 */
export function isValidMcpServerId(id: string): boolean {
  return id.length > 0 && id.length <= MCP_SERVER_ID_MAX_CHARS && !id.includes("__") && /^[A-Za-z0-9_-]+$/.test(id);
}

/** Why one call stopped for an answer. */
export type McpApprovalReason =
  | { readonly kind: "risk"; readonly assessment: RiskAssessment }
  | {
      readonly kind: "policy";
      readonly assessment: RiskAssessment;
      /** The layer that raised the question, e.g. `host:web:2222` or `group:prod`. */
      readonly source: string;
    }
  | { readonly kind: "hostKeyUnknown"; readonly key: PresentedHostKey }
  | { readonly kind: "external"; readonly server: string; readonly tool: string };

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

/**
 * The heading a tool is drawn under, from `sevenshell_api::CatalogCategory`.
 *
 * A closed list rather than a prefix rule: the page groups by what a tool *is*, and a category that
 * arrived only in a newer build's catalog would be drawn under the external heading rather than
 * dropped — see {@link isMcpToolCategory}.
 */
export type McpToolCategory = "sessions" | "exec" | "files" | "containers" | "kubernetes" | "external";

/** The categories, in the order the capabilities page lists them. */
export const MCP_TOOL_CATEGORIES: readonly McpToolCategory[] = [
  "sessions",
  "exec",
  "files",
  "containers",
  "kubernetes",
  "external",
];

/** How carefully a tool has to be read before switching it on, from `sevenshell_api::ToolRisk`. */
export type McpToolRisk = "read" | "write" | "commandGraded";

/** What a tool does to what it touches, as the registry gates it. */
export type McpToolAccess = "read" | "write";

/**
 * One tool as the catalog describes it.
 *
 * `source` is `null` for a built-in tool and the external server's id otherwise. It is the only
 * thing that tells the two apart: an external tool may sit in any category, but its switch is its
 * server's, not its own.
 */
export interface McpCatalogTool {
  readonly name: string;
  readonly title: string;
  readonly description: string;
  readonly access: McpToolAccess;
  readonly risk: McpToolRisk;
  readonly category: McpToolCategory;
  readonly enabled: boolean;
  readonly source: string | null;
}

/** The catalog the capabilities page draws, from `mcp_catalog`. */
export interface McpCatalog {
  readonly tools: readonly McpCatalogTool[];
  readonly counts: {
    readonly total: number;
    readonly enabled: number;
  };
}

/** Whether a value off the wire is a category this build draws. */
export function isMcpToolCategory(value: unknown): value is McpToolCategory {
  return MCP_TOOL_CATEGORIES.includes(value as McpToolCategory);
}
