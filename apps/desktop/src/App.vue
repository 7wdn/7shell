<script setup lang="ts">
/**
 * The application shell: title bar, sidebar, content.
 *
 * The shell decides nothing about what a feature looks like; it owns the frame around them and
 * the one place a backend failure is shown, so a failure is visible wherever the user happens
 * to be.
 *
 * The gate sits below the title bar and above everything else, including the sidebar: nothing
 * that reads a saved credential is drawn before the vault is open. The subscription to terminal
 * events is made here, once, before the first terminal can exist.
 */
import { RotateCw } from "@lucide/vue";
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import HostKeyPrompt from "@/components/sessions/HostKeyPrompt.vue";
import SettingsDialog from "@/components/settings/SettingsDialog.vue";
import AppSidebar from "@/components/shell/AppSidebar.vue";
import TitleBar from "@/components/shell/TitleBar.vue";
import UnlockScreen from "@/components/vault/UnlockScreen.vue";
import HomeView from "@/components/views/HomeView.vue";
import WorkspaceTabs from "@/components/workspace/WorkspaceTabs.vue";
import { describeShellError } from "@/i18n/errors";
import { useAppStore } from "@/stores/app";
import { usePreferencesStore } from "@/stores/preferences";
import { useFilesStore } from "@/stores/files";
import { backend } from "@/lib/backend/api";
import { useTerminalsStore } from "@/stores/terminals";
import { useVaultStore } from "@/stores/vault";

const { t } = useI18n();
const files = useFilesStore();
const app = useAppStore();
const preferences = usePreferencesStore();
const terminals = useTerminalsStore();
const vault = useVaultStore();

const settingsOpen = ref(false);
let stopFollowingSystem: (() => void) | undefined;
let stopTerminalEvents: (() => void) | undefined;
let stopDragEvents: (() => void) | undefined;
let stopDropEvents: (() => void) | undefined;

const backendFailure = computed(() => (app.failure ? describeShellError(translate, app.failure) : null));

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

/** True while a drag is over the window, so the hint can say where the files would land. */
const dragging = ref(false);

/** The browser on screen, if one is: a drop has nowhere to land if the answer is no. */
const dropTarget = computed(() => {
  const tab = terminals.tabs.find((entry) => entry.id === terminals.activeTabId);
  if (!tab || tab.kind !== "files" || !tab.handle) {
    return null;
  }
  const browser = files.browser(tab.handle);
  return browser ? { handle: tab.handle, path: browser.path } : null;
});

/**
 * A drop, once it lands.
 *
 * The token stands for paths the interface never saw; all it contributes is where the files were
 * aimed, which is the directory the person was looking at.
 */
async function dropped(payload: { pending?: { token: string }; error?: unknown }): Promise<void> {
  const target = dropTarget.value;
  if (!target) {
    return;
  }

  if (payload.pending) {
    await files.uploadDropped(target.handle, target.path, payload.pending.token);
    return;
  }

  if (payload.error !== undefined) {
    files.reportFailure(target.handle, payload.error);
  }
}

onMounted(async () => {
  stopFollowingSystem = preferences.initialize();
  stopDragEvents = await backend.events.onFilesDrag((payload) => {
    dragging.value = payload.hovering;
  });
  stopDropEvents = await backend.events.onFilesDropped((payload) => void dropped(payload));
  stopTerminalEvents = terminals.initialize();
  await preferences.load();
  await app.load();
  await vault.initialize();
});

onBeforeUnmount(() => {
  stopFollowingSystem?.();
  stopTerminalEvents?.();
  stopDragEvents?.();
  stopDropEvents?.();
});
</script>

<template>
  <div class="flex h-full flex-col overflow-hidden">
    <TitleBar @open-settings="settingsOpen = true" />

    <div
      v-if="backendFailure"
      class="flex items-start gap-3 border-b border-destructive/40 bg-destructive/10 px-4 py-2"
    >
      <div class="min-w-0 flex-1">
        <p class="text-xs">{{ backendFailure.message }}</p>
        <p class="font-mono text-[10px] text-muted-foreground">{{ backendFailure.code }}</p>
      </div>
      <button
        type="button"
        class="flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-xs transition-colors hover:bg-accent"
        @click="app.load()"
      >
        <RotateCw :size="12" />
        {{ t("common.action.retry") }}
      </button>
    </div>

    <UnlockScreen v-if="!vault.unlocked" />

    <div v-else class="relative flex min-h-0 flex-1">
      <AppSidebar />
      <main class="flex min-w-0 flex-1 flex-col overflow-hidden">
        <WorkspaceTabs v-if="terminals.tabs.length > 0" />
        <div v-else class="min-h-0 flex-1 overflow-auto">
          <HomeView />
        </div>
      </main>
    </div>

    <HostKeyPrompt
      v-if="terminals.decision"
      :decision="terminals.decision"
      @trust="terminals.confirmTrust()"
      @replace="terminals.confirmReplace()"
      @cancel="terminals.dismiss()"
    />

    <div
      v-if="dragging && dropTarget"
      class="pointer-events-none absolute inset-0 z-40 grid place-items-center bg-background/70"
    >
      <p class="rounded-lg border-2 border-dashed border-primary/60 bg-card px-4 py-3 text-sm">
        {{ t("files.drop.hint", { path: dropTarget.path }) }}
      </p>
    </div>

    <SettingsDialog :open="settingsOpen" @update:open="settingsOpen = $event" />
  </div>
</template>
