/**
 * The only backend entry point feature code may use.
 *
 * Every call goes through {@link forward}, which answers a call made without a backend with a real
 * envelope instead of an undefined-function error, and normalizes anything the other side rejects
 * with. When a second transport arrives it is selected inside this module; callers never learn
 * which one answered.
 */

import { isDesktopRuntime } from "@/lib/runtime";
import type { AppInfo, AppSettings } from "@/types/app";
import type {
  KeyInfo,
  KnownHostEntry,
  OpenOutcome,
  OpenTerminal,
  OpenTunnel,
  PresentedHostKey,
  SecretInput,
  Session,
  SessionGroup,
  TerminalClosed,
  TerminalData,
  VaultState,
} from "@/types/session";

import { ShellError, normalizeShellError, noTransportError } from "./errors";
import * as desktop from "./tauri";
import type { Unlisten } from "./tauri";

async function forward<T>(operation: string, run: () => Promise<T>): Promise<T> {
  if (!isDesktopRuntime()) {
    throw noTransportError(operation);
  }
  try {
    return await run();
  } catch (error) {
    throw normalizeShellError(error, operation);
  }
}

/**
 * Subscriptions behave the same way in a browser as in the shell: there is simply never anything
 * to hear, and unsubscribing is still safe to call.
 */
function subscribe<T>(
  register: (listener: (payload: T) => void) => Promise<Unlisten>,
  listener: (payload: T) => void,
): Promise<Unlisten> {
  if (!isDesktopRuntime()) {
    return Promise.resolve(() => {});
  }
  return register(listener);
}

export const backend = {
  /** Name, version, platform and descriptor counts of the running application. */
  appInfo: (): Promise<AppInfo> => forward("app_info", () => desktop.appInfo()),

  /** Language and theme: the only things kept between runs so far, and never secrets. */
  settings: {
    get: (): Promise<AppSettings> => forward("settings_get", () => desktop.settingsGet()),
    set: (settings: AppSettings): Promise<void> => forward("settings_set", () => desktop.settingsSet(settings)),
  },

  /** The window the interface is drawn in. Meaningless without the desktop shell. */
  window: {
    minimize: (): Promise<void> => forward("window.minimize", () => desktop.windowMinimize()),
    toggleMaximize: (): Promise<void> => forward("window.toggleMaximize", () => desktop.windowToggleMaximize()),
    isMaximized: (): Promise<boolean> => forward("window.isMaximized", () => desktop.windowIsMaximized()),
    close: (): Promise<void> => forward("window.close", () => desktop.windowClose()),

    /**
     * Subscribes to window resizes so the maximize button can show the right glyph.
     * In a browser there is no window to watch, so there is nothing to unsubscribe from.
     */
    onResized: (listener: () => void): Promise<Unlisten> =>
      isDesktopRuntime() ? desktop.onWindowResized(listener) : Promise.resolve(() => {}),
  },

  vault: {
    state: (): Promise<VaultState> => forward("vault_state", () => desktop.vaultState()),
    create: (password: string): Promise<VaultState> => forward("vault_create", () => desktop.vaultCreate(password)),
    unlock: (password: string): Promise<VaultState> => forward("vault_unlock", () => desktop.vaultUnlock(password)),
    lock: (): Promise<VaultState> => forward("vault_lock", () => desktop.vaultLock()),
    changePassword: (current: string, next: string): Promise<void> =>
      forward("vault_change_password", () => desktop.vaultChangePassword(current, next)),
    /**
     * Moves a vault nobody can open beside itself, freeing the way to create a new one. Returns
     * the name the old file now has, which is the only thing a person needs to find it again.
     */
    setAside: (): Promise<string> => forward("vault_set_aside", () => desktop.vaultSetAside()),
  },

  sessions: {
    list: (): Promise<Session[]> => forward("sessions_list", () => desktop.sessionsList()),
    /** Absent `secret` means "leave the stored one alone", which is not the same as blank. */
    save: (session: Session, secret?: SecretInput): Promise<Session> =>
      forward("session_save", () => desktop.sessionSave(session, secret)),
    remove: (id: string): Promise<void> => forward("session_delete", () => desktop.sessionDelete(id)),

    groups: {
      list: (): Promise<SessionGroup[]> => forward("groups_list", () => desktop.groupsList()),
      save: (group: SessionGroup): Promise<SessionGroup> => forward("group_save", () => desktop.groupSave(group)),
      remove: (id: string): Promise<void> => forward("group_delete", () => desktop.groupDelete(id)),
    },
  },

  keys: {
    /** Opens a native dialog in Rust; resolves to `undefined` when the person dismissed it. */
    import: (title: string, passphrase?: string): Promise<KeyInfo | undefined> =>
      forward("key_import", () => desktop.keyImport(title, passphrase)),
    forget: (keyRef: string): Promise<void> => forward("key_forget", () => desktop.keyForget(keyRef)),
  },

  knownHosts: {
    list: (): Promise<KnownHostEntry[]> => forward("known_hosts_list", () => desktop.knownHostsList()),
    trust: (key: PresentedHostKey): Promise<void> => forward("known_host_trust", () => desktop.knownHostTrust(key)),
    forget: (host: string, port: number): Promise<void> =>
      forward("known_host_forget", () => desktop.knownHostForget(host, port)),
  },

  terminal: {
    /** May answer that a host key decision is needed first; nothing is sent until it is made. */
    open: (sessionId: string): Promise<OpenOutcome> => forward("session_open", () => desktop.sessionOpen(sessionId)),
    write: (handle: string, base64: string): Promise<void> =>
      forward("terminal_write", () => desktop.terminalWrite(handle, base64)),
    resize: (handle: string, columns: number, rows: number): Promise<void> =>
      forward("terminal_resize", () => desktop.terminalResize(handle, columns, rows)),
    close: (handle: string): Promise<void> => forward("terminal_close", () => desktop.terminalClose(handle)),
    openTerminals: (): Promise<OpenTerminal[]> => forward("terminals_open", () => desktop.terminalsOpen()),

    /** The forwards that are up, and how much they have carried. */
    tunnels: (): Promise<OpenTunnel[]> => forward("tunnels_open", () => desktop.tunnelsOpen()),
    closeTunnel: (id: string): Promise<void> => forward("tunnel_close", () => desktop.tunnelClose(id)),
  },

  events: {
    onTerminalData: (listener: (payload: TerminalData) => void): Promise<Unlisten> =>
      subscribe(desktop.onTerminalData, listener),
    onTerminalClosed: (listener: (payload: TerminalClosed) => void): Promise<Unlisten> =>
      subscribe(desktop.onTerminalClosed, listener),
  },
};

export { ShellError };
export type { Unlisten };
