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
import type { BatchOutcome, BatchProgress, BatchRequest, RiskAssessment } from "@/types/batch";
import type { HostMetrics, ProcessDetail, ProcessPage, SocketRow } from "@/types/monitor";
import type { CommandHistoryEntry, SendOutcome, SendTarget, Snippet, SnippetGroup } from "@/types/snippet";
import type {
  KeyInfo,
  KnownHostEntry,
  OpenOutcome,
  OpenTerminal,
  PresentedHostKey,
  SecretInput,
  OpenTunnel,
  Session,
  SessionExportSummary,
  SessionGroup,
  SessionImportSummary,
  SftpListing,
  SftpOutcome,
  TerminalClosed,
  TerminalData,
  TransferInfo,
  UploadAnswer,
  UploadOutcome,
  VaultState,
} from "@/types/session";

/** Removes a subscription made by one of the `on…` functions below. */
export type Unlisten = () => void;

/** What a drop or a paste left waiting, as the interface is told about it. */
export interface PendingFiles {
  readonly token: string;
  readonly names: readonly string[];
}

/** Whether a drag is hovering over the window. */
export interface DragPayload {
  readonly hovering: boolean;
}

/** A drop that landed: either files to upload, or why they cannot be. */
export interface DropPayload {
  readonly pending?: PendingFiles;
  readonly error?: unknown;
}

// -- Application -------------------------------------------------------------

export function appInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info");
}

/** Tells the shell there is a frame on screen. The window is hidden until it hears this. */
export function appReady(): Promise<void> {
  return invoke<void>("app_ready");
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

/**
 * Says whether closing the window would throw something away.
 *
 * Armed, a close request comes back as `quit-requested` instead of closing; disarmed, the window
 * closes on the first request. The shell keeps no opinion of its own — see the command's own note.
 */
export function windowQuitGuard(armed: boolean): Promise<void> {
  return invoke<void>("window_quit_guard", { armed });
}

/** Calls `listener` when a close was asked for while the guard was armed. */
export async function onQuitRequested(listener: () => void): Promise<Unlisten> {
  return listen("quit-requested", () => listener());
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

/**
 * Asks for a file to write the library into, in Rust, and writes it.
 *
 * `includeSecrets` is the only thing the interface decides: the passwords themselves are read out
 * of the vault and written by the other side, so they never arrive here. Resolves to `undefined`
 * when the dialog was dismissed.
 */
export function sessionsExport(
  title: string,
  fileName: string,
  includeSecrets: boolean,
): Promise<SessionExportSummary | undefined> {
  return invoke<SessionExportSummary | null>("sessions_export", { title, fileName, includeSecrets }).then(
    (summary) => summary ?? undefined,
  );
}

/**
 * Asks which file to read a library out of, in Rust, and adds what it holds.
 *
 * Resolves to `undefined` when the dialog was dismissed.
 */
export function sessionsImport(title: string): Promise<SessionImportSummary | undefined> {
  return invoke<SessionImportSummary | null>("sessions_import", { title }).then((summary) => summary ?? undefined);
}

// -- Keys --------------------------------------------------------------------

/** Opens a native file dialog, reads the chosen key and stores it. `undefined` means dismissed. */
export function keysList(): Promise<KeyInfo[]> {
  return invoke<KeyInfo[]>("keys_list");
}

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

export function tunnelsOpen(): Promise<OpenTunnel[]> {
  return invoke<OpenTunnel[]>("tunnels_open");
}

export function tunnelClose(id: string): Promise<void> {
  return invoke<void>("tunnel_close", { id });
}

// -- Host monitor ------------------------------------------------------------
//
// A sample rides a terminal's connection, so it is asked for by handle and not by session: there
// is nothing to sample for a session that has no terminal open.

export function monitorSample(handle: string): Promise<HostMetrics> {
  return invoke<HostMetrics>("monitor_sample", { handle });
}

/** The process table, cut on the far side to what was asked for. The page orders what it gets. */
export function monitorProcesses(handle: string, limit: number): Promise<ProcessPage> {
  return invoke<ProcessPage>("monitor_processes", { handle, limit });
}

/** One process in full — its command line, its directory and its environment. */
export function monitorProcessDetail(handle: string, pid: number): Promise<ProcessDetail> {
  return invoke<ProcessDetail>("monitor_process_detail", { handle, pid });
}

/** The host's sockets, for the network page: what is waiting on a port and who is on it. */
export function monitorSockets(handle: string): Promise<SocketRow[]> {
  return invoke<SocketRow[]>("monitor_sockets", { handle });
}

// -- Remote files ------------------------------------------------------------
//
// A download and an upload ask for a local path through a dialog that runs in Rust: the interface
// names neither, and receives the id of the transfer that does the work.

export function filesOpen(sessionId: string): Promise<SftpOutcome> {
  return invoke<SftpOutcome>("files_open", { sessionId });
}

export function filesList(handle: string, path: string): Promise<SftpListing> {
  return invoke<SftpListing>("files_list", { handle, path });
}

export function filesCreateFolder(handle: string, path: string): Promise<void> {
  return invoke<void>("files_create_folder", { handle, path });
}

/** Creates an empty file. Refused with `SFTP-EXISTS` when something with that name is already there. */
export function filesCreateFile(handle: string, path: string): Promise<void> {
  return invoke<void>("files_create_file", { handle, path });
}

export function filesRename(handle: string, from: string, to: string): Promise<void> {
  return invoke<void>("files_rename", { handle, from, to });
}

export function filesDelete(handle: string, path: string, kind: string): Promise<void> {
  return invoke<void>("files_delete", { handle, path, kind });
}

export function filesReadText(handle: string, path: string): Promise<string> {
  return invoke<string>("files_read_text", { handle, path });
}

export function filesWriteText(handle: string, path: string, text: string): Promise<void> {
  return invoke<void>("files_write_text", { handle, path, text });
}

export function filesClose(handle: string): Promise<void> {
  return invoke<void>("files_close", { handle });
}

/** The transfer ids, or none at all when the dialog was dismissed. */
export function filesDownload(
  handle: string,
  paths: readonly string[],
  title: string,
  suggestedName: string,
  packed: boolean,
): Promise<string[]> {
  return invoke<string[]>("files_download", { handle, paths, title, suggestedName, packed });
}

/**
 * Asks which file to send, in Rust. The answer is what the upload did: running transfers, or the
 * question it stopped on when a name it would write is already there.
 */
export function filesUpload(handle: string, directory: string, title: string, packed: boolean): Promise<UploadOutcome> {
  return invoke<UploadOutcome>("files_upload", { handle, directory, title, packed });
}

/** The answer to that question. The files that were waiting are sent, or forgotten. */
export function filesUploadAnswer(token: string, answer: UploadAnswer): Promise<UploadOutcome> {
  return invoke<UploadOutcome>("files_upload_answer", { token, answer });
}

export function transfersOpen(): Promise<TransferInfo[]> {
  return invoke<TransferInfo[]>("transfers_open");
}

/** Uploads the files a drop or a paste left behind, by token. */
export function droppedUpload(
  handle: string,
  directory: string,
  token: string,
  packed: boolean,
): Promise<UploadOutcome> {
  return invoke<UploadOutcome>("dropped_upload", { handle, directory, token, packed });
}

/** The file list on the clipboard, as a token and names. Refused on platforms that cannot read it. */
export function clipboardFiles(): Promise<PendingFiles> {
  return invoke<PendingFiles>("clipboard_files");
}

export function transferCancel(id: string): Promise<void> {
  return invoke<void>("transfer_cancel", { id });
}

// -- Clipboard ---------------------------------------------------------------

/**
 * The text on the clipboard, or an empty string when it holds none.
 *
 * Read in Rust rather than through `navigator.clipboard`: WebView2 answers a script's read with a
 * permission request that nothing is waiting to answer, and the promise never settles — a paste that
 * went that way would hang with nothing to show. `APP-UNSUPPORTED` means the platform cannot read
 * one at all.
 */
export function clipboardText(): Promise<string> {
  return invoke<string>("clipboard_text");
}

// -- Batches -----------------------------------------------------------------

/** Runs one command across the hosts in the request, with progress arriving on `batch:event`. */
export function batchRun(request: BatchRequest): Promise<BatchOutcome> {
  return invoke<BatchOutcome>("batch_run", { request });
}

// -- Saved commands ----------------------------------------------------------

export function snippetsOpen(): Promise<SnippetGroup[]> {
  return invoke<SnippetGroup[]>("snippets_open");
}

export function snippetGroupSave(group: SnippetGroup): Promise<SnippetGroup> {
  return invoke<SnippetGroup>("snippet_group_save", { group });
}

export function snippetGroupDelete(id: string): Promise<void> {
  return invoke<void>("snippet_group_delete", { id });
}

export function snippetSave(snippet: Snippet): Promise<Snippet> {
  return invoke<Snippet>("snippet_save", { snippet });
}

export function snippetDelete(id: string): Promise<void> {
  return invoke<void>("snippet_delete", { id });
}

export function commandHistory(): Promise<CommandHistoryEntry[]> {
  return invoke<CommandHistoryEntry[]>("command_history");
}

export function commandHistoryClear(): Promise<void> {
  return invoke<void>("command_history_clear");
}

/** The verdict for a command, without sending anything. */
export function commandClassify(command: string): Promise<RiskAssessment> {
  return invoke<RiskAssessment>("command_classify", { command });
}

/** Writes a command into the terminals in the target. */
export function commandSend(
  target: SendTarget,
  command: string,
  appendCr: boolean,
  acknowledged: boolean,
): Promise<SendOutcome> {
  return invoke<SendOutcome>("command_send", { target, command, appendCr, acknowledged });
}

// -- Events ------------------------------------------------------------------

export async function onBatchEvent(listener: (payload: BatchProgress) => void): Promise<Unlisten> {
  return listen<BatchProgress>("batch:event", (event) => listener(event.payload));
}

export async function onTerminalData(listener: (payload: TerminalData) => void): Promise<Unlisten> {
  return listen<TerminalData>("terminal:data", (event) => listener(event.payload));
}

export async function onTerminalClosed(listener: (payload: TerminalClosed) => void): Promise<Unlisten> {
  return listen<TerminalClosed>("terminal:closed", (event) => listener(event.payload));
}

export async function onFilesDrag(listener: (payload: DragPayload) => void): Promise<Unlisten> {
  return listen<DragPayload>("files:drag", (event) => listener(event.payload));
}

export async function onFilesDropped(listener: (payload: DropPayload) => void): Promise<Unlisten> {
  return listen<DropPayload>("files:dropped", (event) => listener(event.payload));
}
