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
 */

import { defineStore } from "pinia";
import { computed, ref } from "vue";

import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import { useSessionsStore } from "@/stores/sessions";
import { useRecentSessionsStore } from "@/stores/recentSessions";
import type { VaultState } from "@/types/session";

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

  return { state, busy, failure, failedOperation, unlocked, setAsideName, initialize, create, unlock, lock, setAside };
});
