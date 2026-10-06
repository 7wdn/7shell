/**
 * What the tree looks like on screen: one flat list of rows, in the order a person reads them.
 *
 * The list is flat rather than nested for two reasons, and both are about the drawing end: a flat
 * list is what a template can draw without a component that calls itself, and it is what a pointer
 * can be hit-tested against during a drag — a row is at a known place on screen, whatever is above
 * it.
 *
 * Groups come before the sessions that sit directly in them, the way folders come before files
 * everywhere else. A group with nothing in it draws nothing under itself: an empty folder is a
 * folder, and a line saying so is one more line to read in a list somebody is scanning.
 *
 * Sessions that are in no group are drawn at the root, with no heading over them: there is nothing
 * under the ungrouped heading that its rows do not say themselves, and a list of one's own files
 * does not need to be told it is a list of one's own files. What that heading was also *for* — a
 * place to drop something to take it out of every group — is drawn by the sidebar while a drag is
 * in flight, where it is wanted, rather than sitting there the rest of the time.
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
    };

/**
 * The rows to draw, with collapsed groups left closed.
 *
 * `open` names the groups to draw open whatever their own state is. It is how a search shows what
 * it found: a match inside a folder the person had closed is a match they cannot see.
 */
export function visibleRows(
  groups: readonly SessionGroup[],
  sessions: readonly Session[],
  collapsed: ReadonlySet<string>,
  open: ReadonlySet<string> = new Set(),
): TreeRow[] {
  const rows: TreeRow[] = [];

  const walk = (parentId: string | null, depth: number): void => {
    for (const group of childrenOf(groups, parentId)) {
      const children = childrenOf(groups, group.id);
      const own = sessions.filter((session) => sessionParentId(groups, session) === group.id);
      const closed = collapsed.has(group.id) && !open.has(group.id);
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
    }
  };
  walk(null, 0);

  // Whatever is in no group, at the root and unindented: there is no heading left to sit under.
  for (const session of sessions.filter((session) => sessionParentId(groups, session) === null)) {
    rows.push({ kind: "session", key: `session:${session.id}`, session, depth: 0, parentId: null });
  }
  return rows;
}

/** What a row means to a drag: what can be dropped on it, and what that would land in. */
export type DragTarget =
  | { readonly kind: "group"; readonly id: string }
  /** A session row is its group: dropping onto one means the group the row is drawn in. */
  | { readonly kind: "session"; readonly parentId: ParentId };

/** The drag identity of a row, or nothing for a row a drag cannot land on. */
export function rowTarget(row: TreeRow): DragTarget | null {
  if (row.kind === "group") {
    return { kind: "group", id: row.id };
  }
  if (row.kind === "session") {
    return { kind: "session", parentId: row.parentId };
  }
  return null;
}
