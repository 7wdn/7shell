/**
 * The menu layer itself: what is drawn, what the keyboard does, and what closes it.
 *
 * The entries a region hands over are the subject of their own tests; this file is about the layer
 * that draws them — that a menu is reachable without a mouse, that an entry which cannot be run
 * cannot be run, that a row with a list beside it behaves like the menus everybody already knows, and
 * that the keyboard goes back where it came from.
 *
 * The menu is drawn through `openMenu` rather than by clicking a region, because that is the whole
 * interface between the two halves: any region that calls it gets everything below.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import ContextMenu from "@/components/shell/ContextMenu.vue";
import { useContextMenu } from "@/composables/useContextMenu";
import { i18n } from "@/i18n";
import type { MenuEntry } from "@/lib/menu/types";
import { useAppStore } from "@/stores/app";

let app: App | undefined;
let host: HTMLDivElement;

function entries(run: (id: string) => void, disabled = false): MenuEntry[] {
  return [
    { kind: "action", id: "copy", labelKey: "menu.copy", run: () => run("copy") },
    { kind: "separator", id: "sep" },
    { kind: "action", id: "paste", labelKey: "menu.paste", disabled, run: () => run("paste") },
  ];
}

/** A menu whose middle row opens a list of its own, and whose last row is a switch. */
function withSubmenu(run: (id: string) => void, packed: boolean): MenuEntry[] {
  return [
    { kind: "action", id: "refresh", labelKey: "menu.refresh", run: () => run("refresh") },
    {
      kind: "submenu",
      id: "new",
      labelKey: "files.newFolder",
      entries: [
        { kind: "action", id: "new-folder", labelKey: "files.newFolder", run: () => run("new-folder") },
        { kind: "action", id: "new-file", labelKey: "files.newFile", run: () => run("new-file") },
      ],
    },
    {
      kind: "action",
      id: "packed",
      labelKey: "menu.packedTransfer",
      checked: packed,
      run: () => run("packed"),
    },
  ];
}

/** Opens the menu the way a region does, and lets it draw. */
async function open(list: MenuEntry[], title = "menu.terminal"): Promise<MouseEvent> {
  const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 40, clientY: 60 });
  useContextMenu().openMenu(event, { key: title }, list);
  await nextTick();
  await nextTick();
  return event;
}

function menus(): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>('[role="menu"]')];
}

function menu(): HTMLElement | null {
  return menus()[0] ?? null;
}

function submenu(): HTMLElement | null {
  return menus()[1] ?? null;
}

function rows(box: HTMLElement | null): HTMLButtonElement[] {
  return [...(box?.querySelectorAll<HTMLButtonElement>('[role="menuitem"], [role="menuitemcheckbox"]') ?? [])];
}

function items(): HTMLButtonElement[] {
  return rows(menu());
}

function press(key: string, box: HTMLElement | null = menu()): void {
  box?.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
}

/** The row `aria-activedescendant` names, which is what a screen reader would read out. */
function activeRow(box: HTMLElement | null): HTMLElement | null {
  const id = box?.getAttribute("aria-activedescendant");
  return id == null ? null : document.getElementById(id);
}

/** Points at a row the way a mouse does. */
async function pointAt(row: HTMLButtonElement | undefined): Promise<void> {
  row?.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
  await nextTick();
  await nextTick();
}

beforeEach(() => {
  setActivePinia(createPinia());
  document.body.innerHTML = "";
  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(ContextMenu) }).use(i18n);
  app.mount(host);
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
});

describe("the context menu", () => {
  it("draws every entry and says which menu it is", async () => {
    await open(entries(() => {}));

    expect(menu()).not.toBeNull();
    expect(menu()?.getAttribute("aria-label")).toBe("Terminal actions");
    expect(items().map((item) => item.textContent?.trim())).toEqual(["Copy", "Paste"]);
    expect(document.querySelectorAll('[role="separator"]')).toHaveLength(1);
  });

  it("cancels the browser's own menu for the region that opened it", async () => {
    const event = await open(entries(() => {}));

    expect(event.defaultPrevented).toBe(true);
  });

  it("runs what was clicked, and closes", async () => {
    const run = vi.fn();
    await open(entries(run));

    items()[0]?.click();
    await nextTick();

    expect(run).toHaveBeenCalledWith("copy");
    expect(menu()).toBeNull();
  });

  it("draws an entry that cannot be run as one that cannot be run", async () => {
    const run = vi.fn();
    await open(entries(run, true));

    expect(items()[1]?.disabled).toBe(true);
    expect(items()[1]?.getAttribute("aria-disabled")).toBe("true");
  });

  it("moves with the arrow keys, skipping what cannot be run", async () => {
    await open(entries(() => {}, true));

    press("ArrowDown");
    await nextTick();
    // The row is named by the id the layer computes, and it has to be a row that is really drawn.
    expect(activeRow(menu())?.id).toBe("context-menu-root-copy");

    // Down again: the separator and the disabled entry are stepped over, and the ends wrap.
    press("ArrowDown");
    await nextTick();
    expect(activeRow(menu())?.id).toBe("context-menu-root-copy");

    press("ArrowUp");
    await nextTick();
    expect(activeRow(menu())?.id).toBe("context-menu-root-copy");
  });

  it("runs the entry the keyboard is on", async () => {
    const run = vi.fn();
    await open(entries(run));

    press("ArrowDown");
    press("Enter");
    await nextTick();

    expect(run).toHaveBeenCalledWith("copy");
    expect(menu()).toBeNull();
  });

  it("closes on Escape", async () => {
    await open(entries(() => {}));

    press("Escape");
    await nextTick();

    expect(menu()).toBeNull();
  });

  it("hands the keyboard back to whatever had it", async () => {
    const field = document.createElement("button");
    host.append(field);
    field.focus();
    await open(entries(() => {}));

    expect(document.activeElement).toBe(menu());

    press("Escape");
    await nextTick();

    expect(document.activeElement).toBe(field);
  });

  it("closes when a press lands outside it, and stays when one lands inside", async () => {
    await open(entries(() => {}));

    menu()?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    await nextTick();
    expect(menu()).not.toBeNull();

    document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    await nextTick();
    expect(menu()).toBeNull();
  });

  it("opens against the new pointer when a second region is right-clicked", async () => {
    await open(entries(() => {}));

    const second = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 500, clientY: 300 });
    useContextMenu().openMenu(second, { key: "menu.session" }, [
      { kind: "action", id: "edit", labelKey: "menu.refresh", run: () => {} },
    ]);
    await nextTick();
    await nextTick();

    expect(menu()?.getAttribute("aria-label")).toBe("Session actions");
    expect(items().map((item) => item.textContent?.trim())).toEqual(["Read again"]);
  });
});

describe("a row that opens a list of its own", () => {
  it("brings the list out beside it when it is pointed at", async () => {
    await open(withSubmenu(() => {}, false));
    expect(submenu()).toBeNull();

    await pointAt(items()[1]);

    expect(submenu()).not.toBeNull();
    expect(rows(submenu()).map((row) => row.textContent?.trim())).toEqual(["New folder", "New file"]);
    // The keyboard goes into the list that was just opened, because that is where the choice is.
    expect(document.activeElement).toBe(submenu());
    expect(items()[1]?.getAttribute("aria-haspopup")).toBe("menu");
  });

  it("brings it out with the arrow key too, and takes it away with the other one", async () => {
    await open(withSubmenu(() => {}, false));

    press("ArrowDown");
    press("ArrowDown");
    press("ArrowRight");
    await nextTick();
    await nextTick();
    expect(submenu()).not.toBeNull();

    press("ArrowLeft", submenu());
    await nextTick();
    expect(submenu()).toBeNull();
    expect(menu()).not.toBeNull();
    expect(document.activeElement).toBe(menu());
  });

  it("runs what the list offers, and closes the whole menu", async () => {
    const run = vi.fn();
    await open(withSubmenu(run, false));

    await pointAt(items()[1]);
    rows(submenu())[1]?.click();
    await nextTick();

    expect(run).toHaveBeenCalledWith("new-file");
    expect(menus()).toHaveLength(0);
  });

  it("takes the list away on the first Escape and the menu on the second", async () => {
    await open(withSubmenu(() => {}, false));
    await pointAt(items()[1]);

    press("Escape", submenu());
    await nextTick();
    expect(submenu()).toBeNull();
    expect(menu()).not.toBeNull();

    press("Escape");
    await nextTick();
    expect(menu()).toBeNull();
  });

  it("takes the list away when a row that opens nothing is pointed at", async () => {
    await open(withSubmenu(() => {}, false));
    await pointAt(items()[1]);
    expect(submenu()).not.toBeNull();

    await pointAt(items()[0]);

    expect(submenu()).toBeNull();
  });
});

describe("a row that is a switch", () => {
  it("says whether it is on, and runs the same way as any other row", async () => {
    const run = vi.fn();
    await open(withSubmenu(run, true));

    const toggle = items()[2];
    expect(toggle?.getAttribute("role")).toBe("menuitemcheckbox");
    expect(toggle?.getAttribute("aria-checked")).toBe("true");

    toggle?.click();
    await nextTick();

    expect(run).toHaveBeenCalledWith("packed");
    expect(menu()).toBeNull();
  });

  it("says when it is off", async () => {
    await open(withSubmenu(() => {}, false));

    expect(items()[2]?.getAttribute("aria-checked")).toBe("false");
  });
});

/**
 * A menu is chrome rather than a document.
 *
 * Its rows are verbs rather than readings, and a drag across one is somebody leaving in a hurry. The
 * classes are what stop the browser treating the list as text — both levels, since the list beside a
 * row is the same kind of thing as the one it came from. `docs/ui-conventions.md` states the rule.
 */
describe("a menu as text", () => {
  it("refuses to be selected, at both levels", async () => {
    await open(withSubmenu(() => {}, false));
    press("ArrowDown");
    press("ArrowDown");
    press("ArrowRight");
    await nextTick();
    await nextTick();

    const levels = [
      { what: "the menu", box: menu() },
      { what: "the list beside a row", box: submenu() },
    ];

    for (const { what, box } of levels) {
      if (!box) {
        throw new Error(`${what} is not drawn`);
      }
      expect(box.classList.contains("select-none"), `${what} is not selectable`).toBe(true);
      expect(box.classList.contains("cursor-default"), `${what} shows no I-beam`).toBe(true);
    }
  });
});

/**
 * The key hint beside a row is spelled by this layer, not by the table that built the row.
 *
 * The table carries a binding (`Mod+V`) because the same key reads differently per platform; this is
 * the layer that knows which platform it is drawing on, so this is where the two are put together.
 */
describe("a row's key hint", () => {
  it("is spelled for the platform the menu is drawn on", async () => {
    await open([
      { kind: "action", id: "paste", labelKey: "menu.paste", shortcut: "global.uploadClipboard", run: () => {} },
    ]);

    expect(items()[0]?.textContent).toContain("Ctrl + V");

    useAppStore().info = {
      name: "7Shell",
      version: "0.1.0",
      os: "macos",
      arch: "aarch64",
      debugBuild: false,
      protocolCount: 1,
      descriptorSchemaVersion: 1,
    };
    await nextTick();

    expect(items()[0]?.textContent).toContain("⌘ V");
  });
});
