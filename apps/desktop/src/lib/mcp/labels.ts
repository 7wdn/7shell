/**
 * The words the agent interface's surfaces are made of, as keys rather than as sentences.
 *
 * The tables live here rather than inside a component because `check-i18n.mjs` reads literal
 * `t("…")` calls and cannot see a key that arrives through a lookup. `mcp.spec.ts` walks these
 * records and checks every key against both locale files, which is the same guard `rows.spec.ts`
 * puts on the settings table.
 */

import type { McpClientId } from "@/lib/mcp/clients";
import type { McpScope } from "@/types/app";
import type { RiskLevel } from "@/types/batch";
import type { McpAuditStatus, McpServerState, McpServerTransport, McpToolCategory, McpToolRisk } from "@/types/mcp";

/** How loudly each risk level is said, keyed by the wire's own spelling. */
export const RISK_LEVEL_LABEL_KEYS: Record<RiskLevel, string> = {
  safe: "risk.level.safe",
  caution: "risk.level.caution",
  destructive: "risk.level.destructive",
  catastrophic: "risk.level.catastrophic",
};

/** The three tiers, as the dropdown names them. */
export const MCP_SCOPE_LABEL_KEYS: Record<McpScope, string> = {
  readOnly: "mcp.scope.readOnly",
  safeWrite: "mcp.scope.safeWrite",
  highRisk: "mcp.scope.highRisk",
};

/** One line under the chosen tier: what it actually allows, and what it still stops to ask about. */
export const MCP_SCOPE_HINT_KEYS: Record<McpScope, string> = {
  readOnly: "mcp.scope.readOnlyHint",
  safeWrite: "mcp.scope.safeWriteHint",
  highRisk: "mcp.scope.highRiskHint",
};

/** How an audited call ended. */
export const MCP_AUDIT_STATUS_LABEL_KEYS: Record<McpAuditStatus, string> = {
  succeeded: "mcp.audit.status.succeeded",
  failed: "mcp.audit.status.failed",
  denied: "mcp.audit.status.denied",
};

/**
 * Where one external server's supervision has got to.
 *
 * `failed` and `disabled` are drawn apart because only one of them is something to act on: the
 * first is a server somebody wants and cannot reach, the second is one somebody switched off.
 */
export const MCP_SERVER_STATE_LABEL_KEYS: Record<McpServerState, string> = {
  connecting: "mcp.servers.state.connecting",
  ready: "mcp.servers.state.ready",
  failed: "mcp.servers.state.failed",
  disabled: "mcp.servers.state.disabled",
};

/** How an external server is reached, keyed by the wire's spelling. */
export const MCP_SERVER_TRANSPORT_LABEL_KEYS: Record<McpServerTransport, string> = {
  stdio: "mcp.servers.transport.stdio",
  http: "mcp.servers.transport.http",
  sse: "mcp.servers.transport.sse",
};

/** The headings the capabilities page groups tools under. */
export const MCP_TOOL_CATEGORY_LABEL_KEYS: Record<McpToolCategory, string> = {
  sessions: "mcp.capabilities.category.sessions",
  exec: "mcp.capabilities.category.exec",
  files: "mcp.capabilities.category.files",
  containers: "mcp.capabilities.category.containers",
  kubernetes: "mcp.capabilities.category.kubernetes",
  external: "mcp.capabilities.category.external",
};

/**
 * How much care a tool needs before it is switched on.
 *
 * `commandGraded` is its own word rather than "write": the same tool may read or destroy depending
 * on the argument it is given, and the tier is decided per call by the policy classifier.
 */
export const MCP_TOOL_RISK_LABEL_KEYS: Record<McpToolRisk, string> = {
  read: "mcp.capabilities.risk.read",
  write: "mcp.capabilities.risk.write",
  commandGraded: "mcp.capabilities.risk.commandGraded",
};

/** The client names on the access tab's configuration strip. */
export const MCP_CLIENT_LABEL_KEYS: Record<McpClientId, string> = {
  claudeCode: "mcp.clients.name.claudeCode",
  cursor: "mcp.clients.name.cursor",
  vscode: "mcp.clients.name.vscode",
  windsurf: "mcp.clients.name.windsurf",
  qoder: "mcp.clients.name.qoder",
  codex: "mcp.clients.name.codex",
};
