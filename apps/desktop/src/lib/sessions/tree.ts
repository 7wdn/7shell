/**
 * What the tree looks like on screen: one flat list of rows, in the order a person reads them.
 *
 * The list is flat rather than nested for two reasons, and both are about the drawing end: a flat
 * list is what a template can draw without a component that calls itself, and it is what a pointer
 * can be hit-tested against during a drag — a row is at a known place on screen, whatever is above
 * it.
 *
 * Groups come before the sessions that sit directly in them, the way folders come before files
 * everywhere else. A group with nothing in it says so, once, where its contents would have been.
 */
import type { Session, SessionGroup } from "@/types/session";

import { childrenOf, type ParentId, sessionParentId } from "./groups";

/** One line of the tree. `depth` is how far it is indented, in steps. */
export type TreeRow =
  | {
      readonly kind: "group";
      readonly key: string;
      readonly id: string;
      readonly group: SessionGroup;
      readonly depth: number;
      readonly collapsed: boolean;
      /** Whether it holds anything at all: sessions of its own, or groups under it. */
      readonly filled: boolean;
    }
  | {
      readonly kind: "session";
      readonly key: string;
      readonly session: Session;
      readonly depth: number;
      /** The group it is drawn inside, which is what dropping something onto it would mean. */
      readonly parentId: ParentId;
    }
  | {
      readonly kind: "ungrouped";
      readonly key: string;
      readonly depth: number;
    }
  | { readonly kind: "hint"; readonly key: string; readonly depth: number; readonly scope: "group" | "ungrouped" };

/**
 * The rows to draw, with collapsed groups left closed.
 *
 * The ungrouped bucket is drawn whenever there is a group to be outside of — it is a place things
 * can be put, and a place that appears and disappears as sessions come and go is a place nobody can
 * aim at. With no groups at all there is no bucket: a flat list has nothing to be apart from.
 */
export function visibleRows(
  groups: readonly SessionGroup[],
  sessions: readonly Session[],
  collapsed: ReadonlySet<string>,
): TreeRow[] {
  const rows: TreeRow[] = [];

  const walk = (parentId: string | null, depth: number): void => {
    for (const group of childrenOf(groups, parentId)) {
      const children = childrenOf(groups, group.id);
      const own = sessions.filter((session) => sessionParentId(groups, session) === group.id);
      const closed = collapsed.has(group.id);
      rows.push({
        kind: "group",
        key: `group:${group.id}`,
        id: group.id,
        group,
        depth,
        collapsed: closed,
        filled: own.length > 0 || children.length > 0,
      });
      if (closed) {
        continue;
      }
      walk(group.id, depth + 1);
      for (const session of own) {
        rows.push({ kind: "session", key: `session:${session.id}`, session, depth: depth + 1, parentId: group.id });
      }
      if (own.length === 0 && children.length === 0) {
        rows.push({ kind: "hint", key: `hint:${group.id}`, depth: depth + 1, scope: "group" });
      }
    }
  };
  walk(null, 0);

  const loose = sessions.filter((session) => sessionParentId(groups, session) === null);
  if (groups.length === 0) {
    // No groups at all: a flat list, with no heading over it — there is nothing to be outside of.
    for (const session of loose) {
      rows.push({ kind: "session", key: `session:${session.id}`, session, depth: 0, parentId: null });
    }
    return rows;
  }

  rows.push({ kind: "ungrouped", key: "ungrouped", depth: 0 });
  for (const session of loose) {
    rows.push({ kind: "session", key: `session:${session.id}`, session, depth: 1, parentId: null });
  }
  if (loose.length === 0) {
    rows.push({ kind: "hint", key: "hint:ungrouped", depth: 1, scope: "ungrouped" });
  }
  return rows;
}

/** What a row means to a drag: what can be dropped on it, and what that would land in. */
export type DragTarget =
  | { readonly kind: "group"; readonly id: string }
  /** A session row is its group: dropping onto one means the group the row is drawn in. */
  | { readonly kind: "session"; readonly parentId: ParentId }
  | { readonly kind: "ungrouped" };

/** The drag identity of a row, or nothing for a row a drag cannot land on. */
export function rowTarget(row: TreeRow): DragTarget | null {
  if (row.kind === "group") {
    return { kind: "group", id: row.id };
  }
  if (row.kind === "session") {
    return { kind: "session", parentId: row.parentId };
  }
  if (row.kind === "ungrouped") {
    return { kind: "ungrouped" };
  }
  return null;
}
