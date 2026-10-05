import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  monitorSample: vi.fn(),
  monitorProcesses: vi.fn(),
  monitorSockets: vi.fn(),
  monitorProcessDetail: vi.fn(),
  sessionOpen: vi.fn(),
  terminalClose: vi.fn(),
  terminalWrite: vi.fn(),
  terminalResize: vi.fn(),
  knownHostTrust: vi.fn(),
  knownHostForget: vi.fn(),
  tunnelsOpen: vi.fn(),
  tunnelClose: vi.fn(),
  filesOpen: vi.fn(),
  filesList: vi.fn(),
  filesClose: vi.fn(),
  transfersOpen: vi.fn(),
}));
// The bus is mocked rather than run: these tests are about what the monitor does with an answer,
// and the bus already has its own suite for delivering events.
vi.mock("@/lib/terminal/bus", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/terminal/bus")>();
  return {
    ...actual,
    startTerminalBus: vi.fn(async () => {}),
    onTerminalEnded: vi.fn(() => () => {}),
    endedTerminal: vi.fn(() => undefined),
  };
});

import { monitorProcessDetail, monitorProcesses, monitorSample, monitorSockets } from "@/lib/backend/tauri";
import {
  MONITOR_HISTORY,
  MONITOR_MAX_GAP_MS,
  MONITOR_POLL_MS,
  PROCESS_PAGE_LIMIT,
  PROCESS_POLL_MS,
  SOCKET_POLL_MS,
  useMonitorStore,
} from "@/stores/monitor";
import { type TerminalTab, useTerminalsStore } from "@/stores/terminals";
import type { HostMetrics, ProcessPage, ProcessSample, SocketRow } from "@/types/monitor";
import type { Session } from "@/types/session";

const monitorSampleMock = vi.mocked(monitorSample);
const monitorProcessesMock = vi.mocked(monitorProcesses);
const monitorSocketsMock = vi.mocked(monitorSockets);
const monitorProcessDetailMock = vi.mocked(monitorProcessDetail);

const session: Session = {
  id: "session-1",
  name: "web",
  protocol: "ssh",
  host: "example.test",
  port: 22,
  user: "root",
  auth: { method: "password", hasSecret: true },
  terminal: { terminalType: "xterm-256color", keepaliveSeconds: 30, forwardAgent: false },
  updatedAt: 1_700_000_000,
};

/** A sample, with the parts a test is not about filled in plausibly. */
function metrics(overrides: Partial<HostMetrics> = {}): HostMetrics {
  return {
    sampledAt: 1_700_000_000_000,
    roundTripMs: 12,
    hostname: "web-1",
    os: "Ubuntu 24.04 LTS",
    kernel: "6.8.0-31-generic",
    arch: "x86_64",
    cores: 4,
    cpuModel: { name: "Intel(R) Core(TM) i9-14900HX", frequencyMhz: 2419.198, cacheKb: 36_864, bogomips: 4838.39 },
    uptimeSeconds: 90_061,
    load: { one: 0.2, five: 0.1, fifteen: 0.05 },
    cpu: {
      totalTicks: 1000,
      idleTicks: 800,
      states: {
        userTicks: 150,
        niceTicks: 0,
        systemTicks: 50,
        idleTicks: 800,
        ioWaitTicks: 0,
        interruptTicks: 0,
        softInterruptTicks: 0,
        stealTicks: 0,
      },
    },
    memory: {
      totalBytes: 8_000_000_000,
      availableBytes: 4_000_000_000,
      swapTotalBytes: 2_000_000_000,
      swapFreeBytes: 2_000_000_000,
    },
    network: [
      { name: "lo", receivedBytes: 10_000, transmittedBytes: 10_000 },
      { name: "eth0", receivedBytes: 1_000_000, transmittedBytes: 500_000 },
    ],
    disks: [{ mountPoint: "/", filesystem: "overlay", sizeBytes: 100_000_000_000, availableBytes: 40_000_000_000 }],
    processes: [
      {
        pid: 1,
        user: "root",
        name: "systemd",
        command: "/sbin/init splash",
        executable: null,
        cpuPercent: 0.7,
        memoryPercent: 1.2,
        residentBytes: 12_000_000,
        cpuTicks: null,
      },
    ],
    ...overrides,
  };
}

/**
 * CPU counters, with the breakdown filled in from the summary where a test is not about the
 * breakdown: on a real host both come off one line of the kernel's own CPU counters, and a fixture
 * that disagreed with itself would be describing a host that cannot exist.
 */
function cpuCounters(totalTicks: number, idleTicks: number): HostMetrics["cpu"] {
  return {
    totalTicks,
    idleTicks,
    states: {
      userTicks: Math.max(0, totalTicks - idleTicks),
      niceTicks: 0,
      systemTicks: 0,
      idleTicks,
      ioWaitTicks: 0,
      interruptTicks: 0,
      softInterruptTicks: 0,
      stealTicks: 0,
    },
  };
}

/** The terminal tabs the workspace holds, for tests that end one or look at one. */
function terminalTabs(): TerminalTab[] {
  return useTerminalsStore().tabs.filter((entry): entry is TerminalTab => entry.kind === "terminal");
}

/** Puts an open terminal in the workspace, which is what gives the monitor something to follow. */
function workspaceWith(sessions: Session[], handle: (session: Session) => string): void {
  const terminals = useTerminalsStore();
  for (const [index, entry] of sessions.entries()) {
    terminals.tabs.push({
      kind: "terminal",
      id: `tab-${index + 1}`,
      session: entry,
      handle: handle(entry),
      status: "open",
      failure: null,
      panel: null,
    });
  }
  const active = terminals.tabs[terminals.tabs.length - 1];
  if (active) {
    terminals.activate(active.id);
  }
}

/** Lets the pending promise chain and the watchers run. */
async function settle(): Promise<void> {
  await vi.advanceTimersByTimeAsync(0);
}

beforeEach(() => {
  setActivePinia(createPinia());
  vi.useFakeTimers();
  monitorSampleMock.mockReset();
  monitorProcessesMock.mockReset();
  monitorSocketsMock.mockReset();
  monitorProcessDetailMock.mockReset();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("following the connection on screen", () => {
  it("samples the session whose terminal is on screen, at once and then on the clock", async () => {
    workspaceWith([session], () => "t1");
    monitorSampleMock.mockResolvedValue(metrics());

    const monitor = useMonitorStore();
    expect(monitor.target?.handle).toBe("t1");

    monitor.start();
    await settle();
    expect(monitorSampleMock).toHaveBeenCalledWith("t1");
    expect(monitor.sample?.hostname).toBe("web-1");

    await vi.advanceTimersByTimeAsync(MONITOR_POLL_MS);
    expect(monitorSampleMock).toHaveBeenCalledTimes(2);

    monitor.stop();
    await vi.advanceTimersByTimeAsync(MONITOR_POLL_MS * 3);
    expect(monitorSampleMock).toHaveBeenCalledTimes(2);
  });

  it("asks for nothing when no terminal is open", async () => {
    const monitor = useMonitorStore();

    expect(monitor.target).toBeNull();
    monitor.start();
    await settle();

    expect(monitorSampleMock).not.toHaveBeenCalled();
    expect(monitor.sample).toBeNull();
  });

  it("has nothing to follow once the terminal on screen has ended", async () => {
    workspaceWith([session], () => "t1");
    monitorSampleMock.mockResolvedValue(metrics());

    const monitor = useMonitorStore();
    monitor.start();
    await settle();
    expect(monitor.sample).not.toBeNull();

    terminalTabs()[0]!.status = "ended";
    await settle();

    expect(monitor.target).toBeNull();
    expect(monitor.sample).toBeNull();
  });

  it("drops the readings of a session that is no longer the one on screen", async () => {
    const other: Session = { ...session, id: "session-2", name: "db", host: "db.example.test" };
    workspaceWith([session, other], (entry) => `t-${entry.id}`);
    monitorSampleMock.mockResolvedValue(metrics());

    const monitor = useMonitorStore();
    monitor.start();
    await settle();
    expect(monitor.sample).not.toBeNull();
    expect(monitor.target?.sessionId).toBe("session-2");

    // The first session's terminal is gone: what is on screen has no connection to sample.
    terminalTabs()[0]!.status = "ended";
    useTerminalsStore().activate("tab-1");
    await settle();

    expect(monitor.target).toBeNull();
    expect(monitor.sample).toBeNull();
  });
});

describe("what a sample becomes", () => {
  it("has no rate until there are two samples to compare", async () => {
    workspaceWith([session], () => "t1");
    monitorSampleMock.mockResolvedValueOnce(metrics());

    const monitor = useMonitorStore();
    monitor.start();
    await settle();

    expect(monitor.sample).not.toBeNull();
    expect(monitor.cpuPercent).toBeNull();
    expect(monitor.traffic).toEqual([]);
    // Loopback is in the list and is not what the panel picks on its own.
    expect(monitor.interfaces).toEqual(["lo", "eth0"]);
    expect(monitor.interfaceName).toBe("eth0");
  });

  it("measures CPU and traffic between two samples of one interval", async () => {
    workspaceWith([session], () => "t1");
    monitorSampleMock.mockResolvedValueOnce(metrics());
    monitorSampleMock.mockResolvedValueOnce(
      metrics({
        sampledAt: 1_700_000_000_000 + MONITOR_POLL_MS,
        cpu: cpuCounters(1200, 900),
        network: [{ name: "eth0", receivedBytes: 3_000_000, transmittedBytes: 1_500_000 }],
        roundTripMs: 20,
      }),
    );

    const monitor = useMonitorStore();
    monitor.start();
    await vi.advanceTimersByTimeAsync(MONITOR_POLL_MS);

    expect(monitor.cpuPercent).toBe(50);
    expect(monitor.traffic).toHaveLength(1);
    // Two megabytes in two seconds, and half a megabyte the other way.
    expect(monitor.traffic[0]).toEqual({ received: 1_000_000, transmitted: 500_000 });
    expect(monitor.roundTrips).toEqual([12, 20]);
  });

  it("restarts the rate rather than averaging over a gap it did not see", async () => {
    workspaceWith([session], () => "t1");
    monitorSampleMock.mockResolvedValueOnce(metrics());
    monitorSampleMock.mockResolvedValueOnce(
      metrics({
        // The samples are a minute apart — a minimized window, a slept machine — and the chart
        // draws one point per sample, so a minute of averaging would be a shape nobody can read.
        sampledAt: 1_700_000_000_000 + MONITOR_MAX_GAP_MS * 10,
        cpu: cpuCounters(1200, 900),
      }),
    );

    const monitor = useMonitorStore();
    monitor.start();
    await vi.advanceTimersByTimeAsync(MONITOR_POLL_MS);

    expect(monitor.cpuPercent).toBeNull();
    expect(monitor.traffic).toEqual([]);
  });

  it("keeps the last reading on screen when a sample fails, and says why", async () => {
    workspaceWith([session], () => "t1");
    monitorSampleMock.mockResolvedValueOnce(metrics());
    monitorSampleMock.mockRejectedValueOnce({
      version: 1,
      code: "APP-INTERNAL",
      messageKey: "errors.app.internal",
      messageParams: {},
      origin: { subsystem: "app" },
      outcome: "unknown",
    });

    const monitor = useMonitorStore();
    monitor.start();
    await vi.advanceTimersByTimeAsync(MONITOR_POLL_MS);

    expect(monitor.failure?.code).toBe("APP-INTERNAL");
    expect(monitor.sample?.hostname).toBe("web-1");
  });

  it("keeps only the newest samples the charts have room for", async () => {
    workspaceWith([session], () => "t1");
    const monitor = useMonitorStore();
    let at = 1_700_000_000_000;
    monitorSampleMock.mockImplementation(async () => {
      at += MONITOR_POLL_MS;
      return metrics({ sampledAt: at });
    });

    monitor.start();
    await vi.advanceTimersByTimeAsync(MONITOR_POLL_MS * (MONITOR_HISTORY + 5));

    expect(monitor.roundTrips.length).toBe(MONITOR_HISTORY);
    expect(monitor.traffic.length).toBeLessThanOrEqual(MONITOR_HISTORY);
  });

  /**
   * A page asks this store for its numbers, and the store only ever holds one sample — so which
   * session that sample came from is the thing that keeps two hosts' pages from borrowing each
   * other's readings in the moment between switching tabs and the next poll.
   */
  it("says which session the sample on screen came from, and forgets it with the connection", async () => {
    workspaceWith([session], () => "t1");
    monitorSampleMock.mockResolvedValue(metrics());

    const monitor = useMonitorStore();
    monitor.start();
    await settle();
    expect(monitor.sampleSessionId).toBe(session.id);

    terminalTabs()[0]!.status = "ended";
    await settle();

    expect(monitor.sample).toBeNull();
    expect(monitor.sampleSessionId).toBeNull();
  });

  /**
   * The panel and a page can both be showing the same host. Two polls would be two commands on
   * somebody's server every two seconds to answer one question, so the second caller joins the
   * first, and the poll ends when the last of them has gone.
   */
  it("runs one poll however many things are asking for it", async () => {
    workspaceWith([session], () => "t1");
    monitorSampleMock.mockResolvedValue(metrics());

    const monitor = useMonitorStore();
    monitor.start();
    await settle();
    expect(monitorSampleMock).toHaveBeenCalledTimes(1);

    monitor.start();
    await vi.advanceTimersByTimeAsync(MONITOR_POLL_MS);
    expect(monitorSampleMock).toHaveBeenCalledTimes(2);

    monitor.stop();
    await vi.advanceTimersByTimeAsync(MONITOR_POLL_MS);
    expect(monitorSampleMock).toHaveBeenCalledTimes(3);

    monitor.stop();
    await vi.advanceTimersByTimeAsync(MONITOR_POLL_MS * 3);
    expect(monitorSampleMock).toHaveBeenCalledTimes(3);
  });
});

/** One listening socket, with the parts a test is not about filled in. */
function listening(overrides: Partial<SocketRow> = {}): SocketRow {
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

describe("the network page", () => {
  it("reads its own list while it is on screen, and stops when it is not", async () => {
    workspaceWith([session], () => "t1");
    monitorSocketsMock.mockResolvedValue([listening()]);

    const monitor = useMonitorStore();
    monitor.startSockets();
    await settle();

    expect(monitorSocketsMock).toHaveBeenCalledWith("t1");
    expect(monitor.sockets[0]?.port).toBe(22);

    monitor.stopSockets();
    await vi.advanceTimersByTimeAsync(SOCKET_POLL_MS * 3);
    expect(monitorSocketsMock).toHaveBeenCalledTimes(1);
  });

  it("keeps the list on screen when a read fails, and says why", async () => {
    workspaceWith([session], () => "t1");
    monitorSocketsMock.mockResolvedValueOnce([listening()]);
    monitorSocketsMock.mockRejectedValueOnce({ version: 1, code: "APP-INTERNAL", messageKey: "errors.app.internal" });

    const monitor = useMonitorStore();
    monitor.startSockets();
    await settle();
    await vi.advanceTimersByTimeAsync(SOCKET_POLL_MS);

    expect(monitor.socketFailure?.code).toBe("APP-INTERNAL");
    expect(monitor.sockets).toHaveLength(1);
  });

  it("has nothing to read without a terminal open", async () => {
    const monitor = useMonitorStore();
    monitor.startSockets();
    await settle();

    expect(monitorSocketsMock).not.toHaveBeenCalled();
    expect(monitor.sockets).toEqual([]);
  });
});

describe("the interface the figures are about", () => {
  it("picks the busiest one on its own, and keeps loopback out of that choice", async () => {
    workspaceWith([session], () => "t1");
    monitorSampleMock.mockResolvedValueOnce(
      metrics({
        network: [
          { name: "lo", receivedBytes: 900_000, transmittedBytes: 900_000 },
          { name: "eth0", receivedBytes: 1_000, transmittedBytes: 1_000 },
        ],
      }),
    );

    const monitor = useMonitorStore();
    monitor.start();
    await settle();

    expect(monitor.interfaces).toEqual(["lo", "eth0"]);
    // Loopback is in the list and is not what the panel picks on its own.
    expect(monitor.interfaceName).toBe("eth0");
  });

  it("draws the interface somebody picked, and its own chart", async () => {
    workspaceWith([session], () => "t1");
    monitorSampleMock.mockResolvedValueOnce(metrics());
    monitorSampleMock.mockResolvedValueOnce(
      metrics({
        sampledAt: 1_700_000_000_000 + MONITOR_POLL_MS,
        network: [
          { name: "lo", receivedBytes: 20_000, transmittedBytes: 20_000 },
          { name: "eth0", receivedBytes: 3_000_000, transmittedBytes: 1_500_000 },
        ],
      }),
    );

    const monitor = useMonitorStore();
    monitor.start();
    await vi.advanceTimersByTimeAsync(MONITOR_POLL_MS);
    expect(monitor.interfaceName).toBe("eth0");
    expect(monitor.currentRate?.received).toBe(1_000_000);

    monitor.selectInterface("lo");
    expect(monitor.interfaceName).toBe("lo");
    expect(monitor.currentRate?.received).toBe(5_000);
    expect(monitor.currentTotals?.receivedBytes).toBe(20_000);
    expect(monitor.traffic.at(-1)).toEqual({ received: 5_000, transmitted: 5_000 });
  });

  it("goes back to the busiest one when the chosen interface is gone", async () => {
    workspaceWith([session], () => "t1");
    monitorSampleMock.mockResolvedValue(metrics());
    monitorSampleMock.mockResolvedValueOnce(metrics());

    const monitor = useMonitorStore();
    monitor.start();
    await settle();
    monitor.selectInterface("tun0");

    // A pick that is not there is not an empty chart: the automatic answer takes over.
    expect(monitor.interfaceName).toBe("eth0");
  });
});

/** A page of processes, as the host answers it. */
function processesPage(rows: Partial<ProcessSample>[], ticksPerSecond = 100): ProcessPage {
  return { ticksPerSecond, processes: rows.map((row) => process(row)) };
}

/** One process, with the parts a test is not about filled in plausibly. */
function process(overrides: Partial<ProcessSample> = {}): ProcessSample {
  return {
    pid: 42,
    user: "root",
    name: "top",
    command: "top -b -d 1",
    executable: "/usr/bin/top",
    cpuPercent: 3.5,
    memoryPercent: 0.6,
    residentBytes: 6_656_000,
    cpuTicks: 1_000,
    ...overrides,
  };
}

describe("the process page", () => {
  it("reads its own list while it is on screen, and stops when it is not", async () => {
    workspaceWith([session], () => "t1");
    monitorProcessesMock.mockResolvedValue(processesPage([{}]));

    const monitor = useMonitorStore();
    monitor.startProcesses();
    await settle();

    expect(monitorProcessesMock).toHaveBeenCalledWith("t1", PROCESS_PAGE_LIMIT);
    expect(monitor.processes[0]?.sample.command).toBe("top -b -d 1");

    monitor.stopProcesses();
    await vi.advanceTimersByTimeAsync(PROCESS_POLL_MS * 3);
    expect(monitorProcessesMock).toHaveBeenCalledTimes(1);
  });

  /**
   * The CPU column is what each process used between two reads, not the average since it started:
   * the second is the same number every time it is asked, which is what a frozen page looks like.
   */
  it("works out what each process used between two reads", async () => {
    workspaceWith([session], () => "t1");
    monitorProcessesMock.mockResolvedValueOnce(processesPage([{ pid: 7, cpuTicks: 1_000 }]));
    // One interval later, three hundred ticks more: three seconds of processor time — a whole
    // processor, busy for the three seconds between the two reads.
    monitorProcessesMock.mockResolvedValueOnce(processesPage([{ pid: 7, cpuTicks: 1_300 }]));

    const monitor = useMonitorStore();
    monitor.startProcesses();
    await settle();
    expect(monitor.processes[0]?.cpuPercent, "one read is not an interval").toBeNull();

    await vi.advanceTimersByTimeAsync(PROCESS_POLL_MS);

    expect(monitor.processes[0]?.cpuPercent).toBeCloseTo(100, 5);
  });

  it("shows nothing for a process that was not in the previous read", async () => {
    workspaceWith([session], () => "t1");
    monitorProcessesMock.mockResolvedValueOnce(processesPage([{ pid: 7 }]));
    monitorProcessesMock.mockResolvedValueOnce(processesPage([{ pid: 7 }, { pid: 8, name: "fresh" }]));

    const monitor = useMonitorStore();
    monitor.startProcesses();
    await settle();
    vi.setSystemTime(new Date(Date.now() + 2_000));
    await vi.advanceTimersByTimeAsync(PROCESS_POLL_MS);

    const fresh = monitor.processes.find((row) => row.sample.pid === 8);
    expect(fresh?.cpuPercent, "a process nobody has seen twice has no interval yet").toBeNull();
  });

  /**
   * Ordering is a redraw, not another command: the host cut this list once and sent it, and asking
   * it again to sort the same rows would be a `ps` on somebody's server for nothing.
   */
  it("orders what it has without asking the host again", async () => {
    workspaceWith([session], () => "t1");
    monitorProcessesMock.mockResolvedValue(
      processesPage([
        { pid: 1, name: "alpha", residentBytes: 300 },
        { pid: 2, name: "beta", residentBytes: 900 },
      ]),
    );

    const monitor = useMonitorStore();
    monitor.startProcesses();
    await settle();
    const reads = monitorProcessesMock.mock.calls.length;

    monitor.sortProcesses("memory", true);
    expect(monitor.processes.map((row) => row.sample.pid)).toEqual([2, 1]);
    expect(monitorProcessesMock.mock.calls.length, "no command was run to sort").toBe(reads);

    monitor.sortProcesses("memory", false);
    expect(monitor.processes.map((row) => row.sample.pid)).toEqual([1, 2]);
  });

  it("keeps the list on screen when a read fails, and says why", async () => {
    workspaceWith([session], () => "t1");
    monitorProcessesMock.mockResolvedValueOnce(processesPage([{ pid: 1, name: "systemd" }]));
    monitorProcessesMock.mockRejectedValueOnce({ version: 1, code: "APP-INTERNAL", messageKey: "errors.app.internal" });

    const monitor = useMonitorStore();
    monitor.startProcesses();
    await settle();
    await vi.advanceTimersByTimeAsync(PROCESS_POLL_MS);

    expect(monitor.processFailure?.code).toBe("APP-INTERNAL");
    expect(monitor.processes[0]?.sample.name).toBe("systemd");
  });

  /**
   * The one read in this build that can return credentials. It happens because somebody opened a
   * process, and never because a clock came round: a detail is not a thing to gather on a poll.
   */
  it("reads one process in full when it is opened, and forgets it when the tab closes", async () => {
    workspaceWith([session], () => "t1");
    monitorProcessDetailMock.mockResolvedValue({
      pid: 42,
      name: "top",
      commandLine: "top -b -d 1",
      executable: "/usr/bin/top",
      workingDirectory: "/root",
      environment: [{ name: "PATH", value: "/usr/bin:/bin" }],
    });

    const monitor = useMonitorStore();
    await monitor.readProcessDetail(42);

    expect(monitorProcessDetailMock).toHaveBeenCalledWith("t1", 42);
    expect(monitor.details[42]?.workingDirectory).toBe("/root");
    expect(monitor.details[42]?.environment[0]?.name).toBe("PATH");

    monitor.forgetProcess(42);
    expect(monitor.details[42]).toBeUndefined();
  });

  it("has nothing to read without a terminal open", async () => {
    const monitor = useMonitorStore();
    monitor.startProcesses();
    await settle();

    expect(monitorProcessesMock).not.toHaveBeenCalled();
    expect(monitor.processes).toEqual([]);
  });
});
