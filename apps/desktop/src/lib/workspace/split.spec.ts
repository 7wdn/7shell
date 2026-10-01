import { describe, expect, it } from "vitest";

import { MIN_PANE_PX, SPLIT_DIVIDER_PX, clampPaneHeight } from "@/lib/workspace/split";

describe("where the line between two stacked panes may be", () => {
  it("keeps a height that already fits", () => {
    expect(clampPaneHeight(280, 900)).toBe(280);
  });

  it("stops at the floor, because a pane that small is not a pane", () => {
    expect(clampPaneHeight(10, 900)).toBe(MIN_PANE_PX);
  });

  it("leaves the other pane its floor as well, and pays for the line between them", () => {
    // Both panes and the line have to add up: the ceiling is what is left once the upper pane has
    // the same floor the lower one is being held to.
    expect(clampPaneHeight(9000, 900)).toBe(900 - MIN_PANE_PX - SPLIT_DIVIDER_PX);
  });

  it("applies only the floor before anything has been measured", () => {
    // A pane that has not been laid out yet has no ceiling to be wrong about; it has a floor.
    expect(clampPaneHeight(280, 0)).toBe(280);
    expect(clampPaneHeight(10, 0)).toBe(MIN_PANE_PX);
  });

  it("still puts the line inside a region too short for two floors", () => {
    // Degenerate, not impossible: a window can be dragged shorter than two panes and a line, and
    // a ceiling below the floor would be a line that cannot be moved at all.
    expect(clampPaneHeight(280, 200)).toBe(MIN_PANE_PX);
  });
});
