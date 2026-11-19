/**
 * The rules of the session tree, in the order somebody rearranges it.
 *
 * Three kinds of case carry the weight here. A **vault written by hand**: groups that share an
 * order number, orders with gaps, a parent that is gone, and a pair of groups that name each other
 * as parents — none of which may make the list disappear or the walk never end. A **drop that must
 * be refused**: a group into itself, or into anything under it. And a **drop that must change
 * nothing**: putting something back where it already is writes no file at all.
 *
 * Groups and sessions share one order per parent, so the cases that matter most are the ones where
 * the two kinds stand beside each other: a session between two groups, a group between two
 * sessions, and the pinned band that neither may cross.
 */
import { describe, expect, it } from "vitest";

import {
  type ChildPlacement,
  childrenInOrder,
  childrenOf,
  drawOrder,
  effectiveParentId,
  groupPath,
  hasSibling,
  isDescendant,
  nextOrder,
  planGroupDrop,
  planSessionDrop,
  planSiblingMove,
  sessionParentId,
} from "./groups";
import type { Session, SessionGroup } from "@/types/session";

function group(id: string, name: string, order: number, parentId?: string, pinned = false): SessionGroup {
  return { id, name, order, parentId, pinned };
}

const work = group("group-1", "Work", 0);
const lab = group("group-2", "Lab", 1);
const home = group("group-3", "Home", 2);

const web: Session = {
  id: "session-1",
  name: "web",
  protocol: "ssh",
  host: "example.test",
  port: 22,
  user: "root",
  auth: { method: "password", hasSecret: false },
  terminal: {
    terminalType: "xterm-256color",
    keepaliveSeconds: 30,
    encoding: "utf-8",
    backspace: "del",
    forwardAgent: false,
    record: false,
  },
  updatedAt: 1_700_000_000,
};

/** Applying a plan to the groups it was computed from, which is what the vault would end up with. */
function applied(groups: readonly SessionGroup[], plan: readonly ChildPlacement[]): SessionGroup[] {
  return groups.map((entry) => {
    const write = plan.find((placement) => placement.kind === "group" && placement.id === entry.id);
    return write ? { ...entry, parentId: write.parentId ?? undefined, order: write.order } : entry;
  });
}

function order(groups: readonly SessionGroup[], parentId: string | null): string[] {
  return childrenOf(groups, parentId).map((entry) => entry.name);
}

/** The names of one parent's children as the list draws them, groups and sessions together. */
function drawn(groups: readonly SessionGroup[], sessions: readonly Session[], parentId: string | null): string[] {
  return childrenInOrder(groups, sessions, parentId).map((child) =>
    child.kind === "group" ? child.group.name : child.session.name,
  );
}

/** Applying a plan of both kinds to the library it was computed from. */
function appliedAll(
  groups: readonly SessionGroup[],
  sessions: readonly Session[],
  plan: readonly ChildPlacement[],
): { groups: SessionGroup[]; sessions: Session[] } {
  const groupWrites = new Map(plan.filter((entry) => entry.kind === "group").map((entry) => [entry.id, entry]));
  const sessionWrites = new Map(plan.filter((entry) => entry.kind === "session").map((entry) => [entry.id, entry]));
  return {
    groups: groups.map((entry) => {
      const write = groupWrites.get(entry.id);
      return write ? { ...entry, parentId: write.parentId ?? undefined, order: write.order } : entry;
    }),
    sessions: sessions.map((entry) => {
      const write = sessionWrites.get(entry.id);
      return write ? { ...entry, groupId: write.parentId ?? undefined, order: write.order } : entry;
    }),
  };
}

function drawnAfter(
  groups: readonly SessionGroup[],
  sessions: readonly Session[],
  plan: readonly ChildPlacement[],
  parentId: string | null = null,
): string[] {
  const next = appliedAll(groups, sessions, plan);
  return drawn(next.groups, next.sessions, parentId);
}

describe("where a group sits", () => {
  it("draws each parent's children by their order, not by the way they were listed", () => {
    expect(order([home, work, lab], null)).toEqual(["Work", "Lab", "Home"]);
  });

  it("keeps groups that share an order in the order they arrived, so the list does not shuffle", () => {
    const first = group("a", "First", 0);
    const second = group("b", "Second", 0);

    expect(childrenOf([first, second], null).map((entry) => entry.name)).toEqual(["First", "Second"]);
    expect(childrenOf([second, first], null).map((entry) => entry.name)).toEqual(["Second", "First"]);
  });

  it("nests: a child is under its parent and not at the root", () => {
    const inside = group("group-4", "Rack", 0, work.id);

    expect(effectiveParentId([work, inside], inside.id)).toBe(work.id);
    expect(order([work, inside, lab], null)).toEqual(["Work", "Lab"]);
    expect(order([work, inside, lab], work.id)).toEqual(["Rack"]);
  });

  it("treats a parent that is gone as the root, so nothing vanishes with it", () => {
    const orphan = group("group-5", "Orphan", 0, "group-that-is-gone");

    expect(effectiveParentId([orphan], orphan.id)).toBeNull();
    expect(order([orphan], null)).toEqual(["Orphan"]);
  });

  it("treats a chain that loops back as the root, so the walk ends", () => {
    const one = group("a", "One", 0, "b");
    const two = group("b", "Two", 1, "a");
    const self = group("c", "Self", 2, "c");

    expect(effectiveParentId([one, two], "a")).toBeNull();
    expect(effectiveParentId([one, two], "b")).toBeNull();
    expect(effectiveParentId([self], "c")).toBeNull();
    expect(order([one, two, self], null)).toEqual(["One", "Two", "Self"]);
  });

  it("puts a new group after every child under the same parent, sessions included", () => {
    const inside = group("group-4", "Rack", 3, work.id);
    const placed = { ...web, id: "session-9", name: "placed", order: 0 };

    expect(nextOrder([work, lab], [], null)).toBe(2);
    expect(nextOrder([work, inside, lab], [], work.id)).toBe(4);
    expect(nextOrder([work], [placed], null)).toBe(1);
    expect(nextOrder([], [], null)).toBe(0);
  });

  it("walks the tree depth first, with the depth each group sits at", () => {
    const inside = group("group-4", "Rack", 0, work.id);
    const deeper = group("group-5", "Shelf", 0, inside.id);

    expect(drawOrder([work, lab, inside, deeper]).map((entry) => [entry.group.name, entry.depth])).toEqual([
      ["Work", 0],
      ["Rack", 1],
      ["Shelf", 2],
      ["Lab", 0],
    ]);
  });

  it("names a group by its path, because two groups may share a name", () => {
    const inside = group("group-4", "Lab", 0, work.id);

    expect(groupPath([work, inside], inside.id)).toBe("Work / Lab");
    expect(groupPath([work, inside], work.id)).toBe("Work");
    expect(groupPath([work, inside], "group-that-is-gone")).toBe("group-that-is-gone");
  });

  it("knows what sits under what", () => {
    const inside = group("group-4", "Rack", 0, work.id);
    const deeper = group("group-5", "Shelf", 0, inside.id);

    expect(isDescendant([work, lab, inside, deeper], work.id, deeper.id)).toBe(true);
    expect(isDescendant([work, lab, inside, deeper], deeper.id, work.id)).toBe(false);
    expect(isDescendant([work, lab], work.id, work.id)).toBe(false);
  });

  it("knows which end of its own parent's list a group is at", () => {
    const inside = group("group-4", "Rack", 0, work.id);

    expect(hasSibling([work, lab, inside], [], work.id, -1)).toBe(false);
    expect(hasSibling([work, lab, inside], [], work.id, 1)).toBe(true);
    // A nested group's neighbours are its own siblings, not the root's.
    expect(hasSibling([work, lab, inside], [], inside.id, -1)).toBe(false);
    expect(hasSibling([work, lab, inside], [], inside.id, 1)).toBe(false);
    // A session between two groups is a neighbour a group can step past.
    const between = { ...web, id: "session-9", name: "between", order: 1 };
    const spaced = [work, { ...lab, order: 2 }, { ...home, order: 3 }];
    expect(hasSibling(spaced, [between], work.id, 1)).toBe(true);
    const plan = planSiblingMove(spaced, [between], work.id, 1);
    expect(drawnAfter(spaced, [between], plan ?? [])).toEqual(["between", "Work", "Lab", "Home"]);
  });
});

describe("a vault written before sessions had places", () => {
  it("draws its groups first and its sessions after, whatever the numbers say", () => {
    const first = { ...web, id: "session-1", name: "first" };
    const second = { ...web, id: "session-2", name: "second", order: 0 };

    // A parent holding an unplaced session is one nobody has dragged in since sessions carried a
    // place; its old drawing — every group, then every session — is what stays on screen.
    expect(drawn([work, lab], [first, second], null)).toEqual(["Work", "Lab", "first", "second"]);
  });

  it("keeps an arrangement that already interleaves when every session carries a place", () => {
    const early = { ...web, id: "session-1", name: "early", order: 0 };
    const spaced = [
      { ...work, order: 1 },
      { ...lab, order: 2 },
    ];

    expect(drawn(spaced, [early], null)).toEqual(["early", "Work", "Lab"]);
  });
});

describe("a pinned group", () => {
  const rack = group("group-4", "Rack", 7, undefined, true);
  const shelf = group("group-5", "Shelf", 8, undefined, true);

  it("is drawn above everything beside it, whatever order it carries", () => {
    const loose = { ...web, id: "session-9", name: "loose", order: 0 };

    expect(order([rack, work, lab, home], null)).toEqual(["Rack", "Work", "Lab", "Home"]);
    // A session with the lowest order still draws under the pinned band, and above an unpinned
    // group whose order is higher.
    expect(drawn([rack, { ...work, order: 3 }], [loose], null)).toEqual(["Rack", "loose", "Work"]);
  });

  it("keeps the arrangement under the pin, so taking the pin off returns it where it was", () => {
    // Nothing about the order was rewritten when it was pinned: Rack's own place is still last.
    expect(order([rack, work, lab, home], null)).toEqual(["Rack", "Work", "Lab", "Home"]);
    expect(order([{ ...rack, pinned: false }, work, lab, home], null)).toEqual(["Work", "Lab", "Home", "Rack"]);
  });

  it("pins inside its own parent, so a pinned subgroup stays under it", () => {
    const inside = group("group-6", "Drawer", 9, work.id, true);

    expect(order([work, lab, inside], null)).toEqual(["Work", "Lab"]);
    expect(order([work, lab, inside], work.id)).toEqual(["Drawer"]);
  });

  it("is only moved past the ones it is drawn beside", () => {
    const groups = [rack, shelf, work, lab];

    // Work's own band is the two unpinned ones, so there is nothing above it to move past.
    expect(hasSibling(groups, [], work.id, -1)).toBe(false);
    expect(hasSibling(groups, [], lab.id, -1)).toBe(true);
    expect(hasSibling(groups, [], shelf.id, -1)).toBe(true);
    expect(hasSibling(groups, [], rack.id, -1)).toBe(false);

    const plan = planSiblingMove(groups, [], shelf.id, -1);
    expect(order(applied(groups, plan ?? []), null)).toEqual(["Shelf", "Rack", "Work", "Lab"]);
  });

  it("refuses a drop across the pinned line, which is a place the list cannot draw", () => {
    const groups = [rack, work, lab];

    // An unpinned group above a pinned one, and a pinned one below an unpinned one: both are
    // arrangements the sort would undo the moment they were written.
    expect(planGroupDrop(groups, [], lab.id, { kind: "group", id: rack.id }, "before")).toBeNull();
    expect(planGroupDrop(groups, [], rack.id, { kind: "group", id: lab.id }, "after")).toBeNull();

    // Within its own band it still moves, and the pinned group is left where it is.
    const plan = planGroupDrop(groups, [], lab.id, { kind: "group", id: work.id }, "before");
    expect(order(applied(groups, plan ?? []), null)).toEqual(["Rack", "Lab", "Work"]);
  });

  it("refuses a session above the pinned band, which would draw where it was not put", () => {
    const groups = [rack, work];
    const loose = { ...web, id: "session-9", name: "loose", order: 1 };

    expect(planSessionDrop(groups, [loose], loose.id, { kind: "group", id: rack.id }, "before")).toBeNull();
    expect(planSessionDrop(groups, [loose], loose.id, { kind: "group", id: rack.id }, "after")).toBeNull();
    // Inside a pinned group is a place the sort can draw: the session is that group's child.
    const plan = planSessionDrop(groups, [loose], loose.id, { kind: "group", id: rack.id }, "inside");
    expect(plan).toEqual([{ kind: "session", id: "session-9", parentId: "group-4", order: 0 }]);
  });
});

describe("dropping a group somewhere", () => {
  it("puts it inside the group it was dropped on, at the end of that group's children", () => {
    const inside = group("group-4", "Rack", 0, lab.id);

    const plan = planGroupDrop([work, lab, home, inside], [], work.id, { kind: "group", id: lab.id }, "inside");

    expect(plan).toEqual([{ kind: "group", id: work.id, parentId: lab.id, order: 1 }]);
    expect(order(applied([work, lab, home, inside], plan ?? []), lab.id)).toEqual(["Rack", "Work"]);
  });

  it("puts it beside the group it was dropped next to", () => {
    const plan = planGroupDrop([work, lab, home], [], home.id, { kind: "group", id: work.id }, "before");

    // Everything the dragged group passed is renumbered, because two groups at the same order
    // cannot be told apart: what has to come out is the arrangement, not the smallest diff.
    expect(plan).toEqual([
      { kind: "group", id: "group-3", parentId: null, order: 0 },
      { kind: "group", id: "group-1", parentId: null, order: 1 },
      { kind: "group", id: "group-2", parentId: null, order: 2 },
    ]);
    expect(order(applied([work, lab, home], plan ?? []), null)).toEqual(["Home", "Work", "Lab"]);
  });

  it("lands it after the group it was dropped under, not on top of it", () => {
    const plan = planGroupDrop([work, lab, home], [], work.id, { kind: "group", id: lab.id }, "after");

    expect(order(applied([work, lab, home], plan ?? []), null)).toEqual(["Lab", "Work", "Home"]);
  });

  it("puts a group between two sessions", () => {
    const one = { ...web, id: "session-1", name: "one", order: 0 };
    const two = { ...web, id: "session-2", name: "two", order: 2 };

    const plan = planGroupDrop([{ ...work, order: 3 }], [one, two], work.id, { kind: "session", id: two.id }, "before");

    expect(drawn(applied([{ ...work, order: 3 }], plan ?? []), [one, two], null)).toEqual(["one", "Work", "two"]);
  });

  it("takes it out to the root when the drop says root", () => {
    const inside = group("group-4", "Rack", 0, work.id);

    const plan = planGroupDrop([work, lab, inside], [], inside.id, { kind: "root" });

    expect(plan).toEqual([{ kind: "group", id: "group-4", parentId: null, order: 2 }]);
    expect(order(applied([work, lab, inside], plan ?? []), null)).toEqual(["Work", "Lab", "Rack"]);
  });

  it("renumbers the ties, because two equal orders cannot be told apart", () => {
    const groups = [group("a", "First", 0), group("b", "Second", 0), group("c", "Third", 0)];

    const plan = planGroupDrop(groups, [], "c", { kind: "group", id: "a" }, "before");

    expect(order(applied(groups, plan ?? []), null)).toEqual(["Third", "First", "Second"]);
  });

  it("refuses to put a group inside itself", () => {
    expect(planGroupDrop([work, lab], [], work.id, { kind: "group", id: work.id }, "inside")).toBeNull();
  });

  it("refuses to put a group under something that is already under it", () => {
    const inside = group("group-4", "Rack", 0, work.id);
    const deeper = group("group-5", "Shelf", 0, inside.id);
    const groups = [work, lab, inside, deeper];

    expect(planGroupDrop(groups, [], work.id, { kind: "group", id: inside.id }, "inside")).toBeNull();
    expect(planGroupDrop(groups, [], work.id, { kind: "group", id: deeper.id }, "inside")).toBeNull();
    // Beside a descendant is the same refusal: the new parent would be the descendant's parent,
    // which is itself under the dragged group.
    expect(planGroupDrop(groups, [], work.id, { kind: "group", id: deeper.id }, "after")).toBeNull();
  });

  it("writes nothing when the drop would leave the group exactly where it is", () => {
    const groups = [work, lab, home];

    // Already last among the root's children, and already nowhere near the middle of anything.
    expect(planGroupDrop(groups, [], home.id, { kind: "root" })).toBeNull();
    // Dropped just after the group it already follows.
    expect(planGroupDrop(groups, [], lab.id, { kind: "group", id: work.id }, "after")).toBeNull();
    // Dropped into the parent it is already the last child of.
    const inside = group("group-4", "Rack", 0, work.id);
    expect(planGroupDrop([work, lab, inside], [], inside.id, { kind: "group", id: work.id }, "inside")).toBeNull();
  });

  it("writes nothing for a group or a target that is not there", () => {
    expect(planGroupDrop([work], [], "group-that-is-gone", { kind: "root" })).toBeNull();
    expect(planGroupDrop([work], [], work.id, { kind: "group", id: "group-that-is-gone" }, "inside")).toBeNull();
  });
});

describe("moving a group one place among its own siblings", () => {
  it("trades it with the neighbour it was moved towards", () => {
    const plan = planSiblingMove([work, lab, home], [], lab.id, -1);

    expect(plan).toEqual([
      { kind: "group", id: "group-2", parentId: null, order: 0 },
      { kind: "group", id: "group-1", parentId: null, order: 1 },
    ]);
    expect(order(applied([work, lab, home], plan ?? []), null)).toEqual(["Lab", "Work", "Home"]);
  });

  it("moves within its own parent, never past a group that is not its sibling", () => {
    const inside = group("group-4", "Rack", 0, work.id);
    const deeper = group("group-5", "Shelf", 1, work.id);

    const plan = planSiblingMove([work, lab, inside, deeper], [], deeper.id, -1);

    expect(plan?.map((placement) => placement.id)).toEqual(["group-5", "group-4"]);
    expect(order(applied([work, lab, inside, deeper], plan ?? []), work.id)).toEqual(["Shelf", "Rack"]);
  });

  it("writes nothing at either end of the list", () => {
    expect(planSiblingMove([work, lab, home], [], work.id, -1)).toBeNull();
    expect(planSiblingMove([work, lab, home], [], home.id, 1)).toBeNull();
    expect(planSiblingMove([work], [], "group-that-is-gone", 1)).toBeNull();
  });
});

describe("dropping a session somewhere", () => {
  /** Three sessions of Work in the arrangement the list was dragged into. */
  const db: Session = { ...web, id: "session-2", name: "db", groupId: work.id, order: 1 };
  const cache: Session = { ...web, id: "session-3", name: "cache", groupId: work.id, order: 2 };
  const listed = { ...web, id: "session-4", name: "web", groupId: work.id, order: 0 };

  it("moves it into the group it was dropped on, at the end of that group's children", () => {
    expect(planSessionDrop([work, lab], [web], web.id, { kind: "group", id: lab.id })).toEqual([
      { kind: "session", id: "session-1", parentId: "group-2", order: 0 },
    ]);
  });

  it("takes it out of every group when the drop says root", () => {
    const inWork = { ...web, groupId: work.id };

    expect(planSessionDrop([work, lab], [inWork], inWork.id, { kind: "root" })).toEqual([
      { kind: "session", id: "session-1", parentId: null, order: 2 },
    ]);
  });

  it("writes nothing when it is already there", () => {
    const inWork = { ...web, groupId: work.id };

    expect(planSessionDrop([work, lab], [inWork], inWork.id, { kind: "group", id: work.id })).toBeNull();
    expect(planSessionDrop([work, lab], [web], web.id, { kind: "root" })).toBeNull();
  });

  it("writes nothing for a group that is not there", () => {
    expect(planSessionDrop([work], [web], web.id, { kind: "group", id: "group-that-is-gone" })).toBeNull();
  });

  it("puts a session before the neighbour it was dropped on and renumbers the rest", () => {
    const plan = planSessionDrop([work], [listed, db, cache], cache.id, { kind: "session", id: listed.id }, "before");

    expect(plan).toEqual([
      { kind: "session", id: "session-3", parentId: "group-1", order: 0 },
      { kind: "session", id: "session-4", parentId: "group-1", order: 1 },
      { kind: "session", id: "session-2", parentId: "group-1", order: 2 },
    ]);
  });

  it("puts a session after the neighbour it was dropped on", () => {
    const plan = planSessionDrop([work], [listed, db, cache], listed.id, { kind: "session", id: db.id }, "after");

    // db steps over listed (session-4); cache stays third, so only the two that moved are written.
    expect(plan).toEqual([
      { kind: "session", id: "session-2", parentId: "group-1", order: 0 },
      { kind: "session", id: "session-4", parentId: "group-1", order: 1 },
    ]);
  });

  it("writes nothing when the session is already beside that neighbour", () => {
    expect(planSessionDrop([work], [listed, db, cache], db.id, { kind: "session", id: listed.id }, "after")).toBeNull();
    expect(planSessionDrop([work], [listed, db, cache], cache.id, { kind: "session", id: db.id }, "after")).toBeNull();
  });

  it("moves a session into the parent of the neighbour it lands beside", () => {
    const plan = planSessionDrop([work, lab], [listed, db, web], web.id, { kind: "session", id: db.id }, "after");

    // web joins Work at the end; the two already there keep their numbers.
    expect(plan).toEqual([{ kind: "session", id: "session-1", parentId: "group-1", order: 2 }]);
  });

  it("puts a session between two groups", () => {
    const plan = planSessionDrop([work, lab], [web], web.id, { kind: "group", id: work.id }, "after");

    expect(plan).toEqual([
      { kind: "session", id: "session-1", parentId: null, order: 1 },
      { kind: "group", id: "group-2", parentId: null, order: 2 },
    ]);
    expect(drawnAfter([work, lab], [web], plan ?? [])).toEqual(["Work", "web", "Lab"]);
  });

  it("refuses a drop beside itself, and a drop that would leave the list as it is", () => {
    expect(planSessionDrop([work], [listed, db], db.id, { kind: "session", id: db.id })).toBeNull();
    expect(planSessionDrop([work], [listed, db], db.id, { kind: "session", id: listed.id }, "after")).toBeNull();
  });

  it("reads a session whose group is gone as one that is in no group", () => {
    const orphan = { ...web, groupId: "group-that-is-gone" };

    expect(sessionParentId([work], orphan)).toBeNull();
    expect(sessionParentId([work], { ...web, groupId: work.id })).toBe(work.id);
    expect(sessionParentId([work], web)).toBeNull();
  });
});
