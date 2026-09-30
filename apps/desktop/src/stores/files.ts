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
 */

import { defineStore } from "pinia";
import { ref } from "vue";

import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import { parentPath } from "@/lib/files/format";
import type {
  EntryKind,
  SftpEntry,
  SftpListing,
  SftpOutcome,
  TransferDirection,
  TransferInfo,
  TransferState,
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
   */
  async function uploadDropped(handle: string, directory: string, token: string): Promise<void> {
    const open = browsers.value[handle];
    if (open) {
      open.failure = null;
    }

    try {
      const ids = await backend.files.uploadDropped(handle, directory, token);
      if (ids.length > 0) {
        await refreshTransfers();
      }
    } catch (error) {
      reportFailure(handle, error);
    }
  }

  /** Reads the file list off the clipboard and uploads it. A platform that cannot says so. */
  async function uploadFromClipboard(handle: string, directory: string): Promise<void> {
    const open = browsers.value[handle];
    if (open) {
      open.failure = null;
    }

    try {
      const pending = await backend.files.clipboardFiles();
      await uploadDropped(handle, directory, pending.token);
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

  function rename(handle: string, from: string, to: string): Promise<boolean> {
    return run(handle, "files_rename", () => backend.files.rename(handle, from, to));
  }

  /** Deletes a file, or a directory and everything under it: the kind says which this is. */
  function remove(handle: string, path: string, kind: EntryKind): Promise<boolean> {
    return run(handle, "files_delete", () => backend.files.remove(handle, path, kind));
  }

  /**
   * Asks Rust where to put a file, then starts moving it.
   *
   * An `undefined` answer means the save dialog was dismissed, which is not a failure and not a
   * transfer: nothing is recorded, and the list is not even read.
   */
  async function download(handle: string, path: string, title: string, suggestedName: string): Promise<void> {
    transferFailure.value = null;
    try {
      const id = await backend.files.download(handle, path, title, suggestedName);
      if (id !== undefined) {
        // The backend inserted the record before it answered, so the row is there to read.
        await refreshTransfers();
      }
    } catch (error) {
      transferFailure.value = asShellError(error, "files_download");
    }
  }

  /** Asks Rust which file to send, then starts copying it into the directory on screen. */
  async function upload(handle: string, directory: string, title: string): Promise<void> {
    transferFailure.value = null;
    try {
      const id = await backend.files.upload(handle, directory, title);
      if (id !== undefined) {
        await refreshTransfers();
      }
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
        const directory = parentPath(transfer.remotePath);
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

  /** Reads a directory into a browser's state. A failure is recorded and the old list stays. */
  async function read(handle: string, path: string): Promise<void> {
    const current = browsers.value[handle];
    if (current) {
      current.loading = true;
    }

    try {
      const listing: SftpListing = await backend.files.list(handle, path);
      // The browser can be closed while the read is in flight: then there is nothing to fill.
      const open = browsers.value[handle];
      if (!open) {
        return;
      }
      open.path = listing.path;
      open.entries = [...listing.entries];
      open.loading = false;
    } catch (error) {
      const open = browsers.value[handle];
      if (open) {
        open.loading = false;
        open.failure = asShellError(error, "files_list");
      }
    }
  }

  return {
    browsers,
    revision,
    transfers,
    transferFailure,
    unattendedFailures,
    attendFailures,
    open,
    browser,
    directories,
    uploadDropped,
    uploadFromClipboard,
    reportFailure,
    navigate,
    createFolder,
    rename,
    remove,
    download,
    upload,
    close,
    refreshTransfers,
    cancelTransfer,
    dismissTransferFailure,
    stopPolling,
  };
});
