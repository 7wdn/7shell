/**
 * The list behind the download folder: the folders remembered, and the two ways out.
 *
 * What the menu is *about* is a set of names — the interface is never told where a folder is, so the
 * only thing a test can hold it to is the order, the tick and which action each row reaches.
 */
import { describe, expect, it, vi } from "vitest";

import { downloadFolderMenu, type DownloadFolderMenuActions } from "./downloads";
import type { MenuAction, MenuEntry } from "./types";
import type { DownloadFolderView } from "@/types/session";

function actions(): DownloadFolderMenuActions {
  return {
    use: vi.fn<(index: number) => void>(),
    choose: vi.fn<() => void>(),
    ask: vi.fn<() => void>(),
  };
}

function pick(entries: readonly MenuEntry[], id: string): MenuAction {
  const found = entries.find((entry): entry is MenuAction => entry.kind === "action" && entry.id === id);
  if (!found) {
    throw new Error(`the menu has no entry called ${id}`);
  }
  return found;
}

function ids(entries: readonly MenuEntry[]): string[] {
  return entries.filter((entry): entry is MenuAction => entry.kind === "action").map((entry) => entry.id);
}

describe("the download folder's menu", () => {
  it("lists the folders that were used, newest first, and the two ways out", () => {
    const view: DownloadFolderView = { current: "fsdownload", history: ["fsdownload", "Downloads", "Desktop"] };

    const entries = downloadFolderMenu(view, actions());

    expect(ids(entries)).toEqual(["folder-0", "folder-1", "folder-2", "choose", "ask"]);
    expect(pick(entries, "folder-1").params).toEqual({ name: "Downloads" });
    expect(pick(entries, "folder-1").labelKey).toBe("files.transfer.folder.remembered");
  });

  it("ticks the folder in force, by position rather than by name", () => {
    // Two folders on a machine can share their last segment, which is all this side is told, so a
    // tick decided by the name would land on both of them.
    const view: DownloadFolderView = { current: "downloads", history: ["downloads", "downloads"] };

    const entries = downloadFolderMenu(view, actions());

    expect(pick(entries, "folder-0").checked).toBe(true);
    expect(pick(entries, "folder-1").checked).toBe(false);
    expect(pick(entries, "ask").checked).toBe(false);
  });

  it("ticks asking when that is what is in force, and ticks no folder", () => {
    const view: DownloadFolderView = { current: null, history: ["fsdownload"] };

    const entries = downloadFolderMenu(view, actions());

    expect(pick(entries, "folder-0").checked).toBe(false);
    expect(pick(entries, "ask").checked).toBe(true);
  });

  it("has nothing to list before a folder has ever been chosen", () => {
    const view: DownloadFolderView = { current: null, history: [] };

    expect(ids(downloadFolderMenu(view, actions()))).toEqual(["choose", "ask"]);
  });

  it("makes the folder it was opened on the one downloads land in", () => {
    const calls = actions();
    const view: DownloadFolderView = { current: "fsdownload", history: ["fsdownload", "Downloads"] };

    pick(downloadFolderMenu(view, calls), "folder-1").run();
    pick(downloadFolderMenu(view, calls), "choose").run();
    pick(downloadFolderMenu(view, calls), "ask").run();

    expect(calls.use).toHaveBeenCalledWith(1);
    expect(calls.choose).toHaveBeenCalled();
    expect(calls.ask).toHaveBeenCalled();
  });
});
