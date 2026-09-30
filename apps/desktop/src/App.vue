<script setup lang="ts">
/**
 * The application shell: title bar, sidebar, content.
 *
 * The shell decides nothing about what a feature looks like; it owns the frame around them and
 * the one place a backend failure is shown, so a failure is visible wherever the user happens
 * to be. It also owns the transient surface — the toast viewport — because a message about an
 * action belongs to no single feature, and every feature has to be able to raise one.
 *
 * The gate sits below the title bar and above everything else, including the sidebar: nothing
 * that reads a saved credential is drawn before the vault is open. The subscription to terminal
 * events is made here, once, before the first terminal can exist.
 */
import { RotateCw } from "@lucide/vue";
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import HostKeyPrompt from "@/components/sessions/HostKeyPrompt.vue";
import SettingsDialog from "@/components/settings/SettingsDialog.vue";
import AppSidebar from "@/components/shell/AppSidebar.vue";
import TitleBar from "@/components/shell/TitleBar.vue";
import ToastViewport from "@/components/shell/ToastViewport.vue";
import UnlockScreen from "@/components/vault/UnlockScreen.vue";
import HomeView from "@/components/views/HomeView.vue";
import WorkspaceTabs from "@/components/workspace/WorkspaceTabs.vue";
import { describeShellError } from "@/i18n/errors";
import { useTransferNotices } from "@/composables/useTransferNotices";
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

// Watches for transfers that failed while their list was not on screen. Registered here because the
// question it answers — what is on screen — is one only the shell can answer.
useTransferNotices();

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

/**
 * Tells the shell there is a frame on screen, which is when it shows the window.
 *
 * The window is created hidden, so this is what puts it on screen, and it is deliberately late in
 * the startup: the theme, the language and the platform facts are settled by the time it runs, so
 * the first thing a person sees is the interface it is going to keep, not a frame of one that is
 * about to change. The two animation frames are what make "drawn" true rather than "almost drawn" —
 * the class that carries the theme is only on screen once a frame has been painted with it.
 *
 * Nothing depends on this succeeding. A shell that never hears it shows the window on its own
 * deadline instead, which is also the only thing that happens in a browser, where there is no
 * window to show.
 */
async function reportFirstFrame(): Promise<void> {
  await nextTick();
  await new Promise<void>((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
  });

  try {
    await backend.appReady();
  } catch (error) {
    console.warn("the shell was not told the first frame is drawn", error);
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
  await reportFirstFrame();
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

    <!--
      Outside every feature and outside the gate: a toast is about something that already
      happened, and it has to be drawable wherever the person is when it does — including
      while they are still on the gate screen, which is why it is not inside the branch below.
    -->
    <ToastViewport />
  </div>
</template>
