/**
 * Typed wrappers over the Rust command surface and the window the interface is drawn in:
 * one function per `#[tauri::command]`, plus the event subscriptions the terminal needs.
 *
 * Nothing outside this folder imports from `@tauri-apps/*` —
 * `scripts/tests/frontendBoundary.test.mjs` enforces that, because a second transport (a local
 * HTTP backend, for instance) must be addable without hunting the whole code base for
 * transport-specific calls.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import type { AppInfo, AppSettings } from "@/types/app";
import type {
  KeyInfo,
  KnownHostEntry,
  OpenOutcome,
  OpenTerminal,
  PresentedHostKey,
  SecretInput,
  Session,
  SessionGroup,
  TerminalClosed,
  TerminalData,
  VaultState,
} from "@/types/session";

/** Removes a subscription made by one of the `on…` functions below. */
export type Unlisten = () => void;

// -- Application -------------------------------------------------------------

export function appInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info");
}

// -- Settings ----------------------------------------------------------------

export function settingsGet(): Promise<AppSettings> {
  return invoke<AppSettings>("settings_get");
}

export function settingsSet(settings: AppSettings): Promise<void> {
  return invoke<void>("settings_set", { settings });
}

// -- Window ------------------------------------------------------------------

export function windowMinimize(): Promise<void> {
  return getCurrentWindow().minimize();
}

export function windowToggleMaximize(): Promise<void> {
  return getCurrentWindow().toggleMaximize();
}

export function windowIsMaximized(): Promise<boolean> {
  return getCurrentWindow().isMaximized();
}

export function windowClose(): Promise<void> {
  return getCurrentWindow().close();
}

/** Calls `listener` whenever the window is resized, and resolves with the unsubscribe. */
export async function onWindowResized(listener: () => void): Promise<Unlisten> {
  return getCurrentWindow().onResized(() => listener());
}

// -- Vault -------------------------------------------------------------------
//
// Passwords travel inwards only. Nothing here returns one, and nothing here can read one back.

export function vaultState(): Promise<VaultState> {
  return invoke<VaultState>("vault_state");
}

export function vaultCreate(password: string): Promise<VaultState> {
  return invoke<VaultState>("vault_create", { password });
}

export function vaultUnlock(password: string): Promise<VaultState> {
  return invoke<VaultState>("vault_unlock", { password });
}

export function vaultLock(): Promise<VaultState> {
  return invoke<VaultState>("vault_lock");
}

export function vaultChangePassword(current: string, next: string): Promise<void> {
  return invoke<void>("vault_change_password", { current, next });
}

/** Resolves to the name the unopenable vault file was given. */
export function vaultSetAside(): Promise<string> {
  return invoke<string>("vault_set_aside");
}

// -- Sessions and groups -----------------------------------------------------

export function sessionsList(): Promise<Session[]> {
  return invoke<Session[]>("sessions_list");
}

/** Saves a session. `secret` absent means "leave the stored one alone". */
export function sessionSave(session: Session, secret?: SecretInput): Promise<Session> {
  return invoke<Session>("session_save", { session, secret: secret ?? null });
}

export function sessionDelete(id: string): Promise<void> {
  return invoke<void>("session_delete", { id });
}

export function groupsList(): Promise<SessionGroup[]> {
  return invoke<SessionGroup[]>("groups_list");
}

export function groupSave(group: SessionGroup): Promise<SessionGroup> {
  return invoke<SessionGroup>("group_save", { group });
}

export function groupDelete(id: string): Promise<void> {
  return invoke<void>("group_delete", { id });
}

// -- Keys --------------------------------------------------------------------

/** Opens a native file dialog, reads the chosen key and stores it. `undefined` means dismissed. */
export function keyImport(title: string, passphrase?: string): Promise<KeyInfo | undefined> {
  return invoke<KeyInfo | null>("key_import", { title, passphrase: passphrase ?? null }).then(
    (info) => info ?? undefined,
  );
}

export function keyForget(keyRef: string): Promise<void> {
  return invoke<void>("key_forget", { keyRef });
}

// -- Host keys ---------------------------------------------------------------

export function knownHostsList(): Promise<KnownHostEntry[]> {
  return invoke<KnownHostEntry[]>("known_hosts_list");
}

export function knownHostTrust(key: PresentedHostKey): Promise<void> {
  return invoke<void>("known_host_trust", { key });
}

export function knownHostForget(host: string, port: number): Promise<void> {
  return invoke<void>("known_host_forget", { host, port });
}

// -- Terminals ---------------------------------------------------------------

export function sessionOpen(sessionId: string): Promise<OpenOutcome> {
  return invoke<OpenOutcome>("session_open", { sessionId });
}

export function terminalWrite(handle: string, base64: string): Promise<void> {
  return invoke<void>("terminal_write", { handle, base64 });
}

export function terminalResize(handle: string, columns: number, rows: number): Promise<void> {
  return invoke<void>("terminal_resize", { handle, columns, rows });
}

export function terminalClose(handle: string): Promise<void> {
  return invoke<void>("terminal_close", { handle });
}

export function terminalsOpen(): Promise<OpenTerminal[]> {
  return invoke<OpenTerminal[]>("terminals_open");
}

// -- Events ------------------------------------------------------------------

export async function onTerminalData(listener: (payload: TerminalData) => void): Promise<Unlisten> {
  return listen<TerminalData>("terminal:data", (event) => listener(event.payload));
}

export async function onTerminalClosed(listener: (payload: TerminalClosed) => void): Promise<Unlisten> {
  return listen<TerminalClosed>("terminal:closed", (event) => listener(event.payload));
}
