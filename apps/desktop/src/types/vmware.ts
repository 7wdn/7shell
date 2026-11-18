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
  /** The guest's address when it is running and Tools answered. Not stored with the session. */
  readonly ip?: string;
}

export interface VmStatus {
  readonly id: string;
  readonly running: boolean;
  /** Suspended with its memory saved: starting it is a resume, not a boot. */
  readonly suspended: boolean;
  /** The `.vmx` guest identifier; known before the machine is powered on. */
  readonly guestOs?: string;
  /** A lock file exists. While running that is normal; while stopped it blocks the next start. */
  readonly locked: boolean;
  /** What VMware Tools said: `running`, `installed`, `notInstalled`, `notRunning`. */
  readonly toolsState?: string;
  readonly ip?: string;
}

export type VmPowerAction = "start" | "stop" | "suspend";

/** A place on this machine an agent may name: the token, a display name, and nothing else. */
export type HostLocationKind = "file" | "folder";

export interface HostLocationView {
  readonly token: string;
  readonly name: string;
  readonly kind: HostLocationKind;
}
