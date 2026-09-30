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
import TerminalTabs from "@/components/terminal/TerminalTabs.vue";
import UnlockScreen from "@/components/vault/UnlockScreen.vue";
import HomeView from "@/components/views/HomeView.vue";
import { describeShellError } from "@/i18n/errors";
import { useAppStore } from "@/stores/app";
import { usePreferencesStore } from "@/stores/preferences";
import { useTerminalsStore } from "@/stores/terminals";
import { useVaultStore } from "@/stores/vault";

const { t } = useI18n();
const app = useAppStore();
const preferences = usePreferencesStore();
const terminals = useTerminalsStore();
const vault = useVaultStore();

const settingsOpen = ref(false);
let stopFollowingSystem: (() => void) | undefined;
let stopTerminalEvents: (() => void) | undefined;

const backendFailure = computed(() => (app.failure ? describeShellError(translate, app.failure) : null));

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

onMounted(async () => {
  stopFollowingSystem = preferences.initialize();
  stopTerminalEvents = terminals.initialize();
  await preferences.load();
  await app.load();
  await vault.initialize();
});

onBeforeUnmount(() => {
  stopFollowingSystem?.();
  stopTerminalEvents?.();
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

    <div v-else class="flex min-h-0 flex-1">
      <AppSidebar />
      <main class="flex min-w-0 flex-1 flex-col overflow-hidden">
        <TerminalTabs v-if="terminals.tabs.length > 0" />
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

    <SettingsDialog :open="settingsOpen" @update:open="settingsOpen = $event" />
  </div>
</template>
