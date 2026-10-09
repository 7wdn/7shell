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
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import MonitorPanel from "@/components/monitor/MonitorPanel.vue";
import HostKeyPrompt from "@/components/sessions/HostKeyPrompt.vue";
import ImportPassphraseDialog from "@/components/sessions/ImportPassphraseDialog.vue";
import SessionDialog from "@/components/sessions/SessionDialog.vue";
import AppSidebar from "@/components/shell/AppSidebar.vue";
import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import ContextMenu from "@/components/shell/ContextMenu.vue";
import TitleBar from "@/components/shell/TitleBar.vue";
import ToastViewport from "@/components/shell/ToastViewport.vue";
import Wallpaper from "@/components/shell/Wallpaper.vue";
import UnlockScreen from "@/components/vault/UnlockScreen.vue";
import HomeView from "@/components/views/HomeView.vue";
import WorkspaceTabs from "@/components/workspace/WorkspaceTabs.vue";
import { describeShellError } from "@/i18n/errors";
import { useTransferNotices } from "@/composables/useTransferNotices";
import { isTextEntry } from "@/lib/menu/editable";
import { useAppStore } from "@/stores/app";
import { useBatchStore } from "@/stores/batch";
import { usePreferencesStore } from "@/stores/preferences";
import { useFilesStore } from "@/stores/files";
import { backend } from "@/lib/backend/api";
import { useSessionEditorStore } from "@/stores/sessionEditor";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalsStore } from "@/stores/terminals";
import { useVaultStore } from "@/stores/vault";

const { t } = useI18n();
const files = useFilesStore();
const app = useAppStore();
const preferences = usePreferencesStore();
const terminals = useTerminalsStore();
const batch = useBatchStore();
const editor = useSessionEditorStore();
const library = useSessionsStore();
const vault = useVaultStore();

// Watches for transfers that failed while their list was not on screen. Registered here because the
// question it answers — what is on screen — is one only the shell can answer.
useTransferNotices();

let stopFollowingSystem: (() => void) | undefined;
let stopTerminalEvents: (() => void) | undefined;
let stopDragEvents: (() => void) | undefined;
let stopDropEvents: (() => void) | undefined;
let stopPaste: (() => void) | undefined;
let stopContextMenu: (() => void) | undefined;
let stopQuitRequested: (() => void) | undefined;

const backendFailure = computed(() => (app.failure ? describeShellError(translate, app.failure) : null));

/**
 * What the upload that stopped says, or nothing when none did.
 *
 * The question is the store's — an upload can be started from a panel's button, a drop on the
 * window, or a paste, and all three have to be answered once — and this is where it is drawn,
 * because a question about where somebody's bytes are going belongs to the frame rather than to
 * whichever panel happens to be on screen.
 */
const uploadQuestion = computed(() => {
  const asked = files.uploadConflict;
  if (!asked) {
    return null;
  }
  return {
    message: t("files.uploadConflict.message"),
    details: asked.names,
    overwrite: t("files.uploadConflict.overwrite"),
    skip: asked.skippable ? t("files.uploadConflict.skip") : undefined,
  };
});

/** Whether the question about quitting is on screen. */
const quitting = ref(false);

/**
 * What a close right now would throw away, in the order somebody would care about it.
 *
 * An empty list is the answer to "may the window just close?", and it is what arms the shell's quit
 * guard: armed, the close comes back as a question instead of ending the process. Reading it from
 * two facts rather than guessing is the point — the shell can see neither an editor's buffer nor a
 * transfer's progress on its own.
 */
const quitReasons = computed(() => {
  const reasons: string[] = [];
  if (files.unsavedEditors.length > 0) {
    reasons.push(t("shell.quit.unsaved"));
  }
  if (files.transfers.some((entry) => entry.state.kind === "running")) {
    reasons.push(t("shell.quit.transfers"));
  }
  return reasons;
});

/**
 * A close that was held back.
 *
 * The guard is the shell's, so the window never refuses to close on its own — it asks. Nothing to
 * lose closes straight away, which is the path every ordinary quit takes.
 */
async function askedToQuit(): Promise<void> {
  if (quitReasons.value.length === 0) {
    await quitNow();
    return;
  }
  quitting.value = true;
}

/**
 * Closing for real, on the way out of the question above.
 *
 * The window's own close *asks* — the shell may put it away instead, or ask about unsaved work — so
 * this is the answered close: one call that tells the shell the question is settled and closes the
 * window. It used to be a disarmed guard followed by a plain close, which no longer means quit now
 * that a close can mean "hide".
 */
async function quitNow(): Promise<void> {
  quitting.value = false;
  try {
    await backend.window.quit();
  } catch {
    // A browser has no window to close, and a window that will not close is not something this
    // side can do anything about.
  }
}

/**
 * Keeps the shell's guard in step with what is at stake.
 *
 * Armed only while there is something to lose, so an ordinary close is never held up — and so a
 * frontend that has died, leaving the flag where it was, is the only thing a stuck window could be
 * blamed on.
 */
watch(
  () => quitReasons.value.length > 0,
  (armed) => {
    void backend.window.quitGuard(armed).catch((error: unknown) => {
      // Nothing to guard in a browser, and a shell that will not take the flag is worth saying out
      // loud: the window would close over unsaved text without asking.
      console.warn("the quit guard could not be set", error);
    });
  },
  { immediate: true },
);

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
  // Registered here rather than by the panel: a run outlives a look at another tab, and its rows
  // have to keep arriving while the person is somewhere else.
  batch.listen();
  stopQuitRequested = await backend.events.onQuitRequested(() => void askedToQuit());
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
  stopQuitRequested?.();
});
</script>

<template>
  <div class="flex h-full flex-col overflow-hidden">
    <!--
      The picture, first in the markup and last in the stack: behind every panel and in front of the
      window's own painted background, which is what lets a translucent surface show it through.
    -->
    <Wallpaper />

    <TitleBar />

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

    <!--
      The three panels of the working window — the library, the workspace and the monitor — and the
      gutter between them. Which shape that gutter takes is the interface layout's business and is
      drawn in `globals.css` from the document's attribute: it is decoration, and a component that
      had to ask which one is on would be the fourth place the same answer is written down.
    -->
    <div v-else data-panel-gutter class="relative flex min-h-0 flex-1">
      <!--
        Put away rather than unmounted, and the state is the settings': somebody who works with the
        list out of the way is somebody who wants it out of the way the next time the window opens,
        and the way back lives in the title bar where the rest of the window's own controls are.
      -->
      <AppSidebar v-if="preferences.sidebarOpen" @collapse="preferences.setSidebarOpen(false)" />
      <main data-panel-frame class="flex min-w-0 flex-1 flex-col overflow-hidden">
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

    <!--
      The session editor, drawn by the frame rather than by the sidebar it used to live in: it is
      opened from the sidebar's button, from a row's menu and from the home page's shortcuts, and
      the last of those has to work while the sidebar is put away.
    -->
    <SessionDialog :open="editor.open" :session="editor.editing" @update:open="editor.setOpen" />

    <!--
      The question a sealed export asks before it can be read, drawn here for the same reason and in
      the same place as the editor: the import is parked on the answer, and the panel that started
      it may be put away while the question is on screen.
    -->
    <ImportPassphraseDialog :question="library.importQuestion" @answer="library.answerImportPassphrase" />

    <!--
      An upload that would replace a name already on the far side. Answering "no" tells the backend
      to forget the files it was holding, so a cancelled question leaves nothing behind.
    -->
    <ConfirmDialog
      v-if="uploadQuestion"
      :open="true"
      :title="t('files.uploadConflict.title')"
      :message="uploadQuestion.message"
      :details="uploadQuestion.details"
      :discard-label="uploadQuestion.skip"
      :confirm-label="uploadQuestion.overwrite"
      danger
      @confirm="files.answerUpload('overwrite')"
      @discard="files.answerUpload('skip')"
      @update:open="files.answerUpload('cancel')"
    />

    <!--
      A close that was held back because something would be lost. Saying "no" leaves everything as
      it was, which is the point: the window is still there and so is the work.
    -->
    <ConfirmDialog
      v-if="quitting"
      :open="true"
      :title="t('shell.quit.title')"
      :message="t('shell.quit.message')"
      :details="quitReasons"
      :confirm-label="t('shell.quit.confirm')"
      danger
      @confirm="quitNow()"
      @update:open="quitting = false"
    />

    <div
      v-if="dragging && dropTarget"
      class="pointer-events-none absolute inset-0 z-40 grid place-items-center bg-background/70"
    >
      <p class="rounded-lg border-2 border-dashed border-primary/60 bg-card px-4 py-3 text-sm">
        {{ t("files.drop.hint", { path: dropTarget.path }) }}
      </p>
    </div>

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
