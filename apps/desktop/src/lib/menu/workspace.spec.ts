/**
 * What a tab's right-click offers.
 *
 * The strip is where a connection is duplicated, reconnected, dropped or closed, and the rules
 * about which entries are usable are the whole content of this file: what "the others" means when
 * there are none, when a tab is one the interface would offer to reconnect, and which kinds get the
 * connection entries at all.
 */
import { describe, expect, it, vi } from "vitest";

import type { MenuAction, MenuEntry } from "./types";
import { type TabMenuState, canReconnect, panelPlacementMenu, tabMenu } from "./workspace";

function state(overrides: Partial<TabMenuState> = {}): TabMenuState {
  return { kind: "terminal", status: "open", hasHandle: true, index: 1, total: 3, reconnectable: 0, ...overrides };
}

function pick(entries: readonly MenuEntry[], id: string): MenuAction {
  const found = entries.find((entry): entry is MenuAction => entry.kind === "action" && entry.id === id);
  if (!found) {
    throw new Error(`the menu has no entry called ${id}`);
  }
  return found;
}

function actions() {
  return {
    duplicate: vi.fn(),
    close: vi.fn(),
    closeOthers: vi.fn(),
    closeToTheLeft: vi.fn(),
    closeToTheRight: vi.fn(),
    closeAll: vi.fn(),
    reconnect: vi.fn(),
    reconnectAll: vi.fn(),
    disconnect: vi.fn(),
  };
}

function ids(entries: readonly MenuEntry[]): string[] {
  return entries.filter((entry): entry is MenuAction => entry.kind === "action").map((entry) => entry.id);
}

describe("a terminal tab's menu", () => {
  it("offers duplicating, the connection actions, and the closings", () => {
    const entries = tabMenu(state(), actions());

    expect(ids(entries)).toEqual([
      "duplicate",
      "reconnect",
      "reconnect-all",
      "disconnect",
      "close",
      "close-others",
      "close-left",
      "close-right",
      "close-all",
    ]);
  });

  it("offers connecting, connecting all and disconnecting only where they mean something", () => {
    const open = tabMenu(state({ status: "open", reconnectable: 1 }), actions());
    expect(pick(open, "reconnect").disabled).toBe(true);
    expect(pick(open, "reconnect-all").disabled).toBeFalsy();
    expect(pick(open, "disconnect").disabled).toBeFalsy();

    const ended = tabMenu(state({ status: "ended" }), actions());
    expect(pick(ended, "reconnect").disabled).toBeFalsy();
    expect(pick(ended, "disconnect").disabled).toBe(true);

    const noneClosed = tabMenu(state({ reconnectable: 0 }), actions());
    expect(pick(noneClosed, "reconnect-all").disabled).toBe(true);
  });

  it("has nothing to close on either side when the tab stands alone", () => {
    const entries = tabMenu(state({ index: 0, total: 1 }), actions());

    expect(pick(entries, "close-others").disabled).toBe(true);
    expect(pick(entries, "close-left").disabled).toBe(true);
    expect(pick(entries, "close-right").disabled).toBe(true);
  });

  it("has nothing to the right when the tab is the last one, and nothing to the left when it is the first", () => {
    const last = tabMenu(state({ index: 2, total: 3 }), actions());
    expect(pick(last, "close-others").disabled).toBeFalsy();
    expect(pick(last, "close-left").disabled).toBeFalsy();
    expect(pick(last, "close-right").disabled).toBe(true);

    const first = tabMenu(state({ index: 0, total: 3 }), actions());
    expect(pick(first, "close-left").disabled).toBe(true);
    expect(pick(first, "close-right").disabled).toBeFalsy();
  });

  it("offers reconnecting for a terminal that has ended, and for one that failed", () => {
    expect(pick(tabMenu(state({ status: "ended" }), actions()), "reconnect").disabled).toBeFalsy();
    expect(pick(tabMenu(state({ status: "failed" }), actions()), "reconnect").disabled).toBeFalsy();
    expect(pick(tabMenu(state({ status: "open" }), actions()), "reconnect").disabled).toBe(true);
  });

  it("closes what the entry says it closes", () => {
    const wired = actions();
    const entries = tabMenu(state(), wired);

    pick(entries, "duplicate").run();
    pick(entries, "reconnect-all").run();
    pick(entries, "disconnect").run();
    pick(entries, "close").run();
    pick(entries, "close-others").run();
    pick(entries, "close-left").run();
    pick(entries, "close-right").run();
    pick(entries, "close-all").run();

    expect(wired.duplicate).toHaveBeenCalledOnce();
    expect(wired.reconnectAll).toHaveBeenCalledOnce();
    expect(wired.disconnect).toHaveBeenCalledOnce();
    expect(wired.close).toHaveBeenCalledOnce();
    expect(wired.closeOthers).toHaveBeenCalledOnce();
    expect(wired.closeToTheLeft).toHaveBeenCalledOnce();
    expect(wired.closeToTheRight).toHaveBeenCalledOnce();
    expect(wired.closeAll).toHaveBeenCalledOnce();
  });
});

describe("the tabs with no connection of their own", () => {
  it("offers a browser the reconnecting it can do, and none of the terminal entries", () => {
    const failed = state({ kind: "files", status: "failed", hasHandle: false });
    const open = state({ kind: "files", status: "open", hasHandle: true });

    // A second browser on one session is the thing the panel's own retry button refuses to do.
    expect(canReconnect(failed)).toBe(true);
    expect(canReconnect(open)).toBe(false);
    expect(ids(tabMenu(failed, actions()))).toEqual([
      "reconnect",
      "close",
      "close-others",
      "close-left",
      "close-right",
      "close-all",
    ]);
  });

  it("offers the pages and the engine workspaces the closings alone", () => {
    expect(canReconnect(state({ kind: "inspect", status: null }))).toBe(false);
    expect(canReconnect(state({ kind: "container", status: null }))).toBe(false);
    expect(canReconnect(state({ kind: "kubernetes", status: null }))).toBe(false);

    expect(ids(tabMenu(state({ kind: "container", status: null }), actions()))).toEqual([
      "close",
      "close-others",
      "close-left",
      "close-right",
      "close-all",
    ]);
  });
});

describe("the file panel's placement menu", () => {
  it("offers the four positions with the remembered one ticked", () => {
    const entries = panelPlacementMenu("right", { choose: vi.fn() });

    expect(ids(entries)).toEqual(["panel-bottom", "panel-left", "panel-right", "panel-tab"]);
    expect(entries.map((entry) => (entry.kind === "action" ? entry.checked : undefined))).toEqual([
      false,
      false,
      true,
      false,
    ]);
  });

  it("hands the chosen position back", () => {
    const choose = vi.fn();
    const entries = panelPlacementMenu("bottom", { choose });

    pick(entries, "panel-left").run();

    expect(choose).toHaveBeenCalledWith("left");
  });
});
