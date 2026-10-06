/**
 * The process page's own arithmetic, and its own order.
 *
 * Two rules, both inherited from how every other rate in this application is measured:
 *
 * 1. **CPU is a difference, not a column.** `/proc/<pid>/stat` reports how much processor time a
 *    process has used since it started, and `ps` turns that into an average over the process's whole
 *    life — which for a service that has been up for a week is the same number every time it is
 *    asked. A page that showed that number would look frozen while the machine underneath it was
 *    busy. What is shown instead is the difference between two reads, over the time between them:
 *    the same rule the host's own CPU panel and the interface rates already follow.
 * 2. **The order is the table's, not the host's.** The host cuts its table before sending it, and
 *    what is left is ordered by its own measure; sorting that costs a redraw rather than another
 *    command on somebody's server. The one thing this cannot do is show what the cut left out — a
 *    host with more processes than the page asks for shows the busiest of them, which is the same
 *    trade the cut has always made.
 */

import { byText } from "@/lib/sort";
import type { ProcessPage, ProcessSample } from "@/types/monitor";

/** One line of the table: what the host said, and what the process did since the last read. */
export interface ProcessRow {
  readonly sample: ProcessSample;
  /** Percent of one processor over the interval, or null when there is nothing to compare with. */
  readonly cpuPercent: number | null;
}

/** The columns the table can be ordered by, and what each one means. */
export type ProcessSortKey = "cpu" | "memory" | "name";

/** What the page was told last time: how much processor time each process had used, and when. */
export interface TicksBefore {
  readonly at: number;
  readonly ticks: ReadonlyMap<number, number>;
}

/**
 * The rows to draw, each with the share of a processor it used since the previous read.
 *
 * A process that was not in the previous read has no interval to measure — it may have started a
 * moment ago and used a second of processor time, or been running for a week — so its percentage is
 * null and is drawn as a dash for one reading. Inventing a number there would be the one thing a
 * monitor must never do: report the average since a start nobody saw.
 */
export function processRows(before: TicksBefore | null, page: ProcessPage, at: number): ProcessRow[] {
  const elapsedMs = before === null ? 0 : at - before.at;
  const seconds = elapsedMs / 1000;
  const usable = before !== null && seconds > 0;

  return page.processes.map((sample) => {
    const previous = usable ? before.ticks.get(sample.pid) : undefined;
    const current = sample.cpuTicks;

    if (!usable || previous === undefined || current === undefined || current === null) {
      return { sample, cpuPercent: null };
    }

    // Ticks are a whole number of hundredths of a second each (or whatever the host counts in), and
    // the counters can go backwards when a pid is reused by a new process: that reads as nothing
    // used rather than as a negative share.
    const used = Math.max(0, current - previous) / page.ticksPerSecond;
    return { sample, cpuPercent: (used / seconds) * 100 };
  });
}

/** What to remember for the next read. */
export function ticksOf(page: ProcessPage, at: number): TicksBefore {
  const ticks = new Map<number, number>();
  for (const sample of page.processes) {
    if (sample.cpuTicks !== null) {
      ticks.set(sample.pid, sample.cpuTicks);
    }
  }
  return { at, ticks };
}

/** The same rows, in the order somebody asked for. */
export function sortProcesses(rows: readonly ProcessRow[], key: ProcessSortKey, descending: boolean): ProcessRow[] {
  const direction = descending ? -1 : 1;
  return [...rows].sort((left, right) => {
    // A process the last read did not measure stays at the bottom whichever way the column is read:
    // the direction is applied to the readings, and a dash is not one of them.
    const unread = missingLast(left, right, key);
    if (unread !== 0) {
      return unread;
    }

    const compared = compare(left, right, key);
    // The process id breaks every tie, so two rows that read the same keep one order between reads
    // instead of swapping places every few seconds.
    return compared !== 0 ? compared * direction : left.sample.pid - right.sample.pid;
  });
}

/** Whether one of the two has no reading for the column being sorted — and if so, which. */
function missingLast(left: ProcessRow, right: ProcessRow, key: ProcessSortKey): number {
  if (key !== "cpu") {
    return 0;
  }
  if (left.cpuPercent === null && right.cpuPercent === null) {
    return 0;
  }
  if (left.cpuPercent === null) {
    return 1;
  }
  return right.cpuPercent === null ? -1 : 0;
}

function compare(left: ProcessRow, right: ProcessRow, key: ProcessSortKey): number {
  switch (key) {
    case "memory":
      return left.sample.residentBytes - right.sample.residentBytes;
    case "name":
      return byText(left.sample.name, right.sample.name);
    default:
      // Both have a reading or neither has: `missingLast` has already put the one without at the
      // bottom, and a dash is not a zero.
      return (left.cpuPercent ?? 0) - (right.cpuPercent ?? 0);
  }
}
