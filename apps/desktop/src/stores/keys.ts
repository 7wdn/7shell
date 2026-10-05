/**
 * The keys this device holds.
 *
 * The material never comes near this store: what a key is, to the interface, is a name, an
 * algorithm, a fingerprint and whether a passphrase is held. Importing asks Rust for a file — the
 * dialog is opened there, which is why no path is ever named here — and forgetting removes the
 * material, after which any session that used it says so when it tries to connect.
 */

import { ref } from "vue";
import { defineStore } from "pinia";

import { backend } from "@/lib/backend/api";
import { asShellError, type ShellError } from "@/lib/backend/errors";
import type { KeyInfo } from "@/types/session";

export const useKeysStore = defineStore("keys", () => {
  const entries = ref<KeyInfo[]>([]);
  const loaded = ref(false);
  const failure = ref<ShellError | null>(null);
  /** True while the file dialog is open, so the button can say it is waiting for a person. */
  const importing = ref(false);

  async function load(): Promise<void> {
    try {
      entries.value = await backend.keys.list();
      failure.value = null;
      loaded.value = true;
    } catch (error) {
      failure.value = asShellError(error, "keys_list");
    }
  }

  /**
   * Asks for a key file and stores it.
   *
   * A dismissed dialog is not a failure and not a change: `undefined` means nobody chose anything.
   */
  async function importKey(title: string, passphrase?: string): Promise<void> {
    importing.value = true;
    try {
      const imported = await backend.keys.import(title, passphrase);
      if (imported) {
        await load();
      }
    } catch (error) {
      failure.value = asShellError(error, "key_import");
    } finally {
      importing.value = false;
    }
  }

  async function forget(keyRef: string): Promise<void> {
    try {
      await backend.keys.forget(keyRef);
      await load();
    } catch (error) {
      failure.value = asShellError(error, "key_forget");
    }
  }

  return { entries, loaded, failure, importing, load, importKey, forget };
});
