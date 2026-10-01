/**
 * What a file row and a directory's empty space offer.
 *
 * The list of entries is the interesting part, and it is different for each kind of thing in a
 * listing: a socket file cannot be downloaded, a directory cannot be read in the editor, and a
 * directory cannot be *taken* at all until packed transfers are on. The last of those is the one
 * worth stating twice — a menu that offered a directory download while there is no way to do it
 * would be offering a lie.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import { copyText } from "./clipboard";
import { fileAreaMenu, fileEntryMenu } from "./files";
import type { MenuAction, MenuEntry } from "./types";
import type { SftpEntry } from "@/types/session";

vi.mock("./clipboard", () => ({ copyText: vi.fn(async () => true) }));

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
    downloadArchive: vi.fn(),
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
  const plain = { packed: false };
  const packed = { packed: true };

  it("offers a file the editor and the download it already had", () => {
    expect(ids(fileEntryMenu(entry(), actions(), plain))).toEqual([
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
    expect(ids(fileEntryMenu(directory, actions(), plain))).toEqual([
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
    expect(ids(fileEntryMenu(directory, actions(), packed))).toEqual([
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
    expect(ids(fileEntryMenu(entry(), actions(), packed))).toEqual(ids(fileEntryMenu(entry(), actions(), plain)));
  });

  it("offers a socket file nothing but managing it", () => {
    const socket = entry({ name: "sshd.sock", path: "/run/sshd.sock", kind: "other" });
    expect(ids(fileEntryMenu(socket, actions(), plain))).toEqual([
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

    expect(ids(fileEntryMenu(link, actions(), plain))).toContain("copy-target");
    expect(ids(fileEntryMenu(bare, actions(), plain))).not.toContain("copy-target");
  });

  it("wires every action to the entry it was opened on", () => {
    const target = entry();
    const wired = actions();
    const entries = fileEntryMenu(target, wired, plain);

    for (const id of ["download", "edit", "rename", "remove"]) {
      pick(entries, id).run();
    }

    expect(wired.download).toHaveBeenCalledWith(target);
    expect(wired.edit).toHaveBeenCalledWith(target);
    expect(wired.rename).toHaveBeenCalledWith(target);
    expect(wired.remove).toHaveBeenCalledWith(target);
  });

  it("sends a directory's download down the packing path, not the plain one", () => {
    const directory = entry({ name: "logs", path: "/home/user/logs", kind: "directory" });
    const wired = actions();
    const entries = fileEntryMenu(directory, wired, packed);

    pick(entries, "open").run();
    pick(entries, "download-archive").run();

    expect(wired.open).toHaveBeenCalledWith(directory);
    expect(wired.downloadArchive).toHaveBeenCalledWith(directory);
    expect(wired.download).not.toHaveBeenCalled();
  });

  it("copies the name and the path", () => {
    const entries = fileEntryMenu(entry(), actions(), plain);

    pick(entries, "copy-name").run();
    pick(entries, "copy-path").run();

    expect(copyText).toHaveBeenCalledWith("notes.txt");
    expect(copyText).toHaveBeenCalledWith("/home/user/notes.txt");
  });

  it("marks deleting as destructive and nothing else", () => {
    const entries = fileEntryMenu(entry(), actions(), plain);
    const dangerous = entries.filter((item) => item.kind === "action" && item.danger === true);

    expect(dangerous.map((item) => item.id)).toEqual(["remove"]);
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

  it("prints the one shortcut that really works here", () => {
    const entries = fileAreaMenu({ path: "/var/log", packed: false }, areaActions());

    expect(pick(entries, "paste").accel).toBe("Ctrl+V");
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
