<script setup lang="ts">
import { Columns2, ExternalLink, Maximize2, Minimize2, Plus, Rows2, X } from "@lucide/vue";
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import TerminalSplitHandle from "@/components/workspace/TerminalSplitHandle.vue";
import TerminalWorkspacePane from "@/components/workspace/TerminalWorkspacePane.vue";
import { sessionTabTitle } from "@/lib/sessions/reopen";
import { matchesShortcut } from "@/lib/shortcuts/binding";
import { bindingFor, type ShortcutActionId } from "@/lib/shortcuts/registry";
import { cn } from "@/lib/utils";
import { type PaneRect, terminalGeometry, terminalPaneIds } from "@/lib/workspace/terminal-layout";
import { useAppStore } from "@/stores/app";
import { usePreferencesStore } from "@/stores/preferences";
import { type TerminalTab, useTerminalsStore } from "@/stores/terminals";
import { protocolSupports } from "@/types/generated/protocols";

const { t } = useI18n();
const terminals = useTerminalsStore();
const preferences = usePreferencesStore();
const app = useAppStore();
const stage = ref<HTMLElement | null>(null);
const width = ref(0);
const height = ref(0);
let observer: ResizeObserver | undefined;
let pointerPane: string | null = null;
let focusTimer: ReturnType<typeof setTimeout> | undefined;

function pointerDown(paneId: string): void {
  pointerPane = paneId;
}

function focusPane(paneId: string): void {
  if (pointerPane !== paneId) {
    terminals.activatePane(paneId);
  }
}

function clickPane(paneId: string): void {
  pointerPane = null;
  clearTimeout(focusTimer);
  terminals.activatePane(paneId);
}

function pointerUp(): void {
  const paneId = pointerPane;
  pointerPane = null;
  if (paneId !== null) {
    // Focus can show the host monitor and resize the workspace. Wait until the click has reached
    // its original control; moving a close button between pointerdown and pointerup loses it.
    focusTimer = setTimeout(() => terminals.activatePane(paneId), 0);
  }
}

const activeWorkspace = computed(() =>
  terminals.terminalWorkspaces.find((workspace) => workspace.id === terminals.activeWorkspaceId),
);
const geometries = computed(() => {
  const panes: Record<string, PaneRect> = {};
  for (const workspace of terminals.terminalWorkspaces) {
    Object.assign(panes, terminalGeometry(workspace.layout, width.value, height.value).panes);
    if (workspace.zoomedId !== null) {
      panes[workspace.zoomedId] = { left: 0, top: 0, width: width.value, height: height.value };
    }
  }
  return panes;
});
const dividers = computed(() => {
  const workspace = activeWorkspace.value;
  return workspace && workspace.zoomedId === null
    ? terminalGeometry(workspace.layout, width.value, height.value).dividers
    : [];
});

function split(tab: TerminalTab): boolean {
  const workspace = terminals.workspaceForPane(tab.id);
  return workspace !== undefined && terminalPaneIds(workspace.layout).length > 1;
}

function visible(tab: TerminalTab): boolean {
  const workspace = terminals.workspaceForPane(tab.id);
  return (
    workspace?.id === terminals.activeWorkspaceId && (workspace.zoomedId === null || workspace.zoomedId === tab.id)
  );
}

function rect(tab: TerminalTab): PaneRect {
  return geometries.value[tab.id] ?? { left: 0, top: 0, width: width.value, height: height.value };
}

function paneStyle(tab: TerminalTab): Record<string, string> {
  const bounds = rect(tab);
  return { left: `${bounds.left}px`, top: `${bounds.top}px`, width: `${bounds.width}px`, height: `${bounds.height}px` };
}

function label(tab: TerminalTab): string {
  return sessionTabTitle(tab.session.name, terminals.copyOf(tab.id));
}

function canSplit(tab: TerminalTab): boolean {
  return protocolSupports(tab.session.protocol, "shell") && !terminals.connectingSessionIds.includes(tab.session.id);
}

async function focusTerminal(paneId: string): Promise<void> {
  await nextTick();
  const panes = stage.value?.querySelectorAll<HTMLElement>("[data-terminal-pane]");
  const pane = panes && [...panes].find((element) => element.dataset.tabId === paneId);
  pane?.querySelector<HTMLTextAreaElement>(".xterm-helper-textarea")?.focus();
}

function zoom(paneId: string): void {
  terminals.togglePaneZoom(paneId);
  void focusTerminal(paneId);
}

function detach(paneId: string): void {
  terminals.detachPane(paneId);
  void focusTerminal(paneId);
}

async function close(paneId: string): Promise<void> {
  await terminals.closePane(paneId);
  if (terminals.activeTabId !== null) {
    await focusTerminal(terminals.activeTabId);
  }
}

function keydown(event: KeyboardEvent): void {
  const target = event.target instanceof Element ? event.target.closest<HTMLElement>("[data-terminal-pane]") : null;
  if (target?.dataset.tabId) {
    clickPane(target.dataset.tabId);
  }
  const pane = terminals.activeTab;
  if (event.defaultPrevented || event.isComposing || pane?.kind !== "terminal") {
    return;
  }
  const actions: [ShortcutActionId, () => void][] = [
    [
      "terminal.splitRight",
      () => {
        if (!event.repeat) void terminals.splitNewTerminal(pane.id, "right");
      },
    ],
    [
      "terminal.splitDown",
      () => {
        if (!event.repeat) void terminals.splitNewTerminal(pane.id, "down");
      },
    ],
    ["terminal.focusNextPane", () => terminals.focusNextPane(1)],
    ["terminal.focusPreviousPane", () => terminals.focusNextPane(-1)],
    ["terminal.zoomPane", () => zoom(pane.id)],
    [
      "terminal.closePane",
      () => {
        if (!event.repeat) void close(pane.id);
      },
    ],
  ];
  const action = actions.find(([id]) => matchesShortcut(event, bindingFor(id, preferences.keybindings), app.platform));
  if (action) {
    event.preventDefault();
    event.stopPropagation();
    action[1]();
  }
}

onMounted(() => {
  const element = stage.value;
  if (!element) return;
  const measure = () => {
    if (element.clientWidth > 0 && element.clientHeight > 0) {
      width.value = element.clientWidth;
      height.value = element.clientHeight;
    }
  };
  measure();
  observer = new ResizeObserver(measure);
  observer.observe(element);
  window.addEventListener("pointerup", pointerUp);
  window.addEventListener("pointercancel", pointerUp);
});
onBeforeUnmount(() => {
  observer?.disconnect();
  window.removeEventListener("pointerup", pointerUp);
  window.removeEventListener("pointercancel", pointerUp);
  clearTimeout(focusTimer);
});
</script>

<template>
  <div
    ref="stage"
    data-terminal-workspace
    :data-workspace-id="terminals.activeWorkspaceId"
    class="relative min-h-0 min-w-0 flex-1 overflow-hidden"
    @keydown.capture="keydown"
  >
    <!-- A flat, keyed host per instance preserves xterm and editors even across workspace moves. -->
    <div
      v-for="tab in terminals.terminalTabs"
      v-show="visible(tab)"
      :key="tab.id"
      data-terminal-pane
      data-tab-pane
      :data-tab-id="tab.id"
      :data-handle="tab.handle"
      :data-status="tab.status"
      :data-workspace-id="terminals.workspaceForPane(tab.id)?.id"
      :data-active="tab.id === terminals.activeTabId"
      :style="paneStyle(tab)"
      class="absolute flex min-h-0 min-w-0 flex-col overflow-hidden bg-background"
      role="group"
      :aria-label="t('terminal.split.pane', { name: label(tab) })"
      @pointerdown.capture="pointerDown(tab.id)"
      @click.capture="clickPane(tab.id)"
      @contextmenu.capture="clickPane(tab.id)"
      @focusin="focusPane(tab.id)"
    >
      <header
        v-if="split(tab)"
        data-terminal-pane-header
        :class="
          cn(
            'flex h-8 shrink-0 items-center gap-1 border-b px-2 cursor-default select-none',
            tab.id === terminals.activeTabId ? 'border-primary/50 bg-accent' : 'border-border bg-card/50',
          )
        "
      >
        <span
          class="size-1.5 shrink-0 rounded-full"
          :class="tab.status === 'open' ? 'bg-success' : tab.status === 'ended' ? 'bg-warning' : 'bg-destructive'"
        />
        <span class="min-w-0 flex-1 truncate text-[11px] font-medium" :title="label(tab)">{{ label(tab) }}</span>
        <button
          v-if="rect(tab).width >= 320"
          type="button"
          data-split-right
          class="grid size-6 shrink-0 place-items-center rounded-sm text-muted-foreground hover:bg-background hover:text-foreground disabled:opacity-40"
          :disabled="!canSplit(tab)"
          :title="t('terminal.split.right')"
          :aria-label="t('terminal.split.right')"
          @click="terminals.splitNewTerminal(tab.id, 'right')"
        >
          <Columns2 :size="13" />
        </button>
        <button
          v-if="rect(tab).width >= 320"
          type="button"
          data-split-down
          class="grid size-6 shrink-0 place-items-center rounded-sm text-muted-foreground hover:bg-background hover:text-foreground disabled:opacity-40"
          :disabled="!canSplit(tab)"
          :title="t('terminal.split.down')"
          :aria-label="t('terminal.split.down')"
          @click="terminals.splitNewTerminal(tab.id, 'down')"
        >
          <Rows2 :size="13" />
        </button>
        <button
          type="button"
          data-split-picker-open
          class="grid size-6 shrink-0 place-items-center rounded-sm text-muted-foreground hover:bg-background hover:text-foreground"
          :title="t('terminal.split.choose')"
          :aria-label="t('terminal.split.choose')"
          @click="terminals.openSplitPicker(tab.id)"
        >
          <Plus :size="13" />
        </button>
        <button
          v-if="rect(tab).width >= 320"
          type="button"
          data-pane-detach
          class="grid size-6 shrink-0 place-items-center rounded-sm text-muted-foreground hover:bg-background hover:text-foreground"
          :title="t('terminal.split.detach')"
          :aria-label="t('terminal.split.detach')"
          @click="detach(tab.id)"
        >
          <ExternalLink :size="13" />
        </button>
        <button
          type="button"
          data-pane-zoom
          class="grid size-6 shrink-0 place-items-center rounded-sm text-muted-foreground hover:bg-background hover:text-foreground"
          :aria-pressed="terminals.workspaceForPane(tab.id)?.zoomedId === tab.id"
          :title="
            t(
              terminals.workspaceForPane(tab.id)?.zoomedId === tab.id
                ? 'terminal.split.restore'
                : 'terminal.split.zoom',
            )
          "
          :aria-label="
            t(
              terminals.workspaceForPane(tab.id)?.zoomedId === tab.id
                ? 'terminal.split.restore'
                : 'terminal.split.zoom',
            )
          "
          @click="zoom(tab.id)"
        >
          <Minimize2 v-if="terminals.workspaceForPane(tab.id)?.zoomedId === tab.id" :size="13" /><Maximize2
            v-else
            :size="13"
          />
        </button>
        <button
          type="button"
          data-pane-close
          class="grid size-6 shrink-0 place-items-center rounded-sm text-muted-foreground hover:bg-background hover:text-foreground"
          :title="t('terminal.split.close')"
          :aria-label="t('terminal.split.close')"
          @click="close(tab.id)"
        >
          <X :size="13" />
        </button>
      </header>
      <TerminalWorkspacePane
        :tab="tab"
        :active="tab.id === terminals.activeTabId"
        :split="split(tab)"
        :width="rect(tab).width"
        :height="Math.max(0, rect(tab).height - (split(tab) ? 32 : 0))"
      />
    </div>
    <TerminalSplitHandle
      v-for="divider in dividers"
      :key="divider.id"
      :divider="divider"
      @move="activeWorkspace && terminals.resizeSplit(activeWorkspace.id, divider.id, $event)"
    />
  </div>
</template>
