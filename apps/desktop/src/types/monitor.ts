/**
 * Hand-written mirror of the Rust host-sample types, from `sevenshell_types::monitor`.
 *
 * `packages/app-tests/contracts.test.ts` compares every field here with its Rust counterpart, so
 * a rename on one side fails the test suite rather than a release.
 *
 * Every section is nullable, and that is the shape rather than an accident: a sample is taken by
 * running commands on a host that may not have them, and "this host did not say" has to stay
 * distinguishable from a zero — which is a real reading, and the one a gauge must never invent.
 */

/** A reading of one host at one moment. */
export interface HostMetrics {
  /** Milliseconds since the Unix epoch, on the machine that asked. */
  readonly sampledAt: number;
  /** How long the sample took from this side: the round trip plus the commands. */
  readonly roundTripMs: number;
  readonly hostname: string | null;
  readonly os: string | null;
  readonly kernel: string | null;
  readonly arch: string | null;
  readonly cores: number | null;
  /** What the host says its processors are, when it says anything. */
  readonly cpuModel: CpuModel | null;
  readonly uptimeSeconds: number | null;
  readonly load: LoadAverage | null;
  /** Cumulative CPU ticks: a rate needs two of these, a known distance apart. */
  readonly cpu: CpuCounters | null;
  readonly memory: MemoryUsage | null;
  /** Cumulative interface counters, loopback included: the page lists them all. */
  readonly network: readonly NetworkCounters[];
  readonly disks: readonly DiskUsage[];
  readonly processes: readonly ProcessSample[];
}

/** What the host calls its processor, from `/proc/cpuinfo`. */
export interface CpuModel {
  readonly name: string;
  readonly frequencyMhz: number | null;
  readonly cacheKb: number | null;
  readonly bogomips: number | null;
}

export interface LoadAverage {
  readonly one: number;
  readonly five: number;
  readonly fifteen: number;
}

export interface CpuCounters {
  readonly totalTicks: number;
  /** Idle plus waiting on I/O: a disk-bound host is not a busy CPU. */
  readonly idleTicks: number;
  /** The same readings, still split by what the processor was doing. */
  readonly states: CpuStates;
}

/**
 * The kernel's CPU columns, in ticks. Note that `idleTicks` here is the kernel's own idle column,
 * while `CpuCounters.idleTicks` is idle *plus* waiting on I/O: two different questions.
 */
export interface CpuStates {
  readonly userTicks: number;
  readonly niceTicks: number;
  readonly systemTicks: number;
  readonly idleTicks: number;
  readonly ioWaitTicks: number;
  readonly interruptTicks: number;
  readonly softInterruptTicks: number;
  readonly stealTicks: number;
}

export interface MemoryUsage {
  readonly totalBytes: number;
  readonly availableBytes: number;
  readonly swapTotalBytes: number;
  readonly swapFreeBytes: number;
}

export interface NetworkCounters {
  readonly name: string;
  readonly receivedBytes: number;
  readonly transmittedBytes: number;
}

export interface DiskUsage {
  readonly mountPoint: string;
  readonly filesystem: string;
  readonly sizeBytes: number;
  readonly availableBytes: number;
}

export interface ProcessSample {
  readonly pid: number;
  /** The account it runs as. */
  readonly user: string;
  /** What it is called — the executable's own name. */
  readonly name: string;
  /** How it was invoked, capped on the far side. Empty for a kernel thread. */
  readonly command: string;
  /** Where the program is on disk, from a read that asked; null from one that did not. */
  readonly executable: string | null;
  /**
   * Percent of one CPU as `ps` measures it — an average since the process started, which is why it
   * is not what a live table shows: an idle service reads the same number every time it is asked.
   */
  readonly cpuPercent: number;
  readonly memoryPercent: number;
  readonly residentBytes: number;
  /** Cumulative CPU time in the kernel's ticks, or null from a read that did not ask for it. */
  readonly cpuTicks: number | null;
}

/** A whole process page, as the host answered it. */
export interface ProcessPage {
  /** How many ticks a second the host counts in — its `USER_HZ`, almost always 100. */
  readonly ticksPerSecond: number;
  readonly processes: readonly ProcessSample[];
}

/** One process, looked at closely. Read when somebody opens it, never as part of a poll. */
export interface ProcessDetail {
  readonly pid: number;
  readonly name: string;
  /** Empty for a kernel thread, which was started without one. */
  readonly commandLine: string;
  readonly executable: string;
  readonly workingDirectory: string;
  /** The environment it was started with, in the order the kernel lists it. */
  readonly environment: readonly EnvironmentEntry[];
}

export interface EnvironmentEntry {
  readonly name: string;
  readonly value: string;
}

/**
 * One socket the host has open, listening or connected, from `sevenshell_types::SocketRow`.
 *
 * `state` is the host's own word for what the socket is doing, with one spelling: the two tools
 * that report it disagree about the separator (`TIME-WAIT` and `TIME_WAIT`) and nothing else.
 */
export interface SocketRow {
  /** `tcp`, `udp`, or the host's own spelling of a variant of them. */
  readonly protocol: string;
  readonly state: string;
  /** The local address, numeric. `0.0.0.0` or `::` for "every address on this host". */
  readonly address: string;
  readonly port: number;
  /** The process holding the socket, or zero when the host would not say. */
  readonly pid: number;
  readonly process: string;
  /** The other end, empty for a socket that is not connected to anything. */
  readonly peerAddress: string;
  readonly peerPort: number;
  /** What it has carried, where the host reports it: `netstat` does not, and that is a dash. */
  readonly sentBytes: number | null;
  readonly receivedBytes: number | null;
}
