/**
 * The words the agent interface's surfaces are made of, as keys rather than as sentences.
 *
 * The three tables live here rather than inside a component because `check-i18n.mjs` reads literal
 * `t("…")` calls and cannot see a key that arrives through a lookup. `mcp.spec.ts` walks these
 * records and checks every key against both locale files, which is the same guard `rows.spec.ts`
 * puts on the settings table.
 */

import type { McpScope } from "@/types/app";
import type { RiskLevel } from "@/types/batch";
import type { McpAuditStatus } from "@/types/mcp";

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
