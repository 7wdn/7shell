/**
 * The arithmetic behind dragging a rectangle over a listing.
 *
 * Which rows a band covers, and how far the listing should move when the pointer is held past an
 * edge. Both are the kind of thing that looks right in a screenshot and is wrong for the drag that
 * went upwards.
 */
import { describe, expect, it } from "vitest";

import { EDGE_MARGIN_PX, type RowBand, bandBetween, isDrag, rowsInBand, scrollStep } from "@/lib/files/marquee";

/** Four rows of 28 pixels, starting under a 24-pixel header. */
const rows: RowBand[] = [
  { path: "a", top: 24, bottom: 52 },
  { path: "b", top: 52, bottom: 80 },
  { path: "c", top: 80, bottom: 108 },
  { path: "d", top: 108, bottom: 136 },
];

describe("the band a drag makes", () => {
  it("reads the same dragged upwards or downwards", () => {
    expect(bandBetween({ x: 10, y: 100 }, { x: 200, y: 40 })).toEqual({ top: 40, bottom: 100 });
    expect(bandBetween({ x: 10, y: 40 }, { x: 200, y: 100 })).toEqual({ top: 40, bottom: 100 });
  });

  it("is not a drag until the pointer has actually moved", () => {
    expect(isDrag({ x: 10, y: 10 }, { x: 12, y: 12 })).toBe(false);
    expect(isDrag({ x: 10, y: 10 }, { x: 10, y: 30 })).toBe(true);
    expect(isDrag({ x: 10, y: 10 }, { x: 60, y: 12 })).toBe(true);
  });
});

describe("the rows a band covers", () => {
  it("takes every row it touches, including the ones it only clips", () => {
    expect(rowsInBand(rows, { top: 40, bottom: 100 })).toEqual(["a", "b", "c"]);
  });

  it("takes one row when the band sits inside it", () => {
    expect(rowsInBand(rows, { top: 60, bottom: 70 })).toEqual(["b"]);
  });

  it("catches both rows a drag across a boundary reaches", () => {
    expect(rowsInBand(rows, { top: 51, bottom: 53 })).toEqual(["a", "b"]);
  });

  it("is empty for a band between two rows", () => {
    // The gap between rows is one pixel of list padding in the real thing; a band that lands there
    // and touches neither is a band over nothing.
    const apart: RowBand[] = [
      { path: "a", top: 0, bottom: 10 },
      { path: "b", top: 30, bottom: 40 },
    ];
    expect(rowsInBand(apart, { top: 15, bottom: 25 })).toEqual([]);
  });

  it("is empty for a rectangle drawn on empty space", () => {
    expect(rowsInBand(rows, { top: 200, bottom: 300 })).toEqual([]);
  });
});

describe("scrolling while a drag is held past an edge", () => {
  const viewport = { top: 100, height: 300 };

  it("stands still in the middle", () => {
    expect(scrollStep(250, viewport)).toBe(0);
    expect(scrollStep(100 + EDGE_MARGIN_PX, viewport)).toBe(0);
  });

  it("moves upwards when the pointer is near the top, faster the further past it", () => {
    expect(scrollStep(100 + EDGE_MARGIN_PX - 5, viewport)).toBe(-5);
    expect(scrollStep(100, viewport)).toBe(-EDGE_MARGIN_PX);
    expect(scrollStep(0, viewport)).toBe(-EDGE_MARGIN_PX);
  });

  it("moves downwards near the bottom, and no further than the margin", () => {
    expect(scrollStep(400 - EDGE_MARGIN_PX + 5, viewport)).toBe(5);
    expect(scrollStep(500, viewport)).toBe(EDGE_MARGIN_PX);
  });
});
