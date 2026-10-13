/**
 * The vault gate: whether the store of credentials exists, is open, or still needs a password.
 *
 * Three rules shape this store:
 *
 * 1. **The gate does not guess.** Until `vault_state` answers, the state is `unknown` and the
 *    screen says so. A state that cannot be read at all is surfaced as the envelope it came in,
 *    with a retry — never as "locked", which would ask for a password that cannot work.
 * 2. **Unlocking is what loads the session library.** Every reader of sessions sits behind the
 *    gate, so the library is loaded from the one place that knows the vault just opened, rather
 *    than by each screen testing the state.
 * 3. **A rejected password is a failure, not a cleared form.** The envelope is kept so the
 *    screen can show the code that came back; the typed password stays where it was typed.
 * 4. **A failure names the operation that produced it.** The screen has several actions that can
 *    fail, and an envelope shown beside the wrong one is worse than no placement at all; the
 *    operation is recorded here because the message cannot say which button was pressed.
 *
 * The archives of libraries past are the same gate seen from the other side: they are listed,
 * tried and switched from here, and each of those answers with its own outcome rather than with one
 * shared failure — an archive that refused a password belongs under the field it was typed in, and
 * a switch that failed belongs under the row it was asked from.
 */

import { defineStore } from "pinia";
import { computed, ref } from "vue";

import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import { useSessionsStore } from "@/stores/sessions";
import { useRecentSessionsStore } from "@/stores/recentSessions";
import type { SessionImportSummary, VaultArchive, VaultState } from "@/types/session";

/** `unknown` until the backend has answered; the gate draws nothing else until then. */
export type GateState = VaultState | "unknown";

/** The operations this store can report a failure for; each one is a button on the gate screen. */
export type VaultOperation = "vault_state" | "vault_create" | "vault_unlock" | "vault_set_aside" | "vault_lock";

export const useVaultStore = defineStore("vault", () => {
  const state = ref<GateState>("unknown");
  const busy = ref(false);
  const failure = ref<ShellError | null>(null);
  /** Which operation `failure` came from; `null` whenever there is no failure. */
  const failedOperation = ref<VaultOperation | null>(null);

  /** Failures are set and cleared together with their operation, so the two can never disagree. */
  function reportFailure(operation: VaultOperation, error: unknown): void {
    failedOperation.value = operation;
    failure.value = asShellError(error, operation);
  }

  function clearFailure(): void {
    failedOperation.value = null;
    failure.value = null;
  }

  /**
   * Drops the failure on screen without a new operation asking for anything.
   *
   * For the one case where a message has gone *stale* rather than been answered: a refusal that was
   * about what somebody typed, with what they typed now changed. The screen says when that is — it
   * is the only thing that can tell an edit from a retry — and the gate's own operations go on
   * clearing their failures themselves. A failure that is *not* about the thing being edited is not
   * dropped: the caller checks the operation first.
   */
  function dismissFailure(): void {
    clearFailure();
  }

  const unlocked = computed(() => state.value === "unlocked");

  /**
   * Everything the library screens draw, read in one place.
   *
   * The recent list comes along because the home page ranks the library by it: loading one without
   * the other would draw that list in the library's order for a moment and then reorder it under
   * the person's eyes.
   */
  async function loadLibrary(): Promise<void> {
    await useSessionsStore().load();
    useRecentSessionsStore().load();
  }

  /** Asks the backend where the vault stands, and loads the library when it is already open. */
  async function initialize(): Promise<void> {
    clearFailure();
    try {
      state.value = await backend.vault.state();
    } catch (error) {
      reportFailure("vault_state", error);
      return;
    }
    if (state.value === "unlocked") {
      await loadLibrary();
    }
  }

  /**
   * Creates the vault, and reports whether it worked.
   *
   * The answer rather than a throw is deliberate: the create form needs to keep what was typed
   * so a rejected password can be corrected, and the envelope is already in `failure`.
   */
  async function create(password: string): Promise<boolean> {
    return run("vault_create", () => backend.vault.create(password));
  }

  /** Opens the vault, with the same reporting rules as {@link create}. */
  async function unlock(password: string): Promise<boolean> {
    return run("vault_unlock", () => backend.vault.unlock(password));
  }

  async function run(operation: VaultOperation, action: () => Promise<VaultState>): Promise<boolean> {
    busy.value = true;
    clearFailure();
    try {
      state.value = await action();
      await loadLibrary();
      return true;
    } catch (error) {
      reportFailure(operation, error);
      return false;
    } finally {
      busy.value = false;
    }
  }

  /** The name an unopenable vault file was given, once it has been moved aside. */
  const setAsideName = ref<string | null>(null);

  /**
   * Moves a vault that cannot be opened out of the way, freeing the screen to create a new one.
   *
   * The reason this exists is that "nobody remembers the password" must not be a dead end: the
   * file is renamed beside itself rather than deleted, and its new name is reported so the screen
   * can tell the person where their data went and how to bring it back.
   */
  async function setAside(): Promise<boolean> {
    busy.value = true;
    clearFailure();
    try {
      setAsideName.value = await backend.vault.setAside();
      state.value = await backend.vault.state();
      return state.value === "missing";
    } catch (error) {
      reportFailure("vault_set_aside", error);
      return false;
    } finally {
      busy.value = false;
    }
  }

  /** Locks the vault. Open terminals are deliberately left running: they are already inside. */
  async function lock(): Promise<void> {
    clearFailure();
    try {
      state.value = await backend.vault.lock();
    } catch (error) {
      reportFailure("vault_lock", error);
    }
  }

  /**
   * Records that the vault was locked where this interface could not see it.
   *
   * The tray menu is drawn by the operating system, so locking from it reaches the window as an
   * event rather than as the answer to a call. What is left to do here is what any other lock
   * leaves: the gate goes up, and the library screens are behind it.
   */
  function markLocked(): void {
    clearFailure();
    state.value = "locked";
  }

  /** The archived libraries beside the live one, as the last look at them found them. */
  const archives = ref<VaultArchive[]>([]);
  const archivesBusy = ref(false);
  /** The failure of *listing* them; an operation on one archive reports to whoever asked for it. */
  const archivesFailure = ref<ShellError | null>(null);

  /**
   * What one of the vault's own operations came to.
   *
   * The several things a person can ask of an archive or of the master password can each fail for
   * their own reason, and each failure belongs beside the control that caused it: a wrong password
   * under the field it was typed in, a refused switch under the row it was asked from. So these
   * answer with what happened rather than storing one failure for the screen to guess the owner of —
   * which is what the gate above does, and why it is a separate thing.
   */
  type VaultOutcome<T> =
    | { readonly ok: true; readonly value: T }
    | { readonly ok: false; readonly failure: ShellError };

  /** Asks for the archives again, and keeps what came back. */
  async function loadArchives(): Promise<void> {
    archivesBusy.value = true;
    archivesFailure.value = null;
    try {
      archives.value = await backend.vault.archives();
    } catch (error) {
      archivesFailure.value = asShellError(error, "vault_archives");
    } finally {
      archivesBusy.value = false;
    }
  }

  /**
   * Records a failure of looking at the archives, for the one action that has nowhere else to put
   * it: opening the folder is about the archive list as a whole rather than about one row, so its
   * refusal is drawn at the top of that list with the loading and the listing failures.
   */
  function reportArchivesFailure(failure: ShellError | null): void {
    archivesFailure.value = failure;
  }

  /** Whether a password opens one archive. Nothing about the live vault changes either way. */
  async function verifyArchive(name: string, password: string): Promise<VaultOutcome<void>> {
    try {
      await backend.vault.verifyArchive(name, password);
      return { ok: true, value: undefined };
    } catch (error) {
      return { ok: false, failure: asShellError(error, "vault_archive_verify") };
    }
  }

  /**
   * Adds an archived library's sessions, groups and logins to the open vault.
   *
   * Nothing already here is replaced — the rules are the ones an imported file follows — so the
   * counts that come back are what the person is told, missing keys and dropped hops included.
   */
  async function importArchive(name: string, password: string): Promise<VaultOutcome<SessionImportSummary>> {
    try {
      const summary = await backend.vault.importArchive(name, password);
      await loadLibrary();
      return { ok: true, value: summary };
    } catch (error) {
      return { ok: false, failure: asShellError(error, "vault_archive_import") };
    }
  }

  /**
   * Makes an archive the live vault, and archives the one that was live.
   *
   * The gate goes back to `locked` — the file it was answering from is not the one in place any
   * more — so the next thing on screen is a password field, for the library that just arrived.
   */
  async function activateArchive(name: string): Promise<VaultOutcome<string | null>> {
    try {
      const replaced = await backend.vault.activateArchive(name);
      state.value = await backend.vault.state();
      return { ok: true, value: replaced };
    } catch (error) {
      return { ok: false, failure: asShellError(error, "vault_archive_activate") };
    }
  }

  /** Opens the folder the vault and its archives live in, in the system's file manager. */
  async function revealFolder(): Promise<VaultOutcome<void>> {
    try {
      await backend.vault.revealFolder();
      return { ok: true, value: undefined };
    } catch (error) {
      return { ok: false, failure: asShellError(error, "vault_reveal_folder") };
    }
  }

  /** Re-encrypts the vault under a new password, saying nothing about what was typed. */
  async function changePassword(current: string, next: string): Promise<VaultOutcome<void>> {
    try {
      await backend.vault.changePassword(current, next);
      return { ok: true, value: undefined };
    } catch (error) {
      return { ok: false, failure: asShellError(error, "vault_change_password") };
    }
  }

  return {
    state,
    busy,
    failure,
    failedOperation,
    unlocked,
    setAsideName,
    archives,
    archivesBusy,
    archivesFailure,
    initialize,
    create,
    unlock,
    lock,
    markLocked,
    dismissFailure,
    setAside,
    loadArchives,
    reportArchivesFailure,
    verifyArchive,
    importArchive,
    activateArchive,
    revealFolder,
    changePassword,
  };
});
