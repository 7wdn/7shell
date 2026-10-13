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
import type { BatchOutcome, BatchProgress, BatchRequest, RiskAssessment } from "@/types/batch";
import type {
  CommandHistoryEntry,
  CompletionWord,
  SendOutcome,
  SendTarget,
  Snippet,
  SnippetGroup,
} from "@/types/snippet";
import type { HostMetrics, ProcessDetail, ProcessPage, SocketRow } from "@/types/monitor";
import type { HostSystem } from "@/types/system";
import type { UpdateCheck, UpdateProgress } from "@/types/update";
import type {
  CreatedKey,
  DownloadFolderView,
  KeyAlgorithm,
  KeyInfo,
  KnownHostEntry,
  OpenOutcome,
  OpenTerminal,
  OpenTunnel,
  PresentedHostKey,
  SftpListing,
  SftpOutcome,
  TransferInfo,
  SecretInput,
  Session,
  SessionExportSummary,
  SessionGroup,
  SessionImportOutcome,
  SessionImportSummary,
  TerminalClosed,
  TerminalData,
  UploadAnswer,
  UploadOutcome,
  VaultArchive,
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

import { ShellError, normalizeShellError, noTransportError } from "./errors";
import * as desktop from "./tauri";
import type { DragPayload, DropPayload, PendingFiles, Unlisten } from "./tauri";

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

  /**
   * Reports that the interface has drawn its first frame, which is when the shell shows the window.
   * The shell shows it on its own deadline when this never arrives, so it is not a precondition for
   * the window existing — only for its first appearance being the interface rather than an empty
   * webview.
   */
  appReady: (): Promise<void> => forward("app_ready", () => desktop.appReady()),

  /**
   * What the operating system says the application appearance is, or null where this shell has no
   * answer: one of the platforms whose webview follows the system by itself, or a system that could
   * not be read. The interface falls back to its own media query there.
   */
  systemTheme: (): Promise<"dark" | "light" | null> => forward("systemTheme", () => desktop.systemTheme()),

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
    isAlwaysOnTop: (): Promise<boolean> => forward("window.isAlwaysOnTop", () => desktop.windowIsAlwaysOnTop()),
    setAlwaysOnTop: (alwaysOnTop: boolean): Promise<void> =>
      forward("window.setAlwaysOnTop", () => desktop.windowSetAlwaysOnTop(alwaysOnTop)),

    /**
     * Applies the appearance to what the interface does not draw: the window's own chrome and the
     * platform's menus. Sent when the mode is chosen, not when the settings are written.
     */
    applyTheme: (themeMode: string): Promise<void> =>
      forward("window.applyTheme", () => desktop.windowThemeApply(themeMode)),

    /**
     * Draws the interface at `scale` times its own size, inside a window of the same size.
     *
     * Answered rather than refused where there is no shell: a browser tab is drawn by the browser
     * and has its own zoom, so "there is nothing here to scale" is the whole truth there and not a
     * failure the settings page should report.
     */
    setZoom: (scale: number): Promise<void> =>
      isDesktopRuntime() ? forward("webview.setZoom", () => desktop.webviewSetZoom(scale)) : Promise.resolve(),

    /**
     * Says whether closing would throw something away. Armed, the close comes back as
     * `onQuitRequested` instead of closing the window.
     */
    quitGuard: (armed: boolean): Promise<void> => forward("window.quitGuard", () => desktop.windowQuitGuard(armed)),
    /** Closes the window for real, on the interface's word that the close question is answered. */
    quit: (): Promise<void> => forward("window.quit", () => desktop.windowQuit()),

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
    /** The archived libraries beside the live one, most recently written first. */
    archives: (): Promise<VaultArchive[]> => forward("vault_archives", () => desktop.vaultArchives()),
    /**
     * Whether a password opens one archive, without making it the live one.
     *
     * A refusal arrives as the ordinary `VAULT-UNLOCK-FAILED`, which is what the field it was typed
     * in shows: trying a password decides nothing, and the live vault is untouched either way.
     */
    verifyArchive: (name: string, password: string): Promise<void> =>
      forward("vault_archive_verify", () => desktop.vaultArchiveVerify(name, password)),
    /**
     * Makes an archive the live vault, archiving the library that was live. Returns the name the
     * replaced library was given, or `null` when there was no file in place.
     */
    activateArchive: (name: string): Promise<string | null> =>
      forward("vault_archive_activate", () => desktop.vaultArchiveActivate(name)),
    /** Adds an archive's sessions, groups and logins to the open vault, and says how many. */
    importArchive: (name: string, password: string): Promise<SessionImportSummary> =>
      forward("vault_archive_import", () => desktop.vaultArchiveImport(name, password)),
    /** Opens the folder the vault and its archives live in. The interface names no path. */
    revealFolder: (): Promise<void> => forward("vault_reveal_folder", () => desktop.vaultRevealFolder()),
  },

  /** Remote files, and the transfers that move them. */
  files: {
    open: (sessionId: string): Promise<SftpOutcome> => forward("files_open", () => desktop.filesOpen(sessionId)),
    list: (handle: string, path: string): Promise<SftpListing> =>
      forward("files_list", () => desktop.filesList(handle, path)),
    createFolder: (handle: string, path: string): Promise<void> =>
      forward("files_create_folder", () => desktop.filesCreateFolder(handle, path)),
    /** An empty file, and a name that is already taken is refused rather than emptied. */
    createFile: (handle: string, path: string): Promise<void> =>
      forward("files_create_file", () => desktop.filesCreateFile(handle, path)),
    rename: (handle: string, from: string, to: string): Promise<void> =>
      forward("files_rename", () => desktop.filesRename(handle, from, to)),
    remove: (handle: string, path: string, kind: string): Promise<void> =>
      forward("files_delete", () => desktop.filesDelete(handle, path, kind)),
    readText: (handle: string, path: string): Promise<string> =>
      forward("files_read_text", () => desktop.filesReadText(handle, path)),
    writeText: (handle: string, path: string, text: string): Promise<void> =>
      forward("files_write_text", () => desktop.filesWriteText(handle, path, text)),
    close: (handle: string): Promise<void> => forward("files_close", () => desktop.filesClose(handle)),
    /** Uploads what a drop or a paste left behind. The interface never sees a path. */
    uploadDropped: (handle: string, directory: string, token: string, packed: boolean): Promise<UploadOutcome> =>
      forward("dropped_upload", () => desktop.droppedUpload(handle, directory, token, packed)),
    /** Refused with `APP-UNSUPPORTED` where the platform cannot read a file list from it. */
    clipboardFiles: (): Promise<PendingFiles> => forward("clipboard_files", () => desktop.clipboardFiles()),
    /**
     * Asks where to put it, in Rust. An empty answer means a dialog was dismissed.
     *
     * Which dialog that is depends on what was asked for: one file is asked for by name, several
     * rows are asked for as a folder to put them in, and a packed selection as a folder to open the
     * archive into. The question belongs on the side that can ask it.
     */
    download: (
      handle: string,
      paths: readonly string[],
      title: string,
      suggestedName: string,
      packed: boolean,
    ): Promise<string[]> =>
      forward("files_download", () => desktop.filesDownload(handle, paths, title, suggestedName, packed)),
    /**
     * Asks which file to send, in Rust, and says what became of it: transfers that are running, or
     * the question it stopped on when a name it would write is already there.
     */
    upload: (handle: string, directory: string, title: string, packed: boolean): Promise<UploadOutcome> =>
      forward("files_upload", () => desktop.filesUpload(handle, directory, title, packed)),
    /** The answer to that question: send them anyway, send only what is missing, or send nothing. */
    uploadAnswer: (token: string, answer: UploadAnswer): Promise<UploadOutcome> =>
      forward("files_upload_answer", () => desktop.filesUploadAnswer(token, answer)),
  },

  transfers: {
    list: (): Promise<TransferInfo[]> => forward("transfers_open", () => desktop.transfersOpen()),

    /**
     * Asks one transfer to stop. The row stays and says it was cancelled.
     *
     * Not the same as removing it, and the difference is the point: stopping is a fact about the
     * bytes, and a row that disappeared the moment somebody asked would leave them unable to tell a
     * cancellation from a failure they slept through.
     */
    stop: (id: string): Promise<void> => forward("transfer_stop", () => desktop.transferStop(id)),
    /** Takes one off the list, stopping it first if it is still moving. */
    remove: (id: string): Promise<void> => forward("transfer_remove", () => desktop.transferRemove(id)),
    /** Asks everything that is still moving to stop, leaving every row to say so. */
    stopAll: (): Promise<void> => forward("transfer_stop_all", () => desktop.transferStopAll()),
    /** Takes the transfers that have stopped off the list, and leaves what is moving alone. */
    clearSettled: (): Promise<void> => forward("transfer_clear_settled", () => desktop.transferClearSettled()),
    /** Opens the local end of a finished transfer. The path is looked up on the other side. */
    open: (id: string): Promise<void> => forward("transfer_open", () => desktop.transferOpen(id)),
    /** Shows where a transfer's local end is, in this system's file manager. */
    reveal: (id: string): Promise<void> => forward("transfer_reveal", () => desktop.transferReveal(id)),
    /** Holds a running transfer where it is, keeping its connection and its part of the file. */
    pause: (id: string): Promise<void> => forward("transfer_pause", () => desktop.transferPause(id)),
    resume: (id: string): Promise<void> => forward("transfer_resume", () => desktop.transferResume(id)),
    /** Runs a stopped transfer again, continuing from wherever it stopped. */
    retry: (id: string): Promise<void> => forward("transfer_retry", () => desktop.transferRetry(id)),
  },

  /**
   * Where downloads land, when somebody has said where that is.
   *
   * A folder picked once means every download lands there without a dialog, which is the difference
   * between downloading ten files and answering ten dialogs. What travels is the folder's own name —
   * the picker runs in Rust and the path stays there, so choosing is asking for a dialog rather than
   * naming a place.
   */
  downloads: {
    folder: (): Promise<DownloadFolderView> => forward("download_folder_get", () => desktop.downloadFolderGet()),
    /** Answers with nothing when the dialog was dismissed, which is not a change. */
    chooseFolder: (title: string): Promise<DownloadFolderView | undefined> =>
      forward("download_folder_choose", () => desktop.downloadFolderChoose(title)),
    /** Makes one of the remembered folders the one downloads land in. */
    useFolder: (index: number): Promise<DownloadFolderView> =>
      forward("download_folder_select", () => desktop.downloadFolderSelect(index)),
    /** Goes back to asking, which is what an installation nobody has configured does. */
    askEachTime: (): Promise<DownloadFolderView> => forward("download_folder_ask", () => desktop.downloadFolderAsk()),
    revealFolder: (): Promise<void> => forward("download_folder_reveal", () => desktop.downloadFolderReveal()),
  },

  sessions: {
    list: (): Promise<Session[]> => forward("sessions_list", () => desktop.sessionsList()),
    /** Absent `secret` means "leave the stored one alone", which is not the same as blank. */
    save: (session: Session, secret?: SecretInput): Promise<Session> =>
      forward("session_save", () => desktop.sessionSave(session, secret)),
    remove: (id: string): Promise<void> => forward("session_delete", () => desktop.sessionDelete(id)),

    /**
     * Writes the library to a file the user picks. The dialog runs in Rust, and the secrets are read
     * out of the vault and written there too, so none of this crosses the boundary — what crosses is
     * the passphrase that decides whether they travel at all.
     */
    export: (title: string, fileName: string, passphrase: string | null): Promise<SessionExportSummary | undefined> =>
      forward("sessions_export", () => desktop.sessionsExport(title, fileName, passphrase)),

    /**
     * Adds what a chosen file holds. `undefined` means the dialog was dismissed; a sealed file
     * answers that it needs a passphrase instead, and the second call carries it.
     */
    import: (title: string, passphrase: string | null): Promise<SessionImportOutcome | undefined> =>
      forward("sessions_import", () => desktop.sessionsImport(title, passphrase)),

    groups: {
      list: (): Promise<SessionGroup[]> => forward("groups_list", () => desktop.groupsList()),
      save: (group: SessionGroup): Promise<SessionGroup> => forward("group_save", () => desktop.groupSave(group)),
      remove: (id: string): Promise<void> => forward("group_delete", () => desktop.groupDelete(id)),
    },
  },

  sync: {
    /** What this machine could put into a backup, as counts. */
    available: (): Promise<SyncContents> => forward("sync_available", () => desktop.syncAvailable()),

    /**
     * Writes a backup to a file the user picks, in Rust. The passphrase is the whole of what this
     * side decides about credentials, and the values stay on the other side of the boundary.
     */
    backup: (
      title: string,
      fileName: string,
      selection: SyncSelection,
      passphrase: string | null,
    ): Promise<SyncReceipt | undefined> =>
      forward("sync_backup_export", () => desktop.syncBackupExport(title, fileName, selection, passphrase)),

    /** Reads a backup the user picks and says what is inside it. `undefined` means dismissed. */
    inspect: (title: string, passphrase: string | null): Promise<SyncInspectOutcome | undefined> =>
      forward("sync_backup_inspect", () => desktop.syncBackupInspect(title, passphrase)),

    /** Applies the backup that was read, in the shell. */
    restore: (replace: boolean, passphrase: string | null): Promise<SyncApplySummary> =>
      forward("sync_backup_restore", () => desktop.syncBackupRestore(replace, passphrase)),

    /**
     * The server: what is kept for it, and the four things done with one.
     *
     * The address and the user name are not here at all — they are settings, and the shell reads
     * them from the file. What crosses is the selection, the passphrase somebody typed, and answers.
     */
    status: (): Promise<SyncServiceStatus> => forward("sync_status", () => desktop.syncStatus()),
    keepPassword: (password: string | null): Promise<SyncServiceStatus> =>
      forward("sync_password", () => desktop.syncPassword(password)),
    keepPassphrase: (passphrase: string | null): Promise<SyncServiceStatus> =>
      forward("sync_passphrase", () => desktop.syncPassphrase(passphrase)),
    test: (): Promise<void> => forward("sync_test", () => desktop.syncTest()),
    upload: (selection: SyncSelection, passphrase: string | null): Promise<SyncUploadReceipt> =>
      forward("sync_upload", () => desktop.syncUpload(selection, passphrase)),
    download: (passphrase: string | null): Promise<SyncDownloadOutcome> =>
      forward("sync_download", () => desktop.syncDownload(passphrase)),
    apply: (replace: boolean, passphrase: string | null): Promise<SyncApplySummary> =>
      forward("sync_apply", () => desktop.syncApply(replace, passphrase)),

    /**
     * The snippet: the same backup parked in somebody else's code host.
     *
     * One fewer thing to pass than the server's half, because a snippet's configuration *is* settings
     * — the service, its address and the file each account writes to — and the shell reads them from
     * the file. The only secret is the token, which is in the vault, and it goes in by name.
     */
    snippet: {
      status: (): Promise<SnippetStatus> => forward("snippet_sync_status", () => desktop.snippetSyncStatus()),
      keepToken: (token: string | null): Promise<SnippetStatus> =>
        forward("snippet_sync_token", () => desktop.snippetSyncToken(token)),
      test: (): Promise<void> => forward("snippet_sync_test", () => desktop.snippetSyncTest()),
      upload: (selection: SyncSelection, passphrase: string | null): Promise<SnippetUploadReceipt> =>
        forward("snippet_sync_upload", () => desktop.snippetSyncUpload(selection, passphrase)),
      download: (passphrase: string | null): Promise<SyncDownloadOutcome> =>
        forward("snippet_sync_download", () => desktop.snippetSyncDownload(passphrase)),
      apply: (replace: boolean, passphrase: string | null): Promise<SyncApplySummary> =>
        forward("snippet_sync_apply", () => desktop.snippetSyncApply(replace, passphrase)),
    },
  },

  keys: {
    /** The imported keys: names, algorithms, fingerprints. Never the material. */
    list: (): Promise<KeyInfo[]> => forward("keys_list", () => desktop.keysList()),
    /** Opens a native dialog in Rust; resolves to `undefined` when the person dismissed it. */
    import: (title: string, passphrase?: string): Promise<KeyInfo | undefined> =>
      forward("key_import", () => desktop.keyImport(title, passphrase)),
    /** Makes one instead of asking for a file, and answers with the line to put on a server. */
    generate: (algorithm: KeyAlgorithm, comment: string, passphrase: string | null): Promise<CreatedKey> =>
      forward("key_generate", () => desktop.keyGenerate(algorithm, comment, passphrase)),
    /** The public half of a stored key, which is the half that goes into `authorized_keys`. */
    publicKey: (keyRef: string): Promise<string> => forward("key_public", () => desktop.keyPublic(keyRef)),
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
    /** Calls off an attempt that has not finished; false means there was nothing left to stop. */
    cancelOpen: (sessionId: string): Promise<boolean> =>
      forward("session_open_cancel", () => desktop.sessionOpenCancel(sessionId)),
    /** What the host behind a terminal says it is: the mark a session row draws, or nothing. */
    system: (handle: string): Promise<HostSystem> => forward("session_system", () => desktop.sessionSystem(handle)),
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

  /**
   * What the host behind a terminal is doing. Asked for by handle, because a sample rides the
   * connection the terminal already has: there is nothing to sample without one.
   */
  monitor: {
    sample: (handle: string): Promise<HostMetrics> => forward("monitor_sample", () => desktop.monitorSample(handle)),
    /** The process table, cut on the far side to what was asked for. The page orders what it gets. */
    processes: (handle: string, limit: number): Promise<ProcessPage> =>
      forward("monitor_processes", () => desktop.monitorProcesses(handle, limit)),
    /**
     * One process in full, read when somebody opens it. The only read in this build that returns an
     * environment, which can hold credentials — which is why it is never part of a poll.
     */
    processDetail: (handle: string, pid: number): Promise<ProcessDetail> =>
      forward("monitor_process_detail", () => desktop.monitorProcessDetail(handle, pid)),
    /** The host's sockets, for the network page: ports that are waiting, and who is connected. */
    sockets: (handle: string): Promise<SocketRow[]> => forward("monitor_sockets", () => desktop.monitorSockets(handle)),
  },

  /**
   * What the operating system has on its clipboard, read where the webview cannot reach it.
   *
   * Only text, and only outwards: nothing here writes a clipboard, and what it reads is the person's
   * own — a path never passes through it, which is why it needs no token the way a file list does.
   */
  clipboard: {
    /** Empty when the clipboard holds no text. `APP-UNSUPPORTED` where the platform cannot read one. */
    text: (): Promise<string> => forward("clipboard_text", () => desktop.clipboardText()),
  },

  /**
   * What this machine can draw in, for the two settings that choose a font.
   *
   * Read when a picker opens rather than at start-up: it is the machine's whole font list, and most
   * runs of this application never open one. An empty answer is a machine whose fonts could not be
   * read, which the picker shows as "the families that ship with the application" rather than as a
   * failure — the setting still works, and a name can still be typed into it.
   */
  fonts: {
    systemList: (): Promise<string[]> => forward("system_fonts_list", () => desktop.systemFontsList()),
  },

  /**
   * The picture behind the window.
   *
   * None of the three takes a path, and that is the point of them: the picture is copied into this
   * installation's own directory when it is chosen, and read back through a command that names no file
   * at all — so there is no path for the interface to hold, to write into a settings file, or to get
   * wrong.
   */
  background: {
    choose: (title: string): Promise<string | null> =>
      forward("background_image_choose", () => desktop.backgroundImageChoose(title)),
    read: (): Promise<string> => forward("background_image_read", () => desktop.backgroundImageRead()),
    clear: (): Promise<void> => forward("background_image_clear", () => desktop.backgroundImageClear()),
  },

  /**
   * One command, many hosts.
   *
   * The verdict is asked for separately from the run — while somebody types, not when they press —
   * and the run's progress arrives on `events.onBatchEvent` rather than in the answer, because the
   * person watching needs each host's row as it lands and not when the slowest one does.
   */
  batch: {
    // The verdict for a command is not a batch feature and does not live here: `commands.classify`
    // is the one place it is asked for, and the same answer is what the batch gate enforces.
    run: (request: BatchRequest): Promise<BatchOutcome> => forward("batch_run", () => desktop.batchRun(request)),
  },

  /**
   * The commands somebody keeps, and sending one into the shells that are open.
   *
   * A send writes into a running shell rather than running a command somewhere: there is no
   * connection to open and no exit status to read, which is why the answer is a report per terminal
   * and why the whole library lives in the vault rather than in a file the user can read.
   */
  commands: {
    library: (): Promise<SnippetGroup[]> => forward("snippets_open", () => desktop.snippetsOpen()),
    saveGroup: (group: SnippetGroup): Promise<SnippetGroup> =>
      forward("snippet_group_save", () => desktop.snippetGroupSave(group)),
    deleteGroup: (id: string): Promise<void> => forward("snippet_group_delete", () => desktop.snippetGroupDelete(id)),
    save: (snippet: Snippet): Promise<Snippet> => forward("snippet_save", () => desktop.snippetSave(snippet)),
    delete: (id: string): Promise<void> => forward("snippet_delete", () => desktop.snippetDelete(id)),
    /**
     * The words somebody added to what follows a command.
     *
     * A list of its own rather than part of the library: what a command *keeps* is a line, and what
     * a command *knows* is a word, and the two are edited in different places.
     */
    words: (): Promise<CompletionWord[]> => forward("completion_words", () => desktop.completionWords()),
    saveWord: (word: CompletionWord): Promise<CompletionWord> =>
      forward("completion_word_save", () => desktop.completionWordSave(word)),
    deleteWord: (id: string): Promise<void> =>
      forward("completion_word_delete", () => desktop.completionWordDelete(id)),
    /** Forgets every word somebody added: what restoring the defaults means for this list. */
    clearWords: (): Promise<void> => forward("completion_words_clear", () => desktop.completionWordsClear()),
    history: (): Promise<CommandHistoryEntry[]> => forward("command_history", () => desktop.commandHistory()),
    clearHistory: (): Promise<void> => forward("command_history_clear", () => desktop.commandHistoryClear()),
    /** What the classifier makes of a command. Sends nothing. */
    classify: (command: string): Promise<RiskAssessment> =>
      forward("command_classify", () => desktop.commandClassify(command)),
    send: (target: SendTarget, command: string, appendCr: boolean, acknowledged: boolean): Promise<SendOutcome> =>
      forward("command_send", () => desktop.commandSend(target, command, appendCr, acknowledged)),
  },

  /**
   * The update channel: what is published, and installing it.
   *
   * Two calls rather than one, because a person decides between them, and the second installs the
   * release the first showed — the shell holds it in between rather than checking again.
   */
  updates: {
    check: (): Promise<UpdateCheck> => forward("update_check", () => desktop.updateCheck()),
    download: (): Promise<void> => forward("update_download", () => desktop.updateDownload()),
    install: (): Promise<void> => forward("update_install", () => desktop.updateInstall()),
    openReleasePage: (version: string): Promise<void> =>
      forward("update_open_release_page", () => desktop.updateOpenReleasePage(version)),
  },

  events: {
    /** The system's own appearance changing under a window that is following it. */
    onSystemThemeChanged: (listener: (theme: "dark" | "light" | null) => void): Promise<Unlisten> =>
      subscribe(desktop.onSystemThemeChanged, listener),
    /** One host starting or finishing during a batch. */
    onBatchEvent: (listener: (payload: BatchProgress) => void): Promise<Unlisten> =>
      subscribe(desktop.onBatchEvent, listener),
    /** How far an update download has got, and when the installer has taken it over. */
    onUpdateProgress: (listener: (payload: UpdateProgress) => void): Promise<Unlisten> =>
      subscribe(desktop.onUpdateProgress, listener),
    onTerminalData: (listener: (payload: TerminalData) => void): Promise<Unlisten> =>
      subscribe(desktop.onTerminalData, listener),
    onTerminalClosed: (listener: (payload: TerminalClosed) => void): Promise<Unlisten> =>
      subscribe(desktop.onTerminalClosed, listener),
    /** Whether a drag is over the window, for the drop hint. */
    onFilesDrag: (listener: (payload: DragPayload) => void): Promise<Unlisten> =>
      subscribe(desktop.onFilesDrag, listener),
    onFilesDropped: (listener: (payload: DropPayload) => void): Promise<Unlisten> =>
      subscribe(desktop.onFilesDropped, listener),
    /**
     * A close that arrived while the quit guard was armed: the interface asks, then closes.
     *
     * The one subscription with no payload, so it is registered here rather than through the
     * helper above — that helper's listener is shaped for events that carry one.
     */
    onQuitRequested: (listener: () => void): Promise<Unlisten> =>
      isDesktopRuntime() ? desktop.onQuitRequested(listener) : Promise.resolve(() => {}),
    /**
     * A vault that was locked where the interface could not see it — the tray menu, which the
     * operating system draws. The next thing that happens is the gate coming back on screen.
     */
    onVaultLocked: (listener: () => void): Promise<Unlisten> =>
      isDesktopRuntime() ? desktop.onVaultLocked(listener) : Promise.resolve(() => {}),
  },
};

export { ShellError };
export type { Unlisten };
