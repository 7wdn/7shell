import { describe, expect, it } from "vitest";

import { processRows, sortProcesses, ticksOf } from "@/lib/monitor/processes";
import type { ProcessPage, ProcessSample } from "@/types/monitor";

/** One process, with the parts a test is not about filled in. */
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
    cpuTicks: 1_000,
    ...overrides,
  };
}

function page(processes: ProcessSample[], ticksPerSecond = 100): ProcessPage {
  return { ticksPerSecond, processes };
}

describe("processRows", () => {
  it("has nothing to say about the first read, because a counter is not a rate", () => {
    const rows = processRows(null, page([process()]), 1_700_000_000_000);

    expect(rows[0]?.cpuPercent).toBeNull();
    expect(rows[0]?.sample.name, "and the reading itself is still there").toBe("systemd");
  });

  it("measures what each process used over the interval between two reads", () => {
    // Two seconds passed and the counter moved 50 ticks: half a second of processor time, which is
    // a quarter of one processor over that interval.
    const before = ticksOf(page([process({ cpuTicks: 1_000 })]), 1_700_000_000_000);
    const rows = processRows(before, page([process({ cpuTicks: 1_050 })]), 1_700_000_002_000);

    expect(rows[0]?.cpuPercent).toBeCloseTo(25, 5);
  });

  it("reads the counters in whatever the host counts in", () => {
    // A host counting 250 ticks a second: twice as many ticks for the same second of processor time.
    const before = ticksOf(page([process({ cpuTicks: 0 })], 250), 1_700_000_000_000);
    const rows = processRows(before, page([process({ cpuTicks: 250 })], 250), 1_700_000_001_000);

    expect(rows[0]?.cpuPercent, "one processor busy for the whole second").toBeCloseTo(100, 5);
  });

  /** A pid that was not there a moment ago has no interval, and a number for it would be invented. */
  it("says nothing about a process it has not seen twice", () => {
    const before = ticksOf(page([process({ pid: 1 })]), 1_700_000_000_000);
    const rows = processRows(before, page([process({ pid: 1 }), process({ pid: 2 })]), 1_700_000_002_000);

    expect(rows[0]?.cpuPercent).not.toBeNull();
    expect(rows[1]?.cpuPercent).toBeNull();
  });

  /** A reused pid starts its counter again: that is nothing used, not a negative share. */
  it("never reports a share below zero when a counter goes backwards", () => {
    const before = ticksOf(page([process({ cpuTicks: 90_000 })]), 1_700_000_000_000);
    const rows = processRows(before, page([process({ cpuTicks: 10 })]), 1_700_000_002_000);

    expect(rows[0]?.cpuPercent).toBe(0);
  });

  it("does not divide by an interval that is not there", () => {
    const before = ticksOf(page([process()]), 1_700_000_000_000);
    const rows = processRows(before, page([process({ cpuTicks: 2_000 })]), 1_700_000_000_000);

    expect(rows[0]?.cpuPercent).toBeNull();
  });
});

describe("ticksOf", () => {
  it("keeps only what the host actually reported", () => {
    const before = ticksOf(page([process({ pid: 1, cpuTicks: 500 }), process({ pid: 2, cpuTicks: null })]), 42);

    expect(before.at).toBe(42);
    expect([...before.ticks.entries()]).toEqual([[1, 500]]);
  });
});

describe("sortProcesses", () => {
  const rows = processRows(
    null,
    page([
      process({ pid: 1, name: "systemd", residentBytes: 300, cpuTicks: 1 }),
      process({ pid: 2, name: "alpha", residentBytes: 900, cpuTicks: 1 }),
      process({ pid: 3, name: "zeta", residentBytes: 100, cpuTicks: 1 }),
    ]),
    0,
  );

  it("orders by memory, either way", () => {
    expect(sortProcesses(rows, "memory", false).map((row) => row.sample.pid)).toEqual([3, 1, 2]);
    expect(sortProcesses(rows, "memory", true).map((row) => row.sample.pid)).toEqual([2, 1, 3]);
  });

  it("orders by name, from the alphabet unless told otherwise", () => {
    expect(sortProcesses(rows, "name", false).map((row) => row.sample.name)).toEqual(["alpha", "systemd", "zeta"]);
    expect(sortProcesses(rows, "name", true).map((row) => row.sample.name)).toEqual(["zeta", "systemd", "alpha"]);
  });

  /**
   * A process with no reading is not a process using nothing: a dash at the top of a column of
   * numbers reads as a reading, and the one place that must not happen is a sort by CPU.
   */
  it("puts a process with no CPU reading below the ones that have one, at either end", () => {
    const mixed = [
      { sample: process({ pid: 1 }), cpuPercent: 5 },
      { sample: process({ pid: 2 }), cpuPercent: null },
      { sample: process({ pid: 3 }), cpuPercent: 90 },
    ];

    expect(sortProcesses(mixed, "cpu", true).map((row) => row.sample.pid)).toEqual([3, 1, 2]);
    expect(sortProcesses(mixed, "cpu", false).map((row) => row.sample.pid)).toEqual([1, 3, 2]);
  });

  it("leaves the list it was given alone", () => {
    const before = rows.map((row) => row.sample.pid);

    sortProcesses(rows, "memory", true);

    expect(
      rows.map((row) => row.sample.pid),
      "sorting is not editing",
    ).toEqual(before);
  });
});
