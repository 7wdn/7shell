/**
 * The remote directories a file browser is looking at, and the transfers moving bytes for them.
 *
 * The workspace store owns the tabs; this one owns what is inside them. Keeping that line means the
 * tab rules (one browser per session, a host key decision before a handle) live in one place while
 * the directory state — which changes with every click — sits next to the transfers it can start.
 *
 * Two rules the backend imposes are visible here:
 *
 * 1. **A download and an upload ask for a local path in Rust.** The interface names neither: each
 *    call answers with a transfer id, or with nothing when the dialog was dismissed — and a
 *    dismissed dialog is somebody changing their mind, so nothing at all is recorded for it.
 * 2. **Closing a browser cancels whatever it was still moving**, which is why closing re-reads the
 *    transfer list rather than leaving rows behind that are already gone.
 *
 * A browser can also be moved by the thing it belongs to. A terminal's file panel follows the shell
 * under it — `cd /data` in the terminal takes the panel to /data — and that arrives here as
 * `follow`: a directory name, resolved against where the shell is rather than where the panel is
 * showing, and listed before it is gone to.
 */

import { defineStore } from "pinia";
import { ref } from "vue";

import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import { parentPath } from "@/lib/files/format";
import { resolveDirectory } from "@/lib/terminal/follow";
import type {
  SftpEntry,
  SftpListing,
  SftpOutcome,
  TransferDirection,
  TransferInfo,
  TransferState,
  UploadAnswer,
  UploadOutcome,
} from "@/types/session";

/** How often the transfer list is read while something is still moving. */
export const TRANSFER_POLL_MS = 500;

/** A transfer that stopped because it failed, and that nobody has been told about yet. */
export interface UnattendedTransferFailure {
  readonly id: string;
  readonly direction: TransferDirection;
  /** The file's own name, not a path on either side: enough to say which transfer it was. */
  readonly name: string;
}

/** One open browser: the directory it is looking at, and the failure it has to show, if any. */
export interface FileBrowser {
  readonly handle: string;
  readonly sessionId: string;
  /** The path the server resolved for the last listing: what the breadcrumbs show. */
  path: string;
  /**
   * Where the browser opened, which is where the shell on the other end opened too: an SFTP session
   * starts in the user's home, and so does a login shell. It is what `~` means.
   */
  home: string;
  /**
   * Where the shell is, as far as this side knows — deliberately not the same as `path`.
   *
   * The browser can be browsed by hand while the shell stays put, and `cd logs` has to be resolved
   * against the shell's directory rather than against whatever is on screen. It starts at `home`,
   * which is where a shell that has not moved yet is.
   */
  shellPath: string;
  entries: readonly SftpEntry[];
  /** True while a listing is being read. The entries from the last read stay on screen. */
  loading: boolean;
  /** The envelope of the last read or change that failed. The panel shows it over the list. */
  failure: ShellError | null;
}

export const useFilesStore = defineStore("files", () => {
  const browsers = ref<Record<string, FileBrowser>>({});
  /** Every transfer the backend still remembers, running or finished, newest last. */
  const transfers = ref<TransferInfo[]>([]);
  /** A failure from reading the transfer list or stopping one. Shown next to the transfers. */
  const transferFailure = ref<ShellError | null>(null);
  /**
   * The upload that stopped to ask about names already on the far side.
   *
   * It lives here rather than in a browser because an upload can be started from three places — a
   * panel's own button, a drop on the window, a paste — and all three have to be answered by one
   * question. The shell draws it, for the same reason it draws the host key's: a question about
   * where bytes are going is not a browser's question.
   */
  const uploadConflict = ref<{ token: string; names: readonly string[]; skippable: boolean } | null>(null);
  /**
   * The editors holding changes nobody has saved, by the file each one is about.
   *
   * Kept here because the shell has to answer "would closing the window throw something away?"
   * without reaching into a dialog: what an editor is holding is its own state, and a dialog is not
   * something a parent can look inside.
   */
  const unsavedEditors = ref<string[]>([]);

  /** Records that an editor has, or no longer has, something unsaved. */
  function setUnsaved(key: string, unsaved: boolean): void {
    unsavedEditors.value = unsaved
      ? [...new Set([...unsavedEditors.value, key])]
      : unsavedEditors.value.filter((entry) => entry !== key);
  }
  /**
   * Transfers that failed and that nobody has been told about yet.
   *
   * A failed row lives in the transfer list, and the list lives inside a file browser — so when the
   * failure arrives while the person is in a terminal, there is nowhere on screen for it to appear.
   * It waits here until the announcement that watches this list takes it, which is also what keeps
   * one failure from being announced twice: a notice is consumed rather than remembered.
   */
  const unattendedFailures = ref<readonly UnattendedTransferFailure[]>([]);

  /**
   * Takes failures that have now been accounted for out of the waiting list.
   *
   * Called whether or not a toast was raised: a failure the person could see on the list has been
   * accounted for by being visible, and must not be announced later, when they switch tabs.
   */
  function attendFailures(ids: readonly string[]): void {
    const taken = new Set(ids);
    unattendedFailures.value = unattendedFailures.value.filter((entry) => !taken.has(entry.id));
  }
  /**
   * Bumped by anything that changes the shape of a directory.
   *
   * The directory tree caches what it has listed, and a folder created, renamed or deleted
   * anywhere makes every cached listing suspect — including branches that look untouched. A
   * counter is the cheapest way to say "everything you remember may be stale", and it costs one
   * round trip per *open* branch to recover.
   */
  const revision = ref(0);

  /** The interval handle, while there is something worth polling for. */
  let polling: ReturnType<typeof setInterval> | null = null;
  /**
   * What the last read saw each transfer doing, so one that has just finished can be told from one
   * that was already finished. Only the live edge is interesting: a re-read of the directory is
   * owed exactly once, when a transfer that was moving stops being moved.
   */
  const lastStates = new Map<string, TransferState["kind"]>();

  /**
   * Opens a browser for a session and records where it starts.
   *
   * The first listing is part of the open answer, so a browser that opened has something to draw
   * before anything else is asked of it. A host key decision is returned rather than handled here:
   * the one prompt for it belongs to the workspace store, which is where the tab will be.
   */
  async function open(sessionId: string): Promise<SftpOutcome> {
    const outcome = await backend.files.open(sessionId);
    if (outcome.kind === "opened") {
      browsers.value[outcome.handle] = {
        handle: outcome.handle,
        sessionId,
        path: outcome.listing.path,
        home: outcome.listing.path,
        shellPath: outcome.listing.path,
        entries: [...outcome.listing.entries],
        loading: false,
        failure: null,
      };
    }
    return outcome;
  }

  function browser(handle: string): FileBrowser | null {
    return browsers.value[handle] ?? null;
  }

  /** Goes to a path somebody chose. The failure from the last attempt goes with the old path. */
  /**
   * The directories inside a path, for the tree.
   *
   * Deliberately not `read`: the tree lists branches the listing is not in, and pointing the
   * listing at each of them on the way past would move the pane somebody is working in. A branch
   * that cannot be read comes back empty — the listing is where errors are reported, and saying it
   * twice helps nobody.
   */
  async function directories(handle: string, path: string): Promise<SftpEntry[]> {
    try {
      const listing = await backend.files.list(handle, path);
      return listing.entries.filter((entry) => entry.kind === "directory");
    } catch {
      return [];
    }
  }

  /**
   * Uploads what a drop or a paste left behind.
   *
   * The token stands for paths this side never saw: the operating system handed them to Rust, and
   * Rust answers with the transfers it started.
   *
   * `packed` travels with it because the switch means the same thing at every door: a batch dropped
   * from the desktop is a batch, and packing it is one transfer instead of twenty.
   */
  async function uploadDropped(handle: string, directory: string, token: string, packed: boolean): Promise<void> {
    const open = browsers.value[handle];
    if (open) {
      open.failure = null;
    }

    try {
      await absorbUpload(await backend.files.uploadDropped(handle, directory, token, packed));
    } catch (error) {
      reportFailure(handle, error);
    }
  }

  /**
   * What an upload answered: transfers that are running, a question, or nothing at all.
   *
   * The question is kept rather than acted on — the shell asks it, and answering it is what starts
   * or forgets the files that are waiting on the far side of it.
   */
  async function absorbUpload(outcome: UploadOutcome): Promise<void> {
    if (outcome.kind === "conflict") {
      uploadConflict.value = { token: outcome.token, names: outcome.names, skippable: outcome.skippable };
      return;
    }
    if (outcome.kind === "started" && outcome.ids.length > 0) {
      await refreshTransfers();
    }
  }

  /** The answer to the question: send them anyway, send only what is missing, or send nothing. */
  async function answerUpload(answer: UploadAnswer): Promise<void> {
    const asked = uploadConflict.value;
    uploadConflict.value = null;
    if (!asked) {
      return;
    }

    try {
      await absorbUpload(await backend.files.uploadAnswer(asked.token, answer));
    } catch (error) {
      transferFailure.value = asShellError(error, "files_upload");
    }
  }

  /** Reads the file list off the clipboard and uploads it. A platform that cannot says so. */
  async function uploadFromClipboard(handle: string, directory: string, packed: boolean): Promise<void> {
    const open = browsers.value[handle];
    if (open) {
      open.failure = null;
    }

    try {
      const pending = await backend.files.clipboardFiles();
      await uploadDropped(handle, directory, pending.token, packed);
    } catch (error) {
      reportFailure(handle, error);
    }
  }

  /** Records a failure in the browser it belongs to, for callers that caught it elsewhere. */
  function reportFailure(handle: string, error: unknown): void {
    const open = browsers.value[handle];
    if (open) {
      open.failure = asShellError(error, "dropped_upload");
    }
  }

  async function navigate(handle: string, path: string): Promise<void> {
    const current = browsers.value[handle];
    if (current) {
      current.failure = null;
    }
    await read(handle, path);
  }

  function createFolder(handle: string, path: string): Promise<boolean> {
    return run(handle, "files_create_folder", () => backend.files.createFolder(handle, path));
  }

  /** Creates an empty file, refused by the server when something with that name is already there. */
  function createFile(handle: string, path: string): Promise<boolean> {
    return run(handle, "files_create_file", () => backend.files.createFile(handle, path));
  }

  function rename(handle: string, from: string, to: string): Promise<boolean> {
    return run(handle, "files_rename", () => backend.files.rename(handle, from, to));
  }

  /**
   * Deletes every entry in the list, one ask per entry and one read of the directory at the end.
   *
   * One `run` rather than one per entry, and that is not tidiness: `run` is what re-reads the
   * directory, bumps the tree's revision and holds the failure for the panel to show — so a loop of
   * separate calls would read the directory N times and keep only the last failure, which is the one
   * least likely to be the one that mattered.
   *
   * A refusal stops the rest. What was already deleted stays deleted, which is the honest outcome of
   * a batch that failed part-way, and the re-read shows exactly what is left.
   */
  function removeMany(handle: string, entries: readonly SftpEntry[]): Promise<boolean> {
    return run(handle, "files_delete", async () => {
      for (const entry of entries) {
        await backend.files.remove(handle, entry.path, entry.kind);
      }
    });
  }

  /**
   * Asks Rust where to put things, then starts moving them.
   *
   * One door for every case the switch can be in: a single file is asked for by name, several rows
   * are asked for as a folder to put them in, and a packed selection is asked for as a folder to open
   * the archive into. Which dialog that is, is decided where the dialogs are — this side only knows
   * which rows somebody picked.
   *
   * An empty answer means a dialog was dismissed, which is not a failure and not a transfer: nothing
   * is recorded, and the list is not even read.
   */
  async function download(
    handle: string,
    paths: readonly string[],
    title: string,
    suggestedName: string,
    packed: boolean,
  ): Promise<void> {
    transferFailure.value = null;
    try {
      const ids = await backend.files.download(handle, paths, title, suggestedName, packed);
      if (ids.length > 0) {
        // The backend inserted the records before it answered, so the rows are there to read.
        await refreshTransfers();
      }
    } catch (error) {
      transferFailure.value = asShellError(error, "files_download");
    }
  }

  /** Asks Rust which file to send, then starts copying it into the directory on screen. */
  async function upload(handle: string, directory: string, title: string, packed: boolean): Promise<void> {
    transferFailure.value = null;
    try {
      await absorbUpload(await backend.files.upload(handle, directory, title, packed));
    } catch (error) {
      transferFailure.value = asShellError(error, "files_upload");
    }
  }

  /**
   * Closes a browser.
   *
   * The failure is returned rather than recorded: the tab that asked is the thing that has to
   * decide what to do with it, and a browser the backend refused to close is still open on the
   * far side — so its state is kept until it is really gone.
   */
  async function close(handle: string): Promise<ShellError | null> {
    try {
      await backend.files.close(handle);
    } catch (error) {
      return asShellError(error, "files_close");
    }
    delete browsers.value[handle];
    // Closing cancelled whatever this browser was still moving, so the list has changed.
    await refreshTransfers();
    return null;
  }

  /**
   * Reads the transfer list, then decides whether to keep reading it.
   *
   * Polling rather than events: a transfer is started by a command and outlives it, so there is no
   * call left for the backend to answer into. Half a second of counters is one small read, and a
   * second event channel would have to be kept in step with the list the strip already draws.
   */
  async function refreshTransfers(): Promise<void> {
    try {
      transfers.value = await backend.transfers.list();
    } catch (error) {
      transferFailure.value = asShellError(error, "transfers_open");
    }
    await settle();
    follow();
  }

  /**
   * Re-reads the directories an upload has just landed in.
   *
   * A listing read before the upload finished does not contain it, and the person who started the
   * upload is looking at exactly that listing. The destination is the transfer's own `remotePath`,
   * so only the browsers showing that directory are read again.
   */
  async function settle(): Promise<void> {
    const landed: string[] = [];
    const stopped: UnattendedTransferFailure[] = [];
    for (const transfer of transfers.value) {
      const previous = lastStates.get(transfer.id);
      lastStates.set(transfer.id, transfer.state.kind);
      const arrived = previous !== "done" && transfer.state.kind === "done";
      if (arrived && transfer.direction === "upload") {
        // A packed upload lands the files *in* the directory it was aimed at; an ordinary one lands a
        // file inside it. Reading the wrong one leaves the panel showing a directory the transfer
        // never touched — which is exactly the case where somebody would think nothing happened.
        const directory = transfer.packed ? transfer.remotePath : parentPath(transfer.remotePath);
        if (directory !== null) {
          landed.push(directory);
        }
      }
      // A transfer moves into `failed` once. This is the read that sees the change, including the
      // case where it failed before any read ever saw it moving.
      if (previous !== "failed" && transfer.state.kind === "failed") {
        stopped.push({ id: transfer.id, direction: transfer.direction, name: transfer.name });
      }
    }

    if (stopped.length > 0) {
      unattendedFailures.value = [...unattendedFailures.value, ...stopped];
    }

    for (const open of Object.values(browsers.value)) {
      if (landed.includes(open.path)) {
        await read(open.handle, open.path);
        revision.value += 1;
      }
    }
  }

  /** Keeps the list fresh while anything is moving, and stops the moment nothing is. */
  function follow(): void {
    const running = transfers.value.some((entry) => entry.state.kind === "running");
    if (running && polling === null) {
      polling = setInterval(() => void refreshTransfers(), TRANSFER_POLL_MS);
    }
    if (!running && polling !== null) {
      clearInterval(polling);
      polling = null;
    }
  }

  /**
   * Stops a transfer.
   *
   * The same call takes a finished record off the list: the backend drops it from its map, and a
   * second click on a transfer that is already gone is deliberately not an error.
   */
  async function cancelTransfer(id: string): Promise<void> {
    transferFailure.value = null;
    try {
      await backend.transfers.cancel(id);
      await refreshTransfers();
    } catch (error) {
      transferFailure.value = asShellError(error, "transfer_cancel");
    }
  }

  /**
   * Holds a transfer where it is, or lets it go on again.
   *
   * One action for both, because they are one button with two readings: whatever the row says it
   * will do next, this does it. The list is read back afterwards rather than the state being guessed
   * here — the backend is what knows whether the transfer noticed.
   */
  async function setTransferPaused(id: string, paused: boolean): Promise<void> {
    transferFailure.value = null;
    try {
      await (paused ? backend.transfers.pause(id) : backend.transfers.resume(id));
      await refreshTransfers();
    } catch (error) {
      transferFailure.value = asShellError(error, paused ? "transfer_pause" : "transfer_resume");
    }
  }

  /** Runs a stopped transfer again, continuing from wherever it stopped. */
  async function retryTransfer(id: string): Promise<void> {
    transferFailure.value = null;
    try {
      await backend.transfers.retry(id);
      await refreshTransfers();
    } catch (error) {
      transferFailure.value = asShellError(error, "transfer_retry");
    }
  }

  function dismissTransferFailure(): void {
    transferFailure.value = null;
  }

  /** Stops the poll, for tests and for an owner that goes away with transfers still running. */
  function stopPolling(): void {
    if (polling !== null) {
      clearInterval(polling);
      polling = null;
    }
  }

  /**
   * Runs a change and re-reads the directory afterwards.
   *
   * The re-read happens whether the change worked or not: a refused rename still leaves the
   * question "what is actually there now", and that is one listing away. A failure is recorded
   * before the re-read, and a successful re-read does not clear it.
   */
  async function run(handle: string, operation: string, change: () => Promise<void>): Promise<boolean> {
    const current = browsers.value[handle];
    if (current) {
      current.failure = null;
    }

    let worked = true;
    try {
      await change();
    } catch (error) {
      worked = false;
      const open = browsers.value[handle];
      if (open) {
        open.failure = asShellError(error, operation);
      }
    }

    const path = browsers.value[handle]?.path;
    if (path !== undefined) {
      await read(handle, path);
      // Only a change that happened: a refused one left the directory as it was.
      if (worked) {
        revision.value += 1;
      }
    }
    return worked;
  }

  /**
   * Which read each browser is waiting for.
   *
   * A read is a round trip, and two of them can be in flight at once — somebody clicking through a
   * tree, or a case driving the path bar faster than the host answers. Without this the *slower*
   * answer wins and the panel shows a directory nobody is looking at any more: that is how a file
   * that was just created "disappears", and how a listing looks like it went backwards.
   */
  const reads = new Map<string, number>();

  /** Reads a directory into a browser's state. A failure is recorded and the old list stays. */
  async function read(handle: string, path: string, options: { quiet?: boolean } = {}): Promise<boolean> {
    const ticket = (reads.get(handle) ?? 0) + 1;
    reads.set(handle, ticket);

    const current = browsers.value[handle];
    if (current) {
      current.loading = true;
    }

    try {
      const listing: SftpListing = await backend.files.list(handle, path);
      // The browser can be closed while the read is in flight: then there is nothing to fill. And a
      // newer read has been asked for since: this answer is about a directory nobody is looking at.
      const open = browsers.value[handle];
      if (!open || reads.get(handle) !== ticket) {
        return false;
      }
      open.path = listing.path;
      open.entries = [...listing.entries];
      open.loading = false;
      return true;
    } catch (error) {
      const open = browsers.value[handle];
      if (open && reads.get(handle) === ticket) {
        open.loading = false;
        // Quiet is for a read nobody asked for: the shell was already told the directory is not
        // there, and the panel repeating it would be the same complaint twice.
        if (options.quiet !== true) {
          open.failure = asShellError(error, "files_list");
        }
      }
      return false;
    }
  }

  /**
   * Takes a browser to where the shell says it is.
   *
   * Only the shell can say where the shell is, and it does not always say: what arrives here is
   * either a directory the shell reported outright or one read off a `cd` somebody typed. Neither
   * is taken on faith — the directory is listed first, and a directory that cannot be listed is
   * not gone to. The browser stays where it is and nothing is said, because the shell has already
   * refused the `cd` in the terminal, in its own words, and this would be the same complaint twice.
   *
   * The shell's directory is remembered on the way past even when the browser does not move: it is
   * what the next relative `cd` is resolved against.
   */
  async function followDirectory(handle: string, directory: string): Promise<void> {
    const open = browsers.value[handle];
    if (!open) {
      return;
    }
    const target = resolveDirectory(open.shellPath, open.home, directory);
    if (target === null) {
      return;
    }
    if (target === open.shellPath && target === open.path) {
      return;
    }

    // The read is quiet: a directory that cannot be listed has already been refused by the shell,
    // in the terminal, and this would be the same complaint twice. Nothing moves when it fails.
    const arrived = await read(handle, target, { quiet: true });
    const current = browsers.value[handle];
    if (!current || !arrived) {
      return;
    }
    // The shell is where it asked to go even when the server names that place differently — the
    // path from the listing is what the breadcrumbs show, and this is what a relative path is
    // resolved against next.
    current.failure = null;
    current.shellPath = target;
  }

  return {
    browsers,
    revision,
    transfers,
    transferFailure,
    uploadConflict,
    unsavedEditors,
    setUnsaved,
    unattendedFailures,
    attendFailures,
    open,
    browser,
    directories,
    uploadDropped,
    uploadFromClipboard,
    answerUpload,
    reportFailure,
    navigate,
    followDirectory,
    createFolder,
    createFile,
    rename,
    removeMany,
    download,
    upload,
    close,
    refreshTransfers,
    cancelTransfer,
    setTransferPaused,
    retryTransfer,
    dismissTransferFailure,
    stopPolling,
  };
});
