/**
 * The host monitor: which connection is being watched, and what the last samples said.
 *
 * The panel draws; this store decides what there is to draw. Four rules shape it:
 *
 * 1. **It follows the connection on screen.** Not the newest session, and not one the person
 *    picked in a list inside a 320px column: the machine somebody is working on is the one they
 *    are looking at, and the panel has to agree with the terminal beside it. That also decides
 *    when there is nothing to show — a session whose terminal has not opened, or has ended, has no
 *    connection to sample, so the panel draws its empty state instead of guessing.
 * 2. **It samples nothing while nothing is shown.** The panel is mounted while the setting is on
 *    and a detail page is mounted while it is the tab on screen; both ask this store to start, and
 *    the sampling runs only while something is asking. A background poll nobody can see is
 *    somebody else's machine paying for a feature nobody is looking at.
 * 3. **A rate comes from two samples, and only when they are close enough together.** The first
 *    sample has nothing to subtract from, and a sample taken after a gap — a tab switched away, a
 *    window minimized, a sample that failed — would average over that gap and be drawn as if it
 *    were the last two seconds. Both are `null` and both are drawn as a dash.
 * 4. **The process page asks its own question.** Two hundred command lines are not a panel's
 *    business, so the process list is filled by its own command, on its own slower clock, and only
 *    while that page is the one on screen.
 *
 * Nothing is kept between runs and nothing is kept per session: this is a view of a live
 * connection, and a connection that is gone has no history worth keeping.
 */

import { defineStore } from "pinia";
import { computed, ref, watch } from "vue";

import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import {
  type CpuShare,
  type InterfaceRate,
  busiestInterface,
  cpuBreakdown,
  cpuUsage,
  interfaceRates,
  isLoopback,
} from "@/lib/monitor/rates";
import { type TerminalTab, sessionOf, useTerminalsStore } from "@/stores/terminals";
import {
  type ProcessRow,
  type ProcessSortKey,
  type TicksBefore,
  processRows,
  sortProcesses as sortRows,
  ticksOf,
} from "@/lib/monitor/processes";
import type { HostMetrics, ProcessDetail, SocketRow } from "@/types/monitor";

/** How often a sample is taken while something that shows it is on screen. */
export const MONITOR_POLL_MS = 2000;

/**
 * How often the process page re-reads its list.
 *
 * Slower than the panel on purpose: a hundred rows with their command lines are a much larger
 * answer than a sample, and a list of processes does not have to move every two seconds to be read.
 */
export const PROCESS_POLL_MS = 3000;

/** How many processes the page asks for. Enough to cover a busy machine; not the whole of one. */
export const PROCESS_PAGE_LIMIT = 100;

/**
 * How often the network page re-reads the host's sockets.
 *
 * The same reasoning as the process list: a page somebody is reading asks for a list of its own,
 * on a clock slow enough that reading it does not cost more than the list is worth.
 */
export const SOCKET_POLL_MS = 3000;

/**
 * The longest gap between two samples that still counts as an interval.
 *
 * A rate is a difference over time, and the chart draws one point per sample rather than one per
 * second. Beyond a few missed polls the point would stand for a minute of averaging next to points
 * that stand for two seconds, which is a shape nobody can read — so the interval restarts instead.
 */
export const MONITOR_MAX_GAP_MS = MONITOR_POLL_MS * 3;

/** How many samples the charts keep. Two minutes at the interval above, which is what fits the panel. */
export const MONITOR_HISTORY = 60;

/** The connection the monitor follows, and what the header says about it. */
export interface MonitorTarget {
  readonly handle: string;
  readonly sessionId: string;
  readonly name: string;
  readonly host: string;
  readonly port: number;
  readonly user: string;
}

/** One point on the traffic chart: the chosen interface's rate at that sample. */
export interface TrafficPoint {
  readonly received: number;
  readonly transmitted: number;
}

export const useMonitorStore = defineStore("monitor", () => {
  const terminals = useTerminalsStore();

  const sample = ref<HostMetrics | null>(null);
  /**
   * Which session the sample on screen came from.
   *
   * A sample carries no session id of its own — the store only ever holds one — so this is what
   * lets a page tell "these numbers are about my host" from "these numbers are about the host the
   * terminal beside me is on". A page that asked the store instead would show another machine's
   * readings for the moment between switching tabs and the next sample.
   */
  const sampleSessionId = ref<string | null>(null);
  /** Why the last sample did not arrive. The last one that did stays on screen underneath it. */
  const failure = ref<ShellError | null>(null);
  const sampling = ref(false);
  /** Busy CPUs as a percentage, or null until there are two samples to compare. */
  const cpuPercent = ref<number | null>(null);
  /** The same interval split by what the processors were doing, for the page that shows it. */
  const cpuShares = ref<CpuShare[] | null>(null);
  /** The last interval's interface rates, which is what the panel's numbers say. */
  const rates = ref<InterfaceRate[]>([]);
  /** Chart history per interface, so switching the picker does not erase what was drawn. */
  const histories = ref<Record<string, TrafficPoint[]>>({});
  const roundTrips = ref<number[]>([]);
  /** The interface somebody picked, or null for "whichever is busiest". */
  const picked = ref<string | null>(null);

  /** The process page's own list, filled only while that page is on screen. */
  /** The process table as it is drawn: the rows, each with what it used since the last read. */
  const processes = ref<ProcessRow[]>([]);
  const processSort = ref<ProcessSortKey>("cpu");
  const processDescending = ref(true);
  const processFailure = ref<ShellError | null>(null);
  const readingProcesses = ref(false);
  /**
   * Where each process's processor time stood at the last read, and when that was.
   *
   * Not reactive, for the same reason the sample's own counters are not: nothing draws them, and
   * the only thing that reads them is the read that follows.
   */
  let previousTicks: TicksBefore | null = null;

  /** One process opened in full, and why it could not be. Keyed by pid: the page keeps tabs. */
  const details = ref<Record<number, ProcessDetail>>({});
  const detailFailures = ref<Record<number, ShellError>>({});
  const readingDetail = ref<number | null>(null);

  /** The network page's own list, filled only while that page is on screen. */
  const sockets = ref<SocketRow[]>([]);
  const socketFailure = ref<ShellError | null>(null);
  const readingSockets = ref(false);

  /**
   * The counters of the sample before the current one.
   *
   * Deliberately not a `ref`: nothing draws it, and making it reactive would have the panel
   * re-render for a value that is only ever read by the next sample.
   */
  let previous: HostMetrics | null = null;
  let timer: ReturnType<typeof setInterval> | undefined;
  let processTimer: ReturnType<typeof setInterval> | undefined;
  let socketTimer: ReturnType<typeof setInterval> | undefined;
  let inFlight = false;
  let readingInFlight = false;
  let readingSocketsInFlight = false;
  /** How many things on screen are asking for samples: the panel, or a detail page. */
  let watching = 0;
  /** The handle the readings belong to, so an answer that arrives late is not absorbed. */
  let followed: string | null = null;

  /**
   * The connection on screen, when it is one that can be sampled.
   *
   * A terminal tab's session, found through whichever open terminal is carrying that session —
   * which is what makes a file tab beside its terminal keep the monitor running rather than
   * blanking the panel for a click. The handle is a terminal's, because a sample rides the
   * connection that terminal opened. A tab about no host at all — the settings page — is nobody's
   * session, so the panel falls back to saying it has no connection rather than to the last one.
   */
  const target = computed<MonitorTarget | null>(() => {
    const active = terminals.activeTab;
    const activeSession = active ? sessionOf(active) : null;
    if (!activeSession) {
      return null;
    }

    const open = terminals.tabs.find(
      (entry): entry is TerminalTab =>
        entry.kind === "terminal" && entry.session.id === activeSession.id && entry.status === "open",
    );
    const handle = open?.handle;
    if (!open || handle === null || handle === undefined) {
      return null;
    }

    const { session } = open;
    return {
      handle,
      sessionId: session.id,
      name: session.name,
      host: session.host,
      port: session.port,
      user: session.user,
    };
  });

  /** Every interface the host reported, in the order it reported them. */
  const interfaces = computed(() => sample.value?.network.map((entry) => entry.name) ?? []);

  /**
   * The interface the figures are about: what somebody picked, or the busiest one.
   *
   * A pick that is no longer in the host's list — an interface that went away with a VPN — falls
   * back to the automatic answer rather than drawing nothing, because "the interface you chose does
   * not exist any more" is something to learn from the list, not from an empty chart.
   */
  const interfaceName = computed(() => {
    if (picked.value !== null && interfaces.value.includes(picked.value)) {
      return picked.value;
    }

    const busiest = busiestInterface(rates.value);
    if (busiest) {
      return busiest.name;
    }

    // Nothing has moved yet — the first sample after a connection opens has no rates — so this is
    // the first interface that is not loopback, which is the one somebody would name if asked and
    // the one the rates are about a moment later.
    return interfaces.value.find((name) => !isLoopback(name)) ?? interfaces.value[0] ?? null;
  });

  /** How fast the chosen interface is moving right now. */
  const currentRate = computed(() => rates.value.find((rate) => rate.name === interfaceName.value) ?? null);

  /** What the chosen interface has carried since it came up. */
  const currentTotals = computed(
    () => sample.value?.network.find((entry) => entry.name === interfaceName.value) ?? null,
  );

  /** The chart for the chosen interface: switching the picker switches charts rather than erasing one. */
  const traffic = computed(() => {
    const name = interfaceName.value;
    return (name !== null && histories.value[name]) || [];
  });

  /** Starts asking for samples. Called by everything that draws one; five callers are still one poll. */
  function start(): void {
    watching += 1;
    if (timer !== undefined) {
      return;
    }
    timer = setInterval(() => void tick(), MONITOR_POLL_MS);
    void tick();
  }

  /** Stops asking. The poll ends when the last thing that wanted it has gone. */
  function stop(): void {
    watching = Math.max(0, watching - 1);
    if (watching > 0 || timer === undefined) {
      return;
    }
    clearInterval(timer);
    timer = undefined;
  }

  /** Chooses the interface the figures are about, or `null` to go back to the busiest one. */
  function selectInterface(name: string | null): void {
    picked.value = name;
  }

  /**
   * One sample, if there is anything to sample and nothing already on its way.
   *
   * Skipping the tick rather than queueing it is the point: a host that takes ten seconds to answer
   * would otherwise collect five overlapping commands, and the panel would fill with answers to
   * questions nobody is asking any more.
   */
  async function tick(): Promise<void> {
    const watched = target.value;
    if (!watched) {
      forget();
      return;
    }

    if (watched.handle !== followed) {
      // A different connection: its counters are not comparable with the ones held, and neither is
      // the chart. Starting over is the only answer that is not a lie.
      forget();
      followed = watched.handle;
    }

    if (inFlight || (typeof document !== "undefined" && document.hidden)) {
      return;
    }

    inFlight = true;
    sampling.value = true;
    try {
      const answer = await backend.monitor.sample(watched.handle);
      if (followed === watched.handle) {
        absorb(answer, watched.sessionId);
      }
      failure.value = null;
    } catch (error) {
      failure.value = asShellError(error, "monitor_sample");
    } finally {
      inFlight = false;
      sampling.value = false;
    }
  }

  /** Folds one sample into what is on screen, measuring against the sample before it. */
  function absorb(answer: HostMetrics, sessionId: string): void {
    const before = previous;
    const elapsed = before === null ? 0 : answer.sampledAt - before.sampledAt;
    const comparable = before !== null && elapsed > 0 && elapsed <= MONITOR_MAX_GAP_MS;

    if (before !== null && comparable) {
      cpuPercent.value = cpuUsage(before.cpu, answer.cpu);
      cpuShares.value = cpuBreakdown(before.cpu, answer.cpu);
      rates.value = interfaceRates(before.network, answer.network, elapsed);
      draw();
    } else {
      // Nothing to measure against: the readings that need two samples restart rather than being
      // carried over a gap they did not see.
      cpuPercent.value = null;
      cpuShares.value = null;
      rates.value = [];
      histories.value = {};
    }

    roundTrips.value = [...roundTrips.value, answer.roundTripMs].slice(-MONITOR_HISTORY);
    previous = answer;
    sample.value = answer;
    sampleSessionId.value = sessionId;
  }

  /**
   * Adds one point per interface to the charts.
   *
   * Per interface rather than one for whichever is chosen: the choice changes what is drawn, and a
   * chart that restarted every time somebody compared two interfaces would be empty at the exact
   * moment they were looking for something.
   */
  function draw(): void {
    const next = { ...histories.value };
    for (const rate of rates.value) {
      // Rates rather than the raw counters, so the chart is in the same unit as the numbers above
      // it: bytes per second over the interval, not bytes since the interface came up.
      const point = { received: rate.received, transmitted: rate.transmitted };
      next[rate.name] = [...(next[rate.name] ?? []), point].slice(-MONITOR_HISTORY);
    }
    histories.value = next;
  }

  /** Drops everything that was about the connection being left behind. */
  function forget(): void {
    sample.value = null;
    sampleSessionId.value = null;
    previous = null;
    failure.value = null;
    cpuPercent.value = null;
    cpuShares.value = null;
    rates.value = [];
    histories.value = {};
    roundTrips.value = [];
    processes.value = [];
    previousTicks = null;
    processFailure.value = null;
    details.value = {};
    detailFailures.value = {};
    sockets.value = [];
    socketFailure.value = null;
  }

  /**
   * Reads one process in full, for the tab somebody just opened.
   *
   * Once, not on a clock: a command line, a directory and an environment are what they were when
   * the process started, and re-reading them every few seconds would be gathering a page of
   * credentials over and over for something that does not change.
   */
  async function readProcessDetail(pid: number): Promise<void> {
    const watched = target.value;
    if (!watched) {
      return;
    }

    readingDetail.value = pid;
    try {
      const detail = await backend.monitor.processDetail(watched.handle, pid);
      details.value = { ...details.value, [pid]: detail };
      deleteFailure(pid);
    } catch (error) {
      detailFailures.value = { ...detailFailures.value, [pid]: asShellError(error, "monitor_process_detail") };
    } finally {
      if (readingDetail.value === pid) {
        readingDetail.value = null;
      }
    }
  }

  function deleteFailure(pid: number): void {
    if (!(pid in detailFailures.value)) {
      return;
    }
    const next = { ...detailFailures.value };
    delete next[pid];
    detailFailures.value = next;
  }

  /** Forgets one process: the tab was closed, and nothing else needs what it said. */
  function forgetProcess(pid: number): void {
    const next = { ...details.value };
    delete next[pid];
    details.value = next;
    deleteFailure(pid);
  }

  /** Starts reading the process page's list. Called by that page while it is on screen. */
  function startProcesses(): void {
    if (processTimer !== undefined) {
      return;
    }
    processTimer = setInterval(() => void readProcesses(), PROCESS_POLL_MS);
    void readProcesses();
  }

  function stopProcesses(): void {
    if (processTimer === undefined) {
      return;
    }
    clearInterval(processTimer);
    processTimer = undefined;
  }

  /**
   * Orders the table by something else, or turns the order around.
   *
   * A redraw rather than another command: the host cut this list once and sent it, and asking it
   * again to sort the same rows would be a round trip and a `ps` on somebody's server for something
   * this side can do in a millisecond.
   */
  function sortProcesses(key: ProcessSortKey, descending: boolean): void {
    if (processSort.value === key && processDescending.value === descending) {
      return;
    }
    processSort.value = key;
    processDescending.value = descending;
    processes.value = sortRows(processes.value, key, descending);
  }

  /**
   * One read of the process list.
   *
   * The list is not emptied when a read fails: what is on screen is what the host said a moment
   * ago, and the failure is drawn above it rather than instead of it — the rule the panel's sample
   * follows too.
   */
  async function readProcesses(): Promise<void> {
    const watched = target.value;
    if (!watched) {
      processes.value = [];
      return;
    }

    if (readingInFlight || (typeof document !== "undefined" && document.hidden)) {
      return;
    }

    readingInFlight = true;
    readingProcesses.value = true;
    try {
      const answer = await backend.monitor.processes(watched.handle, PROCESS_PAGE_LIMIT);
      if (target.value?.handle === watched.handle) {
        // Measured here rather than on the host: the interval is between two reads taken from this
        // end, and the host's clock has nothing to do with how long that was.
        const at = Date.now();
        const rows = processRows(previousTicks, answer, at);
        previousTicks = ticksOf(answer, at);
        processes.value = sortRows(rows, processSort.value, processDescending.value);
      }
      processFailure.value = null;
    } catch (error) {
      processFailure.value = asShellError(error, "monitor_processes");
    } finally {
      readingInFlight = false;
      readingProcesses.value = false;
    }
  }

  /** Starts reading the network page's sockets. Called by that page while it is on screen. */
  function startSockets(): void {
    if (socketTimer !== undefined) {
      return;
    }
    socketTimer = setInterval(() => void readSockets(), SOCKET_POLL_MS);
    void readSockets();
  }

  function stopSockets(): void {
    if (socketTimer === undefined) {
      return;
    }
    clearInterval(socketTimer);
    socketTimer = undefined;
  }

  /** One read of the socket list, with the same rule as the process list about failures. */
  async function readSockets(): Promise<void> {
    const watched = target.value;
    if (!watched) {
      sockets.value = [];
      return;
    }

    if (readingSocketsInFlight || (typeof document !== "undefined" && document.hidden)) {
      return;
    }

    readingSocketsInFlight = true;
    readingSockets.value = true;
    try {
      const answer = await backend.monitor.sockets(watched.handle);
      if (target.value?.handle === watched.handle) {
        sockets.value = answer;
      }
      socketFailure.value = null;
    } catch (error) {
      socketFailure.value = asShellError(error, "monitor_sockets");
    } finally {
      readingSocketsInFlight = false;
      readingSockets.value = false;
    }
  }

  // Switching tabs must not leave the previous host's numbers on screen for the rest of the poll:
  // the readings belong to a connection, so they go the moment the connection on screen changes.
  watch(
    () => target.value?.handle ?? null,
    () => {
      if (watching === 0) {
        return;
      }
      forget();
      void tick();
      if (processTimer !== undefined) {
        void readProcesses();
      }
      if (socketTimer !== undefined) {
        void readSockets();
      }
    },
  );

  return {
    sample,
    sampleSessionId,
    failure,
    sampling,
    cpuPercent,
    cpuShares,
    rates,
    traffic,
    roundTrips,
    interfaces,
    interfaceName,
    currentRate,
    currentTotals,
    processes,
    processSort,
    processDescending,
    processFailure,
    readingProcesses,
    details,
    detailFailures,
    readingDetail,
    readProcessDetail,
    forgetProcess,
    sockets,
    socketFailure,
    readingSockets,
    target,
    start,
    stop,
    selectInterface,
    startProcesses,
    stopProcesses,
    sortProcesses,
    startSockets,
    stopSockets,
  };
});
