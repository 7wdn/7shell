/**
 * Groups as a tree: which parent a group really sits under, what a drop does, and what a move
 * writes.
 *
 * Three rules shape this module:
 *
 * 1. **The stored shape is flat.** A group names its parent and carries an order; the tree is
 *    computed every time it is needed. Nothing here holds state, so the sidebar, the menus and the
 *    drag all ask the same functions and cannot disagree about the shape.
 * 2. **A vault that was edited by hand has to draw as something.** A parent that is gone, or a
 *    chain of parents that loops back on itself, means "this group sits at the root" rather than an
 *    infinite walk — a file somebody typed is data, not a promise.
 * 3. **A drop answers with writes.** Deciding where a dropped thing goes and deciding what has to
 *    be saved are one question, so they are one function: what comes back is the groups whose
 *    parent or order changes, and nothing else is written.
 */
import type { Session, SessionGroup } from "@/types/session";

/** A parent id, where `null` means the root of the tree. */
export type ParentId = string | null;

/** Where something was dropped: the three bands of a group row. */
export type DropZone = "before" | "inside" | "after";

/**
 * Where a dropped thing goes, whoever it is: into a group, or at the root of the list.
 *
 * Which band of the row was hit is deliberately *not* part of this, because the band does not mean
 * the same thing for the two things that can be dragged: a group dropped beside another group lands
 * between two groups, while a session dropped anywhere on a row lands in the group that row is
 * drawn in. So the band travels as its own argument, to the one planner that can use it.
 */
export type Drop = { readonly kind: "group"; readonly id: string } | { readonly kind: "root" };

/** One group write: where it goes, and which order value makes it land there. */
export interface GroupPlacement {
  readonly id: string;
  readonly parentId: ParentId;
  readonly order: number;
}

function byId(groups: readonly SessionGroup[]): Map<string, SessionGroup> {
  return new Map(groups.map((group) => [group.id, group]));
}

/**
 * The parent a group really sits under: none when it has none, when the group it names is gone, or
 * when following the names loops back — a circle has no top, so every group on it is drawn at the
 * root.
 */
export function effectiveParentId(groups: readonly SessionGroup[], id: string): ParentId {
  const index = byId(groups);
  const start = index.get(id);
  if (!start) {
    return null;
  }
  const seen = new Set<string>([id]);
  let parent = start.parentId ?? null;
  while (parent !== null) {
    const next = index.get(parent);
    if (!next || seen.has(next.id)) {
      return null;
    }
    seen.add(next.id);
    parent = next.parentId ?? null;
  }
  return start.parentId ?? null;
}

/** One parent's children in the order the tree draws them: lowest order first, ties as listed. */
export function childrenOf(groups: readonly SessionGroup[], parentId: ParentId): SessionGroup[] {
  return groups
    .map((group, position) => ({ group, position }))
    .filter((entry) => effectiveParentId(groups, entry.group.id) === parentId)
    .sort((left, right) => left.group.order - right.group.order || left.position - right.position)
    .map((entry) => entry.group);
}

/** Every group in draw order — depth first, each parent's children in their own order. */
export function drawOrder(groups: readonly SessionGroup[]): { readonly group: SessionGroup; readonly depth: number }[] {
  const drawn: { group: SessionGroup; depth: number }[] = [];
  const walk = (parentId: ParentId, depth: number): void => {
    for (const group of childrenOf(groups, parentId)) {
      drawn.push({ group, depth });
      walk(group.id, depth + 1);
    }
  };
  walk(null, 0);
  return drawn;
}

/** Whether `id` sits anywhere under `ancestorId` — the question a cycle is made of. */
export function isDescendant(groups: readonly SessionGroup[], ancestorId: string, id: string): boolean {
  const index = byId(groups);
  const seen = new Set<string>();
  let current = index.get(id)?.parentId ?? null;
  while (current !== null && !seen.has(current)) {
    if (current === ancestorId) {
      return true;
    }
    seen.add(current);
    current = index.get(current)?.parentId ?? null;
  }
  return false;
}

/**
 * The name a group is called in a flat list such as a menu: its own name, with every group above it
 * in front of it. Two groups may share a name; the path is what tells them apart.
 */
export function groupPath(groups: readonly SessionGroup[], id: string): string {
  const index = byId(groups);
  const names = [index.get(id)?.name ?? id];
  const seen = new Set<string>([id]);
  let parent = effectiveParentId(groups, id);
  while (parent !== null && !seen.has(parent)) {
    seen.add(parent);
    names.unshift(index.get(parent)?.name ?? parent);
    parent = effectiveParentId(groups, parent);
  }
  return names.join(" / ");
}

/** Where a group made now goes: after everything already under that parent. */
export function nextOrder(groups: readonly SessionGroup[], parentId: ParentId): number {
  return childrenOf(groups, parentId).reduce((next, group) => Math.max(next, group.order + 1), 0);
}

/** Whether there is a sibling on that side of this group, which is what decides if a move can run. */
export function hasSibling(groups: readonly SessionGroup[], id: string, direction: -1 | 1): boolean {
  const siblings = childrenOf(groups, effectiveParentId(groups, id));
  const at = siblings.findIndex((group) => group.id === id);
  const target = at + direction;
  return at >= 0 && target >= 0 && target < siblings.length;
}

/**
 * Turning a parent's children into their new order values.
 *
 * The list that arrives is the arrangement that was asked for; what comes back is one write per
 * group whose order or parent actually changed. Ties are the reason this exists: two groups may
 * both be at order 0, and "put this one between them" cannot be written as a swap of two equal
 * numbers. A group whose numbers already say where it now sits is not written at all.
 */
function arrange(
  groups: readonly SessionGroup[],
  parentId: ParentId,
  ordered: readonly SessionGroup[],
): GroupPlacement[] {
  const writes: GroupPlacement[] = [];
  ordered.forEach((group, position) => {
    const current = groups.find((entry) => entry.id === group.id);
    const parentChanged = (current?.parentId ?? null) !== parentId;
    if (parentChanged || current?.order !== position) {
      writes.push({ id: group.id, parentId, order: position });
    }
  });
  return writes;
}

/** The siblings of a parent without the group being moved, which is the list a drop inserts into. */
function withoutDragged(groups: readonly SessionGroup[], parentId: ParentId, draggedId: string): SessionGroup[] {
  return childrenOf(groups, parentId).filter((group) => group.id !== draggedId);
}

/**
 * What dropping a group there writes — or nothing, when the drop is refused or changes nothing.
 *
 * `zone` is the band of the row that was hit, and only a group has bands; a caller that has none
 * (the bucket at the root of the list) leaves it at `inside`.
 *
 * Refused: a group dropped onto itself, and a group dropped where it would become its own
 * descendant. Nothing: a drop that would leave the tree exactly as it is.
 */
export function planGroupDrop(
  groups: readonly SessionGroup[],
  draggedId: string,
  drop: Drop,
  zone: DropZone = "inside",
): GroupPlacement[] | null {
  const dragged = groups.find((group) => group.id === draggedId);
  if (!dragged) {
    return null;
  }

  let parentId: ParentId;
  let before: SessionGroup | null = null;
  if (drop.kind === "root") {
    parentId = null;
  } else if (drop.id === draggedId) {
    return null;
  } else {
    const target = groups.find((group) => group.id === drop.id);
    if (!target) {
      return null;
    }
    if (zone === "inside") {
      parentId = target.id;
    } else {
      parentId = effectiveParentId(groups, target.id);
      before = zone === "before" ? target : null;
      if (before === null) {
        // "After" is the same insertion, one place further on: the anchor becomes the sibling that
        // follows the target rather than the target itself.
        const siblings = childrenOf(groups, parentId);
        const at = siblings.findIndex((group) => group.id === target.id);
        before = siblings[at + 1] ?? null;
      }
      if (before?.id === draggedId) {
        // The place just after the target is where the dragged group already is: a drop that would
        // land it back on itself is not a move.
        return null;
      }
    }
  }

  if (parentId === draggedId || (parentId !== null && isDescendant(groups, draggedId, parentId))) {
    return null;
  }

  const siblings = withoutDragged(groups, parentId, draggedId);
  const at = before === null ? siblings.length : siblings.findIndex((group) => group.id === before.id);
  const index = at < 0 ? siblings.length : at;
  const ordered = [...siblings.slice(0, index), dragged, ...siblings.slice(index)];

  const unchanged =
    effectiveParentId(groups, draggedId) === parentId &&
    childrenOf(groups, parentId)
      .map((group) => group.id)
      .join() === ordered.map((group) => group.id).join();
  return unchanged ? null : arrange(groups, parentId, ordered);
}

/** What dropping a group one place up or down among its siblings writes. */
export function planSiblingMove(
  groups: readonly SessionGroup[],
  id: string,
  direction: -1 | 1,
): GroupPlacement[] | null {
  const parentId = effectiveParentId(groups, id);
  const siblings = childrenOf(groups, parentId);
  const at = siblings.findIndex((group) => group.id === id);
  const target = at + direction;
  if (at < 0 || target < 0 || target >= siblings.length) {
    return null;
  }
  const ordered = [...siblings];
  const moved = ordered[at];
  const neighbour = ordered[target];
  if (!moved || !neighbour) {
    return null;
  }
  ordered[at] = neighbour;
  ordered[target] = moved;
  return arrange(groups, parentId, ordered);
}

/**
 * The group a session is really in: its own answer, unless that group is gone — a session whose
 * group was deleted is in none, and has to be drawn somewhere.
 */
export function sessionParentId(groups: readonly SessionGroup[], session: Session): ParentId {
  if (!session.groupId) {
    return null;
  }
  return groups.some((group) => group.id === session.groupId) ? session.groupId : null;
}

/** What dropping a session there writes — or nothing, when it would stay where it is. */
export function planSessionDrop(
  groups: readonly SessionGroup[],
  session: Session,
  drop: Drop,
): { readonly groupId: ParentId } | null {
  let groupId: ParentId;
  if (drop.kind === "root") {
    groupId = null;
  } else {
    if (!groups.some((group) => group.id === drop.id)) {
      return null;
    }
    groupId = drop.id;
  }

  return sessionParentId(groups, session) === groupId ? null : { groupId };
}
