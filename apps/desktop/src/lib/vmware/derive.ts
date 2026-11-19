/**
 * The saved sessions a VM workspace makes to reach the guest.
 *
 * The workspace does not open an ad-hoc connection: a guest is reached exactly like any other
 * machine, through a saved session, so credentials, keys and routes live where they always live.
 * These two builders create that session the first time and rewrite its address every time, which
 * is what keeps a DHCP guest reachable across restarts — the address is resolved before each open
 * and never copied into the VM session itself.
 */
import type { DesktopSettings, Session, TerminalSettings } from "@/types/session";
import type { VmInfo } from "@/types/vmware";

/**
 * What a stored session of another protocol defaults to.
 *
 * Duplicated from `sessionEditor` because the editor is a form and this is a builder: importing
 * the store here would drag a Pinia instance into a pure function.
 */
const TERMINAL: TerminalSettings = {
  terminalType: "xterm-256color",
  keepaliveSeconds: 30,
  encoding: "utf-8",
  backspace: "del",
  forwardAgent: false,
  record: false,
};

const DESKTOP: DesktopSettings = {
  width: 1280,
  height: 1024,
  fullscreen: false,
  redirectDrives: true,
};

export type VmGuestKind = "ssh" | "rdp";

/**
 * The session an import makes for one scanned machine.
 *
 * It carries the identity and the two ports only; the account arrives when somebody opens the
 * workspace and says which one to use. Nothing about the guest's address is written down.
 */
export function vmSessionFromScan(vm: VmInfo, groupId?: string): Session {
  return {
    id: crypto.randomUUID(),
    name: vm.name,
    groupId,
    protocol: "vmware",
    host: "",
    port: 0,
    user: "",
    auth: { method: "password", hasSecret: false },
    terminal: { ...TERMINAL },
    vmware: { vmId: vm.id, sshPort: 22, rdpPort: 3389 },
    updatedAt: Math.floor(Date.now() / 1000),
  };
}

/** What the workspace calls the session it made. A name, not an identity. */
export function vmGuestName(vmName: string, kind: VmGuestKind): string {
  return `${vmName} · ${kind === "ssh" ? "SSH" : "RDP"}`;
}

/**
 * The session that reaches this VM's guest at `host`, or the same session with its address and
 * account brought up to date.
 *
 * `existing` is the session the VM session references, when it still exists. Everything the person
 * may have changed on it — its name, its group, its stored credential, its terminal settings — is
 * kept; only the address, the port and the account follow the VM's own settings.
 */
export function vmGuestSession(vm: Session, kind: VmGuestKind, host: string, existing?: Session): Session {
  const settings = vm.vmware;
  const ssh = kind === "ssh";
  return {
    id: existing?.id ?? crypto.randomUUID(),
    name: existing?.name ?? vmGuestName(vm.name, kind),
    groupId: existing?.groupId ?? vm.groupId,
    protocol: ssh ? "ssh" : "rdp",
    host,
    port: (ssh ? settings?.sshPort : settings?.rdpPort) ?? (ssh ? 22 : 3389),
    user: (ssh ? settings?.sshUser : settings?.rdpUser)?.trim() ?? "",
    auth: existing?.auth ?? { method: "password", hasSecret: false },
    terminal: existing?.terminal ?? { ...TERMINAL },
    desktop: ssh ? existing?.desktop : (existing?.desktop ?? { ...DESKTOP }),
    updatedAt: Math.floor(Date.now() / 1000),
  };
}
