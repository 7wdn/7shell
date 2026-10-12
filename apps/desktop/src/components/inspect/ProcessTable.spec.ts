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

/**
 * The columns, which are drawn at widths of their own rather than at whatever the content asks for.
 *
 * That is the change a fixed layout makes: under the automatic one a width is a suggestion the
 * widest cell overrules, and a suggestion is not something an edge can be dragged to. So what is
 * asserted here is the shape — the table is drawn fixed, every column but the command line is told
 * its width, and the line's own floor is what a drag of the last column stops against.
 *
 * A test document lays nothing out, so the room is told rather than measured: the table's width is
 * defined and the observers are then run, exactly as a real one runs when the window changes size.
 */
describe("the columns of the process table", () => {
  /** Whatever asked to be told when the table changes size, so that a case can tell it. */
  let watching: (() => void)[] = [];

  beforeEach(() => {
    watching = [];
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(notify: () => void) {
          watching.push(notify);
        }
        observe(): void {}
        disconnect(): void {}
      },
    );
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  function table(): HTMLTableElement {
    const found = host.querySelector("table");
    if (!found) {
      throw new Error("the page drew no table");
    }
    return found;
  }

  /** Says how wide the table is, and lets whatever is watching it hear about it. */
  async function room(width: number): Promise<void> {
    Object.defineProperty(table(), "clientWidth", { value: width });
    for (const notify of watching) {
      notify();
    }
    await nextTick();
  }

  function columnWidth(key: string): string {
    return table().style.getPropertyValue(`--col-${key}`);
  }

  /** Drags a column's edge this many pixels to the right, the way a pointer does. */
  async function dragEdge(key: string, delta: number): Promise<void> {
    const edge = table().querySelector<HTMLElement>(`[data-column-divider][data-column='${key}']`);
    if (!edge) {
      throw new Error(`no edge is drawn for the ${key} column`);
    }
    edge.setPointerCapture = vi.fn();
    edge.releasePointerCapture = vi.fn();
    edge.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, button: 0, clientX: 400 }));
    edge.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, clientX: 400 + delta }));
    edge.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
    await nextTick();
  }

  it("draws every column at a width of its own, the command line included", async () => {
    await render();

    expect(table().className, "a width is a width only under a fixed layout").toContain("table-fixed");
    const edges = [...table().querySelectorAll<HTMLElement>("[data-column-divider]")].map(
      (edge) => edge.dataset.column,
    );
    expect(edges).toEqual(["pid", "user", "memory", "cpu", "name", "command", "executable"]);
    // Every column is told its width, so every column is one an edge can move — the command line too,
    // which is the column this page is read for and the one somebody widens first.
    expect(columnWidth("command")).not.toBe("");
  });

  it("tells the table how narrow it may be drawn, so a narrow page scrolls instead of squeezing", async () => {
    await render();

    // The seven columns at 80 + 80 + 96 + 80 + 160 + 260 + 224. A table spends nothing on padding and
    // gaps of its own.
    expect(table().style.minWidth).toBe("980px");
  });

  it("divides the room a page has to spare between the columns nobody has moved", async () => {
    await render();
    await room(1200);

    // 1200 less the 980 the columns ask for: 220 left over, 31 each and the first three a pixel more.
    expect(columnWidth("pid")).toBe("112px");
    expect(columnWidth("command")).toBe("291px");
    expect(columnWidth("executable")).toBe("255px");
  });

  it("widens a column when its edge is dragged, and lets a drag go as wide as the pointer asks", async () => {
    await render();
    await room(1200);

    // The drag starts from the width the column is drawn at — 224 plus the 31 it shares — rather than
    // from the width this build states.
    await dragEdge("executable", 40);
    expect(columnWidth("executable")).toBe("295px");

    // And there is no furthest point: the columns simply take more room than the page has, which is
    // what the table's own scroller is for.
    await dragEdge("executable", 2000);
    expect(columnWidth("executable")).toBe("2295px");
  });
});
