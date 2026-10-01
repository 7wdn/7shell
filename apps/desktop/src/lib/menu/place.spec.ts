/**
 * Where a menu lands when the pointer is near an edge.
 *
 * The window is the only thing these cases are about: the menu is drawn at the pointer, and the
 * interesting question is never the middle of the window but the four corners of it.
 */
import { describe, expect, it } from "vitest";

import { MENU_MARGIN_PX, placeMenu, placeSubmenu } from "@/lib/menu/place";

const viewport = { width: 1000, height: 800 };
const size = { width: 200, height: 300 };

describe("placing a menu", () => {
  it("opens at the pointer when it fits there", () => {
    expect(placeMenu({ x: 100, y: 100 }, size, viewport)).toEqual({ left: 100, top: 100 });
  });

  it("flips above the pointer when the bottom edge is too close", () => {
    // The pointer's line stays the menu's bottom edge: what was right-clicked has to stay beside it.
    expect(placeMenu({ x: 100, y: 700 }, size, viewport)).toEqual({ left: 100, top: 400 });
  });

  it("slides left when the right edge is too close", () => {
    expect(placeMenu({ x: 990, y: 100 }, size, viewport)).toEqual({ left: 1000 - 200 - MENU_MARGIN_PX, top: 100 });
  });

  it("keeps its first entry on screen when it is taller than the window", () => {
    const tall = { width: 200, height: 900 };
    expect(placeMenu({ x: 100, y: 700 }, tall, viewport)).toEqual({ left: 100, top: MENU_MARGIN_PX });
  });
});

describe("placing a submenu", () => {
  const row = { left: 100, top: 300, width: 180, height: 28 };
  const size = { width: 160, height: 90 };

  it("opens against the right edge of the row that owns it", () => {
    expect(placeSubmenu(row, size, viewport)).toEqual({ left: 280, top: 300 });
  });

  it("folds over to the left of the row when there is no room beside it", () => {
    const last = { ...row, left: 900 };
    expect(placeSubmenu(last, size, viewport)).toEqual({ left: 740, top: 300 });
  });

  it("keeps the row's own line when the list is taller than the window", () => {
    const tall = { width: 160, height: 900 };
    expect(placeSubmenu({ ...row, top: 700 }, tall, viewport)).toEqual({ left: 280, top: MENU_MARGIN_PX });
  });
});
