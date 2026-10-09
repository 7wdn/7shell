/**
 * Which rows get drawn, at every position of the scrollbar.
 *
 * The ends of the list and the exact seams are the cases a screenshot never shows: a listing whose
 * last row is missing, a strip of nothing at the bottom, a spacer that is a row too tall. They are
 * all arithmetic, so they are all here.
 */
import { describe, expect, it } from "vitest";

import { UNMEASURED_ROWS, rowBands, rowWindow } from "./window";

/** A listing of `count` rows, a screenful of `viewport`, rows `pitch` tall. */
function windowAt(scrolled: number, options: Partial<Parameters<typeof rowWindow>[0]> = {}) {
  return rowWindow({ scrolled, viewport: 300, pitch: 30, count: 1000, overscan: 3, ...options });
}

/** The whole content height, which the spacers and the drawn rows have to add up to. */
function covered(window: ReturnType<typeof rowWindow>, pitch = 30): number {
  return window.before + (window.last - window.first) * pitch + window.after;
}

/** How many rows the window covers, spacers included. */
function coveredInRows(window: ReturnType<typeof rowWindow>): number {
  return window.last - window.first;
}

describe("the window over a long listing", () => {
  it("draws a screenful at the top, with a spacer for everything under it", () => {
    const window = windowAt(0);

    expect(window.first).toBe(0);
    expect(window.before).toBe(0);
    // Ten rows fit in 300px, plus the overscan below.
    expect(window.last).toBe(13);
    expect(window.after).toBe((1000 - 13) * 30);
  });

  it("follows the scrollbar", () => {
    const window = windowAt(3000);

    // 3000 / 30 = row 100, minus the overscan above.
    expect(window.first).toBe(97);
    expect(window.before).toBe(97 * 30);
    expect(window.last).toBe(113);
  });

  it("draws to the last row and no further when the bottom is reached", () => {
    const window = windowAt(29_700);

    expect(window.last).toBe(1000);
    expect(window.after).toBe(0);
    expect(window.first).toBeLessThan(1000);
  });

  it("keeps the content as tall as the listing is, wherever it is scrolled", () => {
    for (const scrolled of [0, 7, 3000, 12_345, 29_700, 40_000]) {
      expect(covered(windowAt(scrolled)), `scrolled to ${scrolled}`).toBe(1000 * 30);
    }
  });

  it("asks for nothing when the listing has no rows", () => {
    expect(windowAt(0, { count: 0 })).toEqual({ first: 0, last: 0, before: 0, after: 0 });
  });

  it("draws a screenful of rows before any of them has been measured", () => {
    // A panel that has not been laid out yet reports no row height, and answering "draw nothing"
    // would leave it empty until something else woke it up.
    const window = windowAt(0, { pitch: 0 });

    expect(window.first).toBe(0);
    expect(window.last).toBe(UNMEASURED_ROWS);
    expect(coveredInRows(window)).toBe(UNMEASURED_ROWS);
  });

  it("does not draw more rows than there are, before they are measured", () => {
    expect(windowAt(0, { pitch: 0, count: 3 }).last).toBe(3);
  });

  it("draws a screenful when the container has not been measured yet", () => {
    // A panel inside a tab that has not been shown is nought pixels tall, and answering "draw
    // nothing" would leave it looking empty until something else woke it up.
    const window = windowAt(0, { viewport: 0 });

    expect(window.last).toBeGreaterThan(0);
  });

  it("does not ask for rows before the first or past the last", () => {
    const top = windowAt(0, { overscan: 50 });
    const bottom = windowAt(29_700, { overscan: 50 });

    expect(top.first).toBe(0);
    expect(bottom.last).toBe(1000);
    expect(bottom.first).toBeGreaterThanOrEqual(0);
  });

  it("places every row of a listing, drawn or not", () => {
    const bands = rowBands(["a", "b", "c"], 100, 30);

    expect(bands).toEqual([
      { path: "a", top: 100, bottom: 130 },
      { path: "b", top: 130, bottom: 160 },
      { path: "c", top: 160, bottom: 190 },
    ]);
  });

  it("places no rows when the height of one is not known", () => {
    // Nothing to multiply by: a caller in this state falls back to what is drawn.
    expect(rowBands(["a", "b"], 0, 0)).toEqual([]);
  });

  it("treats a scroll above the first row as the first row", () => {
    // A browser can report a negative scroll offset during an elastic overscroll.
    expect(windowAt(-120).first).toBe(0);
  });
});
