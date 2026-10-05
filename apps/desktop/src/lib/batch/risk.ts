/**
 * What a risk level means to the interface.
 *
 * The verdict itself comes from the backend — the rules live there, with the corpus that locks
 * them — so this file holds only the two questions the interface asks about it, and both are about
 * presentation rather than about policy.
 */

import type { RiskAssessment, RiskLevel } from "@/types/batch";

/** Worst last, which is the order the levels are declared in on both sides of the wire. */
const ORDER: readonly RiskLevel[] = ["safe", "caution", "destructive", "catastrophic"];

/**
 * Whether this level waits for a person before it runs.
 *
 * The same levels the backend gates on. Duplicating the *rule* here would be a second policy that
 * could disagree with the enforced one; this is not the rule — it is which levels the rule names,
 * and `needs_approval` in `sevenshell-types` is the single definition of that.
 */
export function needsApproval(level: RiskLevel): boolean {
  return level === "destructive" || level === "catastrophic";
}

/** Whether a verdict has anything to say at all: a clean command explains nothing. */
export function hasReasons(assessment: RiskAssessment | null): boolean {
  return assessment !== null && assessment.reasons.length > 0;
}

/** True when `level` is at least as bad as `floor`, for deciding how loudly to draw it. */
export function atLeast(level: RiskLevel, floor: RiskLevel): boolean {
  return ORDER.indexOf(level) >= ORDER.indexOf(floor);
}
