import { describe, expect, it } from "vitest";

import {
  type TerminalLayout,
  removeTerminalPane,
  resizeTerminalSplit,
  splitTerminalPane,
  terminalGeometry,
  terminalPaneIds,
} from "@/lib/workspace/terminal-layout";

const first: TerminalLayout = { kind: "pane", paneId: "a" };
const pair = splitTerminalPane(first, "a", "b", "right", "ab");
const nested = splitTerminalPane(pair, "b", "c", "down", "bc");

describe("terminal layouts", () => {
  it("splits the chosen pane without moving its sibling or duplicating a terminal", () => {
    expect(terminalPaneIds(nested)).toEqual(["a", "b", "c"]);
    expect(nested).toMatchObject({
      direction: "right",
      first,
      second: { direction: "down", first: { paneId: "b" }, second: { paneId: "c" } },
    });
    expect(splitTerminalPane(nested, "a", "b", "down", "duplicate")).toBe(nested);
  });

  it("fills the space left by any closed pane and removes the final empty layout", () => {
    expect(removeTerminalPane(nested, "b")).toMatchObject({ first, second: { kind: "pane", paneId: "c" } });
    expect(removeTerminalPane(nested, "a")).toMatchObject({ id: "bc", direction: "down" });
    expect(removeTerminalPane(pair, "b")).toEqual(first);
    expect(removeTerminalPane(first, "a")).toBeNull();
  });

  it("lays out a nested split with eight pixel gutters and no overlapping panes", () => {
    const { panes, dividers } = terminalGeometry(nested, 1000, 600);
    expect(panes.a).toEqual({ left: 0, top: 0, width: 496, height: 600 });
    expect(panes.b).toEqual({ left: 504, top: 0, width: 496, height: 296 });
    expect(panes.c).toEqual({ left: 504, top: 304, width: 496, height: 296 });
    expect(dividers.map((divider) => divider.rect)).toEqual([
      { left: 496, top: 0, width: 8, height: 600 },
      { left: 504, top: 296, width: 496, height: 8 },
    ]);
  });

  it("resizes only the named branch and respects both terminal floors", () => {
    const resized = resizeTerminalSplit(nested, "bc", 0.8);
    const { panes } = terminalGeometry(resized, 1000, 600);
    expect(panes.a?.width).toBe(496);
    expect(panes.b?.height).toBe(432);
    expect(panes.c?.height).toBe(160);
    expect(resizeTerminalSplit(nested, "bc", Number.NaN)).toBe(nested);
  });

  it("keeps every pane inside a small window and restores its chosen ratio when it grows", () => {
    const resized = resizeTerminalSplit(nested, "ab", 0.7);
    const small = terminalGeometry(resized, 320, 200);
    for (const rect of Object.values(small.panes)) {
      expect(rect.width).toBeGreaterThan(0);
      expect(rect.height).toBeGreaterThan(0);
      expect(rect.left + rect.width).toBeLessThanOrEqual(320);
      expect(rect.top + rect.height).toBeLessThanOrEqual(200);
    }
    expect(terminalGeometry(resized, 1200, 800).panes.a?.width).toBeCloseTo(1192 * 0.7);
  });
});
