<script setup lang="ts">
/**
 * Pages and terminal screens stay mounted across tab switches. The terminal workspace keeps its
 * hosts outside its layout tree, so splitting or moving a pane cannot discard the screen's history.
 */
import { CircleAlert, RotateCw } from "@lucide/vue";
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import BatchPanel from "@/components/batch/BatchPanel.vue";
import ContainerPane from "@/components/container/ContainerPane.vue";
import FileBrowser from "@/components/files/FileBrowser.vue";
import HostPage from "@/components/inspect/HostPage.vue";
import NetworkReport from "@/components/inspect/NetworkReport.vue";
import ProcessTable from "@/components/inspect/ProcessTable.vue";
import SystemReport from "@/components/inspect/SystemReport.vue";
import KubernetesPane from "@/components/k8s/KubernetesPane.vue";
import SettingsPage from "@/components/settings/SettingsPage.vue";
import VmwarePane from "@/components/vmware/VmwarePane.vue";
import TabStrip from "@/components/workspace/TabStrip.vue";
import TerminalWorkspace from "@/components/workspace/TerminalWorkspace.vue";
import { describeShellError } from "@/i18n/errors";
import { cn } from "@/lib/utils";
import { usePreferencesStore } from "@/stores/preferences";
import { type FilesTab, type TerminalTab, useTerminalsStore } from "@/stores/terminals";
import type { TerminalPanelPlacement } from "@/types/app";

const { t } = useI18n();
const preferences = usePreferencesStore();
const terminals = useTerminalsStore();
const pages = computed(() => terminals.tabs.filter((tab) => tab.kind !== "terminal"));
const vertical = computed(() => preferences.tabPlacement === "left" || preferences.tabPlacement === "right");
const stripInWorkspace = computed(() => preferences.tabPlacement !== "titleBar");
const atEnd = computed(() => preferences.tabPlacement === "bottom" || preferences.tabPlacement === "right");

function filesTabOwner(tab: FilesTab): TerminalTab | null {
  return (
    terminals.terminalTabs.find(
      (candidate) => candidate.session.id === tab.session.id && terminals.effectivePlacement(candidate) === "tab",
    ) ?? null
  );
}

function filesTabPlacement(tab: FilesTab): TerminalPanelPlacement | undefined {
  return filesTabOwner(tab) === null ? undefined : "tab";
}

function chooseFilesTabPlacement(tab: FilesTab, placement: TerminalPanelPlacement): void {
  const owner = filesTabOwner(tab);
  if (owner) {
    void terminals.changeTabPanelPlacement(owner, placement, { remember: true, openDock: placement !== "tab" });
  }
}

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}
</script>

<template>
  <section :class="cn('flex min-h-0 min-w-0 flex-1', vertical ? 'flex-row' : 'flex-col')">
    <TabStrip v-if="stripInWorkspace" />
    <TerminalWorkspace v-show="terminals.activeTab?.kind === 'terminal'" :class="atEnd ? 'order-1' : 'order-2'" />

    <div
      v-for="tab in pages"
      v-show="tab.id === terminals.activeTabId"
      :key="tab.id"
      data-tab-pane
      :data-tab-id="tab.id"
      :data-active="tab.id === terminals.activeTabId"
      :class="cn('flex min-h-0 min-w-0 flex-1 flex-col', atEnd ? 'order-1' : 'order-2')"
    >
      <template v-if="tab.kind === 'files'">
        <FileBrowser
          v-if="tab.handle"
          :handle="tab.handle"
          :protocol="tab.session.protocol"
          :packed-transfers="tab.packed"
          :panel-placement="filesTabPlacement(tab)"
          class="min-h-0 flex-1"
          @choose-panel-placement="chooseFilesTabPlacement(tab, $event)"
        />
        <div v-else class="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-6 text-center">
          <CircleAlert :size="18" class="text-destructive" />
          <p class="text-xs text-muted-foreground">{{ t("files.tab.failedTitle") }}</p>
        </div>
        <footer
          v-if="tab.status === 'failed'"
          class="shrink-0 border-t border-destructive/40 bg-destructive/10 px-4 py-2"
        >
          <div class="flex flex-wrap items-center justify-between gap-2">
            <div class="min-w-0">
              <p v-if="tab.failure" class="text-xs">{{ describeShellError(translate, tab.failure).message }}</p>
              <p v-if="tab.failure" class="font-mono text-[11px] text-muted-foreground">
                {{ describeShellError(translate, tab.failure).code }}
              </p>
            </div>
            <button
              v-if="tab.handle === null"
              type="button"
              class="flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-xs hover:bg-accent"
              @click="terminals.reconnect(tab.id)"
            >
              <RotateCw :size="12" />
              {{ t("files.tab.retry") }}
            </button>
          </div>
        </footer>
      </template>

      <HostPage v-else-if="tab.kind === 'inspect'" :session="tab.session" :active="tab.id === terminals.activeTabId">
        <template #default="{ live }">
          <SystemReport v-if="tab.view === 'system'" :live="live" />
          <ProcessTable v-else-if="tab.view === 'processes'" :active="tab.id === terminals.activeTabId" />
          <NetworkReport v-else :live="live" :active="tab.id === terminals.activeTabId" />
        </template>
      </HostPage>
      <SettingsPage v-else-if="tab.kind === 'settings'" />
      <BatchPanel v-else-if="tab.kind === 'batch'" />
      <ContainerPane v-else-if="tab.kind === 'container'" :tab="tab" :active="tab.id === terminals.activeTabId" />
      <KubernetesPane v-else-if="tab.kind === 'kubernetes'" :tab="tab" :active="tab.id === terminals.activeTabId" />
      <VmwarePane v-else-if="tab.kind === 'vmware'" :tab="tab" :active="tab.id === terminals.activeTabId" />
    </div>
  </section>
</template>
