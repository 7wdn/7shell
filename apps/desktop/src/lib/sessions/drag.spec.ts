/**
 * Reading a pointer: which band of a row it is in, and what dropping there would mean.
 *
 * The bands are the only thing that asks about height, and they exist for groups only. A session
 * row is one band from top to bottom, because a session has no place between two things to be put.
 */
import { describe, expect, it } from "vitest";

import { groupDropFor, sessionDropFor, zoneIn } from "./drag";

/** A row from y=100 to y=140, which is all the banding needs to know. */
const box = { top: 100, height: 40 };

describe("the bands of a group row", () => {
  it("reads the top and the bottom as the places beside the row", () => {
    expect(zoneIn(box, 102)).toBe("before");
    expect(zoneIn(box, 138)).toBe("after");
  });

  it("reads the middle as the row itself", () => {
    expect(zoneIn(box, 120)).toBe("inside");
  });

  it("treats a row with no height at all as one band rather than dividing by zero", () => {
    expect(zoneIn({ top: 100, height: 0 }, 100)).toBe("before");
  });
});

describe("what dropping on a row would mean", () => {
  it("sends a dragged group into the middle of a group and beside it at the ends", () => {
    const target = { kind: "group", id: "group-2" } as const;

    expect(groupDropFor(target, box, 120)).toEqual({ drop: { kind: "group", id: "group-2" }, zone: "inside" });
    expect(groupDropFor(target, box, 102)).toEqual({ drop: { kind: "group", id: "group-2" }, zone: "before" });
    expect(groupDropFor(target, box, 138)).toEqual({ drop: { kind: "group", id: "group-2" }, zone: "after" });
  });

  it("sends a dragged group into the group a session row is drawn in, whatever band was hit", () => {
    expect(groupDropFor({ kind: "session", parentId: "group-2" }, box, 102)).toEqual({
      drop: { kind: "group", id: "group-2" },
      zone: "inside",
    });
  });

  it("sends a session into the group it was dropped on or through", () => {
    expect(sessionDropFor({ kind: "group", id: "group-2" })).toEqual({
      drop: { kind: "group", id: "group-2" },
      zone: "inside",
    });
    expect(sessionDropFor({ kind: "session", parentId: "group-2" })).toEqual({
      drop: { kind: "group", id: "group-2" },
      zone: "inside",
    });
  });

  it("sends either out to the root when the row it lands on is in no group", () => {
    expect(groupDropFor({ kind: "ungrouped" }, box, 120)).toEqual({ drop: { kind: "root" }, zone: "inside" });
    expect(groupDropFor({ kind: "session", parentId: null }, box, 120)).toEqual({
      drop: { kind: "root" },
      zone: "inside",
    });
    expect(sessionDropFor({ kind: "ungrouped" })).toEqual({ drop: { kind: "root" }, zone: "inside" });
    expect(sessionDropFor({ kind: "session", parentId: null })).toEqual({ drop: { kind: "root" }, zone: "inside" });
  });
});
