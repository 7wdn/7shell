/**
 * What the two host pages offer.
 *
 * A process row knows how it was started; a socket row knows who is on it. Both menus are mostly
 * copies, and the rules that matter are about what is *not* offered: a kernel thread has no
 * executable path, a socket nobody claims has no process, and copying an empty string would be a lie
 * told quietly.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import { copyText } from "./clipboard";
import { connectionMenu, environmentMenu, listenerMenu, processDetailMenu, processRowMenu } from "./inspect";
import type { MenuAction, MenuEntry } from "./types";
import type { ProcessDetail, ProcessSample, SocketRow } from "@/types/monitor";

vi.mock("./clipboard", () => ({ copyText: vi.fn(async () => true) }));

function process(overrides: Partial<ProcessSample> = {}): ProcessSample {
  return {
    pid: 1,
    user: "root",
    name: "nginx",
    command: "nginx: worker process",
    executable: "/usr/sbin/nginx",
    cpuPercent: 0.4,
    memoryPercent: 1.2,
    residentBytes: 12_000_000,
    cpuTicks: 100,
    ...overrides,
  };
}

const detail: ProcessDetail = {
  pid: 1,
  name: "nginx",
  commandLine: "nginx: worker process",
  executable: "/usr/sbin/nginx",
  workingDirectory: "/",
  environment: [{ name: "PATH", value: "/usr/sbin:/usr/bin" }],
};

function socket(overrides: Partial<SocketRow> = {}): SocketRow {
  return {
    protocol: "tcp",
    state: "LISTEN",
    address: "0.0.0.0",
    port: 22,
    pid: 812,
    process: "sshd",
    peerAddress: "",
    peerPort: 0,
    sentBytes: null,
    receivedBytes: null,
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

describe("a process row's menu", () => {
  it("opens the detail the same way clicking the row does, and copies every field", () => {
    const open = vi.fn();
    const entries = processRowMenu(process(), { open });

    expect(entries.filter((entry) => entry.kind === "action").map((entry) => entry.id)).toEqual([
      "open-detail",
      "copy-pid",
      "copy-name",
      "copy-command",
      "copy-executable",
    ]);

    pick(entries, "open-detail").run();
    expect(open).toHaveBeenCalledWith(1, "nginx");

    pick(entries, "copy-command").run();
    pick(entries, "copy-executable").run();
    expect(copyText).toHaveBeenCalledWith("nginx: worker process");
    expect(copyText).toHaveBeenCalledWith("/usr/sbin/nginx");
  });

  it("offers no path for a process the host would not name one for", () => {
    const kernelThread = process({ executable: null, command: "" });

    expect(pick(processRowMenu(kernelThread, { open: vi.fn() }), "copy-executable").disabled).toBe(true);
    expect(pick(processRowMenu(kernelThread, { open: vi.fn() }), "copy-command").disabled).toBe(true);
  });
});

describe("an open process's menu", () => {
  it("copies each field the panel is showing, and reads it again", () => {
    const refresh = vi.fn();
    const entries = processDetailMenu(detail, { refresh });

    pick(entries, "copy-command").run();
    pick(entries, "copy-executable").run();
    pick(entries, "copy-working-directory").run();
    expect(copyText).toHaveBeenCalledWith("nginx: worker process");
    expect(copyText).toHaveBeenCalledWith("/usr/sbin/nginx");
    expect(copyText).toHaveBeenCalledWith("/");

    pick(entries, "refresh").run();
    expect(refresh).toHaveBeenCalledOnce();
  });

  it("offers no working directory for a process that reports none", () => {
    const entries = processDetailMenu({ ...detail, workingDirectory: "" }, { refresh: vi.fn() });

    expect(pick(entries, "copy-working-directory").disabled).toBe(true);
  });
});

describe("a variable's menu", () => {
  it("copies the value, and the pair with its name", () => {
    const entries = environmentMenu({ name: "PATH", value: "/usr/sbin:/usr/bin" });

    pick(entries, "copy-value").run();
    pick(entries, "copy-pair").run();

    expect(copyText).toHaveBeenCalledWith("/usr/sbin:/usr/bin");
    expect(copyText).toHaveBeenCalledWith("PATH=/usr/sbin:/usr/bin");
  });
});

describe("a listening socket's menu", () => {
  it("shows its connections and copies the four things it knows", () => {
    const showConnections = vi.fn();
    const row = socket();
    const entries = listenerMenu(row, { showConnections });

    pick(entries, "show-connections").run();
    expect(showConnections).toHaveBeenCalledWith(row);

    pick(entries, "copy-address").run();
    pick(entries, "copy-port").run();
    pick(entries, "copy-endpoint").run();
    pick(entries, "copy-process").run();
    pick(entries, "copy-pid").run();

    expect(copyText).toHaveBeenCalledWith("0.0.0.0");
    expect(copyText).toHaveBeenCalledWith("22");
    expect(copyText).toHaveBeenCalledWith("0.0.0.0:22");
    expect(copyText).toHaveBeenCalledWith("sshd");
    expect(copyText).toHaveBeenCalledWith("812");
  });

  it("offers no process for a socket the host did not attribute", () => {
    const unnamed = socket({ pid: 0, process: "" });
    const entries = listenerMenu(unnamed, { showConnections: vi.fn() });

    expect(pick(entries, "copy-process").disabled).toBe(true);
    expect(pick(entries, "copy-pid").disabled).toBe(true);
    // The address is still the address, whoever is holding the socket.
    expect(pick(entries, "copy-endpoint").disabled).toBeFalsy();
  });
});

describe("a connection's menu", () => {
  it("copies the other end and what the socket is doing", () => {
    const connection = socket({ state: "ESTABLISHED", peerAddress: "203.0.113.9", peerPort: 51515 });
    const entries = connectionMenu(connection);

    pick(entries, "copy-peer-address").run();
    pick(entries, "copy-peer-port").run();
    pick(entries, "copy-state").run();

    expect(copyText).toHaveBeenCalledWith("203.0.113.9");
    expect(copyText).toHaveBeenCalledWith("51515");
    expect(copyText).toHaveBeenCalledWith("ESTABLISHED");
  });
});
