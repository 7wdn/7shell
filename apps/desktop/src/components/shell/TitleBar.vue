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
import { ListChecks, Monitor, Moon, Settings, Sun } from "@lucide/vue";
import { computed, markRaw } from "vue";
import { useI18n } from "vue-i18n";

import AppMark from "@/components/brand/AppMark.vue";
import WindowControls from "@/components/shell/WindowControls.vue";
import { useWindowControls } from "@/composables/useWindowControls";
import { useAppStore } from "@/stores/app";
import { usePreferencesStore } from "@/stores/preferences";
import { useTerminalsStore } from "@/stores/terminals";

const { t } = useI18n();
const app = useAppStore();
const preferences = usePreferencesStore();
const terminals = useTerminalsStore();

const { isMac, showControls, isMaximized, minimize, toggleMaximize, close } = useWindowControls(
  () => app.info?.os ?? "",
);

const version = computed(() => (app.info ? `v${app.info.version}` : ""));

/** One button covers all three modes, cycling in the order the settings page lists them. */
const modeIcon = computed(() =>
  markRaw(preferences.themeMode === "system" ? Monitor : preferences.themeMode === "dark" ? Moon : Sun),
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

    <div data-tauri-drag-region class="h-full min-w-4 flex-1"></div>

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
      class="grid size-8 place-items-center rounded-md transition-colors"
      :class="
        settingsShowing ? 'bg-accent text-foreground' : 'text-muted-foreground hover:bg-accent hover:text-foreground'
      "
      :title="t('settings.title')"
      :aria-label="t('settings.title')"
      :aria-current="settingsShowing ? 'page' : undefined"
      @click="terminals.openSettings()"
    >
      <Settings :size="15" />
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
