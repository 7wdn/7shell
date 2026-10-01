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
import { filesCreateFile, filesDownload } from "@/lib/backend/tauri";
import type { MenuAction, MenuEntry } from "@/lib/menu/types";
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
    // What the dialog asks for is a folder rather than a file name, and the switch travels with the
    // call: the archive is a vehicle that never reaches this side.
    expect(vi.mocked(filesDownload)).toHaveBeenCalledWith(
      HANDLE,
      "/home/user/logs",
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
      "/home/user/notes.txt",
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
