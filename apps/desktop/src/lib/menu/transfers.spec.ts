/**
 * What a transfer row offers.
 *
 * The same call, under two words: a transfer that is running is cancelled and one that has finished is
 * dismissed, and calling the second "Cancel" would promise something that is no longer possible.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import { copyText } from "./clipboard";
import { transferMenu } from "./transfers";
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

function pick(entries: readonly MenuEntry[], id: string): MenuAction {
  const found = entries.find((entry): entry is MenuAction => entry.kind === "action" && entry.id === id);
  if (!found) {
    throw new Error(`the menu has no entry called ${id}`);
  }
  return found;
}

beforeEach(() => {
  vi.mocked(copyText).mockClear();
});

describe("a transfer's menu", () => {
  it("calls a running transfer's only entry Cancel", () => {
    expect(pick(transferMenu(transfer(), { cancel: vi.fn() }), "cancel").labelKey).toBe("files.transfer.cancel");
  });

  it("calls a finished transfer's only entry Close", () => {
    const done = transfer({ state: { kind: "done" } });

    expect(pick(transferMenu(done, { cancel: vi.fn() }), "cancel").labelKey).toBe("files.transfer.dismiss");
  });

  it("ends the transfer it was opened on", () => {
    const cancel = vi.fn();
    const entries = transferMenu(transfer({ id: "transfer-9" }), { cancel });

    pick(entries, "cancel").run();

    expect(cancel).toHaveBeenCalledWith("transfer-9");
  });

  it("copies the remote path, which is the only end the interface ever held", () => {
    pick(transferMenu(transfer(), { cancel: vi.fn() }), "copy-path").run();

    expect(copyText).toHaveBeenCalledWith("/etc/nginx/nginx.conf");
  });
});
