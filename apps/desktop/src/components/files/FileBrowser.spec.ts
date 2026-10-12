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

/**
 * A directory deliberately not in the order a server would send it.
 *
 * The order the panel draws is the claim in these cases, so the rows arriving shuffled is what keeps
 * them from passing by handing back what they were given.
 */
const unsorted: SftpEntry[] = [
  { name: "b.txt", path: "/home/user/b.txt", kind: "file", size: 30 },
  { name: "logs", path: "/home/user/logs", kind: "directory", size: 0 },
  { name: "c.txt", path: "/home/user/c.txt", kind: "file", size: 10 },
  { name: "a.txt", path: "/home/user/a.txt", kind: "file", size: 20 },
];

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

/** The paths drawn as selected, in the order the listing draws them. */
function selectedPaths(): string[] {
  return rows()
    .filter((row) => row.getAttribute("aria-selected") === "true")
    .map((row) => row.parentElement?.dataset.path ?? "");
}

/** The rows as they are drawn, by file name, which is the order the headings decide. */
function drawn(): string[] {
  return [...(listing()?.querySelectorAll<HTMLElement>("li[data-path]") ?? [])].map(
    (row) => row.dataset.path?.split("/").pop() ?? "",
  );
}

/** Presses the heading that says this, which is how a person orders the listing. */
async function pressHeading(label: string): Promise<void> {
  const heading = [...(listing()?.querySelectorAll<HTMLButtonElement>("[data-listing-header] button") ?? [])].find(
    (button) => button.textContent?.trim() === label,
  );
  if (!heading) {
    throw new Error(`no heading says ${label}`);
  }
  heading.click();
  await nextTick();
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
  // The order a server sends a directory in: the folders first, then the names. It matters that the
  // fixture is in it, because the panel now draws every directory in that order — so a case that
  // reaches for "the third row" is reaching for the third row somebody would see.
  const many: SftpEntry[] = [
    { name: "logs", path: "/home/user/logs", kind: "directory", size: 0 },
    { name: "a.txt", path: "/home/user/a.txt", kind: "file", size: 1 },
    { name: "b.txt", path: "/home/user/b.txt", kind: "file", size: 2 },
    { name: "c.txt", path: "/home/user/c.txt", kind: "file", size: 3 },
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

    await clickRow(0);
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

  /**
   * The headings are pinned above rows that scroll under them.
   *
   * A translucent strip there is not a tint over the rows, it is a window onto them: the row that
   * has scrolled underneath is read through it, which is two lines of text written on top of each
   * other. So the headings carry the surface's own colour, and the class is what says so.
   */
  it("draws the column headings over the rows rather than through them", async () => {
    await render(many);

    const headings = listing()?.querySelector("[data-listing-header]");
    expect(headings?.classList.contains("sticky")).toBe(true);
    expect(headings?.classList.contains("bg-background"), "the pinned strip is opaque").toBe(true);
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
    expect(file?.textContent, "a file says its type rather than only that it is one").toContain("Text file");
    expect(file?.textContent, "the permissions are the ones the server sent").toContain("rw-r--r--");
    expect(file?.textContent, "the owner and the group share their column").toContain("labuser/labuser");
  });

  it("orders the rows by a heading, and turns them around when it is pressed again", async () => {
    await render(unsorted);

    expect(drawn(), "the heading it opens on is the order the server sends: folders, then names").toEqual([
      "logs",
      "a.txt",
      "b.txt",
      "c.txt",
    ]);

    await pressHeading("Size");
    expect(drawn(), "the biggest file first, with the folder still above it").toEqual([
      "logs",
      "b.txt",
      "a.txt",
      "c.txt",
    ]);

    await pressHeading("Size");
    expect(drawn(), "and the smallest first when the same heading is pressed again").toEqual([
      "logs",
      "c.txt",
      "a.txt",
      "b.txt",
    ]);
  });

  it("orders the rows by the type it shows, and puts the folders last when it is turned over", async () => {
    await render([
      { name: "b.rpm", path: "/home/user/b.rpm", kind: "file", size: 2 },
      { name: "logs", path: "/home/user/logs", kind: "directory", size: 0 },
      { name: "a.deb", path: "/home/user/a.deb", kind: "file", size: 1 },
      { name: "c.tar.gz", path: "/home/user/c.tar.gz", kind: "file", size: 3 },
      { name: "d.txt", path: "/home/user/d.txt", kind: "file", size: 4 },
    ]);

    await pressHeading("Type");
    expect(drawn(), "folders first, then the types from the alphabet").toEqual([
      "logs",
      "c.tar.gz",
      "a.deb",
      "b.rpm",
      "d.txt",
    ]);

    await pressHeading("Type");
    expect(drawn(), "and the folders go over with the column, the way a file manager's does").toEqual([
      "d.txt",
      "b.rpm",
      "a.deb",
      "c.tar.gz",
      "logs",
    ]);
  });

  it("takes a range over the rows as they are drawn rather than as they arrived", async () => {
    await render(unsorted);
    await pressHeading("Size");

    await clickRow(1);
    await clickRow(3, "shift");

    expect(selectedPaths(), "the rows between the two clicks, in the order they are drawn").toEqual([
      "/home/user/b.txt",
      "/home/user/a.txt",
      "/home/user/c.txt",
    ]);
  });

  it("leaves a row's actions to the menu, with nothing drawn on the row itself", async () => {
    await render(many);

    for (const row of rows()) {
      expect(row.querySelector("button"), "a row is not a toolbar").toBeNull();
    }

    // Nothing went away with the buttons: the same actions answer a right-click.
    const entries = await rightClickRow(1);
    expect(ids(entries)).toContain("download");
    expect(ids(entries)).toContain("rename");
    expect(ids(entries)).toContain("remove");
  });

  it("counts it in the menu's own words when several rows are chosen", async () => {
    await render(many);

    await clickRow(1);
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

  it("leaves what somebody typed in the path bar alone when an answer lands", async () => {
    await render(unsorted);

    const field = host.querySelector<HTMLInputElement>('input[placeholder="Path"]');
    if (field === null) {
      throw new Error("the panel drew no path bar");
    }
    expect(field.value, "the field shows where the panel is").toBe("/home/user");

    // Somebody starts typing the next path while the previous read is still in flight...
    field.value = "/home/user/logs";
    field.dispatchEvent(new Event("input"));
    await nextTick();

    // ...and the answer to the navigation before it lands.
    const browser = useFilesStore().browsers[HANDLE];
    if (browser === undefined) {
      throw new Error("the browser went away");
    }
    browser.path = "/";
    await nextTick();

    expect(field.value, "an answer does not put the old path back under somebody's hands").toBe("/home/user/logs");
  });
});

/**
 * The columns: one width per column, shared by the heading and the rows.
 *
 * The two used to write their widths down separately — `w-24` over `w-20` — and were sixteen pixels
 * apart for as long as nobody looked, so what is asserted here is that they cannot be: a heading and
 * the cell under it read the same variable, and a drag writes that variable and nothing else.
 *
 * A test document lays nothing out, so the room the columns have is told rather than measured: the
 * width is defined on the strip and the observers are then run, which is what a real one does when
 * the window changes size. Without that the handle is told nothing about the room — nought — and
 * every drag stops at the column's own floor.
 */
describe("the columns of the listing", () => {
  /** Whatever asked to be told when the strip changes size, so that a case can tell it. */
  let watching: (() => void)[] = [];

  beforeEach(() => {
    watching = [];
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(notify: () => void) {
          watching.push(notify);
        }
        observe(): void {}
        disconnect(): void {}
      },
    );
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  /** The strip the headings are drawn in, whose width is the room the columns have. */
  function header(): HTMLElement {
    const strip = listing()?.querySelector<HTMLElement>("[data-listing-header]");
    if (!strip) {
      throw new Error("the listing drew no headings");
    }
    return strip;
  }

  /** Says how wide the strip is, and lets whatever is watching it hear about it. */
  async function room(width: number): Promise<void> {
    Object.defineProperty(header(), "clientWidth", { value: width });
    for (const notify of watching) {
      notify();
    }
    await nextTick();
  }

  /** The edge of one column, as it is drawn. */
  function edge(key: string): HTMLElement {
    const found = listing()?.querySelector<HTMLElement>(`[data-column-divider][data-column='${key}']`);
    if (!found) {
      throw new Error(`no edge is drawn for the ${key} column`);
    }
    return found;
  }

  /** The width a column is drawn at, as the scroller both the strip and the rows are inside is told. */
  function columnWidth(key: string): string {
    return listing()?.style.getPropertyValue(`--col-${key}`) ?? "";
  }

  /** The width variable a cell is drawn with, or nothing for a cell that is not a column of its own. */
  function widthOfCell(cell: Element | undefined): string | null {
    if (!(cell instanceof HTMLElement)) {
      return null;
    }
    return /w-\[var\((--col-[\w-]+)\)\]/.exec(cell.className)?.[1] ?? null;
  }

  /** Drags a column's edge this many pixels to the right, the way a pointer does. */
  async function dragEdge(key: string, delta: number): Promise<void> {
    const element = edge(key);
    element.setPointerCapture = vi.fn();
    element.releasePointerCapture = vi.fn();
    element.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, button: 0, clientX: 400 }));
    element.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, clientX: 400 + delta }));
    element.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
    await nextTick();
  }

  it("gives every column one width, which its heading and its rows both read", async () => {
    await render(unsorted);

    const edges = [...header().querySelectorAll<HTMLElement>("[data-column-divider]")];
    const cells = [...(rows()[0]?.children ?? [])];

    // Every column, the name one included: a column whose edge answered to nothing would read as a
    // broken one rather than as a column with a rule of its own.
    expect(edges.map((mark) => widthOfCell(mark.parentElement ?? undefined))).toEqual([
      "--col-name",
      "--col-size",
      "--col-kind",
      "--col-modified",
      "--col-mode",
      "--col-owner",
    ]);
    expect(edges.map((mark) => widthOfCell(mark.parentElement ?? undefined))).toEqual(cells.map(widthOfCell));
    // And the width is written where both of them can read it — the element they are both inside —
    // rather than on the strip, which the rows are beside rather than within.
    expect(header().style.getPropertyValue("--col-size"), "the strip declares no width of its own").toBe("");
  });

  it("tells the line how narrow it may be drawn, so a narrow panel scrolls instead of squeezing", async () => {
    await render(unsorted);

    // Twelve pixels of padding on each side, eight between six cells, and the six columns at
    // 320 + 80 + 80 + 160 + 96 + 112.
    expect(header().style.minWidth).toBe("912px");
    expect(listing()?.querySelector<HTMLElement>("ul.p-1\\.5")?.style.minWidth).toBe("912px");
  });

  it("divides the room a panel has to spare between the columns nobody has moved", async () => {
    await render(unsorted);

    // 1200 wide, 64 spent on chrome: the columns have 1136 and ask for 848, so each of the six takes
    // 48 of the 288 left over.
    await room(1200);

    expect(columnWidth("name")).toBe("368px");
    expect(columnWidth("size")).toBe("128px");
    expect(columnWidth("owner")).toBe("160px");
  });

  it("leaves a column that has been moved out of that division", async () => {
    await render(unsorted);
    await room(1200);
    await dragEdge("size", 40);

    // The size column is the one somebody put at 168, and the other five divide what is left — 200 of
    // it, 40 each, so the name column is drawn at 360 rather than at the 368 it was drawn at before.
    expect(columnWidth("size")).toBe("168px");
    expect(columnWidth("name")).toBe("360px");
  });

  it("widens a column when its edge is dragged, and the rows follow it", async () => {
    await render(unsorted);
    await room(900);

    expect(columnWidth("size")).toBe("80px");
    await dragEdge("size", 40);

    expect(columnWidth("size")).toBe("120px");
  });

  it("lets a drag go as wide as the pointer asks, however wide the panel is", async () => {
    await render(unsorted);
    await room(900);

    // No ceiling: a column wider than the panel is what the strip's own scrollbar is for, and the
    // line's least width grows with it so that there is something to scroll.
    await dragEdge("size", 2000);

    expect(columnWidth("size")).toBe("2080px");
    expect(header().style.minWidth).toBe("2912px");
  });

  it("keeps every column off its own floor", async () => {
    await render(unsorted);
    await room(900);

    await dragEdge("name", -2000);

    expect(columnWidth("name"), "a name column narrower than this says nothing").toBe("160px");
  });

  it("hands a column back to the width the panel draws it at when its edge is double-clicked", async () => {
    await render(unsorted);
    await room(900);
    await dragEdge("size", 40);

    edge("size").dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    await nextTick();

    expect(columnWidth("size")).toBe("80px");
  });
});
