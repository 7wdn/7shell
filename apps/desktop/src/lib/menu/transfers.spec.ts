/**
 * What a transfer row offers.
 *
 * One entry plus the copy, and the entry is whatever that row needs next: hold a running transfer,
 * let a paused one go, try a stopped one again, dismiss a finished one. A transfer that was cancelled
 * is dismissed rather than cancelled — calling it "Cancel" would promise something no longer
 * possible — and a failed one offers the retry rather than the dismissal, because somebody looking at
 * a failure wants it to work rather than to go away.
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
    size: 1024,
    transferred: 512,
    state: { kind: "running" },
    packed: false,
    ...overrides,
  };
}

/** The three actions the menu can ask for, each its own spy. */
function actions(): TransferMenuActions {
  return {
    cancel: vi.fn<(id: string) => void>(),
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
  it("offers to hold a running transfer, and to cancel it", () => {
    const entries = transferMenu(transfer(), actions());

    expect(ids(entries)).toEqual(["pause", "cancel", "copy-path"]);
    expect(pick(entries, "cancel").labelKey).toBe("files.transfer.cancel");
  });

  it("offers to continue a paused transfer rather than to hold it again", () => {
    const entries = transferMenu(transfer({ state: { kind: "paused" } }), actions());

    expect(ids(entries)).toEqual(["resume", "cancel", "copy-path"]);
  });

  it("offers to try a stopped transfer again, and to close the row", () => {
    const entries = transferMenu(transfer({ state: { kind: "failed", detail: "the channel failed" } }), actions());

    expect(ids(entries)).toEqual(["retry", "dismiss", "copy-path"]);
  });

  it("calls a finished transfer's only entry Close", () => {
    const entries = transferMenu(transfer({ state: { kind: "done" } }), actions());

    expect(ids(entries)).toEqual(["dismiss", "copy-path"]);
    expect(pick(entries, "dismiss").labelKey).toBe("files.transfer.dismiss");
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

  it("ends the transfer it was opened on", () => {
    const calls = actions();
    const entries = transferMenu(transfer({ id: "transfer-9" }), calls);

    pick(entries, "cancel").run();

    expect(calls.cancel).toHaveBeenCalledWith("transfer-9");
  });

  it("copies the remote path, which is the only end the interface ever held", () => {
    pick(transferMenu(transfer(), actions()), "copy-path").run();

    expect(copyText).toHaveBeenCalledWith("/etc/nginx/nginx.conf");
  });
});
