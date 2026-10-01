/**
 * What the monitor panel's small tables offer.
 *
 * A row's way out is the page that shows the same subject in full — the same call its section heading
 * makes — and then the two values somebody would want in another window.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import { copyText } from "./clipboard";
import { diskMenu, monitorProcessMenu } from "./monitor";
import type { MenuAction, MenuEntry } from "./types";
import type { DiskUsage, ProcessSample } from "@/types/monitor";

vi.mock("./clipboard", () => ({ copyText: vi.fn(async () => true) }));

const process: ProcessSample = {
  pid: 4242,
  user: "postgres",
  name: "postgres",
  command: "/usr/lib/postgresql/16/bin/postgres",
  executable: null,
  cpuPercent: 12.5,
  memoryPercent: 8.1,
  residentBytes: 200_000_000,
  cpuTicks: 900,
};

const disk: DiskUsage = {
  mountPoint: "/var/lib/postgresql",
  filesystem: "/dev/sdb1",
  sizeBytes: 500_000_000_000,
  availableBytes: 120_000_000_000,
};

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

describe("the panel's process rows", () => {
  it("opens the process page and copies the two things a row is", () => {
    const openPage = vi.fn();
    const entries = monitorProcessMenu(process, { openPage });

    expect(entries.filter((entry) => entry.kind === "action").map((entry) => entry.id)).toEqual([
      "open-page",
      "copy-pid",
      "copy-name",
    ]);

    pick(entries, "open-page").run();
    expect(openPage).toHaveBeenCalledOnce();

    pick(entries, "copy-pid").run();
    pick(entries, "copy-name").run();
    expect(copyText).toHaveBeenCalledWith("4242");
    expect(copyText).toHaveBeenCalledWith("postgres");
  });
});

describe("the panel's file system rows", () => {
  it("opens the system page and copies both ends of a mount", () => {
    const openPage = vi.fn();
    const entries = diskMenu(disk, { openPage });

    pick(entries, "open-page").run();
    expect(openPage).toHaveBeenCalledOnce();

    pick(entries, "copy-mount").run();
    pick(entries, "copy-filesystem").run();
    expect(copyText).toHaveBeenCalledWith("/var/lib/postgresql");
    expect(copyText).toHaveBeenCalledWith("/dev/sdb1");
  });
});
