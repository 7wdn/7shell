import { describe, expect, it } from "vitest";

import {
  busiestInterface,
  cpuBreakdown,
  cpuUsage,
  interfaceRates,
  isLoopback,
  memoryUsed,
  share,
  swapUsed,
} from "@/lib/monitor/rates";
import type { CpuCounters, CpuStates, MemoryUsage, NetworkCounters } from "@/types/monitor";

/** A memory reading, where "available" is what the kernel could still hand out. */
function memory(overrides: Partial<MemoryUsage> = {}): MemoryUsage {
  return { totalBytes: 1000, availableBytes: 400, swapTotalBytes: 500, swapFreeBytes: 500, ...overrides };
}

function interfaceCounters(name: string, received: number, transmitted: number): NetworkCounters {
  return { name, receivedBytes: received, transmittedBytes: transmitted };
}

/** A full set of CPU columns, with the ones a test does not name left at zero. */
function states(given: Partial<CpuStates> = {}): CpuStates {
  return {
    userTicks: 0,
    niceTicks: 0,
    systemTicks: 0,
    idleTicks: 0,
    ioWaitTicks: 0,
    interruptTicks: 0,
    softInterruptTicks: 0,
    stealTicks: 0,
    ...given,
  };
}

/**
 * CPU counters as a host reports them.
 *
 * The breakdown is given rather than derived from the two summary figures, because on a real host
 * all of them come off one line of `/proc/stat` and a fixture that computed one from the other
 * would be able to describe a host that cannot exist — while the tests below are about the
 * arithmetic on top of the reading, not about the reading.
 */
function counters(summary: { totalTicks: number; idleTicks: number }, breakdown: Partial<CpuStates> = {}): CpuCounters {
  return { totalTicks: summary.totalTicks, idleTicks: summary.idleTicks, states: states(breakdown) };
}

describe("cpuUsage", () => {
  it("is null with only one sample, because a counter is not a rate", () => {
    expect(cpuUsage(null, counters({ totalTicks: 1000, idleTicks: 800 }))).toBeNull();
    expect(cpuUsage(counters({ totalTicks: 1000, idleTicks: 800 }), null)).toBeNull();
  });

  it("measures the busy share of the interval between two samples", () => {
    // 200 ticks passed, 100 of them idle: half the machine was busy.
    const usage = cpuUsage(
      counters({ totalTicks: 1000, idleTicks: 800 }),
      counters({ totalTicks: 1200, idleTicks: 900 }),
    );

    expect(usage).toBe(50);
  });

  it("refuses a counter that went backwards rather than reporting a negative rate", () => {
    // A rebooted host, or a different host altogether.
    expect(
      cpuUsage(counters({ totalTicks: 5000, idleTicks: 4000 }), counters({ totalTicks: 10, idleTicks: 8 })),
    ).toBeNull();
    expect(
      cpuUsage(counters({ totalTicks: 1000, idleTicks: 800 }), counters({ totalTicks: 1000, idleTicks: 800 })),
    ).toBeNull();
  });

  it("never reports more than the whole machine", () => {
    // Idle ticks cannot exceed total ticks, but a reading that says they did must not become 150%.
    expect(cpuUsage(counters({ totalTicks: 100, idleTicks: 10 }), counters({ totalTicks: 110, idleTicks: 100 }))).toBe(
      0,
    );
  });
});

describe("cpuBreakdown", () => {
  it("is null while there is nothing to subtract, like every other rate", () => {
    expect(cpuBreakdown(null, counters({ totalTicks: 1000, idleTicks: 800 }))).toBeNull();
    expect(cpuBreakdown(counters({ totalTicks: 1000, idleTicks: 800 }), null)).toBeNull();
  });

  it("lists every column, in the order the page shows them", () => {
    const shares = cpuBreakdown(
      counters({ totalTicks: 1000, idleTicks: 800 }, { userTicks: 200, idleTicks: 800 }),
      counters({ totalTicks: 1200, idleTicks: 900 }, { userTicks: 300, idleTicks: 900 }),
    );

    expect(shares?.map((entry) => entry.state)).toEqual([
      "userTicks",
      "niceTicks",
      "systemTicks",
      "interruptTicks",
      "softInterruptTicks",
      "ioWaitTicks",
      "stealTicks",
      "idleTicks",
    ]);
    expect(shares?.find((entry) => entry.state === "userTicks")?.percent).toBe(50);
    expect(shares?.find((entry) => entry.state === "idleTicks")?.percent).toBe(50);
  });

  /** The one thing the breakdown shows that the summary cannot: idle and waiting on a disk apart. */
  it("keeps waiting on I/O out of idle", () => {
    const shares = cpuBreakdown(
      counters({ totalTicks: 100, idleTicks: 60 }, { idleTicks: 40, ioWaitTicks: 20 }),
      counters({ totalTicks: 200, idleTicks: 120 }, { idleTicks: 80, ioWaitTicks: 40 }),
    );

    expect(shares?.find((entry) => entry.state === "idleTicks")?.percent).toBe(40);
    expect(shares?.find((entry) => entry.state === "ioWaitTicks")?.percent).toBe(20);
  });
});

describe("interfaceRates", () => {
  it("divides the difference by the interval", () => {
    const rates = interfaceRates([interfaceCounters("eth0", 1000, 500)], [interfaceCounters("eth0", 3000, 1500)], 2000);

    expect(rates).toEqual([{ name: "eth0", received: 1000, transmitted: 500 }]);
  });

  it("leaves out an interface only one of the samples knows", () => {
    const rates = interfaceRates(
      [interfaceCounters("eth0", 1000, 500)],
      [interfaceCounters("eth0", 2000, 1000), interfaceCounters("docker0", 10, 20)],
      1000,
    );

    expect(rates.map((rate) => rate.name)).toEqual(["eth0"]);
  });

  it("reads a restarted interface as having carried nothing, not as a negative rate", () => {
    const rates = interfaceRates([interfaceCounters("eth0", 5000, 900)], [interfaceCounters("eth0", 10, 20)], 1000);

    expect(rates).toEqual([{ name: "eth0", received: 0, transmitted: 0 }]);
  });

  it("returns nothing when there is no interval to divide by", () => {
    expect(interfaceRates([interfaceCounters("eth0", 0, 0)], [interfaceCounters("eth0", 100, 100)], 0)).toEqual([]);
  });
});

describe("busiestInterface", () => {
  it("picks whatever moved the most in either direction", () => {
    const busiest = busiestInterface([
      { name: "eth0", received: 10, transmitted: 10 },
      { name: "docker0", received: 500, transmitted: 20 },
    ]);

    expect(busiest?.name).toBe("docker0");
  });

  it("is null with nothing to choose from, and the first when nothing is moving", () => {
    expect(busiestInterface([])).toBeNull();
    expect(
      busiestInterface([
        { name: "eth0", received: 0, transmitted: 0 },
        { name: "docker0", received: 0, transmitted: 0 },
      ])?.name,
    ).toBe("eth0");
  });

  /**
   * Loopback is a real interface and is listed like one, but it is never picked on its own: a host
   * that talks to itself would otherwise be drawn as a host with heavy traffic, and the panel would
   * be describing a conversation nobody outside the machine is having.
   */
  it("never picks loopback while another interface is there", () => {
    const busyLoopback = busiestInterface([
      { name: "lo", received: 900_000, transmitted: 900_000 },
      { name: "eth0", received: 10, transmitted: 10 },
    ]);

    expect(busyLoopback?.name).toBe("eth0");
  });

  it("still picks loopback when it is the only interface there is", () => {
    expect(busiestInterface([{ name: "lo", received: 10, transmitted: 10 }])?.name).toBe("lo");
    expect(isLoopback("lo")).toBe(true);
    expect(isLoopback("eth0")).toBe(false);
  });
});

describe("memory", () => {
  it("counts what the kernel could not hand out as used", () => {
    expect(memoryUsed(memory())).toBe(600);
    expect(swapUsed(memory({ swapFreeBytes: 200 }))).toBe(300);
  });

  it("does not report more used than there is", () => {
    expect(memoryUsed(memory({ totalBytes: 100, availableBytes: 500 }))).toBe(0);
  });
});

describe("share", () => {
  it("is a percentage of a whole, and null when there is no whole", () => {
    expect(share(25, 100)).toBe(25);
    expect(share(0, 0)).toBeNull();
    expect(share(200, 100)).toBe(100);
  });
});
