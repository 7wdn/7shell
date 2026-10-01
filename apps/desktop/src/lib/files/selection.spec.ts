/**
 * What each gesture leaves selected.
 *
 * The whole point of this file is that the platform's conventions are written down: which click moves
 * the range's start, whether the keyboard carries the selection with it, and what happens to a
 * selection when the directory underneath it is read again.
 */
import { describe, expect, it } from "vitest";

import {
  NOTHING_SELECTED,
  clear,
  jumpLead,
  moveLead,
  retain,
  selectAll,
  selectOne,
  selectRange,
  selectedEntries,
  toggle,
} from "@/lib/files/selection";
import type { SftpEntry } from "@/types/session";

const rows = ["/home/a.txt", "/home/b.txt", "/home/c.txt", "/home/d.txt"];

describe("a plain click", () => {
  it("selects that row and forgets the rest", () => {
    const selection = selectOne("/home/b.txt");

    expect(selection.paths).toEqual(["/home/b.txt"]);
    expect(selection.anchor).toBe("/home/b.txt");
    expect(selection.lead).toBe("/home/b.txt");
  });
});

describe("a ctrl-click", () => {
  it("adds a row to what is already selected", () => {
    const selection = toggle(selectOne("/home/a.txt"), "/home/c.txt");

    expect(selection.paths).toEqual(["/home/a.txt", "/home/c.txt"]);
  });

  it("takes a row back out again, and leaves the keyboard on it", () => {
    const two = toggle(selectOne("/home/a.txt"), "/home/c.txt");
    const one = toggle(two, "/home/a.txt");

    expect(one.paths).toEqual(["/home/c.txt"]);
    // The anchor and the lead follow the pointer even when the row leaves the selection, so a
    // shift-click straight afterwards ranges from the row that was just touched.
    expect(one.anchor).toBe("/home/a.txt");
    expect(one.lead).toBe("/home/a.txt");
  });

  it("can leave nothing selected, which is not the same as never having clicked", () => {
    const empty = toggle(selectOne("/home/a.txt"), "/home/a.txt");

    expect(empty.paths).toEqual([]);
    expect(empty.anchor).toBe("/home/a.txt");
  });
});

describe("a shift-click", () => {
  it("takes everything between the last plain click and this one", () => {
    const selection = selectRange(selectOne("/home/b.txt"), rows, "/home/d.txt");

    expect(selection.paths).toEqual(["/home/b.txt", "/home/c.txt", "/home/d.txt"]);
  });

  it("works upwards too, and keeps the range's start where it was", () => {
    const selection = selectRange(selectOne("/home/c.txt"), rows, "/home/a.txt");

    expect(selection.paths).toEqual(["/home/a.txt", "/home/b.txt", "/home/c.txt"]);
    expect(selection.anchor).toBe("/home/c.txt");
    expect(selection.lead).toBe("/home/a.txt");
  });

  it("grows and shrinks from the same anchor as the pointer moves", () => {
    const anchored = selectOne("/home/a.txt");
    const grown = selectRange(anchored, rows, "/home/d.txt");
    const shrunk = selectRange(grown, rows, "/home/b.txt");

    expect(grown.paths).toHaveLength(4);
    expect(shrunk.paths).toEqual(["/home/a.txt", "/home/b.txt"]);
  });

  it("is a plain click when nothing has been clicked yet", () => {
    expect(selectRange(NOTHING_SELECTED, rows, "/home/c.txt").paths).toEqual(["/home/c.txt"]);
  });
});

describe("selecting everything", () => {
  it("takes the whole listing, anchored at both ends", () => {
    const selection = selectAll(rows);

    expect(selection.paths).toEqual(rows);
    expect(selection.anchor).toBe("/home/a.txt");
    expect(selection.lead).toBe("/home/d.txt");
  });

  it("selects nothing in an empty directory", () => {
    expect(selectAll([])).toEqual(NOTHING_SELECTED);
  });
});

describe("the keyboard", () => {
  it("carries the selection with it, one row at a time", () => {
    expect(moveLead(selectOne("/home/a.txt"), rows, 1, false).paths).toEqual(["/home/b.txt"]);
    expect(moveLead(selectOne("/home/c.txt"), rows, -1, false).paths).toEqual(["/home/b.txt"]);
  });

  it("grows a range instead when shift is held", () => {
    const selection = moveLead(selectOne("/home/a.txt"), rows, 1, true);
    expect(selection.paths).toEqual(["/home/a.txt", "/home/b.txt"]);

    const grown = moveLead(selection, rows, 1, true);
    expect(grown.paths).toEqual(["/home/a.txt", "/home/b.txt", "/home/c.txt"]);
    expect(grown.anchor).toBe("/home/a.txt");
  });

  it("stops at the ends rather than wrapping around", () => {
    expect(moveLead(selectOne("/home/d.txt"), rows, 1, false).paths).toEqual(["/home/d.txt"]);
    expect(moveLead(selectOne("/home/a.txt"), rows, -1, false).paths).toEqual(["/home/a.txt"]);
  });

  it("starts from the bottom when a walk upwards begins with nothing chosen", () => {
    expect(moveLead(NOTHING_SELECTED, rows, -1, false).paths).toEqual(["/home/d.txt"]);
  });

  it("jumps to either end for Home and End", () => {
    expect(jumpLead(NOTHING_SELECTED, rows, "first", false).paths).toEqual(["/home/a.txt"]);
    expect(jumpLead(NOTHING_SELECTED, rows, "last", false).paths).toEqual(["/home/d.txt"]);
    expect(jumpLead(selectOne("/home/b.txt"), rows, "last", true).paths).toEqual([
      "/home/b.txt",
      "/home/c.txt",
      "/home/d.txt",
    ]);
  });

  it("has nothing to do in an empty directory", () => {
    expect(moveLead(NOTHING_SELECTED, [], 1, false)).toEqual(NOTHING_SELECTED);
    expect(jumpLead(NOTHING_SELECTED, [], "first", true)).toEqual(NOTHING_SELECTED);
  });
});

describe("clicking away", () => {
  it("leaves nothing selected", () => {
    expect(clear()).toEqual(NOTHING_SELECTED);
  });
});

describe("a directory that is read again", () => {
  it("drops the rows that are gone", () => {
    const three = selectRange(selectOne("/home/a.txt"), rows, "/home/c.txt");
    const after = retain(three, ["/home/a.txt", "/home/c.txt", "/home/d.txt"]);

    expect(after.paths).toEqual(["/home/a.txt", "/home/c.txt"]);
  });

  it("keeps the order the listing draws, not the order they were clicked in", () => {
    const clicked = toggle(toggle(selectOne("/home/c.txt"), "/home/a.txt"), "/home/d.txt");
    const after = retain(clicked, rows);

    expect(after.paths).toEqual(["/home/a.txt", "/home/c.txt", "/home/d.txt"]);
  });

  it("lets go of an anchor and a lead that are no longer there", () => {
    const three = selectRange(selectOne("/home/b.txt"), rows, "/home/d.txt");
    const after = retain(three, ["/home/a.txt", "/home/c.txt"]);

    expect(after.anchor).toBeNull();
    expect(after.lead).toBeNull();
    expect(after.paths).toEqual(["/home/c.txt"]);
  });

  it("answers with the same value when nothing changed, so a watcher does not fire for nothing", () => {
    const selection = selectOne("/home/b.txt");

    expect(retain(selection, rows)).toBe(selection);
  });
});

describe("what an action is about", () => {
  const entry = (name: string): SftpEntry => ({ name, path: `/home/${name}`, kind: "file", size: 1 });

  it("is the selected entries, in the order the listing draws them", () => {
    const selection = toggle(toggle(selectOne("/home/c.txt"), "/home/a.txt"), "/home/d.txt");

    expect(
      selectedEntries(selection, [entry("a.txt"), entry("b.txt"), entry("c.txt"), entry("d.txt")]).map((it) => it.name),
    ).toEqual(["a.txt", "c.txt", "d.txt"]);
  });

  it("is nothing at all when nothing is selected", () => {
    expect(selectedEntries(NOTHING_SELECTED, [entry("a.txt")])).toEqual([]);
  });
});
