/**
 * The numbers that only exist as a difference.
 *
 * A sample carries counters, not rates — the far side reports how many ticks it has spent since
 * it booted and how many bytes have crossed an interface since it came up, which are readings and
 * not answers. What a person wants to see is the difference between two of them over a known
 * interval, and that is arithmetic: pure, testable, and kept out of the store so it can be proved
 * against fixed numbers rather than against a timer.
 *
 * Two rules run through all of it:
 *
 * 1. **A rate needs two samples.** The first sample after a connection opens has nothing to be
 *    subtracted from, so the reading is `null` — drawn as a dash — rather than the number a single
 *    counter would suggest. Reporting "the average since boot" as if it were the current load is
 *    the one answer that is always wrong and always looks right.
 * 2. **A counter that goes backwards is not a negative rate.** It is a reboot, an interface that
 *    was restarted, or two different hosts. The difference is thrown away rather than drawn,
 *    because a spike below zero on a chart is a rendering bug the reader has to interpret.
 */

import type { CpuCounters, CpuStates, MemoryUsage, NetworkCounters } from "@/types/monitor";

/** How fast one interface is moving, in bytes per second over the interval between two samples. */
export interface InterfaceRate {
  readonly name: string;
  readonly received: number;
  readonly transmitted: number;
}

/** One column of the CPU breakdown, as a share of the interval between two samples. */
export interface CpuShare {
  readonly state: keyof CpuStates;
  readonly percent: number;
}

/**
 * The order the breakdown is shown in: what the machine did with its time first — for the user,
 * for the system, on interrupts — and then what it did not, with waiting on disks next to idle
 * rather than lost among the busy states.
 */
export const CPU_STATE_ORDER = [
  "userTicks",
  "niceTicks",
  "systemTicks",
  "interruptTicks",
  "softInterruptTicks",
  "ioWaitTicks",
  "stealTicks",
  "idleTicks",
] as const satisfies readonly (keyof CpuStates)[];

/**
 * What each column is called on screen.
 *
 * Beside the order because they are one list read two ways, and because the two drifting apart —
 * a column renamed where it is listed and not where it is labelled — is how a table ends up with
 * the numbers of one thing under the heading of another.
 */
export const CPU_STATE_LABELS: Record<keyof CpuStates, string> = {
  userTicks: "inspect.cpu.state.user",
  niceTicks: "inspect.cpu.state.nice",
  systemTicks: "inspect.cpu.state.system",
  interruptTicks: "inspect.cpu.state.interrupt",
  softInterruptTicks: "inspect.cpu.state.softInterrupt",
  ioWaitTicks: "inspect.cpu.state.ioWait",
  stealTicks: "inspect.cpu.state.steal",
  idleTicks: "inspect.cpu.state.idle",
};

/** How busy the CPUs were between two samples, as a percentage, or `null` with nothing to compare. */
export function cpuUsage(previous: CpuCounters | null, current: CpuCounters | null): number | null {
  if (!previous || !current) {
    return null;
  }

  const total = current.totalTicks - previous.totalTicks;
  const idle = current.idleTicks - previous.idleTicks;
  if (total <= 0 || idle < 0) {
    return null;
  }

  const busy = Math.min(Math.max(total - idle, 0), total);
  return (busy / total) * 100;
}

/**
 * Every interface both samples know about, at its current rate.
 *
 * An interface in only one of the two is left out: there is no interval to measure across a
 * missing reading, and inventing one would report a container starting up as traffic.
 */
export function interfaceRates(
  previous: readonly NetworkCounters[],
  current: readonly NetworkCounters[],
  elapsedMs: number,
): InterfaceRate[] {
  const seconds = elapsedMs / 1000;
  if (!(seconds > 0)) {
    return [];
  }

  const rates: InterfaceRate[] = [];
  for (const before of previous) {
    const now = current.find((entry) => entry.name === before.name);
    if (!now) {
      continue;
    }

    rates.push({
      name: now.name,
      // Clamped at zero for the reason in the module comment: a counter that went backwards is a
      // restart, and a restarted interface has carried nothing this interval.
      received: Math.max(0, now.receivedBytes - before.receivedBytes) / seconds,
      transmitted: Math.max(0, now.transmittedBytes - before.transmittedBytes) / seconds,
    });
  }

  return rates;
}

/**
 * The interface to draw: whatever moved the most this interval.
 *
 * A machine has one interface that matters at a time, and it is not always the first one — a
 * container host is busiest on a bridge, and a storage host on a bond. Loopback is measured like
 * any other and picked like none of them: on a host that talks to itself it is always the busiest,
 * and the panel would be describing a conversation nobody is having. It is still the answer when it
 * is the only interface there is, and it is always in the list the picker offers.
 */
export function busiestInterface(rates: readonly InterfaceRate[]): InterfaceRate | null {
  const candidates = rates.filter((rate) => !isLoopback(rate.name));
  const pool = candidates.length > 0 ? candidates : rates;

  let busiest: InterfaceRate | null = null;
  for (const rate of pool) {
    const moved = rate.received + rate.transmitted;
    const best = busiest ? busiest.received + busiest.transmitted : -1;
    if (moved > best) {
      busiest = rate;
    }
  }
  return busiest;
}

/** Whether an interface is the host talking to itself. */
export function isLoopback(name: string): boolean {
  return name === "lo" || name.startsWith("lo:");
}

/**
 * What the processors spent the interval doing, as a percentage each.
 *
 * Null with only one sample, for the same reason a rate is: these are differences. The shares are
 * of the total the kernel reported rather than of the sum of the parts, so a column the kernel adds
 * later shows up as the parts not quite filling the whole — which is a truer picture than quietly
 * rescaling everything to fit.
 */
export function cpuBreakdown(previous: CpuCounters | null, current: CpuCounters | null): CpuShare[] | null {
  if (!previous || !current) {
    return null;
  }

  const total = current.totalTicks - previous.totalTicks;
  if (total <= 0) {
    return null;
  }

  return CPU_STATE_ORDER.map((state) => {
    const delta = current.states[state] - previous.states[state];
    return { state, percent: (Math.min(Math.max(delta, 0), total) / total) * 100 };
  });
}

/** Memory in use, by the kernel's own account: what it could not hand out without swapping. */
export function memoryUsed(memory: MemoryUsage): number {
  return Math.max(0, memory.totalBytes - memory.availableBytes);
}

/** Swap in use. */
export function swapUsed(memory: MemoryUsage): number {
  return Math.max(0, memory.swapTotalBytes - memory.swapFreeBytes);
}

/** A part of a whole, as a percentage, or `null` when the whole is nothing. */
export function share(part: number, whole: number): number | null {
  if (!(whole > 0)) {
    return null;
  }
  return (Math.min(Math.max(part, 0), whole) / whole) * 100;
}
