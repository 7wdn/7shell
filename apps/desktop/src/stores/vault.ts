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
 */

import { defineStore } from "pinia";
import { computed, ref } from "vue";

import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import { useSessionsStore } from "@/stores/sessions";
import type { VaultState } from "@/types/session";

/** `unknown` until the backend has answered; the gate draws nothing else until then. */
export type GateState = VaultState | "unknown";

export const useVaultStore = defineStore("vault", () => {
  const state = ref<GateState>("unknown");
  const busy = ref(false);
  const failure = ref<ShellError | null>(null);

  const unlocked = computed(() => state.value === "unlocked");

  /** Asks the backend where the vault stands, and loads the library when it is already open. */
  async function initialize(): Promise<void> {
    failure.value = null;
    try {
      state.value = await backend.vault.state();
    } catch (error) {
      failure.value = asShellError(error, "vault_state");
      return;
    }
    if (state.value === "unlocked") {
      await useSessionsStore().load();
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

  async function run(operation: string, action: () => Promise<VaultState>): Promise<boolean> {
    busy.value = true;
    failure.value = null;
    try {
      state.value = await action();
      await useSessionsStore().load();
      return true;
    } catch (error) {
      failure.value = asShellError(error, operation);
      return false;
    } finally {
      busy.value = false;
    }
  }

  /** Locks the vault. Open terminals are deliberately left running: they are already inside. */
  async function lock(): Promise<void> {
    failure.value = null;
    try {
      state.value = await backend.vault.lock();
    } catch (error) {
      failure.value = asShellError(error, "vault_lock");
    }
  }

  return { state, busy, failure, unlocked, initialize, create, unlock, lock };
});
