/**
 * Groups and sessions as one tree: where each thing sits, what a drop does, and what a move
 * writes.
 *
 * Three rules shape this module:
 *
 * 1. **The stored shape is flat.** A group names its parent and carries an order; a session names
 *    its group and carries an order in the same number space. The tree is computed every time it
 *    is needed, and a parent's children — groups and sessions together — are drawn in one order,
 *    so a session can sit between two groups and a group between two sessions. A pinned group is
 *    the one thing laid over that: it is drawn above everything else beside it whatever order it
 *    carries, and its order is left alone so that unpinning puts it back.
 * 2. **A vault that was edited by hand has to draw as something.** A parent that is gone, or a
 *    chain of parents that loops back on itself, means "this group sits at the root" rather than an
 *    infinite walk — a file somebody typed is data, not a promise.
 * 3. **A drop answers with writes.** Deciding where a dropped thing goes and deciding what has to
 *    be saved are one question, so they are one function: what comes back is the children whose
 *    parent or order changes, and nothing else is written. Renumbering covers both kinds, because
 *    a group and a session share one list.
 */
import type { Session, SessionGroup } from "@/types/session";

/** A parent id, where `null` means the root of the tree. */
export type ParentId = string | null;

/** Where something was dropped: the three bands of a group row. */
export type DropZone = "before" | "inside" | "after";

/**
 * Where a dropped thing goes.
 *
 * A group row means "inside this group" in its middle band and "beside it, at its parent" in the
 * two ends. A session row has two bands and means "beside this session, in its parent" in both —
 * which is also how a thing changes groups, since the row it lands beside is drawn in the parent it
 * will join. Nothing here says whether the drop is allowed; that is the planner's answer, so the
 * line the drag draws and the place the release writes cannot disagree.
 */
export type Drop =
  | { readonly kind: "group"; readonly id: string }
  | { readonly kind: "root" }
  | { readonly kind: "session"; readonly id: string };

/** One child of a parent: the two things a tree level holds, in the order it draws them. */
export type Child =
  | { readonly kind: "group"; readonly group: SessionGroup }
  | { readonly kind: "session"; readonly session: Session };

/** One write a drop makes: which child lands where, and the order value that puts it there. */
export interface ChildPlacement {
  readonly kind: "group" | "session";
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

/**
 * Which of a parent's two bands a group is drawn in: the pinned ones above, everything else below.
 *
 * A number rather than a boolean so that the band is the first thing the sort compares, and so
 * that a group with no pin at all — read from a vault written before there was one — is sorted
 * with the rest rather than on its own. Sessions are never pinned, so they always sit below.
 */
function bandOf(group: SessionGroup): number {
  return group.pinned ? 0 : 1;
}

function bandOfChild(child: Child): number {
  return child.kind === "group" ? bandOf(child.group) : 1;
}

/** The order a child carries; a session nobody has placed is level with zero. */
function orderOfChild(child: Child): number {
  return child.kind === "group" ? child.group.order : (child.session.order ?? 0);
}

/** Ties inside one band: a group before a session, so an older vault draws groups first. */
function rankOfChild(child: Child): number {
  return child.kind === "group" ? 0 : 1;
}

function sameChild(left: Child, right: Child): boolean {
  return (
    left.kind === right.kind &&
    (left.kind === "group" ? left.group.id : left.session.id) ===
      (right.kind === "group" ? right.group.id : right.session.id)
  );
}

function childId(child: Child): string {
  return child.kind === "group" ? child.group.id : child.session.id;
}

/**
 * One parent's children in the order the tree draws them: pinned groups first, then lowest order,
 * ties with groups before sessions and then as listed.
 *
 * A parent holding a session nobody has placed is one nobody has dragged in since sessions carried
 * a place — an older vault, or a pool the list has not been rearranged in yet. Its drawing stays
 * what it always was: every group of the parent, then every session. The vault's own migration
 * numbers such a parent the same way, so the two sides agree about what the list looks like.
 *
 * This is the one place the arrangement is decided. The list, the menus that name groups by their
 * path, the drop planner and the move planner all ask this function, so none of them can end up
 * with a different answer about what is above what.
 */
export function childrenInOrder(
  groups: readonly SessionGroup[],
  sessions: readonly Session[],
  parentId: ParentId,
): Child[] {
  const legacy = sessions.some(
    (session) => sessionParentId(groups, session) === parentId && session.order === undefined,
  );
  const entries: { child: Child; band: number; order: number; rank: number; position: number }[] = [];
  groups.forEach((group, position) => {
    if (effectiveParentId(groups, group.id) !== parentId) {
      return;
    }
    const child: Child = { kind: "group", group };
    entries.push({ child, band: bandOfChild(child), order: orderOfChild(child), rank: rankOfChild(child), position });
  });
  sessions.forEach((session, position) => {
    if (sessionParentId(groups, session) !== parentId) {
      return;
    }
    const child: Child = { kind: "session", session };
    entries.push({ child, band: bandOfChild(child), order: orderOfChild(child), rank: rankOfChild(child), position });
  });
  return entries
    .sort((left, right) => {
      const byBand = left.band - right.band;
      if (byBand !== 0) {
        return byBand;
      }
      const byKindFirst = legacy ? left.rank - right.rank : 0;
      if (byKindFirst !== 0) {
        return byKindFirst;
      }
      const byOrder = left.order - right.order;
      if (byOrder !== 0) {
        return byOrder;
      }
      const byKind = legacy ? 0 : left.rank - right.rank;
      return byKind !== 0 ? byKind : left.position - right.position;
    })
    .map((entry) => entry.child);
}

/** The groups of one parent, in the order the tree draws them. Kept for the menus that list groups. */
export function childrenOf(groups: readonly SessionGroup[], parentId: ParentId): SessionGroup[] {
  return childrenInOrder(groups, [], parentId)
    .filter((child): child is { kind: "group"; group: SessionGroup } => child.kind === "group")
    .map((child) => child.group);
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

/** Where a child made now goes: after everything already under that parent. */
export function nextOrder(groups: readonly SessionGroup[], sessions: readonly Session[], parentId: ParentId): number {
  return childrenInOrder(groups, sessions, parentId).reduce(
    (next, child) => Math.max(next, orderOfChild(child) + 1),
    0,
  );
}

/**
 * The children of one parent a group may step past: its own band, since the two bands do not mix.
 *
 * With a pinned group above it, the first unpinned group has nothing it can be moved past, and a
 * menu entry that offers a move that would not happen is a menu that looks broken.
 */
function stepCandidates(groups: readonly SessionGroup[], sessions: readonly Session[], id: string): Child[] {
  const group = groups.find((entry) => entry.id === id);
  if (!group) {
    return [];
  }
  return childrenInOrder(groups, sessions, effectiveParentId(groups, id)).filter((child) =>
    child.kind === "group" ? child.group.pinned === group.pinned : !group.pinned,
  );
}

/** Whether there is a group to move past on that side, which is what decides if a move can run. */
export function hasSibling(
  groups: readonly SessionGroup[],
  sessions: readonly Session[],
  id: string,
  direction: -1 | 1,
): boolean {
  const candidates = stepCandidates(groups, sessions, id);
  const at = candidates.findIndex((child) => childId(child) === id);
  const target = at + direction;
  return at >= 0 && target >= 0 && target < candidates.length;
}

/** Whether an arrangement can be drawn at all: no unpinned child left above a pinned group. */
function bandsHold(ordered: readonly Child[]): boolean {
  const firstBelow = ordered.findIndex((child) => bandOfChild(child) === 1);
  return firstBelow < 0 || ordered.slice(firstBelow).every((child) => bandOfChild(child) === 1);
}

/**
 * Turning one parent's children into their new order values.
 *
 * The list that arrives is the arrangement that was asked for; what comes back is one write per
 * child whose group or order actually changed. Ties are the reason this exists: two children may
 * both be at order 0 — an older vault keeps groups and sessions in separate lists — and "put this
 * one between them" cannot be written as a swap of two equal numbers. A child whose numbers
 * already say where it now sits is not written at all.
 */
function arrange(
  groups: readonly SessionGroup[],
  parentId: ParentId,
  ordered: readonly Child[],
): ChildPlacement[] | null {
  if (!bandsHold(ordered)) {
    return null;
  }
  return ordered.flatMap<ChildPlacement>((child, position) => {
    if (child.kind === "group") {
      const parentChanged = effectiveParentId(groups, child.group.id) !== parentId;
      if (!parentChanged && child.group.order === position) {
        return [];
      }
      return [{ kind: "group" as const, id: child.group.id, parentId, order: position }];
    }
    const parentChanged = sessionParentId(groups, child.session) !== parentId;
    if (!parentChanged && child.session.order === position) {
      return [];
    }
    return [{ kind: "session" as const, id: child.session.id, parentId, order: position }];
  });
}

/** Where a drop lands: which parent, and beside which child (none means at the end). */
interface Landing {
  readonly parentId: ParentId;
  readonly anchor: Child | null;
}

/**
 * The place a drop asks for, or nothing when no drop there is possible.
 *
 * The rules that refuse a drop are the ones the list cannot draw: a pinned group moved out of its
 * band, a session put above a pinned group, a group dropped beside itself. The band guards are
 * here rather than in the menu because the drag must not draw a line where nothing can land.
 */
function landingFor(
  groups: readonly SessionGroup[],
  sessions: readonly Session[],
  dragged: Child,
  drop: Drop,
  zone: DropZone,
): Landing | null {
  if (drop.kind === "root") {
    return { parentId: null, anchor: null };
  }

  if (drop.kind === "group") {
    const target = groups.find((group) => group.id === drop.id);
    if (!target) {
      return null;
    }
    if (zone === "inside") {
      return { parentId: target.id, anchor: null };
    }
    if (childId(dragged) === target.id) {
      return null;
    }
    if (dragged.kind === "group" && dragged.group.pinned !== target.pinned) {
      return null;
    }
    if (dragged.kind === "session" && target.pinned) {
      return null;
    }
    return { parentId: effectiveParentId(groups, target.id), anchor: { kind: "group", group: target } };
  }

  const target = sessions.find((session) => session.id === drop.id);
  if (!target || zone === "inside" || childId(dragged) === target.id) {
    return null;
  }
  // A pinned group is drawn above every session beside it, so a drop between sessions cannot be
  // the place it lands; the line would be a promise the release could not keep.
  if (dragged.kind === "group" && dragged.group.pinned) {
    return null;
  }
  return { parentId: sessionParentId(groups, target), anchor: { kind: "session", session: target } };
}

/**
 * The one path both drops take: resolve the place, splice the dragged child in, renumber, and
 * answer with the writes. `null` means the drop would change nothing or cannot be drawn.
 */
function planDrop(
  groups: readonly SessionGroup[],
  sessions: readonly Session[],
  dragged: Child,
  drop: Drop,
  zone: DropZone,
): ChildPlacement[] | null {
  const landing = landingFor(groups, sessions, dragged, drop, zone);
  if (!landing) {
    return null;
  }

  if (
    dragged.kind === "group" &&
    (landing.parentId === dragged.group.id ||
      (landing.parentId !== null && isDescendant(groups, dragged.group.id, landing.parentId)))
  ) {
    return null;
  }

  const current = childrenInOrder(groups, sessions, landing.parentId);
  const inside = current.some((child) => sameChild(child, dragged));
  const siblings = current.filter((child) => !sameChild(child, dragged));
  let index = siblings.length;
  if (landing.anchor !== null) {
    const at = siblings.findIndex((child) => sameChild(child, landing.anchor ?? dragged));
    // An anchor that is not among the siblings cannot happen — it was found in the library and its
    // parent is `landing.parentId` — but an append is the answer that neither crashes nor loses
    // the drop.
    index = at < 0 ? siblings.length : at + (zone === "before" ? 0 : 1);
  }
  const ordered = [...siblings.slice(0, index), dragged, ...siblings.slice(index)];

  const unchanged = inside && current.map(childId).join() === ordered.map(childId).join();
  if (unchanged) {
    return null;
  }
  return arrange(groups, landing.parentId, ordered);
}

/**
 * What dropping a group there writes — or nothing, when the drop is refused or changes nothing.
 *
 * `zone` is the band of the row that was hit: a group has three, a session two. A group dropped at
 * the ends of another group lands beside it at that group's parent; dropped on a session row it
 * lands beside that session. Both are how groups and sessions interleave.
 */
export function planGroupDrop(
  groups: readonly SessionGroup[],
  sessions: readonly Session[],
  draggedId: string,
  drop: Drop,
  zone: DropZone = "inside",
): ChildPlacement[] | null {
  const group = groups.find((entry) => entry.id === draggedId);
  if (!group) {
    return null;
  }
  return planDrop(groups, sessions, { kind: "group", group }, drop, zone);
}

/**
 * What dropping a session there writes — or nothing, when it would stay where it is.
 *
 * Three drops exist for a session. Onto a group (or the bucket at the root) it goes inside that
 * parent, at the end of its children: a heading names a folder rather than a place between two
 * rows. Beside another session or a group it lands before or after that row — which is also how it
 * changes parents, since the row it is put beside is drawn in the parent it will join.
 */
export function planSessionDrop(
  groups: readonly SessionGroup[],
  sessions: readonly Session[],
  sessionId: string,
  drop: Drop,
  zone: DropZone = "inside",
): ChildPlacement[] | null {
  const session = sessions.find((entry) => entry.id === sessionId);
  if (!session) {
    return null;
  }
  return planDrop(groups, sessions, { kind: "session", session }, drop, zone);
}

/**
 * What moving a group one place up or down writes.
 *
 * One place among the children it is drawn beside, in its own band: a pinned group steps over the
 * pinned ones, and a group below the line steps over the others below it — a session in between is
 * stepped over, not around.
 */
export function planSiblingMove(
  groups: readonly SessionGroup[],
  sessions: readonly Session[],
  id: string,
  direction: -1 | 1,
): ChildPlacement[] | null {
  const group = groups.find((entry) => entry.id === id);
  if (!group) {
    return null;
  }
  const parentId = effectiveParentId(groups, id);
  const candidates = stepCandidates(groups, sessions, id);
  const at = candidates.findIndex((child) => childId(child) === id);
  const target = at + direction;
  const neighbour = candidates[target];
  if (at < 0 || !neighbour) {
    return null;
  }

  const current = childrenInOrder(groups, sessions, parentId);
  const draggedAt = current.findIndex((child) => childId(child) === id);
  const neighbourAt = current.findIndex((child) => sameChild(child, neighbour));
  if (draggedAt < 0 || neighbourAt < 0) {
    return null;
  }
  const ordered = [...current];
  ordered[draggedAt] = neighbour;
  ordered[neighbourAt] = { kind: "group", group };
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
