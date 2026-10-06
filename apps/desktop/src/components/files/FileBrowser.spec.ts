/**
 * The file panel, as a person meets it: what its menus offer and what choosing one does.
 *
 * The builders have their own suite for the shape of the lists; what can only be seen here is the
 * wiring — that the panel hands the packing switch over, that a menu entry reaches the call it is
 * supposed to reach, and that the two kinds of "new" do what their names say.
 *
 * It is mounted against a browser the store is simply told about: opening one is a conversation with
 * a server, which is `live_files.rs`'s business rather than this file's.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  filesCreateFile: vi.fn(async () => {}),
  filesCreateFolder: vi.fn(async () => {}),
  filesDownload: vi.fn(async () => "x1"),
  filesList: vi.fn(async (_handle: string, path: string) => ({ path, entries: [] })),
  filesOpen: vi.fn(),
  filesClose: vi.fn(),
  transfersOpen: vi.fn(async () => []),
  settingsGet: vi.fn(),
  settingsSet: vi.fn(async () => {}),
}));

import FileBrowser from "@/components/files/FileBrowser.vue";
import { useContextMenu } from "@/composables/useContextMenu";
import { i18n } from "@/i18n";
import { filesCreateFile, filesDownload, filesList } from "@/lib/backend/tauri";
import type { MenuAction, MenuEntry } from "@/lib/menu/types";
import { useAppStore } from "@/stores/app";
import { useFilesStore } from "@/stores/files";
import { usePreferencesStore } from "@/stores/preferences";
import type { SftpEntry } from "@/types/session";

const HANDLE = "f1";

const folder: SftpEntry = { name: "logs", path: "/home/user/logs", kind: "directory", size: 0 };
const file: SftpEntry = { name: "notes.txt", path: "/home/user/notes.txt", kind: "file", size: 12 };

let host: HTMLDivElement;
let app: App | undefined;

/** Mounts the panel over a directory the store is told about, which is what the panel draws. */
async function render(entries: readonly SftpEntry[]): Promise<void> {
  useFilesStore().browsers[HANDLE] = {
    handle: HANDLE,
    sessionId: "session-1",
    path: "/home/user",
    home: "/home/user",
    shellPath: "/home/user",
    entries: [...entries],
    loading: false,
    failure: null,
  };

  app = createApp({ render: () => h(FileBrowser, { handle: HANDLE }) }).use(i18n);
  app.mount(host);
  await nextTick();
}

/**
 * The listing's scroll container, which is what carries the directory's menu.
 *
 * Found through the rows themselves rather than by a class the whole panel shares: the directory tree
 * on the left is a list of its own with a scroller of its own, and a query that caught both would
 * make every case below a coin toss between the two panes.
 */
function listing(): HTMLElement | null {
  return host.querySelector("ul.p-1\\.5")?.parentElement ?? null;
}

/** Right-clicks the nth row of the listing, which is how a row's own menu is opened. */
async function rightClickRow(index: number): Promise<readonly MenuEntry[]> {
  const row = listing()?.querySelectorAll("li > div")[index];
  row?.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
  await nextTick();
  return useContextMenu().entries.value;
}

/** Right-clicks the listing's empty space, which is where the directory's menu lives. */
async function rightClickArea(): Promise<readonly MenuEntry[]> {
  listing()?.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
  await nextTick();
  return useContextMenu().entries.value;
}

function pick(entries: readonly MenuEntry[], id: string): MenuAction {
  const found = entries.find((item): item is MenuAction => item.kind === "action" && item.id === id);
  if (!found) {
    throw new Error(`the menu has no entry called ${id}`);
  }
  return found;
}

function ids(entries: readonly MenuEntry[]): string[] {
  return entries.map((entry) => entry.id);
}

/** The rows of the listing, which is what the selection is made of. */
function rows(): HTMLElement[] {
  return [...(listing()?.querySelectorAll<HTMLElement>("li > div") ?? [])];
}

/** The rows that are drawn as selected. */
function selectedRows(): number[] {
  return rows()
    .map((row, index) => (row.getAttribute("aria-selected") === "true" ? index : -1))
    .filter((index) => index >= 0);
}

/** Clicks a row, with whichever modifiers the gesture calls for. */
async function clickRow(index: number, ...modifiers: ("ctrl" | "shift" | "meta")[]): Promise<void> {
  const row = rows()[index];
  row?.dispatchEvent(
    new MouseEvent("click", {
      bubbles: true,
      ctrlKey: modifiers.includes("ctrl"),
      shiftKey: modifiers.includes("shift"),
      metaKey: modifiers.includes("meta"),
    }),
  );
  await nextTick();
}

/** Presses a key on the listing, which is where the keyboard for the rows lives. */
async function press(key: string, ...modifiers: ("ctrl" | "shift" | "meta")[]): Promise<void> {
  listing()?.dispatchEvent(
    new KeyboardEvent("keydown", {
      key,
      bubbles: true,
      cancelable: true,
      ctrlKey: modifiers.includes("ctrl"),
      shiftKey: modifiers.includes("shift"),
      metaKey: modifiers.includes("meta"),
    }),
  );
  await nextTick();
}

/**
 * The question the panel asks before a removal, which is teleported out of it and onto the body.
 *
 * It is about the selection rather than about one row — a strip inside the listing would be drawn
 * under whichever row happened to be first — so it is looked for where it is actually drawn.
 */
function confirmDialog(): HTMLElement {
  const dialog = document.querySelector<HTMLElement>('[role="dialog"]');
  if (!dialog) {
    throw new Error("no confirmation is on screen");
  }
  return dialog;
}

beforeEach(() => {
  setActivePinia(createPinia());
  host = document.createElement("div");
  document.body.append(host);
  vi.mocked(filesCreateFile).mockClear();
  vi.mocked(filesDownload).mockClear();
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
});

describe("a directory in the listing", () => {
  it("offers nothing to take it with while packing is off", async () => {
    await render([folder]);

    // A menu that opened at all, so the case cannot pass by the row simply not answering.
    const entries = await rightClickRow(0);
    expect(ids(entries)).toContain("open");
    expect(ids(entries)).not.toContain("download-archive");
  });

  it("offers a download the moment the switch in the directory's own menu is on", async () => {
    await render([folder]);
    const preferences = usePreferencesStore();

    // Turned on the way the menu does it, so what is asserted is the same value the entry reads.
    await preferences.setPackedTransfer(true);

    const entries = await rightClickRow(0);
    expect(ids(entries)).toContain("download-archive");

    pick(entries, "download-archive").run();
    await nextTick();
    // The whole selection travels as one archive, and the dialog behind it asks for a folder.
    expect(vi.mocked(filesDownload)).toHaveBeenCalledWith(
      HANDLE,
      ["/home/user/logs"],
      "Choose the folder to download into",
      "",
      true,
    );
  });

  it("leaves a single file with the plain download it always had", async () => {
    await render([file]);
    await usePreferencesStore().setPackedTransfer(true);

    const entries = await rightClickRow(0);
    pick(entries, "download").run();
    await nextTick();

    expect(vi.mocked(filesDownload)).toHaveBeenCalledWith(
      HANDLE,
      ["/home/user/notes.txt"],
      "Choose where to save this file",
      "notes.txt",
      false,
    );
  });
});

describe("the directory's own menu", () => {
  it("carries the switch, both ways round", async () => {
    await render([]);
    const preferences = usePreferencesStore();

    const off = await rightClickArea();
    expect(pick(off, "packed").checked).toBe(false);

    pick(off, "packed").run();
    await nextTick();
    expect(preferences.packedTransfer).toBe(true);

    const on = await rightClickArea();
    expect(pick(on, "packed").checked).toBe(true);
  });

  it("offers both kinds of new, and creates the file it is asked for", async () => {
    await render([]);
    const fromMenu = await rightClickArea();
    const newMenu = fromMenu.find((entry) => entry.kind === "submenu");
    if (newMenu?.kind !== "submenu") {
      throw new Error("the directory's menu has no 新建 list");
    }
    expect(ids(newMenu.entries)).toEqual(["new-folder", "new-file"]);

    pick(newMenu.entries, "new-file").run();
    await nextTick();

    // The prompt that appears says which of the two it is, and it is answered in the panel — in its
    // own form, which is found through the field rather than as "the panel's first form".
    const field = host.querySelector<HTMLInputElement>('input[aria-label="File name"]');
    expect(field, "the file prompt is open").not.toBeNull();
    if (field) {
      field.value = "notes.txt";
      field.dispatchEvent(new Event("input"));
      field.closest("form")?.dispatchEvent(new Event("submit"));
    }
    await vi.waitFor(() => expect(vi.mocked(filesCreateFile)).toHaveBeenCalled());

    expect(vi.mocked(filesCreateFile)).toHaveBeenCalledWith(HANDLE, "/home/user/notes.txt");
  });
});

/**
 * Selecting rows, in the shapes the platforms agree on.
 *
 * Ctrl and shift are the whole of it — with one exception that is not a detail: on a Mac the modifier
 * is Command, because **Ctrl+click is the secondary click there** and taking it would take the context
 * menu away from every macOS user.
 */
describe("selecting rows", () => {
  const many: SftpEntry[] = [
    { name: "a.txt", path: "/home/user/a.txt", kind: "file", size: 1 },
    { name: "b.txt", path: "/home/user/b.txt", kind: "file", size: 2 },
    { name: "c.txt", path: "/home/user/c.txt", kind: "file", size: 3 },
    { name: "logs", path: "/home/user/logs", kind: "directory", size: 0 },
  ];

  it("takes one row on a plain click, and only that one", async () => {
    await render(many);

    await clickRow(1);

    expect(selectedRows()).toEqual([1]);
  });

  it("adds and removes a row with the platform's modifier", async () => {
    await render(many);

    await clickRow(0);
    await clickRow(2, "ctrl");
    expect(selectedRows()).toEqual([0, 2]);

    await clickRow(0, "ctrl");
    expect(selectedRows()).toEqual([2]);
  });

  it("takes the range between the last plain click and a shift-click", async () => {
    await render(many);

    await clickRow(1);
    await clickRow(3, "shift");

    expect(selectedRows()).toEqual([1, 2, 3]);
  });

  it("takes every row on the modifier and A", async () => {
    await render(many);

    await clickRow(2);
    await press("a", "ctrl");

    expect(selectedRows()).toEqual([0, 1, 2, 3]);
  });

  it("walks the rows with the arrow keys, carrying the selection along", async () => {
    await render(many);

    await clickRow(0);
    await press("ArrowDown");
    expect(selectedRows()).toEqual([1]);

    await press("ArrowDown", "shift");
    expect(selectedRows()).toEqual([1, 2]);

    await press("End");
    expect(selectedRows()).toEqual([3]);
  });

  it("lets go of everything when the empty space is clicked", async () => {
    await render(many);
    await clickRow(0);
    await clickRow(2, "ctrl");
    expect(selectedRows()).toEqual([0, 2]);

    listing()?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    window.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
    await nextTick();

    expect(selectedRows()).toEqual([]);
  });

  it("asks about every selected row when Delete is pressed, in one question", async () => {
    await render(many);

    await clickRow(0);
    await clickRow(2, "ctrl");
    await press("Delete");

    expect(confirmDialog().textContent).toContain("Delete these 2 items?");
    // The question names what it takes: two rows at once, and the button counts them too.
    expect(confirmDialog().textContent).toContain("This cannot be undone");
  });

  it("renames the one row, and only when there is one", async () => {
    await render(many);

    await clickRow(1);
    await press("F2");
    expect(host.querySelector('input[aria-label="New name"]')).not.toBeNull();

    // Closed the way the panel closes it, rather than by submitting it: this case is about what F2
    // offers, not about what renaming does.
    host.querySelector<HTMLButtonElement>("form button[type='button']")?.click();
    await nextTick();
    expect(host.querySelector('input[aria-label="New name"]')).toBeNull();

    await clickRow(1);
    await clickRow(2, "ctrl");
    await press("F2");

    expect(host.querySelector('input[aria-label="New name"]')).toBeNull();
  });

  it("opens the one selected row on Enter, and nothing when several are chosen", async () => {
    await render(many);

    await clickRow(3);
    await press("Enter");
    await vi.waitFor(() => expect(vi.mocked(filesList)).toHaveBeenCalledWith(HANDLE, "/home/user/logs"));

    vi.mocked(filesList).mockClear();
    await clickRow(1);
    await clickRow(2, "ctrl");
    await press("Enter");

    expect(vi.mocked(filesList)).not.toHaveBeenCalled();
  });

  /**
   * The listing is a list of rows, and the browser has to be told that.
   *
   * Left to itself it treats every name and date as text: a shift-click then extends the *text*
   * selection from wherever the last click landed — which looked like the shift-selection had smeared
   * itself across the timestamps — and the pointer becomes an I-beam over every row. Both are the
   * browser's default for selectable content, and both are switched off by the container saying it is
   * not selectable: the class is the mechanism, and this is the case that would have caught it.
   */
  it("tells the browser the rows are not text to be selected", async () => {
    await render(many);

    const container = listing();
    expect(container?.classList.contains("select-none")).toBe(true);
    expect(container?.classList.contains("cursor-default")).toBe(true);
  });

  it("takes the rows a rectangle is dragged over", async () => {
    await render(many);
    const container = listing();
    if (!container) {
      throw new Error("the listing is not there");
    }
    // Laid out by hand: a test document gives every element a size of nothing, and a rectangle over
    // rows that are all zero tall would cover either all of them or none.
    container.getBoundingClientRect = () => ({ left: 0, top: 0, width: 600, height: 400 }) as DOMRect;
    for (const [index, row] of [...container.querySelectorAll<HTMLElement>("li[data-path]")].entries()) {
      Object.defineProperty(row, "offsetTop", { value: index * 30, configurable: true });
      Object.defineProperty(row, "offsetHeight", { value: 30, configurable: true });
    }

    container.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientY: 10 }));
    window.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, clientY: 50 }));
    await nextTick();
    window.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
    await nextTick();

    expect(selectedRows()).toEqual([0, 1]);
  });

  it("lets go of a row that is gone when the directory is read again", async () => {
    await render(many);

    await clickRow(0);
    await clickRow(1, "ctrl");
    expect(selectedRows()).toEqual([0, 1]);

    // What a delete or somebody else's change leaves behind: the listing reads itself again.
    const files = useFilesStore();
    const open = files.browsers[HANDLE];
    if (!open) {
      throw new Error("the browser is not there");
    }
    open.entries = many.slice(1);
    await nextTick();

    expect(selectedRows()).toEqual([0]);
  });

  it("reads a row in the columns a file manager lists it in", async () => {
    const owned: SftpEntry = {
      name: "notes.txt",
      path: "/home/user/notes.txt",
      kind: "file",
      size: 12,
      modifiedAt: 1_700_000_000,
      mode: 0o644,
      owner: "labuser",
      group: "labuser",
    };
    await render([folder, owned]);

    const headings = listing()?.querySelector("[data-listing-header]")?.textContent ?? "";
    for (const column of ["File name", "Size", "Type", "Modified", "Permissions", "User/Group"]) {
      expect(headings, `the header names the ${column} column`).toContain(column);
    }

    const [directory, file] = rows();
    expect(directory?.textContent, "a directory says what it is").toContain("Folder");
    expect(file?.textContent, "the permissions are the ones the server sent").toContain("rw-r--r--");
    expect(file?.textContent, "the owner and the group share their column").toContain("labuser/labuser");
  });

  it("leaves a row's actions to the menu, with nothing drawn on the row itself", async () => {
    await render(many);

    for (const row of rows()) {
      expect(row.querySelector("button"), "a row is not a toolbar").toBeNull();
    }

    // Nothing went away with the buttons: the same actions answer a right-click.
    const entries = await rightClickRow(0);
    expect(ids(entries)).toContain("download");
    expect(ids(entries)).toContain("rename");
    expect(ids(entries)).toContain("remove");
  });

  it("counts it in the menu's own words when several rows are chosen", async () => {
    await render(many);

    await clickRow(0);
    await clickRow(2, "ctrl");
    const entries = await rightClickRow(2);

    expect(useContextMenu().title.value).toEqual({ key: "menu.files", params: { count: "2" } });
    expect(ids(entries)).toEqual(["download", "sep-manage", "remove", "sep-copy", "copy-name", "copy-path"]);
    expect(pick(entries, "remove").labelKey).toBe("menu.deleteItems");
    expect(ids(entries)).not.toContain("rename");
  });

  it("keeps the whole selection when a row inside it is right-clicked", async () => {
    await render(many);

    await clickRow(0);
    await clickRow(2, "ctrl");
    await rightClickRow(0);
    expect(selectedRows()).toEqual([0, 2]);

    // And a row outside it becomes the selection, because the menu is about what was pointed at.
    await rightClickRow(3);
    expect(selectedRows()).toEqual([3]);
  });

  it("uses Command rather than Control on a Mac, where Control is the secondary click", async () => {
    await render(many);
    const app = useAppStore();
    // The store reads this from the backend on start; this case is about what the panel does with it.
    app.info = {
      name: "7Shell",
      version: "0.1.0",
      os: "macos",
      arch: "aarch64",
      debugBuild: true,
      protocolCount: 1,
      descriptorSchemaVersion: 1,
    };
    await nextTick();

    await clickRow(0);
    await clickRow(2, "ctrl");
    expect(selectedRows(), "control is the secondary click on a Mac").toEqual([2]);

    await clickRow(0);
    await clickRow(2, "meta");
    expect(selectedRows()).toEqual([0, 2]);
  });
});
