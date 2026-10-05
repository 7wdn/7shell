/**
 * The keys this device has been told to trust.
 *
 * A list somebody can read and revoke, which is the other half of "a host key is verified before
 * anything is sent": a key that was trusted once — during a hurried first connection, or for a
 * machine that has since been rebuilt — was until now impossible to look at or take back from the
 * interface.
 *
 * Trusting happens where the decision is raised (the prompt in front of a connection); this store
 * only reads, and forgets.
 */

import { ref } from "vue";
import { defineStore } from "pinia";

import { backend } from "@/lib/backend/api";
import { asShellError, type ShellError } from "@/lib/backend/errors";
import type { KnownHostEntry } from "@/types/session";

export const useKnownHostsStore = defineStore("knownHosts", () => {
  const entries = ref<KnownHostEntry[]>([]);
  const loaded = ref(false);
  const failure = ref<ShellError | null>(null);

  async function load(): Promise<void> {
    try {
      entries.value = await backend.knownHosts.list();
      failure.value = null;
      loaded.value = true;
    } catch (error) {
      failure.value = asShellError(error, "known_hosts_list");
    }
  }

  /**
   * Forgets one key.
   *
   * The next connection to that host asks again, which is the point: this is how somebody answers
   * "I no longer believe that key" after something looked wrong.
   */
  async function forget(host: string, port: number): Promise<void> {
    try {
      await backend.knownHosts.forget(host, port);
      await load();
    } catch (error) {
      failure.value = asShellError(error, "known_host_forget");
    }
  }

  return { entries, loaded, failure, load, forget };
});
