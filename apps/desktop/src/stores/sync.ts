/**
 * Backups: what this machine could put into one, writing one, reading one, and applying one.
 *
 * The two directions are separate flows because they are separate journeys. Writing a backup is one
 * decision plus a file; reading one is a file, a look at what is inside it, and then a decision.
 *
 * Two things are worth knowing about the shape:
 *
 * 1. **A sealed file is read with the passphrase on the panel.** There is no separate question
 *    dialog, because the field that answers it is already on screen — and a refusal lands beside
 *    that field rather than somewhere else. The shell keeps the chosen file between the two calls,
 *    so a wrong passphrase is retyped rather than the file being chosen again.
 * 2. **Nothing here reloads the library.** A restore writes sessions, groups, commands, words and
 *    possibly the settings, and only the page knows which of those it is showing; the store answers
 *    what happened, and the caller reads what it draws again.
 */

import { defineStore } from "pinia";
import { ref } from "vue";

import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import type {
  SnippetStatus,
  SnippetUploadReceipt,
  SyncApplySummary,
  SyncCatalog,
  SyncContents,
  SyncDownloadOutcome,
  SyncReceipt,
  SyncSelection,
  SyncServiceStatus,
  SyncUploadReceipt,
} from "@/types/sync";

/**
 * Which of the things a page can be doing is running, for a panel that disables the rest.
 *
 * One field for all of them rather than one per panel: two syncs at once is not a thing anybody
 * means, and a second one's sentence would arrive after the first's and overwrite it.
 */
export type SyncWork = "" | "export" | "import" | "test" | "upload" | "download" | "password" | "passphrase" | "token";

/** The server actions, which share that field with the two file ones. */
type ServiceWork = "test" | "upload" | "download" | "password" | "passphrase";

/** The snippet's actions, which are the server's with a token instead of a password. */
type SnippetWork = "test" | "upload" | "download" | "token";

/** A backup that was read and is waiting to be applied. */
export interface ReadBackup {
  readonly fileName: string;
  readonly catalog: SyncCatalog;
}

export const useSyncStore = defineStore("sync", () => {
  /** What this machine could back up, as counts. `null` until it has been asked. */
  const available = ref<SyncContents | null>(null);
  const busy = ref<SyncWork>("");
  const failure = ref<ShellError | null>(null);
  /** What just happened, in a sentence — the receipt of an import, the end of an export. */
  const notice = ref("");
  /** The file that was read, waiting for the answer about replacing what is here. */
  const read = ref<ReadBackup | null>(null);

  /** Reads what this machine holds, for the sentence above the buttons. */
  async function load(): Promise<void> {
    try {
      available.value = await backend.sync.available();
      failure.value = null;
    } catch (error) {
      failure.value = asShellError(error, "sync_available");
    }
  }

  /**
   * Writes a backup to a file the user picks.
   *
   * `passphrase` is the whole of what this decides about credentials: with one the file carries the
   * stored logins and key material sealed, with `null` it carries neither. `null` comes back when
   * nothing was written — a dismissed file dialog, or a failure already in `failure`.
   */
  async function writeBackup(
    title: string,
    fileName: string,
    selection: SyncSelection,
    passphrase: string | null,
  ): Promise<SyncReceipt | null> {
    failure.value = null;
    notice.value = "";
    busy.value = "export";
    try {
      return (await backend.sync.backup(title, fileName, selection, passphrase)) ?? null;
    } catch (error) {
      failure.value = asShellError(error, "sync_backup_export");
      return null;
    } finally {
      busy.value = "";
    }
  }

  /**
   * Reads a backup the user picks and answers with what is inside it.
   *
   * A sealed file that the passphrase does not open leaves `failure` set and the file still waiting
   * in the shell: the panel's field is where the answer goes, so the next press tries the same file
   * again.
   */
  async function readBackup(title: string, passphrase: string | null): Promise<ReadBackup | null> {
    failure.value = null;
    notice.value = "";
    busy.value = "import";
    try {
      const outcome = await backend.sync.inspect(title, passphrase);
      if (!outcome || outcome.kind === "needsPassphrase") {
        // The question is the panel's field, so nothing is shown here for it: the shell is holding
        // the file, and the next press of the same button carries the passphrase.
        return null;
      }
      read.value = { fileName: outcome.fileName, catalog: outcome.catalog };
      return read.value;
    } catch (error) {
      failure.value = asShellError(error, "sync_backup_inspect");
      return null;
    } finally {
      busy.value = "";
    }
  }

  /** Applies the file that was read. `null` is a refusal, already drawn on the panel. */
  async function applyBackup(replace: boolean, passphrase: string | null): Promise<SyncApplySummary | null> {
    failure.value = null;
    notice.value = "";
    busy.value = "import";
    try {
      const summary = await backend.sync.restore(replace, passphrase);
      read.value = null;
      return summary;
    } catch (error) {
      failure.value = asShellError(error, "sync_backup_restore");
      return null;
    } finally {
      busy.value = "";
    }
  }

  /** Puts the question away without applying anything. The shell forgets the file on its own. */
  function forgetRead(): void {
    read.value = null;
  }

  /**
   * What this installation has for a server: whether a password and a passphrase are kept.
   *
   * Two booleans and no values. The address and the user name are on the settings side, and the two
   * secrets are in the vault — this is only what the page needs to draw its boxes.
   */
  const service = ref<SyncServiceStatus | null>(null);

  async function loadService(): Promise<void> {
    try {
      service.value = await backend.sync.status();
    } catch (error) {
      failure.value = asShellError(error, "sync_status");
    }
  }

  /**
   * Runs one of the four server actions under the shared shape: one busy kind, one failure, and the
   * answer back to the caller.
   *
   * Reading a server and writing to one both take a network round trip, so the page disables
   * everything while any of them runs — two syncs at once is not a thing anybody means, and the
   * second one's result would arrive after the first's and overwrite its sentence.
   */
  async function runService<T>(kind: ServiceWork, action: () => Promise<T>): Promise<T | null> {
    failure.value = null;
    notice.value = "";
    busy.value = kind satisfies SyncWork;
    try {
      return await action();
    } catch (error) {
      failure.value = asShellError(error, `sync_${kind}`);
      return null;
    } finally {
      busy.value = "";
    }
  }

  /** Keeps the server's password, or forgets it. `null` is forgetting. */
  async function keepPassword(password: string | null): Promise<void> {
    await runService("password", async () => {
      service.value = await backend.sync.keepPassword(password);
    });
  }

  /** Keeps the passphrase an upload is sealed under, or forgets it. */
  async function keepPassphrase(passphrase: string | null): Promise<void> {
    await runService("passphrase", async () => {
      service.value = await backend.sync.keepPassphrase(passphrase);
    });
  }

  /** Asks the configured server whether it is a WebDAV one. `true` is an answer. */
  async function testServer(): Promise<boolean> {
    return (await runService("test", () => backend.sync.test())) !== null;
  }

  /** Writes the library to the server. `null` is a refusal, already in `failure`. */
  async function uploadToServer(
    selection: SyncSelection,
    passphrase: string | null,
  ): Promise<SyncUploadReceipt | null> {
    return runService("upload", () => backend.sync.upload(selection, passphrase));
  }

  /** Reads the file the server holds. `null` is a refusal, already in `failure`. */
  async function downloadFromServer(passphrase: string | null): Promise<SyncDownloadOutcome | null> {
    return runService("download", () => backend.sync.download(passphrase));
  }

  /** Applies the file the server holds, fetching it again. */
  async function applyFromServer(replace: boolean, passphrase: string | null): Promise<SyncApplySummary | null> {
    return runService("download", () => backend.sync.apply(replace, passphrase));
  }

  // -- The snippet, which is the same three things somewhere else ----------------------

  /**
   * What this installation has for the snippet account its settings describe.
   *
   * `null` until it has been asked, and asked again whenever the account changes: the answer names
   * the account, so it is the only side that can say which file id belongs to what is on screen.
   */
  const snippet = ref<SnippetStatus | null>(null);

  async function loadSnippet(): Promise<void> {
    await runSnippet("token", async () => {
      snippet.value = await backend.sync.snippet.status();
    });
  }

  /** Keeps the token for the configured account, or forgets it. `null` is forgetting. */
  async function keepSnippetToken(token: string | null): Promise<void> {
    await runSnippet("token", async () => {
      snippet.value = await backend.sync.snippet.keepToken(token);
    });
  }

  /** Asks the service whether the token, and the file when there is one, is good. */
  async function testSnippet(): Promise<boolean> {
    return (await runSnippet("test", () => backend.sync.snippet.test())) !== null;
  }

  /** Writes the library into the gist or snippet. `null` is a refusal, already in `failure`. */
  async function uploadToSnippet(
    selection: SyncSelection,
    passphrase: string | null,
  ): Promise<SnippetUploadReceipt | null> {
    return runSnippet("upload", () => backend.sync.snippet.upload(selection, passphrase));
  }

  async function downloadFromSnippet(passphrase: string | null): Promise<SyncDownloadOutcome | null> {
    return runSnippet("download", () => backend.sync.snippet.download(passphrase));
  }

  async function applyFromSnippet(replace: boolean, passphrase: string | null): Promise<SyncApplySummary | null> {
    return runSnippet("download", () => backend.sync.snippet.apply(replace, passphrase));
  }

  /**
   * The shared shape of a snippet action, which is the server's shape with different words for the
   * failure: its kinds are all names the busy field already has, so one flag still covers everything
   * a panel disables while something is running.
   */
  async function runSnippet<T>(kind: SnippetWork, action: () => Promise<T>): Promise<T | null> {
    failure.value = null;
    notice.value = "";
    busy.value = kind;
    try {
      return await action();
    } catch (error) {
      failure.value = asShellError(error, `snippet_sync_${kind}`);
      return null;
    } finally {
      busy.value = "";
    }
  }

  return {
    available,
    busy,
    failure,
    notice,
    read,
    service,
    load,
    writeBackup,
    readBackup,
    applyBackup,
    forgetRead,
    loadService,
    keepPassword,
    keepPassphrase,
    testServer,
    uploadToServer,
    downloadFromServer,
    applyFromServer,
    snippet,
    loadSnippet,
    keepSnippetToken,
    testSnippet,
    uploadToSnippet,
    downloadFromSnippet,
    applyFromSnippet,
  };
});
