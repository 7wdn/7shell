/**
 * The keys this device holds.
 *
 * The material never comes near this store: what a key is, to the interface, is a name, an
 * algorithm, a fingerprint and whether a passphrase is held. Importing asks Rust for a file — the
 * dialog is opened there, which is why no path is ever named here — making one asks Rust to make it,
 * and forgetting removes the material, after which any session that used it says so when it tries to
 * connect.
 *
 * One key-shaped thing does come back, and only from a key that was just made: its **public** half,
 * as the single line that goes into `authorized_keys`. That is not a secret — putting it on machines
 * is what it is for — and it is what makes "make a key" worth having in a client whose whole point is
 * that private keys stay in the vault.
 */

import { ref } from "vue";
import { defineStore } from "pinia";

import { backend } from "@/lib/backend/api";
import { asShellError, type ShellError } from "@/lib/backend/errors";
import type { CreatedKey, KeyAlgorithm, KeyInfo } from "@/types/session";

export const useKeysStore = defineStore("keys", () => {
  const entries = ref<KeyInfo[]>([]);
  const loaded = ref(false);
  const failure = ref<ShellError | null>(null);
  /** True while the file dialog is open, so the button can say it is waiting for a person. */
  const importing = ref(false);
  /** True while a key is being made: seconds of arithmetic for RSA, and the button says so. */
  const making = ref(false);

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

  /**
   * Makes a key, keeps it in the vault, and answers with what was made and the line that goes on a
   * server.
   *
   * `null` is a refusal already in `failure`: nothing was made, and nothing is added to the list.
   */
  async function generate(
    algorithm: KeyAlgorithm,
    comment: string,
    passphrase: string | null,
  ): Promise<CreatedKey | null> {
    failure.value = null;
    making.value = true;
    try {
      const created = await backend.keys.generate(algorithm, comment, passphrase);
      await load();
      return created;
    } catch (error) {
      failure.value = asShellError(error, "key_generate");
      return null;
    } finally {
      making.value = false;
    }
  }

  /** The public line of a key that is already stored. `null` is a refusal, already in `failure`. */
  async function publicKey(keyRef: string): Promise<string | null> {
    failure.value = null;
    try {
      return await backend.keys.publicKey(keyRef);
    } catch (error) {
      failure.value = asShellError(error, "key_public");
      return null;
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

  return { entries, loaded, failure, importing, making, load, importKey, generate, publicKey, forget };
});
