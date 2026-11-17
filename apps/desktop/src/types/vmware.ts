/**
 * The mirror of the Rust wire types for VMware virtual machines.
 *
 * `id` is Workstation's own identity for a machine (`uuid.bios`), never a `.vmx` path: the
 * interface is given a token, and the host resolves it against its own inventory whenever it acts.
 */
export interface VmInfo {
  readonly id: string;
  readonly name: string;
  /** The `.vmx` guest identifier (`debian12-64`, `windows11-64`); a hint, not a promise. */
  readonly guestOs?: string;
  readonly running: boolean;
}

export interface VmStatus {
  readonly id: string;
  readonly running: boolean;
  /** A lock file exists. While running that is normal; while stopped it blocks the next start. */
  readonly locked: boolean;
  /** What VMware Tools said: `running`, `installed`, `notInstalled`, `notRunning`. */
  readonly toolsState?: string;
  readonly ip?: string;
}

export type VmPowerAction = "start" | "stop" | "suspend";
