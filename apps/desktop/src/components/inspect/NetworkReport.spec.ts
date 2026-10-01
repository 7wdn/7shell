/**
 * The socket table, as it is drawn.
 *
 * The helpers underneath are pinned by their own tests, and the store by its own; what is only
 * visible here is the shape a person meets — which sockets are listed, what the row says, and that
 * picking one brings the connections of *that* port onto the screen underneath it.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  monitorSample: vi.fn(),
  monitorProcesses: vi.fn(),
  monitorSockets: vi.fn(async () => []),
  sessionOpen: vi.fn(),
  terminalClose: vi.fn(),
  terminalWrite: vi.fn(),
  terminalResize: vi.fn(),
  knownHostTrust: vi.fn(),
  knownHostForget: vi.fn(),
  tunnelsOpen: vi.fn(async () => []),
  tunnelClose: vi.fn(),
  filesOpen: vi.fn(),
  filesList: vi.fn(),
  filesClose: vi.fn(),
  transfersOpen: vi.fn(),
}));
vi.mock("@/lib/terminal/bus", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/terminal/bus")>();
  return {
    ...actual,
    startTerminalBus: vi.fn(async () => {}),
    onTerminalEnded: vi.fn(() => () => {}),
    endedTerminal: vi.fn(() => undefined),
  };
});

import NetworkReport from "@/components/inspect/NetworkReport.vue";
import { i18n } from "@/i18n";
import { monitorSockets } from "@/lib/backend/tauri";
import { useMonitorStore } from "@/stores/monitor";
import { useTerminalsStore } from "@/stores/terminals";
import type { SocketRow } from "@/types/monitor";
import type { Session } from "@/types/session";

/** One socket, with the parts a test is not about filled in. */
function socket(overrides: Partial<SocketRow> = {}): SocketRow {
  return {
    protocol: "tcp",
    state: "LISTEN",
    address: "0.0.0.0",
    port: 22,
    pid: 1741,
    process: "sshd",
    peerAddress: "",
    peerPort: 0,
    sentBytes: null,
    receivedBytes: null,
    ...overrides,
  };
}

const session: Session = {
  id: "session-1",
  name: "web",
  protocol: "ssh",
  host: "example.test",
  port: 22,
  user: "root",
  auth: { method: "password", hasSecret: true },
  terminal: { terminalType: "xterm-256color", keepaliveSeconds: 30 },
  updatedAt: 1_700_000_000,
};

/**
 * An open terminal for the page to read through.
 *
 * A page holds no connection of its own — that is the design — so a spec that wants one to read
 * anything has to give it a session whose terminal is open, exactly as the application does.
 */
function openTerminal(): void {
  const terminals = useTerminalsStore();
  terminals.tabs.push({
    kind: "terminal",
    id: "tab-1",
    session,
    handle: "t1",
    status: "open",
    failure: null,
    panel: null,
  });
  terminals.activate("tab-1");
}

/** A connection that landed on a listener of the same family. */
function connection(overrides: Partial<SocketRow> = {}): SocketRow {
  return socket({
    state: "ESTAB",
    address: "192.168.254.133",
    peerAddress: "192.168.254.1",
    peerPort: 52344,
    ...overrides,
  });
}

let host: HTMLDivElement;
let app: App | undefined;

/**
 * The page, reading the sockets it is given.
 *
 * The read goes through the mock rather than into the store directly, because the reading is the
 * page's own: it asks while it is the tab on screen, and a test that filled the store instead would
 * be testing a page that never asks.
 */
async function render(rows: readonly SocketRow[]): Promise<void> {
  vi.mocked(monitorSockets).mockResolvedValue([...rows]);
  openTerminal();

  app = createApp({ render: () => h(NetworkReport, { live: true, active: true }) }).use(i18n);
  app.mount(host);
  await vi.waitFor(() => {
    expect(useMonitorStore().sockets).toHaveLength(rows.length);
  });
  await nextTick();
}

/** The ports the table is showing, in the order it is showing them. */
function ports(): string[] {
  return [...host.querySelectorAll("tbody tr")].map((row) => row.querySelectorAll("td")[4]?.textContent?.trim() ?? "");
}

/** The connection count of each row, in the same order. */
function counts(): string[] {
  return [...host.querySelectorAll("tbody tr")].map((row) => row.querySelectorAll("td")[6]?.textContent?.trim() ?? "");
}

/** A heading button, by the label it starts with. */
function heading(label: string): HTMLElement {
  const found = [...host.querySelectorAll("th button")].find((button) =>
    (button.textContent ?? "").trim().startsWith(label),
  );
  if (!found) {
    throw new Error(`no heading says ${label}: ${host.textContent}`);
  }
  return found as HTMLElement;
}

/** The cells of one row, by the row's own text. */
function row(text: string): string[] {
  const found = [...host.querySelectorAll("tbody tr")].find((candidate) => candidate.textContent?.includes(text));
  if (!found) {
    throw new Error(`no row says ${text}: ${host.textContent}`);
  }
  return [...found.querySelectorAll("td")].map((cell) => (cell.textContent ?? "").trim());
}

beforeEach(() => {
  setActivePinia(createPinia());
  host = document.createElement("div");
  document.body.appendChild(host);
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
});

describe("the socket table", () => {
  it("lists what is waiting on a port, with who holds it and what it is carrying", async () => {
    await render([
      socket({ port: 22, pid: 1741, process: "sshd" }),
      socket({
        protocol: "udp",
        state: "UNCONN",
        address: "127.0.0.53",
        port: 53,
        pid: 848,
        process: "systemd-resolve",
      }),
      socket({ address: "192.168.254.133", state: "ESTAB", peerAddress: "192.168.254.1", peerPort: 52344 }),
    ]);

    // Two listeners, and the established socket is not a third row: it is counted under the port
    // it is connected to, which is the whole point of the column.
    expect(host.querySelectorAll("tbody tr")).toHaveLength(2);
    expect(row("sshd")).toEqual(["1741", "sshd", "tcp", "0.0.0.0", "22", "1", "1", "-", "-"]);
    expect(row("systemd-resolve")[3], "a waiting udp socket is listed like any other").toBe("127.0.0.53");
  });

  it("counts the connections of a port, and how many addresses are behind them", async () => {
    await render([
      socket({ port: 22 }),
      socket({
        state: "ESTAB",
        address: "192.168.254.133",
        peerAddress: "10.0.0.1",
        peerPort: 1000,
        sentBytes: 1400,
        receivedBytes: 560,
      }),
      socket({
        state: "ESTAB",
        address: "192.168.254.133",
        peerAddress: "10.0.0.1",
        peerPort: 1001,
        sentBytes: 600,
        receivedBytes: 40,
      }),
      socket({ state: "ESTAB", address: "192.168.254.133", peerAddress: "10.0.0.2", peerPort: 1002 }),
    ]);

    // Three connections from two addresses, and the bytes that two of them reported.
    expect(row("sshd").slice(5)).toEqual(["2", "3", "2.0 KB", "600 B"]);
  });

  it("puts the connections of a picked port under it, and takes them away when it is picked again", async () => {
    await render([
      socket({ port: 22 }),
      socket({
        address: "192.168.254.133",
        state: "ESTAB",
        peerAddress: "192.168.254.1",
        peerPort: 52344,
        sentBytes: 1400,
        receivedBytes: 560,
      }),
    ]);

    expect(host.textContent).not.toContain("192.168.254.1");

    const listener = [...host.querySelectorAll<HTMLTableRowElement>("tbody tr")][0]!;
    listener.click();
    await nextTick();

    expect(host.textContent).toContain("0.0.0.0:22");
    const connection = row("192.168.254.1");
    expect(connection).toEqual(["192.168.254.1", "52344", "ESTAB", "1.4 KB", "560 B"]);

    listener.click();
    await nextTick();
    expect(host.textContent).not.toContain("192.168.254.1");
  });

  /**
   * The heading is the control, and it is the same control on both pages: pressing a column orders
   * by it, pressing the one in use turns the order around.
   */
  it("orders by a column when its heading is pressed, and turns the order around when it is pressed again", async () => {
    await render([
      socket({ port: 631, address: "127.0.0.1", process: "cupsd" }),
      socket({ port: 22, address: "0.0.0.0", process: "sshd" }),
      socket({ port: 53, address: "127.0.0.53", process: "systemd-resolve" }),
    ]);

    expect(ports()).toEqual(["22", "53", "631"]);

    heading("Port").click();
    await nextTick();
    expect(ports(), "pressing the column in use turns it around").toEqual(["631", "53", "22"]);
  });

  /** A column of figures reads from the top and an address from the alphabet — on either page. */
  it("starts each column at the end it is read from", async () => {
    await render([
      socket({ port: 631, address: "127.0.0.1", process: "cupsd" }),
      socket({ port: 22, address: "0.0.0.0", process: "sshd" }),
      socket({ port: 53, address: "127.0.0.53", process: "systemd-resolve" }),
      connection({ port: 631, peerAddress: "10.0.0.1", peerPort: 1000 }),
      connection({ port: 631, peerAddress: "10.0.0.2", peerPort: 1001 }),
      connection({ port: 22, peerAddress: "10.0.0.3", peerPort: 1002 }),
    ]);

    // The table opens in port order, so the counts are 22's, 53's and 631's — one, none, two.
    expect(counts(), "the default order is by port").toEqual(["1", "0", "2"]);

    heading("Connections").click();
    await nextTick();
    expect(ports(), "a count reads from the top").toEqual(["631", "22", "53"]);

    heading("Connections").click();
    await nextTick();
    expect(ports(), "and turns around").toEqual(["53", "22", "631"]);

    heading("Listen IP").click();
    await nextTick();
    expect(ports(), "an address reads from the alphabet").toEqual(["22", "631", "53"]);
  });

  it("says so when a host reports no listening ports", async () => {
    await render([]);

    expect(host.textContent).toContain("no listening ports");
  });
});
