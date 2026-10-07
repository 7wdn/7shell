/**
 * The monitor panel as it is met with a pointer.
 *
 * What the readings are is the store's business and is tested beside it; what is only visible here
 * is the panel's manners — that a drag across it selects nothing while the code somebody has to
 * quote still can be selected, and that its two small lists are drawn as tables rather than as lines
 * of text: a rule under the headings, and the row the pointer is on marked.
 *
 * The classes are what is checked, as the file listing's own spec does for the same kind of claim.
 * A test document lays nothing out and no pointer is there to hover, so neither the absence of a
 * selection nor a band that appears under the cursor is something an assertion could read back out
 * of the DOM — the class is the mechanism, and the case fails the day it is dropped.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  monitorSample: vi.fn(),
  monitorProcesses: vi.fn(async () => []),
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

import MonitorPanel from "@/components/monitor/MonitorPanel.vue";
import { i18n } from "@/i18n";
import { monitorSample } from "@/lib/backend/tauri";
import { useMonitorStore } from "@/stores/monitor";
import { useTerminalsStore } from "@/stores/terminals";
import type { DiskUsage, HostMetrics, ProcessSample } from "@/types/monitor";
import type { Session } from "@/types/session";

const session: Session = {
  id: "session-1",
  name: "web",
  protocol: "ssh",
  host: "example.test",
  port: 22,
  user: "root",
  auth: { method: "password", hasSecret: true },
  terminal: { terminalType: "xterm-256color", keepaliveSeconds: 30, forwardAgent: false, record: false },
  updatedAt: 1_700_000_000,
};

/** One process, with the parts a test is not about filled in plausibly. */
function process(overrides: Partial<ProcessSample> = {}): ProcessSample {
  return {
    pid: 1,
    user: "root",
    name: "systemd",
    command: "/sbin/init",
    executable: "/usr/lib/systemd/systemd",
    cpuPercent: 0.4,
    memoryPercent: 1.2,
    residentBytes: 12_000_000,
    cpuTicks: 100,
    ...overrides,
  };
}

/** One file system, as `df` reports it. */
function disk(overrides: Partial<DiskUsage> = {}): DiskUsage {
  return {
    mountPoint: "/",
    filesystem: "/dev/sda1",
    sizeBytes: 40_000_000_000,
    availableBytes: 12_000_000_000,
    ...overrides,
  };
}

/**
 * One sample, with the parts a test is not about filled in plausibly.
 *
 * The counters a rate is measured from are left null: this spec is about what the panel is made of,
 * and a rate would need two samples a known distance apart to exist at all.
 */
function metrics(overrides: Partial<HostMetrics> = {}): HostMetrics {
  return {
    sampledAt: 1_700_000_000_000,
    roundTripMs: 12,
    hostname: "lab",
    os: "Ubuntu 24.04",
    kernel: "6.8.0-45-generic",
    arch: "x86_64",
    cores: 4,
    cpuModel: null,
    uptimeSeconds: 3_600,
    load: { one: 0.1, five: 0.2, fifteen: 0.3 },
    cpu: null,
    memory: null,
    network: [{ name: "eth0", receivedBytes: 1_000, transmittedBytes: 500 }],
    disks: [],
    processes: [],
    ...overrides,
  };
}

let host: HTMLDivElement;
let app: App | undefined;

/**
 * An open terminal for the panel to follow.
 *
 * The panel holds no connection of its own — it follows whichever session is on screen, and samples
 * through that session's terminal — so a spec that wants it to read anything gives it one, exactly
 * as the application does.
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

/** The panel, drawing whatever the next sample turns out to be. */
function mount(): void {
  openTerminal();
  app = createApp({ render: () => h(MonitorPanel) }).use(i18n);
  app.mount(host);
}

/** The panel drawing one sample. */
async function render(sample: HostMetrics): Promise<void> {
  vi.mocked(monitorSample).mockResolvedValue(sample);
  mount();
  await vi.waitFor(() => expect(useMonitorStore().sample).not.toBeNull());
  await nextTick();
}

/** The panel drawing what it says when the sample did not arrive. */
async function renderFailure(): Promise<void> {
  vi.mocked(monitorSample).mockRejectedValue(new Error("the far side said no"));
  mount();
  await vi.waitFor(() => expect(useMonitorStore().failure).not.toBeNull());
  await nextTick();
}

/** One of the panel's small tables, by the name it carries for a reader who cannot see it. */
function table(label: string): HTMLTableElement {
  const found = host.querySelector(`table[aria-label="${label}"]`);
  if (!found) {
    throw new Error(`no table is named ${label}: ${host.textContent}`);
  }
  return found as HTMLTableElement;
}

/** The rows of a table that hold readings, which is every row but the headings'. */
function readings(sheet: HTMLTableElement): HTMLTableRowElement[] {
  return [...sheet.querySelectorAll<HTMLTableRowElement>("tbody tr")].filter(
    (row) => row.querySelectorAll("td").length > 1,
  );
}

/** Whether a row says, in its classes, that it answers to the pointer. */
function marksThePointer(row: Element): boolean {
  return [...row.classList].some((name) => name.startsWith("hover:bg-"));
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

describe("the monitor panel", () => {
  /**
   * The panel is an instrument, not a document.
   *
   * Left to itself the browser treats every reading and label as text, and both halves of that are
   * wrong here: a drag across the panel smears a selection over the rows, and the pointer becomes an
   * I-beam over the numbers. The host and the rows have their own ways of being copied, so nothing
   * in the panel is meant to be taken by hand — which is what these two classes say.
   */
  it("refuses to be read as a document, pointer included", async () => {
    await render(metrics());

    const panel = host.querySelector("aside");
    if (!panel) {
      throw new Error("the panel is not there");
    }
    expect(panel.classList.contains("select-none"), "a drag across the panel selects nothing").toBe(true);
    expect(panel.classList.contains("cursor-default"), "and the pointer stays an arrow over the readings").toBe(true);
  });

  /**
   * The one exception, and the reason it is one: a failure is the only thing on the panel that is
   * meant to leave it — it goes into a bug report or a chat — and the code is the part somebody has
   * to quote. Everything else has a copy button or a menu entry; this has nothing but itself.
   */
  it("keeps the failure's message and code selectable", async () => {
    await renderFailure();

    const block = host.querySelector("[data-monitor-failure]");
    if (!block) {
      throw new Error(`no failure is drawn: ${host.textContent}`);
    }
    expect(block.classList.contains("select-text")).toBe(true);
    expect(block.textContent, "and it says what went wrong, with the code to quote").toContain("APP-INTERNAL");
  });

  /**
   * Both lists are tables, and say so the way the tables on the inspect pages do: a rule under the
   * headings, and the row under the pointer marked. The marking is not decoration — a row here is a
   * right-click target rather than a click, so it is the only thing telling anyone the row is
   * anything at all.
   */
  it("marks the row the pointer is on, in both of its tables", async () => {
    await render(
      metrics({
        processes: [process({ pid: 1, name: "systemd" }), process({ pid: 712, name: "sshd" })],
        disks: [disk({ mountPoint: "/", filesystem: "/dev/sda1" }), disk({ mountPoint: "/boot" })],
      }),
    );

    for (const name of ["Processes", "File systems"]) {
      const sheet = table(name);
      const headings = sheet.querySelector("thead tr");
      expect(headings?.classList.contains("border-b"), `${name}: the headings are ruled off from the rows`).toBe(true);

      const rows = readings(sheet);
      expect(rows.length, `${name}: the sample's rows are drawn`).toBeGreaterThan(0);
      for (const row of rows) {
        expect(marksThePointer(row), `${name}: the row under the pointer is marked`).toBe(true);
      }
    }
  });

  /**
   * A table with a heading over nothing is what a collapsed list looks like: the empty line is a row
   * of the table, spanning its columns, rather than a paragraph that happened to land underneath.
   */
  it("says a section is empty inside its own table", async () => {
    await render(metrics());

    const processes = table("Processes");
    expect(readings(processes), "no reading is drawn").toHaveLength(0);
    const empty = processes.querySelector("tbody td");
    expect(empty?.getAttribute("colspan"), "and the line spans the whole table").toBe("3");
    expect(empty?.textContent?.trim()).toBe(i18n.global.t("monitor.processes.empty"));
  });
});
