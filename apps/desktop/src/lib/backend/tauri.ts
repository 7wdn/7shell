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
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";

import type { AppInfo, AppSettings } from "@/types/app";
import type { BatchOutcome, BatchProgress, BatchRequest, RiskAssessment } from "@/types/batch";
import type { HostMetrics, ProcessDetail, ProcessPage, SocketRow } from "@/types/monitor";
import type { HostSystem } from "@/types/system";
import type { UpdateCheck, UpdateProgress } from "@/types/update";
import type {
  CommandHistoryEntry,
  CompletionWord,
  SendOutcome,
  SendTarget,
  Snippet,
  SnippetGroup,
} from "@/types/snippet";
import type {
  CreatedKey,
  DownloadFolderView,
  KeyAlgorithm,
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
  SessionImportOutcome,
  SftpListing,
  SftpOutcome,
  TerminalClosed,
  TerminalData,
  TransferInfo,
  UploadAnswer,
  UploadOutcome,
  VaultState,
} from "@/types/session";
import type {
  SnippetStatus,
  SnippetUploadReceipt,
  SyncApplySummary,
  SyncContents,
  SyncDownloadOutcome,
  SyncInspectOutcome,
  SyncReceipt,
  SyncSelection,
  SyncServiceStatus,
  SyncUploadReceipt,
} from "@/types/sync";

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

// -- Updates -----------------------------------------------------------------

/**
 * Asks the update channel whether a newer release is published.
 *
 * The answer is the whole result: a failure to reach the channel comes back as a `ShellError`, which
 * is what lets the page say "could not check" rather than "you are up to date" — two states that
 * look identical and mean opposite things.
 */
export function updateCheck(): Promise<UpdateCheck> {
  return invoke<UpdateCheck>("update_check");
}

/**
 * Downloads and installs the release the last check found, then restarts into it.
 *
 * The call does not come back on Windows — the installer takes the process over — and on macOS and
 * Linux the application is gone before a caller could read a result. Treating it as an ordinary
 * awaitable that returns would be the interface drawing a page into a window that is being replaced.
 */
export function updateInstall(): Promise<void> {
  return invoke<void>("update_install");
}

export async function onUpdateProgress(listener: (payload: UpdateProgress) => void): Promise<Unlisten> {
  return listen<UpdateProgress>("update:progress", (event) => listener(event.payload));
}

// -- Settings ----------------------------------------------------------------

export function settingsGet(): Promise<AppSettings> {
  return invoke<AppSettings>("settings_get");
}

export function settingsSet(settings: AppSettings): Promise<void> {
  return invoke<void>("settings_set", { settings });
}

/** Every font family this machine has, for the two settings that choose one. */
/**
 * Opens the system's file picker and copies what comes back into this installation's own directory.
 *
 * The title is passed in for the same reason every dialog title in this build is: it is a word
 * somebody reads, and the words live with the interface's translations. `null` is somebody closing the
 * picker, which is not a failure and not a change.
 */
export function backgroundImageChoose(title: string): Promise<string | null> {
  return invoke<string | null>("background_image_choose", { title });
}

/** The stored picture, base64-encoded. Takes no path: the interface does not know one. */
export function backgroundImageRead(): Promise<string> {
  return invoke<string>("background_image_read");
}

/** Takes the stored picture away. */
export function backgroundImageClear(): Promise<void> {
  return invoke<void>("background_image_clear");
}

export function systemFontsList(): Promise<string[]> {
  return invoke<string[]>("system_fonts_list");
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
/**
 * Closes the window for real, on the interface's word that the close question is answered.
 *
 * Not the same as `windowClose` above it: that one *asks* — the shell may put the window away, or ask
 * about unsaved work — while this one is the answer to what it asked.
 */
export function windowQuit(): Promise<void> {
  return invoke<void>("window_quit");
}

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

/**
 * Draws the interface at `scale` times its own size, inside a window of the same size.
 *
 * The webview's business rather than the document's: a CSS transform or a root font size would
 * leave the terminal's canvas, the scrollbars and the hit testing behind, each in its own way. The
 * zoom factor moves all of it together, which is what "the interface is bigger" has to mean.
 */
export function webviewSetZoom(scale: number): Promise<void> {
  return getCurrentWebview().setZoom(scale);
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
 * `passphrase` is the only thing the interface decides about the credentials, and there is no value
 * of it that writes one in the clear: with a passphrase the file carries the stored logins sealed,
 * without one it carries none. The passwords themselves are read out of the vault and written by the
 * other side, so they never arrive here. Resolves to `undefined` when the dialog was dismissed.
 */
export function sessionsExport(
  title: string,
  fileName: string,
  passphrase: string | null,
): Promise<SessionExportSummary | undefined> {
  return invoke<SessionExportSummary | null>("sessions_export", { title, fileName, passphrase }).then(
    (summary) => summary ?? undefined,
  );
}

/**
 * Asks which file to read a library out of, in Rust, and adds what it holds.
 *
 * `passphrase` is null on the first call, which either imports what it read or answers that the file
 * is sealed and is now waiting. The second call carries the answer and reads that same file again —
 * the shell remembers which one, so no dialog re-opens. Resolves to `undefined` when the file dialog
 * was dismissed.
 */
export function sessionsImport(title: string, passphrase: string | null): Promise<SessionImportOutcome | undefined> {
  return invoke<SessionImportOutcome | null>("sessions_import", { title, passphrase }).then(
    (outcome) => outcome ?? undefined,
  );
}

// -- Sync and backups --------------------------------------------------------

/** What this machine could put into a backup, as counts. */
export function syncAvailable(): Promise<SyncContents> {
  return invoke<SyncContents>("sync_available");
}

/**
 * Writes a backup where the user says, with what they chose and under the passphrase they gave.
 *
 * `passphrase` is the whole of what the interface decides about credentials: with one the file
 * carries the stored logins and key material sealed, without one it carries neither. The values
 * themselves are read out of the vault and written by the other side. `undefined` means the file
 * dialog was dismissed.
 */
export function syncBackupExport(
  title: string,
  fileName: string,
  selection: SyncSelection,
  passphrase: string | null,
): Promise<SyncReceipt | undefined> {
  return invoke<SyncReceipt | null>("sync_backup_export", { title, fileName, selection, passphrase }).then(
    (receipt) => receipt ?? undefined,
  );
}

/**
 * Asks which file to read a backup out of, and says what it holds.
 *
 * `passphrase` is null on the first call, which either reads the file or answers that it is sealed
 * and is now waiting; the second call carries the answer and reads that same file again, because
 * the shell remembers which one it was. Resolves to `undefined` when the file dialog was dismissed.
 */
export function syncBackupInspect(title: string, passphrase: string | null): Promise<SyncInspectOutcome | undefined> {
  return invoke<SyncInspectOutcome | null>("sync_backup_inspect", { title, passphrase }).then(
    (outcome) => outcome ?? undefined,
  );
}

/** Applies the backup that was read, with the passphrase it was read under. */
export function syncBackupRestore(replace: boolean, passphrase: string | null): Promise<SyncApplySummary> {
  return invoke<SyncApplySummary>("sync_backup_restore", { replace, passphrase });
}

// -- The sync server ---------------------------------------------------------

/** What this installation has for a server: whether a password and a passphrase are kept. */
export function syncStatus(): Promise<SyncServiceStatus> {
  return invoke<SyncServiceStatus>("sync_status");
}

/** Keeps the server's password, or forgets it. `null` is forgetting, not "leave it alone". */
export function syncPassword(password: string | null): Promise<SyncServiceStatus> {
  return invoke<SyncServiceStatus>("sync_password", { password });
}

/** Keeps the passphrase an upload is sealed under, or forgets it. */
export function syncPassphrase(passphrase: string | null): Promise<SyncServiceStatus> {
  return invoke<SyncServiceStatus>("sync_passphrase", { passphrase });
}

/** Asks the configured server whether it is a WebDAV one. Nothing is written. */
export function syncTest(): Promise<void> {
  return invoke<void>("sync_test");
}

/** Writes the library to the server. Resolves with what went and where it went. */
export function syncUpload(selection: SyncSelection, passphrase: string | null): Promise<SyncUploadReceipt> {
  return invoke<SyncUploadReceipt>("sync_upload", { selection, passphrase });
}

/** Reads the file the server holds and says what is inside it. */
export function syncDownload(passphrase: string | null): Promise<SyncDownloadOutcome> {
  return invoke<SyncDownloadOutcome>("sync_download", { passphrase });
}

/** Applies the file the server holds, fetching it again. */
export function syncApply(replace: boolean, passphrase: string | null): Promise<SyncApplySummary> {
  return invoke<SyncApplySummary>("sync_apply", { replace, passphrase });
}

// -- The snippet -----------------------------------------------------------------

/**
 * What this installation has for the account its settings describe.
 *
 * The shell reads the settings file, so the caller commits what is on screen first — the same rule
 * every server action on that page follows, and the reason the answer is about the account somebody
 * is looking at rather than the one that was chosen last time.
 */
export function snippetSyncStatus(): Promise<SnippetStatus> {
  return invoke<SnippetStatus>("snippet_sync_status");
}

/** Keeps the token for that account, or forgets the one that was kept. `null` is forgetting. */
export function snippetSyncToken(token: string | null): Promise<SnippetStatus> {
  return invoke<SnippetStatus>("snippet_sync_token", { token });
}

/** Asks the service whether the token, and the snippet when there is one, is good. */
export function snippetSyncTest(): Promise<void> {
  return invoke<void>("snippet_sync_test");
}

/** Writes the library into the gist or snippet, creating one when there is none. */
export function snippetSyncUpload(selection: SyncSelection, passphrase: string | null): Promise<SnippetUploadReceipt> {
  return invoke<SnippetUploadReceipt>("snippet_sync_upload", { selection, passphrase });
}

/** Reads the file the account holds and says what is inside it. */
export function snippetSyncDownload(passphrase: string | null): Promise<SyncDownloadOutcome> {
  return invoke<SyncDownloadOutcome>("snippet_sync_download", { passphrase });
}

/** Applies the file the account holds, fetching it again. */
export function snippetSyncApply(replace: boolean, passphrase: string | null): Promise<SyncApplySummary> {
  return invoke<SyncApplySummary>("snippet_sync_apply", { replace, passphrase });
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

/**
 * Makes a key and keeps it in the vault, answering with the line that goes on a server.
 *
 * The private half does not come back: this application is what uses it, and the public line is the
 * only key-shaped thing that belongs in the interface.
 */
export function keyGenerate(algorithm: KeyAlgorithm, comment: string, passphrase: string | null): Promise<CreatedKey> {
  return invoke<CreatedKey>("key_generate", { algorithm, comment, passphrase });
}

/** The `authorized_keys` line for a key the vault holds. */
export function keyPublic(keyRef: string): Promise<string> {
  return invoke<string>("key_public", { keyRef });
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

/**
 * Calls off an open that is still going. Answers whether there was one to call off, which is what
 * the interface says out loud — a click that stopped nothing must not read as though it did.
 */
export function sessionOpenCancel(sessionId: string): Promise<boolean> {
  return invoke<boolean>("session_open_cancel", { sessionId });
}

/**
 * What the host behind a terminal says it is.
 *
 * A question of its own rather than a field on the open: it costs a round trip, and the open is
 * answered the moment it can be. Asked by handle, because it rides the connection that is up.
 */
export function sessionSystem(handle: string): Promise<HostSystem> {
  return invoke<HostSystem>("session_system", { handle });
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

/** Asks one transfer to stop. The row stays, and goes on saying it was cancelled. */
export function transferStop(id: string): Promise<void> {
  return invoke<void>("transfer_stop", { id });
}

/** Takes one transfer off the list, stopping it first if it is still moving. */
export function transferRemove(id: string): Promise<void> {
  return invoke<void>("transfer_remove", { id });
}

/** Asks everything that is still moving to stop, leaving every row to say so. */
export function transferStopAll(): Promise<void> {
  return invoke<void>("transfer_stop_all");
}

/** Takes the transfers that have stopped off the list, and leaves what is moving alone. */
export function transferClearSettled(): Promise<void> {
  return invoke<void>("transfer_clear_settled");
}

/**
 * Opens one transfer's local end, by the row it is about.
 *
 * The path stays on the other side: it was chosen in a native dialog and the interface has never
 * held it. Only a finished transfer is opened — the other side checks that itself, so a click that
 * arrives late does nothing rather than handing a half-written file to an application.
 */
export function transferOpen(id: string): Promise<void> {
  return invoke<void>("transfer_open", { id });
}

/** Shows where one transfer's local end is, which is how the part file of a stopped one is found. */
export function transferReveal(id: string): Promise<void> {
  return invoke<void>("transfer_reveal", { id });
}

/** Where downloads land: the folder's own name, and the names of the folders used before. */
export function downloadFolderGet(): Promise<DownloadFolderView> {
  return invoke<DownloadFolderView>("download_folder_get");
}

/**
 * Asks which folder downloads should land in, and remembers it.
 *
 * The dialog runs in Rust, so the answer is a name rather than a path — and `null` is somebody
 * closing it, which is not a failure and not a change.
 */
export function downloadFolderChoose(title: string): Promise<DownloadFolderView | undefined> {
  return invoke<DownloadFolderView | null>("download_folder_choose", { title }).then((view) => view ?? undefined);
}

/** Makes one of the remembered folders the one downloads land in. */
export function downloadFolderSelect(index: number): Promise<DownloadFolderView> {
  return invoke<DownloadFolderView>("download_folder_select", { index });
}

/** Stops using a fixed folder, so every download asks again. What was used stays in the list. */
export function downloadFolderAsk(): Promise<DownloadFolderView> {
  return invoke<DownloadFolderView>("download_folder_ask");
}

/** Shows where downloads land, in the system's file manager. */
export function downloadFolderReveal(): Promise<void> {
  return invoke<void>("download_folder_reveal");
}

/** Holds a transfer where it is, keeping its connection and what it has already copied. */
export function transferPause(id: string): Promise<void> {
  return invoke<void>("transfer_pause", { id });
}

/** Lets a paused transfer go on. */
export function transferResume(id: string): Promise<void> {
  return invoke<void>("transfer_resume", { id });
}

/** Runs a transfer that stopped, from wherever it stopped. */
export function transferRetry(id: string): Promise<void> {
  return invoke<void>("transfer_retry", { id });
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

export function completionWords(): Promise<CompletionWord[]> {
  return invoke<CompletionWord[]>("completion_words");
}

export function completionWordSave(word: CompletionWord): Promise<CompletionWord> {
  return invoke<CompletionWord>("completion_word_save", { word });
}

export function completionWordDelete(id: string): Promise<void> {
  return invoke<void>("completion_word_delete", { id });
}

export function completionWordsClear(): Promise<void> {
  return invoke<void>("completion_words_clear");
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
