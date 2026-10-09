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
import type { SyncApplySummary, SyncCatalog, SyncContents, SyncReceipt, SyncSelection } from "@/types/sync";

/** Which of the two directions is running, for a panel that disables the other one. */
export type SyncWork = "" | "export" | "import";

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

  return { available, busy, failure, notice, read, load, writeBackup, readBackup, applyBackup, forgetRead };
});
