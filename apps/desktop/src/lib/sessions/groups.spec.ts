/**
 * The rules of the group tree, in the order somebody rearranges it.
 *
 * Three kinds of case carry the weight here. A **vault written by hand**: groups that share an
 * order number, orders with gaps, a parent that is gone, and a pair of groups that name each other
 * as parents — none of which may make the list disappear or the walk never end. A **drop that must
 * be refused**: a group into itself, or into anything under it. And a **drop that must change
 * nothing**: putting a group back where it already is writes no file at all.
 */
import { describe, expect, it } from "vitest";

import {
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
  terminal: { terminalType: "xterm-256color", keepaliveSeconds: 30, forwardAgent: false, record: false },
  updatedAt: 1_700_000_000,
};

/** Applying a plan to the groups it was computed from, which is what the vault would end up with. */
function applied(
  groups: readonly SessionGroup[],
  plan: readonly { id: string; parentId: string | null; order: number }[],
) {
  return groups.map((entry) => {
    const write = plan.find((placement) => placement.id === entry.id);
    return write ? { ...entry, parentId: write.parentId ?? undefined, order: write.order } : entry;
  });
}

function order(groups: readonly SessionGroup[], parentId: string | null): string[] {
  return childrenOf(groups, parentId).map((entry) => entry.name);
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

  it("puts a new group after the ones under the same parent", () => {
    const inside = group("group-4", "Rack", 3, work.id);

    expect(nextOrder([work, lab], null)).toBe(2);
    expect(nextOrder([work, inside, lab], work.id)).toBe(4);
    expect(nextOrder([], null)).toBe(0);
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

    expect(hasSibling([work, lab, inside], work.id, -1)).toBe(false);
    expect(hasSibling([work, lab, inside], work.id, 1)).toBe(true);
    // A nested group's neighbours are its own siblings, not the root's.
    expect(hasSibling([work, lab, inside], inside.id, -1)).toBe(false);
    expect(hasSibling([work, lab, inside], inside.id, 1)).toBe(false);
  });
});

describe("a pinned group", () => {
  const rack = group("group-4", "Rack", 7, undefined, true);
  const shelf = group("group-5", "Shelf", 8, undefined, true);

  it("is drawn above the groups it was made beside, whatever order it carries", () => {
    expect(order([rack, work, lab, home], null)).toEqual(["Rack", "Work", "Lab", "Home"]);
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
    expect(hasSibling(groups, work.id, -1)).toBe(false);
    expect(hasSibling(groups, lab.id, -1)).toBe(true);
    expect(hasSibling(groups, shelf.id, -1)).toBe(true);
    expect(hasSibling(groups, rack.id, -1)).toBe(false);

    const plan = planSiblingMove(groups, shelf.id, -1);
    expect(order(applied(groups, plan ?? []), null)).toEqual(["Shelf", "Rack", "Work", "Lab"]);
  });

  it("refuses a drop across the pinned line, which is a place the list cannot draw", () => {
    const groups = [rack, work, lab];

    // An unpinned group above a pinned one, and a pinned one below an unpinned one: both are
    // arrangements the sort would undo the moment they were written.
    expect(planGroupDrop(groups, lab.id, { kind: "group", id: rack.id }, "before")).toBeNull();
    expect(planGroupDrop(groups, rack.id, { kind: "group", id: lab.id }, "after")).toBeNull();

    // Within its own band it still moves, and the pinned group is left where it is.
    const plan = planGroupDrop(groups, lab.id, { kind: "group", id: work.id }, "before");
    expect(order(applied(groups, plan ?? []), null)).toEqual(["Rack", "Lab", "Work"]);
  });
});

describe("dropping a group somewhere", () => {
  it("puts it inside the group it was dropped on, at the end of that group's children", () => {
    const inside = group("group-4", "Rack", 0, lab.id);

    const plan = planGroupDrop([work, lab, home, inside], work.id, { kind: "group", id: lab.id }, "inside");

    expect(plan).toEqual([{ id: work.id, parentId: lab.id, order: 1 }]);
    const after = applied([work, lab, home, inside], plan ?? []);
    expect(order(after, lab.id)).toEqual(["Rack", "Work"]);
  });

  it("puts it beside the group it was dropped next to", () => {
    const plan = planGroupDrop([work, lab, home], home.id, { kind: "group", id: work.id }, "before");

    // Everything the dragged group passed is renumbered, because two groups at the same order
    // cannot be told apart: what has to come out is the arrangement, not the smallest diff.
    expect(plan).toEqual([
      { id: "group-3", parentId: null, order: 0 },
      { id: "group-1", parentId: null, order: 1 },
      { id: "group-2", parentId: null, order: 2 },
    ]);
    expect(order(applied([work, lab, home], plan ?? []), null)).toEqual(["Home", "Work", "Lab"]);
  });

  it("lands it after the group it was dropped under, not on top of it", () => {
    const plan = planGroupDrop([work, lab, home], work.id, { kind: "group", id: lab.id }, "after");

    expect(order(applied([work, lab, home], plan ?? []), null)).toEqual(["Lab", "Work", "Home"]);
  });

  it("takes it out to the root when the drop says root", () => {
    const inside = group("group-4", "Rack", 0, work.id);

    const plan = planGroupDrop([work, lab, inside], inside.id, { kind: "root" });

    expect(plan).toEqual([{ id: "group-4", parentId: null, order: 2 }]);
    expect(order(applied([work, lab, inside], plan ?? []), null)).toEqual(["Work", "Lab", "Rack"]);
  });

  it("renumbers the ties, because two equal orders cannot be told apart", () => {
    const groups = [group("a", "First", 0), group("b", "Second", 0), group("c", "Third", 0)];

    const plan = planGroupDrop(groups, "c", { kind: "group", id: "a" }, "before");

    expect(order(applied(groups, plan ?? []), null)).toEqual(["Third", "First", "Second"]);
  });

  it("refuses to put a group inside itself", () => {
    expect(planGroupDrop([work, lab], work.id, { kind: "group", id: work.id }, "inside")).toBeNull();
  });

  it("refuses to put a group under something that is already under it", () => {
    const inside = group("group-4", "Rack", 0, work.id);
    const deeper = group("group-5", "Shelf", 0, inside.id);
    const groups = [work, lab, inside, deeper];

    expect(planGroupDrop(groups, work.id, { kind: "group", id: inside.id }, "inside")).toBeNull();
    expect(planGroupDrop(groups, work.id, { kind: "group", id: deeper.id }, "inside")).toBeNull();
    // Beside a descendant is the same refusal: the new parent would be the descendant's parent,
    // which is itself under the dragged group.
    expect(planGroupDrop(groups, work.id, { kind: "group", id: deeper.id }, "after")).toBeNull();
  });

  it("writes nothing when the drop would leave the group exactly where it is", () => {
    const groups = [work, lab, home];

    // Already last among the root groups, and already nowhere near the middle of anything.
    expect(planGroupDrop(groups, home.id, { kind: "root" })).toBeNull();
    // Dropped just after the group it already follows.
    expect(planGroupDrop(groups, lab.id, { kind: "group", id: work.id }, "after")).toBeNull();
    // Dropped into the parent it is already the last child of.
    const inside = group("group-4", "Rack", 0, work.id);
    expect(planGroupDrop([work, lab, inside], inside.id, { kind: "group", id: work.id }, "inside")).toBeNull();
  });

  it("writes nothing for a group or a target that is not there", () => {
    expect(planGroupDrop([work], "group-that-is-gone", { kind: "root" })).toBeNull();
    expect(planGroupDrop([work], work.id, { kind: "group", id: "group-that-is-gone" }, "inside")).toBeNull();
  });
});

describe("moving a group one place among its own siblings", () => {
  it("trades it with the neighbour it was moved towards", () => {
    const plan = planSiblingMove([work, lab, home], lab.id, -1);

    expect(plan).toEqual([
      { id: "group-2", parentId: null, order: 0 },
      { id: "group-1", parentId: null, order: 1 },
    ]);
    expect(order(applied([work, lab, home], plan ?? []), null)).toEqual(["Lab", "Work", "Home"]);
  });

  it("moves within its own parent, never past a group that is not its sibling", () => {
    const inside = group("group-4", "Rack", 0, work.id);
    const deeper = group("group-5", "Shelf", 1, work.id);

    const plan = planSiblingMove([work, lab, inside, deeper], deeper.id, -1);

    expect(plan?.map((placement) => placement.id)).toEqual(["group-5", "group-4"]);
    expect(order(applied([work, lab, inside, deeper], plan ?? []), work.id)).toEqual(["Shelf", "Rack"]);
  });

  it("writes nothing at either end of the list", () => {
    expect(planSiblingMove([work, lab, home], work.id, -1)).toBeNull();
    expect(planSiblingMove([work, lab, home], home.id, 1)).toBeNull();
    expect(planSiblingMove([work], "group-that-is-gone", 1)).toBeNull();
  });
});

describe("dropping a session somewhere", () => {
  it("moves it into the group it was dropped on", () => {
    expect(planSessionDrop([work, lab], web, { kind: "group", id: lab.id })).toEqual({ groupId: "group-2" });
  });

  it("takes it out of every group when the drop says root", () => {
    const inWork = { ...web, groupId: work.id };

    expect(planSessionDrop([work, lab], inWork, { kind: "root" })).toEqual({ groupId: null });
  });

  it("writes nothing when it is already there", () => {
    const inWork = { ...web, groupId: work.id };

    expect(planSessionDrop([work, lab], inWork, { kind: "group", id: work.id })).toBeNull();
    expect(planSessionDrop([work, lab], web, { kind: "root" })).toBeNull();
  });

  it("writes nothing for a group that is not there", () => {
    expect(planSessionDrop([work], web, { kind: "group", id: "group-that-is-gone" })).toBeNull();
  });

  it("reads a session whose group is gone as one that is in no group", () => {
    const orphan = { ...web, groupId: "group-that-is-gone" };

    expect(sessionParentId([work], orphan)).toBeNull();
    expect(sessionParentId([work], { ...web, groupId: work.id })).toBe(work.id);
    expect(sessionParentId([work], web)).toBeNull();
  });
});
