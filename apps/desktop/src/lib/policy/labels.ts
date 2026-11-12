/**
 * The two vocabularies the policy page draws: the four risk levels and the three answers.
 *
 * Written once because three surfaces use them — the matrix's global rule, its exception rows and
 * the audit table — and a page that named one level differently from the table beside it would be
 * asking a reader to learn two words for the same thing.
 */

import type { RiskLevel } from "@/types/batch";
import type { PolicyAction } from "@/types/app";

/** The four levels, in the order the matrix draws them: least to most dangerous. */
export const POLICY_LEVELS: readonly RiskLevel[] = ["safe", "caution", "destructive", "catastrophic"];

/** The three answers, in the order the dropdown offers them: loosest to strictest. */
export const POLICY_ACTIONS: readonly PolicyAction[] = ["allow", "ask", "deny"];

export const POLICY_LEVEL_LABEL_KEYS: Record<RiskLevel, string> = {
  safe: "risk.level.safe",
  caution: "risk.level.caution",
  destructive: "risk.level.destructive",
  catastrophic: "risk.level.catastrophic",
};

export const POLICY_ACTION_LABEL_KEYS: Record<PolicyAction, string> = {
  allow: "settings.security.policy.actions.allow",
  ask: "settings.security.policy.actions.ask",
  deny: "settings.security.policy.actions.deny",
};
