/**
 * The arithmetic of a table's columns: what each is drawn at, and where a drag of it stops.
 *
 * The numbers here are a listing's three columns rather than its six, because what is being checked
 * is the shape of the rule and not the width of any particular heading: two columns that carry a
 * width, one that has been moved, and a line that spends something on padding and gaps. The geometry
 * a browser would do is not asserted here — a test document lays nothing out — and the real widths
 * are the e2e run's business.
 */
import { describe, expect, it } from "vitest";

import {
  type ColumnLayout,
  type ColumnLine,
  type ColumnWidth,
  drawnWidths,
  floorOf,
  layoutOf,
  lineMinWidth,
  movableWidth,
  movedTo,
  widthOf,
  widthVars,
} from "@/lib/columns";

/** The shape of a file listing: a name and two columns beside it. */
const COLUMNS: readonly ColumnWidth[] = [
  { key: "name", defaultPx: 320, minPx: 160 },
  { key: "size", defaultPx: 80, minPx: 56 },
  { key: "kind", defaultPx: 80, minPx: 64 },
];

/** `px-3` on each side and `gap-2` between three cells: 24 + 16. */
const LINE: ColumnLine = { paddingPx: 12, gapPx: 8 };

/** A listing nobody has moved anything in. */
const layout = layoutOf(COLUMNS, {}, LINE);

describe("the width a column asks for", () => {
  it("is the width this build draws it at until somebody moves it", () => {
    expect(widthOf(layout, "size")).toBe(80);
    expect(widthOf(layoutOf(COLUMNS, { size: 123 }, LINE), "size")).toBe(123);
  });

  it("is never below the column's floor, whatever was moved before", () => {
    // A width written down by a build whose floor was lower, or by hand in a file: the floor this
    // build states is the one that holds.
    expect(widthOf(layoutOf(COLUMNS, { size: 12 }, LINE), "size")).toBe(56);
    expect(floorOf(layout, "size")).toBe(56);
  });

  it("is nothing at all for a column this table does not have", () => {
    expect(widthOf(layout, "owner")).toBe(0);
    expect(floorOf(layout, "owner")).toBe(0);
  });
});

describe("the surplus a panel has", () => {
  it("is divided between the columns nobody has moved", () => {
    // 900 wide, 40 spent on chrome: the columns have 860 and ask for 480, so each of the three shares
    // 126 of the 380 that are left over.
    const drawn = drawnWidths(layout, 900);

    expect(drawn).toEqual({ name: 320 + 127, size: 80 + 127, kind: 80 + 126 });
  });

  it("is handed out to the pixel, so the columns add up to the panel exactly", () => {
    const drawn = drawnWidths(layout, 901);
    const sum = Object.values(drawn).reduce((total, width) => total + width, 0);

    expect(sum).toBe(901 - 40);
  });

  it("is shared by the columns nobody has moved, and not by the ones that have", () => {
    // With the size column moved, it stays where it was put and the other two divide the 260 that are
    // left between them.
    const drawn = drawnWidths(layoutOf(COLUMNS, { size: 200 }, LINE), 900);

    expect(drawn.size).toBe(200);
    expect(drawn.name).toBe(320 + 130);
    expect(drawn.kind).toBe(80 + 130);
  });

  it("is left where it is when every column has been moved", () => {
    const drawn = drawnWidths(layoutOf(COLUMNS, { name: 300, size: 90, kind: 70 }, LINE), 900);

    expect(drawn).toEqual({ name: 300, size: 90, kind: 70 });
  });

  it("is nothing at all when the panel is narrower than the columns", () => {
    // 400 wide leaves the columns 360, which is less than the 480 they ask for: the widths are the
    // widths, and a narrower panel is a panel the strip scrolls in.
    expect(drawnWidths(layout, 400)).toEqual({ name: 320, size: 80, kind: 80 });
  });

  it("is nothing at all when the line has not been laid out yet", () => {
    expect(drawnWidths(layout, 0)).toEqual({ name: 320, size: 80, kind: 80 });
  });
});

describe("where a drag of a column's edge stops", () => {
  it("is wherever the pointer asked, however wide that is", () => {
    // No ceiling: a column wider than the panel is a column the strip scrolls to reach.
    expect(movedTo(layout, "size", 4000)).toBe(4000);
    expect(movedTo(layout, "name", 1200)).toBe(1200);
  });

  it("is never below the column's own floor", () => {
    expect(movedTo(layout, "size", 10)).toBe(56);
    expect(movableWidth(0, 64)).toBe(64);
  });
});

describe("the line the columns are drawn on", () => {
  it("has to be as wide as the columns ask for, and what it spends around them", () => {
    // 24 of padding, 16 of gaps, and 480 of columns.
    expect(lineMinWidth(layout)).toBe(520);
  });

  it("leaves no room for chrome of its own when it is a real table", () => {
    const table = layoutOf(COLUMNS, {}, { paddingPx: 0, gapPx: 0 });

    expect(lineMinWidth(table)).toBe(480);
  });
});

describe("what the stylesheet is told", () => {
  it("carries the width every column is drawn at, the surplus divided", () => {
    expect(widthVars(layout, 900)).toEqual({
      "--col-name": "447px",
      "--col-size": "207px",
      "--col-kind": "206px",
    });
  });

  it("carries the widths themselves when there is no surplus to divide", () => {
    const vars = widthVars(layout as ColumnLayout, 400);

    expect(vars).toEqual({ "--col-name": "320px", "--col-size": "80px", "--col-kind": "80px" });
  });
});
