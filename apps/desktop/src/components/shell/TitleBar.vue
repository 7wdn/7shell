<script setup lang="ts">
/**
 * The title bar, which the interface draws itself: the window is undecorated on Windows and
 * Linux, so dragging, the window buttons and the top strip of pixels all belong to the app.
 *
 * `data-tauri-drag-region` is what makes dragging work, and it is deliberately on the bar and
 * the empty space in the middle rather than on the whole subtree: a bare attribute means "only
 * clicks on this element drag", and interactive elements are skipped by the built-in handler.
 * Double-clicking a drag region toggles maximize — also built in, which is why there is no
 * double-click handler here (adding one would toggle twice and appear to do nothing).
 *
 * On macOS the bar keeps its native traffic lights, so the content is inset to make room.
 */
import { ArrowUpDown, ChevronsRight, ListChecks, Moon, Pin, PinOff, Settings, Sun, SunMoon } from "@lucide/vue";
import { computed, markRaw } from "vue";
import { useI18n } from "vue-i18n";

import AppMark from "@/components/brand/AppMark.vue";
import UpdateEntry from "@/components/shell/UpdateEntry.vue";
import WindowControls from "@/components/shell/WindowControls.vue";
import { useWindowControls } from "@/composables/useWindowControls";
import { useAppStore } from "@/stores/app";
import { useFilesStore } from "@/stores/files";
import { usePreferencesStore } from "@/stores/preferences";
import { useTerminalsStore } from "@/stores/terminals";
import { useUpdatesStore } from "@/stores/updates";

const { t } = useI18n();
const app = useAppStore();
const preferences = usePreferencesStore();
const terminals = useTerminalsStore();
const files = useFilesStore();
const updates = useUpdatesStore();

const { isMac, showControls, isMaximized, isAlwaysOnTop, minimize, toggleMaximize, toggleAlwaysOnTop, close } =
  useWindowControls(() => app.info?.os ?? "");

const version = computed(() => (app.info ? `v${app.info.version}` : ""));

/**
 * One button covers all three modes, cycling in the order the settings page lists them.
 *
 * Following the system is drawn as a sun and a moon together rather than as the display it is
 * currently reading: the two things the mode can be at once, which is what "whichever the system
 * says" looks like — and it is the same mark the settings page puts on that choice, so the two
 * places that name the mode name it the same way.
 */
const modeIcon = computed(() =>
  markRaw(preferences.themeMode === "system" ? SunMoon : preferences.themeMode === "dark" ? Moon : Sun),
);

const modeLabel = computed(() => {
  switch (preferences.themeMode) {
    case "dark":
      return t("settings.theme.dark");
    case "light":
      return t("settings.theme.light");
    default:
      return t("settings.theme.system");
  }
});

async function cycleThemeMode(): Promise<void> {
  const next = preferences.themeMode === "system" ? "dark" : preferences.themeMode === "dark" ? "light" : "system";
  await preferences.setThemeMode(next);
}

/**
 * Whether the settings page is the tab on screen.
 *
 * The gear is a way back to a page that may already be open rather than a switch for a dialog, so
 * it says when it is the one that is showing — otherwise pressing it over a settings page looks
 * like nothing happened.
 */
const settingsShowing = computed(() => terminals.activeTab?.kind === "settings");

/** The same "is it already the one on screen" answer, for the batch panel's button. */
const batchShowing = computed(() => terminals.activeTab?.kind === "batch");

/**
 * What the transfers button is carrying: how many are still moving, and whether anything stopped
 * badly.
 *
 * On the button rather than only in the panel, because the panel is not always on screen — and the
 * question somebody has while it is away is exactly "is anything happening, and did anything go
 * wrong", which is a count and a colour rather than a list.
 */
const movingTransfers = computed(
  () => files.transfers.filter((entry) => entry.state.kind === "running" || entry.state.kind === "paused").length,
);
const failedTransfers = computed(() => files.transfers.some((entry) => entry.state.kind === "failed"));
</script>

<template>
  <header
    data-tauri-drag-region
    class="flex h-10 shrink-0 select-none items-center gap-2.5 border-b border-border bg-card/50"
    :class="isMac ? 'pl-20' : 'pl-3'"
  >
    <AppMark :size="17" />
    <span class="text-sm font-medium tracking-tight">7Shell</span>
    <span
      v-if="version"
      class="rounded-sm border border-border px-1.5 py-px font-mono text-[10px] leading-4 text-muted-foreground"
    >
      {{ version }}
    </span>

    <!--
      The way back to the sidebar, drawn only while it is away: the window's own frame is where a
      person looks for the panels it is made of, and a control that is there all the time would be
      one more thing in the way of the ones that are used.
    -->
    <button
      v-if="!preferences.sidebarOpen"
      type="button"
      class="grid size-8 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      :title="t('shell.sidebar.expand')"
      :aria-label="t('shell.sidebar.expand')"
      @click="preferences.setSidebarOpen(true)"
    >
      <ChevronsRight :size="15" />
    </button>

    <div data-tauri-drag-region class="h-full min-w-4 flex-1"></div>

    <!--
      The release channel first, then the pin: what is published, and whether this window stays above
      the others. Both are about the application rather than about anything open in it, which is why
      they stand at the edge of the bar rather than beside the pages they would open.
    -->
    <UpdateEntry />

    <!--
      Keeping the window on top. The window's own state, drawn from what it says — and drawn the way
      the world draws it: a pin crossed out is the one that means "not on top", and a pin filled in is
      the state itself. The tooltip is the other half: it says what pressing does, which is "keep on
      top" while it is not, and "stop" while it is.
    -->
    <button
      type="button"
      data-window-pin
      class="grid size-8 place-items-center rounded-md transition-colors"
      :class="
        isAlwaysOnTop ? 'bg-accent text-foreground' : 'text-muted-foreground hover:bg-accent hover:text-foreground'
      "
      :title="isAlwaysOnTop ? t('shell.window.stopKeepingOnTop') : t('shell.window.keepOnTop')"
      :aria-label="isAlwaysOnTop ? t('shell.window.stopKeepingOnTop') : t('shell.window.keepOnTop')"
      :aria-pressed="isAlwaysOnTop"
      @click="toggleAlwaysOnTop()"
    >
      <Pin v-if="isAlwaysOnTop" :size="15" class="fill-current" />
      <PinOff v-else :size="15" />
    </button>

    <!--
      The way to the transfers without one, and the reason it is here rather than in a menu: the panel
      used to appear only when a transfer did, which left somebody who wanted to look at it — or at
      the folder downloads land in — waiting for one to happen. It carries the count of what is still
      moving, so "is anything going on" is answerable without pressing it.
    -->
    <button
      type="button"
      data-transfers-toggle
      class="relative grid size-8 place-items-center rounded-md transition-colors"
      :class="
        files.panelExpanded
          ? 'bg-accent text-foreground'
          : 'text-muted-foreground hover:bg-accent hover:text-foreground'
      "
      :title="t('files.transfer.title')"
      :aria-label="t('files.transfer.title')"
      :aria-pressed="files.panelExpanded"
      @click="files.togglePanel()"
    >
      <ArrowUpDown :size="15" />
      <span
        v-if="movingTransfers > 0"
        data-transfers-count
        class="absolute -top-0.5 -right-0.5 grid min-w-3.5 place-items-center rounded-full bg-primary px-1 text-[9px] leading-3.5 text-primary-foreground"
      >
        {{ movingTransfers }}
      </span>
      <span
        v-else-if="failedTransfers"
        data-transfers-count
        class="absolute -top-0.5 -right-0.5 size-1.5 rounded-full bg-destructive"
      ></span>
    </button>

    <!--
      Batch execution, then the mode, then the gear. The three are ordered by what they are about:
      the two that put a page in the workspace stand together, and the two that are about the window
      itself — how it is lit, and the settings for everything — come last.
    -->
    <button
      type="button"
      class="grid size-8 place-items-center rounded-md transition-colors"
      :class="
        batchShowing ? 'bg-accent text-foreground' : 'text-muted-foreground hover:bg-accent hover:text-foreground'
      "
      :title="t('batch.title')"
      :aria-label="t('batch.title')"
      :aria-current="batchShowing ? 'page' : undefined"
      @click="terminals.openBatch()"
    >
      <ListChecks :size="15" />
    </button>

    <button
      type="button"
      class="grid size-8 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      :title="modeLabel"
      :aria-label="modeLabel"
      @click="cycleThemeMode()"
    >
      <component :is="modeIcon" :size="15" />
    </button>

    <button
      type="button"
      class="grid size-8 place-items-center rounded-md transition-colors"
      :class="
        settingsShowing ? 'bg-accent text-foreground' : 'text-muted-foreground hover:bg-accent hover:text-foreground'
      "
      :title="t('settings.title')"
      :aria-label="t('settings.title')"
      :aria-current="settingsShowing ? 'page' : undefined"
      @click="terminals.openSettings()"
    >
      <!--
        The mark that says the settings page has something for you: a release is published. On the
        button that opens the page rather than in a message that would leave on its own — where the
        update lives is a *place*, and the point of the mark is that it is still there when somebody
        comes back to look.
      -->
      <span class="relative grid place-items-center">
        <Settings :size="15" />
        <span
          v-if="updates.release"
          data-update-mark
          class="absolute -top-0.5 -right-0.5 size-1.5 rounded-full bg-primary ring-2 ring-background"
          ><span class="sr-only">{{ t("settings.update.available", { version: updates.release.version }) }}</span></span
        >
      </span>
    </button>

    <WindowControls
      v-if="showControls"
      class="ml-1 self-stretch"
      :is-maximized="isMaximized"
      :minimize="minimize"
      :toggle-maximize="toggleMaximize"
      :close="close"
    />
  </header>
</template>
