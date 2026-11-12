/**
 * The permission arithmetic behind the permissions tab, as pure functions.
 *
 * The rules are the contract's: an explicit session tier wins over the nearest group with a tier,
 * which wins over the default; visibility is an allowlist only once somebody hides a session. Every
 * one of those is easy to get subtly wrong on a page that draws them, so the decisions live here
 * where they can be held still and the components only draw their answers.
 *
 * A group tier cascades through the tree exactly as the host resolves it: the nearest ancestor with
 * a tier wins. A page that showed a parent's tier while the tools used the default would be worse
 * than showing no inheritance at all.
 */

import type { McpAccessPolicy, McpScope } from "@/types/app";
import type { Session, SessionGroup } from "@/types/session";

/** Where an effective tier came from. */
export type McpScopeSource = "session" | "group" | "global";

/** What one session's tier is, and which rule chose it. */
export interface EffectiveScope {
  readonly scope: McpScope;
  readonly source: McpScopeSource;
  /** The group whose tier applies, when the source is `group`. */
  readonly groupId?: string;
}

/** Whether a session is visible to an agent at all. */
export function isSessionVisible(policy: McpAccessPolicy, sessionId: string): boolean {
  return policy.visibleSessionIds === null || policy.visibleSessionIds.includes(sessionId);
}

/**
 * The tier one session runs at: its own, else its group's, else the default.
 *
 * The group consulted is the session's direct group, because that is the one the host knows how to
 * look up when it builds the runtime snapshot.
 */
export function effectiveScopeFor(
  session: Session,
  policy: McpAccessPolicy,
  globalScope: McpScope,
  groups: readonly SessionGroup[] = [],
): EffectiveScope {
  const own = policy.sessionScopes[session.id];
  if (own !== undefined) {
    return { scope: own, source: "session" };
  }
  const inherited = inheritedGroupScope(session.groupId, policy, groups);
  if (inherited !== undefined) {
    return { scope: inherited.scope, source: "group", groupId: inherited.groupId };
  }
  return { scope: globalScope, source: "global" };
}

/** A group's own tier, or the nearest ancestor's, or the default. */
export function groupScopeFor(
  groupId: string,
  policy: McpAccessPolicy,
  globalScope: McpScope,
  groups: readonly SessionGroup[] = [],
): EffectiveScope {
  const inherited = inheritedGroupScope(groupId, policy, groups);
  return inherited === undefined
    ? { scope: globalScope, source: "global" }
    : { scope: inherited.scope, source: "group", groupId: inherited.groupId };
}

/** The nearest group at or above `groupId` that carries a tier. */
function inheritedGroupScope(
  groupId: string | undefined,
  policy: McpAccessPolicy,
  groups: readonly SessionGroup[],
): { scope: McpScope; groupId: string } | undefined {
  if (groupId === undefined) {
    return undefined;
  }
  const parents = new Map(groups.map((group) => [group.id, group.parentId]));
  const seen = new Set<string>();
  let current: string | undefined = groupId;
  while (current !== undefined) {
    if (!seen.add(current)) {
      return undefined;
    }
    const own = policy.groupScopes[current];
    if (own !== undefined) {
      return { scope: own, groupId: current };
    }
    current = parents.get(current);
  }
  return undefined;
}

/** The same policy with a session's tier set, or with the override taken away for `null`. */
export function withSessionScope(policy: McpAccessPolicy, sessionId: string, scope: McpScope | null): McpAccessPolicy {
  const sessionScopes = { ...policy.sessionScopes };
  if (scope === null) {
    delete sessionScopes[sessionId];
  } else {
    sessionScopes[sessionId] = scope;
  }
  return { ...policy, sessionScopes };
}

/** The same policy with a group's tier set, or with the override taken away for `null`. */
export function withGroupScope(policy: McpAccessPolicy, groupId: string, scope: McpScope | null): McpAccessPolicy {
  const groupScopes = { ...policy.groupScopes };
  if (scope === null) {
    delete groupScopes[groupId];
  } else {
    groupScopes[groupId] = scope;
  }
  return { ...policy, groupScopes };
}

/**
 * Hides sessions, materialising the allowlist the first time.
 *
 * `visibleSessionIds: null` means every saved session. Nought minus anything is not a list, so the
 * first hide takes every id that exists at that moment and removes the hidden ones: from then on
 * the list is what is visible, and a session saved later is not on it. That is the intended
 * semantics — somebody who hid a session asked for a narrower set, not for a filter over the old
 * one — and the page says so beside the list.
 */
export function hideSessions(
  policy: McpAccessPolicy,
  sessionIds: readonly string[],
  allSessionIds: readonly string[],
): McpAccessPolicy {
  const hidden = new Set(sessionIds);
  const visible = new Set(policy.visibleSessionIds ?? allSessionIds);
  for (const id of hidden) {
    visible.delete(id);
  }
  return { ...policy, visibleSessionIds: [...visible] };
}

/** Shows sessions again; with no allowlist there is nothing to change. */
export function showSessions(policy: McpAccessPolicy, sessionIds: readonly string[]): McpAccessPolicy {
  if (policy.visibleSessionIds === null) {
    return policy;
  }
  const visible = new Set(policy.visibleSessionIds);
  for (const id of sessionIds) {
    visible.add(id);
  }
  return { ...policy, visibleSessionIds: [...visible] };
}
