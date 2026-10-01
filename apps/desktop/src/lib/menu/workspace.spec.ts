/**
 * What a tab's right-click offers.
 *
 * The tab strip is where a session is closed, and closing several is the thing it never had. The
 * rules about which entries are usable are the whole content of this file: what "the others" means
 * when there are none, and when a tab is one the interface would offer to reconnect.
 */
import { describe, expect, it, vi } from "vitest";

import type { MenuAction, MenuEntry } from "./types";
import { type TabMenuState, canReconnect, tabMenu } from "./workspace";

function state(overrides: Partial<TabMenuState> = {}): TabMenuState {
  return { kind: "terminal", status: "open", hasHandle: true, index: 1, total: 3, ...overrides };
}

function pick(entries: readonly MenuEntry[], id: string): MenuAction {
  const found = entries.find((entry): entry is MenuAction => entry.kind === "action" && entry.id === id);
  if (!found) {
    throw new Error(`the menu has no entry called ${id}`);
  }
  return found;
}

function actions() {
  return { close: vi.fn(), closeOthers: vi.fn(), closeToTheRight: vi.fn(), reconnect: vi.fn() };
}

describe("a tab's menu", () => {
  it("offers closing, closing the others, closing the right, and reconnecting", () => {
    const entries = tabMenu(state(), actions());

    expect(entries.filter((entry) => entry.kind === "action").map((entry) => entry.id)).toEqual([
      "close",
      "close-others",
      "close-right",
      "reconnect",
    ]);
  });

  it("has nothing to close on either side when the tab stands alone", () => {
    const entries = tabMenu(state({ index: 0, total: 1 }), actions());

    expect(pick(entries, "close-others").disabled).toBe(true);
    expect(pick(entries, "close-right").disabled).toBe(true);
  });

  it("has nothing to the right when the tab is the last one", () => {
    const entries = tabMenu(state({ index: 2, total: 3 }), actions());

    expect(pick(entries, "close-others").disabled).toBeFalsy();
    expect(pick(entries, "close-right").disabled).toBe(true);
  });

  it("offers reconnecting for a terminal that has ended, and for one that failed", () => {
    expect(pick(tabMenu(state({ status: "ended" }), actions()), "reconnect").disabled).toBeFalsy();
    expect(pick(tabMenu(state({ status: "failed" }), actions()), "reconnect").disabled).toBeFalsy();
    expect(pick(tabMenu(state({ status: "open" }), actions()), "reconnect").disabled).toBe(true);
  });

  it("offers reconnecting for a browser that never opened, and never for one that did", () => {
    const failed = state({ kind: "files", status: "failed", hasHandle: false });
    const open = state({ kind: "files", status: "open", hasHandle: true });

    // A second browser on one session is the thing the panel's own retry button refuses to do.
    expect(canReconnect(failed)).toBe(true);
    expect(canReconnect(open)).toBe(false);
  });

  it("offers no reconnecting for a page about a host", () => {
    expect(canReconnect(state({ kind: "inspect", status: null }))).toBe(false);
  });

  it("closes what the entry says it closes", () => {
    const wired = actions();
    const entries = tabMenu(state(), wired);

    pick(entries, "close").run();
    pick(entries, "close-others").run();
    pick(entries, "close-right").run();

    expect(wired.close).toHaveBeenCalledOnce();
    expect(wired.closeOthers).toHaveBeenCalledOnce();
    expect(wired.closeToTheRight).toHaveBeenCalledOnce();
  });
});
