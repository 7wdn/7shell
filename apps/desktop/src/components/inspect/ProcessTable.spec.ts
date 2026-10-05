/**
 * The process table, as it is drawn and as it keeps itself up to date.
 *
 * The question this exists to answer is the one a screenshot cannot: whether the table is reading
 * the host again, or whether it drew once and stayed. A page whose numbers move slowly looks the
 * same as a page that is frozen unless something asserts that it asks.
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

import ProcessTable from "@/components/inspect/ProcessTable.vue";
import { useContextMenu } from "@/composables/useContextMenu";
import { i18n } from "@/i18n";
import { monitorProcesses } from "@/lib/backend/tauri";
import { PROCESS_POLL_MS, PROCESS_PAGE_LIMIT } from "@/stores/monitor";
import { useTerminalsStore } from "@/stores/terminals";
import type { ProcessPage, ProcessSample } from "@/types/monitor";
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

/** A page of processes, as the host answers it. */
function page(processes: ProcessSample[]): ProcessPage {
  return { ticksPerSecond: 100, processes };
}

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

let host: HTMLDivElement;
let app: App | undefined;

/** The page, reading through a terminal that is open, exactly as the application does. */
async function render(): Promise<void> {
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

  app = createApp({ render: () => h(ProcessTable, { active: true }) }).use(i18n);
  app.mount(host);
  await vi.waitFor(() => expect(vi.mocked(monitorProcesses).mock.calls.length).toBeGreaterThan(0));
  await nextTick();
}

/** The names the table is showing, in the order it is showing them. */
function names(): string[] {
  return [...host.querySelectorAll("tbody tr")].map((row) => row.querySelectorAll("td")[4]?.textContent?.trim() ?? "");
}

beforeEach(() => {
  setActivePinia(createPinia());
  vi.useFakeTimers();
  vi.mocked(monitorProcesses).mockReset().mockResolvedValue(page([]));
  host = document.createElement("div");
  document.body.appendChild(host);
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
  vi.useRealTimers();
});

describe("the process table", () => {
  it("asks the host again on its own clock, and draws what comes back", async () => {
    vi.mocked(monitorProcesses).mockResolvedValue(page([process({ name: "systemd" })]));
    await render();

    expect(vi.mocked(monitorProcesses)).toHaveBeenCalledWith("t1", PROCESS_PAGE_LIMIT);
    expect(names()).toEqual(["systemd"]);

    // The host answers something else a moment later: what is on screen has to be the answer, not
    // the first thing it ever said.
    vi.mocked(monitorProcesses).mockResolvedValue(page([process({ pid: 2, name: "sshd" })]));
    await vi.advanceTimersByTimeAsync(PROCESS_POLL_MS);

    expect(vi.mocked(monitorProcesses).mock.calls.length).toBeGreaterThanOrEqual(2);
    expect(names()).toEqual(["sshd"]);
  });

  it("keeps the stream of commands to one at a time", async () => {
    await render();
    const after = vi.mocked(monitorProcesses).mock.calls.length;

    await vi.advanceTimersByTimeAsync(PROCESS_POLL_MS * 3);

    // Three intervals, three reads — not five, and not none.
    expect(vi.mocked(monitorProcesses).mock.calls.length).toBe(after + 3);
  });

  /**
   * The row's right-click, which is what turns a table nobody can copy out of into one somebody can.
   *
   * What is asserted is the hand-over: the row builds its own list and the shell's layer is what
   * would draw it. Whether the entries are the right ones is the builder's own suite.
   */
  it("offers a row its own menu when it is right-clicked", async () => {
    vi.mocked(monitorProcesses).mockResolvedValue(page([process({ name: "systemd" })]));
    await render();

    const row = host.querySelectorAll("tbody tr")[0];
    const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 10, clientY: 10 });
    row?.dispatchEvent(event);
    await nextTick();

    const { visible, entries } = useContextMenu();
    expect(visible.value).toBe(true);
    expect(event.defaultPrevented).toBe(true);
    expect(entries.value.map((entry) => entry.id)).toEqual([
      "open-detail",
      "sep-copy",
      "copy-pid",
      "copy-name",
      "copy-command",
      "copy-executable",
    ]);
  });
});
