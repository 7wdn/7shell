/**
 * How the keyboard moves through a menu.
 *
 * A menu is a list with holes in it — separators that mean nothing and entries that cannot be run —
 * and these are the rules for stepping over both.
 */
import { describe, expect, it } from "vitest";

import { firstEnabledIndex, lastEnabledIndex, nextEnabledIndex } from "@/lib/menu/navigate";
import type { MenuEntry, MenuLeaf } from "@/lib/menu/types";

function action(id: string, disabled = false): MenuLeaf {
  return { kind: "action", id, labelKey: "menu.copy", disabled, run: () => {} };
}

const separator: MenuEntry = { kind: "separator", id: "sep" };
const entries = [action("one"), separator, action("two", true), action("three")];

/** A row that opens a list of its own: not runnable, but somewhere the keyboard can land. */
const parent: MenuEntry = { kind: "submenu", id: "new", labelKey: "menu.new", entries: [action("file")] };

describe("moving through a menu", () => {
  it("steps over separators and over entries that cannot be run", () => {
    expect(nextEnabledIndex(entries, -1, 1)).toBe(0);
    expect(nextEnabledIndex(entries, 0, 1)).toBe(3);
    expect(nextEnabledIndex(entries, 3, -1)).toBe(0);
  });

  it("lands on a row that opens a list, because that is something the keyboard can act on", () => {
    const mixed = [action("one"), parent, action("three")];

    expect(nextEnabledIndex(mixed, 0, 1)).toBe(1);
    expect(nextEnabledIndex(mixed, 1, 1)).toBe(2);
  });

  it("wraps around at both ends", () => {
    expect(nextEnabledIndex(entries, 3, 1)).toBe(0);
    expect(nextEnabledIndex(entries, 0, -1)).toBe(3);
  });

  it("finds the two ends for Home and End", () => {
    expect(firstEnabledIndex(entries)).toBe(0);
    expect(lastEnabledIndex(entries)).toBe(3);
  });

  it("answers nothing when there is nowhere to land", () => {
    expect(nextEnabledIndex([action("off", true), separator], -1, 1)).toBeNull();
    expect(firstEnabledIndex([])).toBeNull();
    expect(lastEnabledIndex([])).toBeNull();
  });
});
