import { defineStore } from "pinia";
import { computed, ref } from "vue";

import { backend } from "@/lib/backend/api";
import { type ShellError, normalizeShellError } from "@/lib/backend/errors";
import type { AppInfo } from "@/types/app";

export type BackendStatus = "loading" | "ready" | "failed";

/**
 * What the interface knows about the application it is running inside.
 *
 * Only the transport is faked in tests; everything else here is what the real window
 * does, so the states and the failure path are exercised by the test suite rather than
 * discovered in a release.
 */
export const useAppStore = defineStore("app", () => {
  const info = ref<AppInfo | null>(null);
  const status = ref<BackendStatus>("loading");
  const failure = ref<ShellError | null>(null);

  async function load(): Promise<void> {
    status.value = "loading";
    failure.value = null;
    try {
      info.value = await backend.appInfo();
      status.value = "ready";
    } catch (error) {
      failure.value = normalizeShellError(error, "app_info");
      status.value = "failed";
    }
  }

  const platform = computed(() => (info.value ? `${info.value.os} ${info.value.arch}` : ""));

  return { info, status, failure, platform, load };
});
