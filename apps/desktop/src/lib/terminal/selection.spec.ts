/**
 * Where the toolbar about a selection stands.
 *
 * The boxes this reads belong to xterm's renderer — and one of them, the band a one-line selection
 * is drawn with, has no height at all and must not count. What a test can pin down is the reading
 * and the arithmetic; whether the renderer draws the boxes there is the end-to-end run's question.
 */
import { describe, expect, it } from "vitest";

import {
  TOOLS_ANCHOR_INSET_PX,
  TOOLS_GAP_PX,
  TOOLS_MARGIN_PX,
  placeSelectionTools,
  selectionShape,
  type ScreenBox,
  type SelectionShape,
  type Size,
} from "./selection";

function box(left: number, top: number, right: number, bottom: number): DOMRect {
  return {
    left,
    top,
    right,
    bottom,
    width: right - left,
    height: bottom - top,
    x: left,
    y: top,
    toJSON: () => ({}),
  } as DOMRect;
}

/** A stand-in for the surface xterm draws into, with the selection layer holding the given parts. */
function surfaceWith(parts: readonly DOMRect[], bounds: DOMRect): HTMLElement {
  const surface = document.createElement("div");
  const layer = document.createElement("div");
  layer.className = "xterm-selection";
  for (const part of parts) {
    const element = document.createElement("div");
    element.getBoundingClientRect = () => part;
    layer.append(element);
  }
  surface.append(layer);
  surface.getBoundingClientRect = () => bounds;
  document.body.append(surface);
  return surface;
}

const pane: ScreenBox = box(0, 0, 400, 300);
const tools: Size = { width: 92, height: 28 };

/** A selection of one line, the way `selectionShape` hands one back. */
function selectionOf(shape: ScreenBox, end?: { right: number; bottom: number }): SelectionShape {
  return { box: shape, end: end ?? { right: shape.right, bottom: shape.bottom } };
}

describe("reading a selection off the screen", () => {
  it("reads a drawn segment as the selection, and the point it stops at", () => {
    const surface = surfaceWith([box(735, 227, 893, 242)], box(309, 95, 1271, 479));

    const shape = selectionShape(surface);

    expect(shape).toEqual({
      box: { left: 735, top: 227, right: 893, bottom: 242 },
      end: { right: 893, bottom: 242 },
    });
  });

  it("ignores the band a one-line selection is drawn with, which has no height of its own", () => {
    // What xterm emits for a single-line selection: the line's own segment, and a band across the
    // whole row whose height is a negative length, which the browser lays out as none at all.
    const surface = surfaceWith([box(735, 227, 893, 242), box(321, 242, 1243, 242)], box(309, 95, 1271, 479));

    const shape = selectionShape(surface);

    expect(shape?.box, "a box built around the band reaches across the whole row").toEqual({
      left: 735,
      top: 227,
      right: 893,
      bottom: 242,
    });
    expect(shape?.end).toEqual({ right: 893, bottom: 242 });
  });

  it("reads every drawn line as one box, and the last of them as the end", () => {
    const surface = surfaceWith(
      [box(40, 100, 200, 116), box(0, 116, 400, 132), box(0, 132, 120, 148)],
      box(0, 0, 400, 300),
    );

    const shape = selectionShape(surface);

    expect(shape?.box).toEqual({ left: 0, top: 100, right: 400, bottom: 148 });
    expect(shape?.end, "the end of the line the highlight stops on, not the widest line").toEqual({
      right: 120,
      bottom: 148,
    });
  });

  it("reads nothing when nothing is drawn", () => {
    const surface = surfaceWith([], box(0, 0, 400, 300));

    expect(selectionShape(surface)).toBeNull();
  });

  it("reads nothing when all that is drawn has no area", () => {
    const surface = surfaceWith([box(0, 242, 400, 242)], box(0, 0, 400, 300));

    expect(selectionShape(surface)).toBeNull();
  });

  it("reads nothing when the whole selection is scrolled out of the drawn rows", () => {
    const surface = surfaceWith([box(40, 500, 200, 516)], box(0, 0, 400, 300));

    expect(selectionShape(surface)).toBeNull();
  });

  it("cuts the box, and the point it stops at, back to the rows the pane is showing", () => {
    // The segments are not clipped by their container — a selection running past the last drawn row
    // keeps its full box — so the box has to be cut back here or the toolbar would hang under rows
    // that are not on screen.
    const surface = surfaceWith([box(40, 280, 200, 340)], box(0, 0, 400, 300));

    const shape = selectionShape(surface);

    expect(shape?.box).toEqual({ left: 40, top: 280, right: 200, bottom: 300 });
    expect(shape?.end).toEqual({ right: 200, bottom: 300 });
  });
});

describe("placing the toolbar", () => {
  it("hangs from the end of the selection, its first button under it", () => {
    const placed = placeSelectionTools(selectionOf(box(40, 100, 200, 116)), pane, tools);

    expect(placed).toEqual({
      left: 200 - TOOLS_ANCHOR_INSET_PX,
      top: 116 + TOOLS_GAP_PX,
    });
  });

  it("stands above the selection when the pane's floor is in the way", () => {
    const placed = placeSelectionTools(selectionOf(box(40, 240, 200, 290)), pane, tools);

    expect(placed.top).toBe(240 - tools.height - TOOLS_GAP_PX);
  });

  it("keeps its buttons inside the pane at either edge", () => {
    // A selection stopping closer to the edge than the first button reaches back over it.
    const atTheStart = placeSelectionTools(selectionOf(box(0, 10, 10, 26)), pane, tools);
    const atTheEnd = placeSelectionTools(selectionOf(box(300, 10, 396, 26)), pane, tools);

    expect(atTheStart.left).toBe(TOOLS_MARGIN_PX);
    expect(atTheEnd.left).toBe(pane.right - tools.width - TOOLS_MARGIN_PX);
  });

  it("keeps standing inside a pane with no room for it at all", () => {
    const tiny: ScreenBox = box(0, 0, 60, 20);

    const placed = placeSelectionTools(selectionOf(box(0, 0, 60, 18)), tiny, tools);

    expect(placed).toEqual({ left: TOOLS_MARGIN_PX, top: TOOLS_MARGIN_PX });
  });
});
