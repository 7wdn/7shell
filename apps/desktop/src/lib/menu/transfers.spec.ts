/**
 * What a transfer row offers.
 *
 * Three groups: the two entries about the file itself, which depend on whether this side holds a
 * local end and on whether the transfer got there; whatever that row needs next — hold a running
 * transfer, let a paused one go, try a stopped one again; and then the two ways of being finished
 * with it, which are as different as they sound. Stopping keeps the row, so the outcome stays
 * visible; removing takes it away, and stops the bytes first if there are still any.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import { copyText } from "./clipboard";
import { transferMenu, type TransferMenuActions } from "./transfers";
import type { MenuAction, MenuEntry } from "./types";
import type { TransferInfo } from "@/types/session";

vi.mock("./clipboard", () => ({ copyText: vi.fn(async () => true) }));

function transfer(overrides: Partial<TransferInfo> = {}): TransferInfo {
  return {
    id: "transfer-1",
    direction: "download",
    name: "nginx.conf",
    remotePath: "/etc/nginx/nginx.conf",
    sessionId: "s1",
    size: 1024,
    transferred: 512,
    state: { kind: "running" },
    hasLocalPath: true,
    packed: false,
    ...overrides,
  };
}

/** Every action the menu can ask for, each its own spy. */
function actions(): TransferMenuActions {
  return {
    stop: vi.fn<(id: string) => void>(),
    remove: vi.fn<(id: string) => void>(),
    open: vi.fn<(id: string) => void>(),
    reveal: vi.fn<(id: string) => void>(),
    setPaused: vi.fn<(id: string, paused: boolean) => void>(),
    retry: vi.fn<(id: string) => void>(),
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

beforeEach(() => {
  vi.mocked(copyText).mockClear();
});

describe("a transfer's menu", () => {
  it("offers to hold a running transfer, to stop it, and to take the row away", () => {
    const entries = transferMenu(transfer(), actions());

    expect(ids(entries)).toEqual(["reveal", "pause", "stop", "remove", "copy-path"]);
    expect(pick(entries, "stop").labelKey).toBe("files.transfer.cancel");
    expect(pick(entries, "remove").labelKey).toBe("files.transfer.remove");
  });

  it("offers to continue a paused transfer rather than to hold it again", () => {
    expect(ids(transferMenu(transfer({ state: { kind: "paused" } }), actions()))).toEqual([
      "reveal",
      "resume",
      "stop",
      "remove",
      "copy-path",
    ]);
  });

  it("offers to try a stopped transfer again, and to take the row away", () => {
    expect(ids(transferMenu(transfer({ state: { kind: "failed", detail: "the channel failed" } }), actions()))).toEqual(
      ["reveal", "retry", "remove", "copy-path"],
    );
  });

  it("offers to open a finished transfer, and to remove its row", () => {
    const entries = transferMenu(transfer({ state: { kind: "done" } }), actions());

    expect(ids(entries)).toEqual(["open", "reveal", "remove", "copy-path"]);
  });

  it("does not offer to open a transfer that has not finished", () => {
    // A part file handed to an application looks like a file that is merely short, which is the trap
    // this entry waits for `done` to avoid — and pointing at where it is stays available, because
    // that is how the part file is found and deleted.
    for (const state of [{ kind: "running" }, { kind: "paused" }, { kind: "cancelled" }] as const) {
      expect(ids(transferMenu(transfer({ state }), actions()))).not.toContain("open");
      expect(ids(transferMenu(transfer({ state }), actions()))).toContain("reveal");
    }
  });

  it("offers neither entry about the file when there is no local end to point at", () => {
    // A packed upload: the archive that carried the files was deleted once it had been read.
    const entries = transferMenu(transfer({ direction: "upload", packed: true, hasLocalPath: false }), actions());

    expect(ids(entries)).toEqual(["pause", "stop", "remove", "copy-path"]);
  });

  it("holds, continues and retries the transfer it was opened on", () => {
    const calls = actions();

    pick(transferMenu(transfer({ id: "transfer-9" }), calls), "pause").run();
    expect(calls.setPaused).toHaveBeenCalledWith("transfer-9", true);

    pick(transferMenu(transfer({ id: "transfer-9", state: { kind: "paused" } }), calls), "resume").run();
    expect(calls.setPaused).toHaveBeenLastCalledWith("transfer-9", false);

    pick(transferMenu(transfer({ id: "transfer-9", state: { kind: "failed", detail: "x" } }), calls), "retry").run();
    expect(calls.retry).toHaveBeenCalledWith("transfer-9");
  });

  it("stops and removes by id, because that is all this side knows of a transfer", () => {
    const calls = actions();
    const running = transferMenu(transfer({ id: "transfer-9" }), calls);

    pick(running, "stop").run();
    pick(running, "remove").run();

    expect(calls.stop).toHaveBeenCalledWith("transfer-9");
    expect(calls.remove).toHaveBeenCalledWith("transfer-9");
  });

  it("opens and reveals by id, never by path", () => {
    // The local end is looked up on the other side; the interface has never held it.
    const calls = actions();
    const done = transferMenu(transfer({ id: "transfer-9", state: { kind: "done" } }), calls);

    pick(done, "open").run();
    pick(done, "reveal").run();

    expect(calls.open).toHaveBeenCalledWith("transfer-9");
    expect(calls.reveal).toHaveBeenCalledWith("transfer-9");
  });

  it("copies the remote path, which is the only end the interface ever held", () => {
    pick(transferMenu(transfer(), actions()), "copy-path").run();

    expect(copyText).toHaveBeenCalledWith("/etc/nginx/nginx.conf");
  });
});
