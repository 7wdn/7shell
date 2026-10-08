/**
 * What a file row and a directory's empty space offer.
 *
 * The list of entries is the interesting part, and it depends on three things at once: what kind of
 * thing was chosen, how many of them there are, and whether the packing switch is on.
 *
 * - A single file has the plain download it always had; several files have a folder to go into.
 * - A directory has no plain way down at all, so it is offered one only while packing is on.
 * - Editing, renaming and opening are one row at a time — they always were — while deleting, copying
 *   and taking are the whole selection.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import { copyText } from "./clipboard";
import { fileAreaMenu, fileEntryMenu } from "./files";
import type { MenuAction, MenuEntry } from "./types";
import type { SftpEntry } from "@/types/session";

vi.mock("./clipboard", () => ({ copyText: vi.fn(async () => true) }));

const plain = { packed: false };
const packed = { packed: true };

function entry(overrides: Partial<SftpEntry> = {}): SftpEntry {
  return { name: "notes.txt", path: "/home/user/notes.txt", kind: "file", size: 12, ...overrides };
}

function ids(entries: readonly MenuEntry[]): string[] {
  return entries.map((item) => item.id);
}

function pick(entries: readonly MenuEntry[], id: string): MenuAction {
  const found = entries.find((item): item is MenuAction => item.kind === "action" && item.id === id);
  if (!found) {
    throw new Error(`the menu has no entry called ${id}`);
  }
  return found;
}

/** The list a row of this menu opens, which is where the two kinds of "new" live. */
function submenu(entries: readonly MenuEntry[], id: string): readonly MenuEntry[] {
  const found = entries.find((item) => item.kind === "submenu" && item.id === id);
  return found?.kind === "submenu" ? found.entries : [];
}

function actions() {
  return {
    open: vi.fn(),
    download: vi.fn(),
    edit: vi.fn(),
    rename: vi.fn(),
    remove: vi.fn(),
  };
}

function areaActions() {
  return {
    newFolder: vi.fn(),
    newFile: vi.fn(),
    upload: vi.fn(),
    paste: vi.fn(),
    refresh: vi.fn(),
    togglePacked: vi.fn(),
  };
}

beforeEach(() => {
  vi.mocked(copyText).mockClear();
});

describe("a remote file's menu", () => {
  it("offers a file the editor and the download it already had", () => {
    expect(ids(fileEntryMenu([entry()], actions(), plain))).toEqual([
      "download",
      "edit",
      "sep-manage",
      "rename",
      "remove",
      "sep-copy",
      "copy-name",
      "copy-path",
    ]);
  });

  it("offers a directory opening, and nothing to take it with", () => {
    const directory = entry({ name: "logs", path: "/home/user/logs", kind: "directory" });
    expect(ids(fileEntryMenu([directory], actions(), plain))).toEqual([
      "open",
      "sep-manage",
      "rename",
      "remove",
      "sep-copy",
      "copy-name",
      "copy-path",
    ]);
  });

  it("offers a directory a download once packing makes one possible", () => {
    const directory = entry({ name: "logs", path: "/home/user/logs", kind: "directory" });
    expect(ids(fileEntryMenu([directory], actions(), packed))).toEqual([
      "open",
      "download-archive",
      "sep-manage",
      "rename",
      "remove",
      "sep-copy",
      "copy-name",
      "copy-path",
    ]);
  });

  it("leaves a single file alone when packing is on, because there is nothing to pack", () => {
    expect(ids(fileEntryMenu([entry()], actions(), packed))).toEqual(ids(fileEntryMenu([entry()], actions(), plain)));
  });

  it("offers a socket file nothing but managing it", () => {
    const socket = entry({ name: "sshd.sock", path: "/run/sshd.sock", kind: "other" });
    expect(ids(fileEntryMenu([socket], actions(), plain))).toEqual([
      "rename",
      "remove",
      "sep-copy",
      "copy-name",
      "copy-path",
    ]);
  });

  it("offers a link's target only when the server named one", () => {
    const link = entry({ kind: "symlink", linkTarget: "/etc/nginx/sites" });
    const bare = entry({ kind: "symlink" });

    expect(ids(fileEntryMenu([link], actions(), plain))).toContain("copy-target");
    expect(ids(fileEntryMenu([bare], actions(), plain))).not.toContain("copy-target");
  });

  it("wires every action to the entry it was opened on", () => {
    const target = entry();
    const wired = actions();
    const entries = fileEntryMenu([target], wired, plain);

    pick(entries, "download").run();
    pick(entries, "edit").run();
    pick(entries, "rename").run();
    pick(entries, "remove").run();

    expect(wired.download).toHaveBeenCalledWith([target], false);
    expect(wired.edit).toHaveBeenCalledWith(target);
    expect(wired.rename).toHaveBeenCalledWith(target);
    expect(wired.remove).toHaveBeenCalledWith([target]);
  });

  it("copies the name and the path", () => {
    const entries = fileEntryMenu([entry()], actions(), plain);

    pick(entries, "copy-name").run();
    pick(entries, "copy-path").run();

    expect(copyText).toHaveBeenCalledWith("notes.txt");
    expect(copyText).toHaveBeenCalledWith("/home/user/notes.txt");
  });

  it("marks deleting as destructive and nothing else", () => {
    const entries = fileEntryMenu([entry()], actions(), plain);
    const dangerous = entries.filter((item) => item.kind === "action" && item.danger === true);

    expect(dangerous.map((item) => item.id)).toEqual(["remove"]);
  });
});

describe("a menu over several rows", () => {
  const three = [
    entry({ name: "a.txt", path: "/home/a.txt" }),
    entry({ name: "b.txt", path: "/home/b.txt" }),
    entry({ name: "c.txt", path: "/home/c.txt" }),
  ];
  const mixed = [
    entry({ name: "logs", path: "/home/logs", kind: "directory" }),
    entry({ name: "a.txt", path: "/home/a.txt" }),
  ];

  it("counts them in the entries that are about the whole selection", () => {
    const entries = fileEntryMenu(three, actions(), plain);

    expect(pick(entries, "download").labelKey).toBe("menu.downloadSeveral");
    expect(pick(entries, "download").params).toEqual({ count: "3" });
    expect(pick(entries, "remove").labelKey).toBe("menu.deleteItems");
    expect(pick(entries, "remove").params).toEqual({ count: "3" });
    expect(pick(entries, "copy-name").labelKey).toBe("menu.copyNames");
    expect(pick(entries, "copy-path").labelKey).toBe("menu.copyPaths");
  });

  it("leaves out the entries that are one row at a time", () => {
    const entries = ids(fileEntryMenu(three, actions(), plain));

    expect(entries).not.toContain("open");
    expect(entries).not.toContain("edit");
    expect(entries).not.toContain("rename");
    expect(entries).not.toContain("copy-target");
  });

  it("takes all of them when the packed switch is on, whatever they are", () => {
    const wired = actions();
    const entries = fileEntryMenu(mixed, wired, packed);

    expect(ids(entries)).toContain("download-archive");
    expect(pick(entries, "download-archive").labelKey).toBe("menu.downloadItems");

    pick(entries, "download-archive").run();
    expect(wired.download).toHaveBeenCalledWith(mixed, true);
  });

  it("offers the unpacked way only when every one of them is a file", () => {
    // A directory has no unpacked way down: there would be nothing to ask for file by file.
    expect(ids(fileEntryMenu(mixed, actions(), plain))).not.toContain("download");
    expect(ids(fileEntryMenu(three, actions(), plain))).toContain("download");
  });

  it("copies every path on its own line", () => {
    const entries = fileEntryMenu(three, actions(), plain);

    pick(entries, "copy-path").run();

    expect(copyText).toHaveBeenCalledWith("/home/a.txt\n/home/b.txt\n/home/c.txt");
  });

  it("asks to delete all of them at once", () => {
    const wired = actions();
    const entries = fileEntryMenu(three, wired, plain);

    pick(entries, "remove").run();

    expect(wired.remove).toHaveBeenCalledWith(three);
  });
});

describe("the directory's own menu", () => {
  it("offers the panel's actions, the packing switch and the two kinds of new", () => {
    const entries = fileAreaMenu({ path: "/var/log", packed: false }, areaActions());

    expect(ids(entries)).toEqual([
      "refresh",
      "sep-transfer",
      "upload",
      "paste",
      "packed",
      "sep-new",
      "new",
      "sep-copy",
      "copy-path",
    ]);
    expect(ids(submenu(entries, "new"))).toEqual(["new-folder", "new-file"]);
  });

  it("prints the one shortcut that really works here, as a binding the menu layer spells out", () => {
    const entries = fileAreaMenu({ path: "/var/log", packed: false }, areaActions());

    expect(pick(entries, "paste").accel).toBe("Mod+V");
  });

  it("says whether packing is on, and turns it over when pressed", () => {
    const off = fileAreaMenu({ path: "/var/log", packed: false }, areaActions());
    expect(pick(off, "packed").checked).toBe(false);

    const wired = areaActions();
    const on = fileAreaMenu({ path: "/var/log", packed: true }, wired);
    expect(pick(on, "packed").checked).toBe(true);

    pick(on, "packed").run();
    expect(wired.togglePacked).toHaveBeenCalledOnce();
  });

  it("renames uploading when packing turns it into an unpacking", () => {
    const plain = fileAreaMenu({ path: "/var/log", packed: false }, areaActions());
    const packed = fileAreaMenu({ path: "/var/log", packed: true }, areaActions());

    expect(pick(plain, "upload").labelKey).toBe("files.upload");
    expect(pick(packed, "upload").labelKey).toBe("menu.packedUpload");
  });

  it("opens the two prompts through their own entries", () => {
    const wired = areaActions();
    const entries = fileAreaMenu({ path: "/var/log", packed: false }, wired);
    const newEntries = submenu(entries, "new");

    pick(newEntries, "new-folder").run();
    pick(newEntries, "new-file").run();

    expect(wired.newFolder).toHaveBeenCalledOnce();
    expect(wired.newFile).toHaveBeenCalledOnce();
  });

  it("refuses to copy a directory nobody has read yet", () => {
    const entries = fileAreaMenu({ path: "", packed: false }, areaActions());

    expect(pick(entries, "copy-path").disabled).toBe(true);
  });
});
