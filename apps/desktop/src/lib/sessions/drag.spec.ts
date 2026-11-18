/**
 * Reading a pointer: which band of a row it is in, and what dropping there would mean.
 *
 * The bands are the only thing that asks about height, and the two kinds of row are divided
 * differently. A group row has three bands — the top and bottom are the places beside it, the
 * middle is the group itself. A session row has two: the halves are the places a dragged session
 * can land, before the session or after it.
 */
import { describe, expect, it } from "vitest";

import { dropAt, halfIn, zoneIn } from "./drag";

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

describe("the bands of a session row", () => {
  it("reads the top half as before the session and the bottom half as after it", () => {
    expect(halfIn(box, 102)).toBe("before");
    expect(halfIn(box, 138)).toBe("after");
  });

  it("keeps a row with no height at all deciding rather than dividing by zero", () => {
    expect(halfIn({ top: 100, height: 0 }, 100)).toBe("before");
  });
});

describe("what dropping on a row would mean", () => {
  it("sends something into the middle of a group and beside it at the ends", () => {
    const target = { kind: "group", id: "group-2" } as const;

    expect(dropAt(target, box, 120)).toEqual({ drop: { kind: "group", id: "group-2" }, zone: "inside" });
    expect(dropAt(target, box, 102)).toEqual({ drop: { kind: "group", id: "group-2" }, zone: "before" });
    expect(dropAt(target, box, 138)).toEqual({ drop: { kind: "group", id: "group-2" }, zone: "after" });
  });

  it("puts something beside the session row it was dropped on, in the half that was hit", () => {
    const target = { kind: "session", id: "session-9", parentId: "group-2" } as const;

    expect(dropAt(target, box, 105)).toEqual({ drop: { kind: "session", id: "session-9" }, zone: "before" });
    expect(dropAt(target, box, 135)).toEqual({ drop: { kind: "session", id: "session-9" }, zone: "after" });
  });

  it("keeps a root session a place at the root when something lands beside it", () => {
    expect(dropAt({ kind: "session", id: "session-9", parentId: null }, box, 120)).toEqual({
      drop: { kind: "session", id: "session-9" },
      zone: "after",
    });
  });
});
