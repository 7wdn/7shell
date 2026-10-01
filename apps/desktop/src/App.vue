<script setup lang="ts">
/**
 * The application shell: title bar, sidebar, content.
 *
 * The shell decides nothing about what a feature looks like; it owns the frame around them and
 * the one place a backend failure is shown, so a failure is visible wherever the user happens
 * to be. It also owns the transient surface — the toast viewport — because a message about an
 * action belongs to no single feature, and every feature has to be able to raise one.
 *
 * It also owns where a file coming from outside the window lands: the drop overlay and the paste
 * shortcut both ask the same question — which browser is on screen — and that is a question only
 * the shell can answer, because the panes behind other tabs are mounted too and would each answer
 * "me".
 *
 * The host monitor is mounted here too, in the same branch as the workspace and on the far side of
 * it: it describes whichever connection is on screen rather than belonging to a tab, and mounting
 * it is what starts the sampling — so a hidden monitor runs nothing at all.
 *
 * The gate sits below the title bar and above everything else, including the sidebar: nothing
 * that reads a saved credential is drawn before the vault is open. The subscription to terminal
 * events is made here, once, before the first terminal can exist.
 */
import { RotateCw } from "@lucide/vue";
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import MonitorPanel from "@/components/monitor/MonitorPanel.vue";
import HostKeyPrompt from "@/components/sessions/HostKeyPrompt.vue";
import SettingsDialog from "@/components/settings/SettingsDialog.vue";
import AppSidebar from "@/components/shell/AppSidebar.vue";
import ContextMenu from "@/components/shell/ContextMenu.vue";
import TitleBar from "@/components/shell/TitleBar.vue";
import ToastViewport from "@/components/shell/ToastViewport.vue";
import UnlockScreen from "@/components/vault/UnlockScreen.vue";
import HomeView from "@/components/views/HomeView.vue";
import WorkspaceTabs from "@/components/workspace/WorkspaceTabs.vue";
import { describeShellError } from "@/i18n/errors";
import { useTransferNotices } from "@/composables/useTransferNotices";
import { isTextEntry } from "@/lib/menu/editable";
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
let stopPaste: (() => void) | undefined;
let stopContextMenu: (() => void) | undefined;

const backendFailure = computed(() => (app.failure ? describeShellError(translate, app.failure) : null));

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

/** True while a drag is over the window, so the hint can say where the files would land. */
const dragging = ref(false);

/**
 * The browser on screen, if one is: a drop has nowhere to land if the answer is no.
 *
 * Which browser that is — a file tab's own, or the one in the panel under a terminal — is the
 * workspace store's answer, because it is the store that knows a panel is drawing a browser at all.
 */
const dropTarget = computed(() => {
  const tab = terminals.activeTab;
  const handle = tab ? terminals.browserHandleOf(tab) : null;
  const browser = handle === null ? null : files.browser(handle);
  return handle === null || !browser ? null : { handle, path: browser.path };
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
    await files.uploadDropped(target.handle, target.path, payload.pending.token, preferences.packedTransfer);
    return;
  }

  if (payload.error !== undefined) {
    files.reportFailure(target.handle, payload.error);
  }
}

/**
 * The same upload, from the keyboard.
 *
 * Ctrl+V is the first thing somebody tries with a file on the clipboard, and it aims at the same
 * directory a drop would. It is registered here rather than in the browser panel for the reason
 * this file exists at all: a panel cannot answer "am I the one on screen" — every browser is
 * mounted, including the ones behind other tabs — and a shortcut each of them registered would
 * upload one clipboard into all of them at once.
 *
 * Two things are deliberately left alone: a paste aimed at a text field, which is a text paste and
 * not an upload, and a platform whose clipboard this build cannot read — there the backend refuses
 * with a reason and the panel shows it, instead of the shortcut seeming dead.
 */
function pasteInto(event: KeyboardEvent): void {
  if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== "v") {
    return;
  }

  const focused = event.target as HTMLElement | null;
  if (focused && (focused.tagName === "INPUT" || focused.tagName === "TEXTAREA")) {
    return;
  }

  const target = dropTarget.value;
  if (!target) {
    return;
  }

  event.preventDefault();
  void files.uploadFromClipboard(target.handle, target.path, preferences.packedTransfer);
}

/**
 * The right-click that has no menu of its own.
 *
 * The browser's menu is a browser's menu — "Back", "Reload", "Save page as" — and this is not a
 * browser, so it is cancelled everywhere. The regions that do have something to offer cancel it
 * themselves, in the same gesture that opens their menu, which is why this only has to be a backstop.
 *
 * Text fields are left alone. There the platform's own menu is cut, copy, paste and the IME, already
 * in the person's language, and nothing this application could draw would be better than it.
 *
 * Captured rather than bubbled: the decision is the shell's, and it is made before anything the
 * event passes through on the way down gets to act on it.
 */
function suppressDefaultMenu(event: MouseEvent): void {
  if (isTextEntry(event.target)) {
    return;
  }
  event.preventDefault();
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
  window.addEventListener("keydown", pasteInto);
  stopPaste = () => window.removeEventListener("keydown", pasteInto);
  window.addEventListener("contextmenu", suppressDefaultMenu, { capture: true });
  stopContextMenu = () => window.removeEventListener("contextmenu", suppressDefaultMenu, { capture: true });
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
  stopPaste?.();
  stopContextMenu?.();
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

      <!--
        The monitor belongs to the frame rather than to a tab: it describes whichever connection is
        on screen, and it is mounted here — below the gate, first in the branch above — so that the
        panel appearing and disappearing is also what starts and stops the sampling. Nothing is
        asked of a host while the setting is off.
      -->
      <MonitorPanel v-if="preferences.monitorEnabled" />
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
      The context menu, like the toast, belongs to the frame rather than to any feature: it is opened
      from wherever somebody right-clicked, and it has to be drawable there whatever else is on
      screen. The features contribute its contents and nothing else.
    -->
    <ContextMenu />

    <!--
      Outside every feature and outside the gate: a toast is about something that already
      happened, and it has to be drawable wherever the person is when it does — including
      while they are still on the gate screen, which is why it is not inside the branch below.
    -->
    <ToastViewport />
  </div>
</template>
