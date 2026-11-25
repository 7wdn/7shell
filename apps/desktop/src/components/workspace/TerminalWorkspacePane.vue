<script setup lang="ts">
import { CircleAlert, Columns2, Folder, ListChecks, LoaderCircle, Plus, RotateCw, Rows2 } from "@lucide/vue";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import CommandInputBar from "@/components/commands/CommandInputBar.vue";
import CommandsPanel from "@/components/commands/CommandsPanel.vue";
import FileBrowser from "@/components/files/FileBrowser.vue";
import TerminalPane from "@/components/terminal/TerminalPane.vue";
import SplitDivider from "@/components/workspace/SplitDivider.vue";
import TerminalSchemeMenu from "@/components/workspace/TerminalSchemeMenu.vue";
import { describeShellError } from "@/i18n/errors";
import { sessionEndpoint } from "@/lib/menu/sessions";
import { cn } from "@/lib/utils";
import { MIN_FILE_PANEL_PX, MIN_PANE_PX, SIDE_DIVIDER_PX, SIDE_PANE_KEEP_PX } from "@/lib/workspace/split";
import { usePreferencesStore } from "@/stores/preferences";
import { type TerminalTab, useTerminalsStore } from "@/stores/terminals";
import type { DockedPanelPlacement, TerminalPanelPlacement } from "@/types/app";
import { protocolSupports } from "@/types/generated/protocols";
import type { BackspaceMode, TerminalEncoding } from "@/types/session";

const props = defineProps<{ tab: TerminalTab; active: boolean; split: boolean; width: number; height: number }>();
const { t } = useI18n();
const preferences = usePreferencesStore();
const terminals = useTerminalsStore();
/** The selected half stays with this terminal when its workspace or geometry changes. */
const panelTab = ref<"files" | "commands">("files");

const panelShowing = computed(() => props.tab.panel !== null || panelTab.value === "commands");
const filesShowing = computed(() => panelShowing.value && panelTab.value !== "commands");
const placement = computed(() =>
  panelTab.value === "commands"
    ? terminals.effectiveCommandPlacement(props.tab)
    : terminals.effectivePlacement(props.tab),
);
const side = computed(() => (placement.value === "left" || placement.value === "right" ? placement.value : null));
// The command bar and status strip need room beyond xterm's own floor; ignoring them overlays
// those controls with the file panel when a nested split becomes short.
const keepHeight = computed(() => (props.tab.status === "open" && props.tab.failure === null ? 160 : 280));
const capacity = computed(() => Math.max(0, side.value ? props.width - 208 : props.height - keepHeight.value));
const panelMinimum = computed(() => (side.value ? MIN_FILE_PANEL_PX : MIN_PANE_PX));
const panelSize = computed(() => {
  const requested = side.value ? preferences.resolvedPanelWidth : preferences.resolvedPanelHeight;
  return props.split
    ? Math.max(Math.min(panelMinimum.value, capacity.value), Math.min(requested, capacity.value))
    : requested;
});
// A panel keeps its channel while temporarily hidden in a small split. Zooming restores it,
// rather than closing a browser or discarding an editor merely because the window became smaller.
const panelFits = computed(() => !props.split || capacity.value >= (side.value ? MIN_FILE_PANEL_PX : 80));
const panelStyle = computed((): Record<string, string> => {
  if (side.value === null) {
    return {
      height: `${panelSize.value}px`,
      minHeight: `${props.split ? Math.min(MIN_PANE_PX, capacity.value) : MIN_PANE_PX}px`,
    };
  }
  return {
    width: `${panelSize.value}px`,
    minWidth: `${props.split ? Math.min(MIN_FILE_PANEL_PX, capacity.value) : MIN_FILE_PANEL_PX}px`,
    maxWidth: `calc(100% - ${props.split ? 208 : SIDE_PANE_KEEP_PX + SIDE_DIVIDER_PX}px)`,
  };
});

const encoding = computed<TerminalEncoding>(() => {
  const session = props.tab.session;
  return session.protocol === "telnet"
    ? (session.telnet?.encoding ?? "utf-8")
    : session.protocol === "ssh"
      ? session.terminal.encoding
      : "utf-8";
});
const backspace = computed<BackspaceMode>(() =>
  props.tab.session.protocol === "ssh" ? props.tab.session.terminal.backspace : "del",
);
const canSplit = computed(() => protocolSupports(props.tab.session.protocol, "shell"));

async function showCommands(): Promise<void> {
  if (panelShowing.value && panelTab.value === "commands") {
    if (props.tab.panel) {
      await terminals.togglePanel(props.tab);
    }
    panelTab.value = "files";
    return;
  }
  panelTab.value = "commands";
}

function showFiles(): void {
  if (!protocolSupports(props.tab.session.protocol, "sftp")) {
    return;
  }
  if (terminals.effectivePlacement(props.tab) === "tab") {
    panelTab.value = "files";
    void terminals.openFiles(props.tab.session, { origin: "placement" });
    return;
  }
  if (filesShowing.value) {
    void terminals.togglePanel(props.tab);
    return;
  }
  panelTab.value = "files";
  if (props.tab.panel === null && props.tab.status === "open") {
    void terminals.openPanel(props.tab);
  }
}

function filesButtonTitle(): string {
  if (terminals.effectivePlacement(props.tab) === "tab") {
    return t("terminal.panel.openTab");
  }
  return filesShowing.value ? t("terminal.panel.close") : t("terminal.panel.open");
}

async function choosePlacement(value: TerminalPanelPlacement): Promise<void> {
  const openDock = value !== "tab" && !panelShowing.value;
  const moved = await terminals.changeTabPanelPlacement(props.tab, value, { remember: true, openDock });
  if (moved && value === "tab") {
    panelTab.value = "files";
  }
}

function chooseCommandPlacement(value: DockedPanelPlacement): void {
  void terminals.changeTabCommandPlacement(props.tab, value, { remember: true });
}

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const endedReason = computed(() => {
  const tab = props.tab;
  if (tab.status !== "ended" || tab.disconnected) {
    return null;
  }
  return tab.reason ? t("terminal.tab.endedReason", { reason: tab.reason }) : t("terminal.tab.endedNoReason");
});
</script>

<template>
  <div class="flex min-h-0 min-w-0 flex-1" :class="side ? 'flex-row' : 'flex-col'">
    <div :class="cn('flex min-h-0 min-w-0 flex-1 flex-col', side === 'left' ? 'order-3' : 'order-1')">
      <TerminalPane
        v-if="tab.handle"
        :handle="tab.handle"
        :active="active"
        :interactive="tab.status === 'open'"
        :encoding="encoding"
        :backspace="backspace"
        class="min-h-0 min-w-0 flex-1"
        :style="tab.panel && side === null && panelFits ? { minHeight: `${split ? 80 : MIN_PANE_PX}px` } : undefined"
      />
      <div v-else class="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-6 text-center">
        <CircleAlert :size="18" class="text-destructive" />
        <p class="text-xs text-muted-foreground">{{ t("terminal.tab.failedTitle") }}</p>
      </div>

      <CommandInputBar v-if="tab.status === 'open'" :handle="tab.handle ?? undefined" />

      <footer
        data-terminal-strip
        class="flex shrink-0 items-center gap-1 border-t border-border bg-card/30 px-2 py-1 cursor-default select-none"
      >
        <span class="min-w-0 flex-1 truncate font-mono text-[11px] text-muted-foreground">
          {{ sessionEndpoint(tab.session, t) }}
        </span>
        <template v-if="!split">
          <button
            type="button"
            data-split-right
            class="grid size-6 shrink-0 place-items-center rounded-sm text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-40"
            :disabled="!canSplit || terminals.connectingSessionIds.includes(tab.session.id)"
            :title="t('terminal.split.right')"
            :aria-label="t('terminal.split.right')"
            @click="terminals.splitNewTerminal(tab.id, 'right')"
          >
            <Columns2 :size="13" />
          </button>
          <button
            type="button"
            data-split-down
            class="grid size-6 shrink-0 place-items-center rounded-sm text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-40"
            :disabled="!canSplit || terminals.connectingSessionIds.includes(tab.session.id)"
            :title="t('terminal.split.down')"
            :aria-label="t('terminal.split.down')"
            @click="terminals.splitNewTerminal(tab.id, 'down')"
          >
            <Rows2 :size="13" />
          </button>
          <button
            type="button"
            data-split-picker-open
            class="grid size-6 shrink-0 place-items-center rounded-sm text-muted-foreground hover:bg-accent hover:text-foreground"
            :title="t('terminal.split.choose')"
            :aria-label="t('terminal.split.choose')"
            @click="terminals.openSplitPicker(tab.id)"
          >
            <Plus :size="13" />
          </button>
        </template>
        <TerminalSchemeMenu :compact="split && width < 340" />
        <button
          v-if="protocolSupports(tab.session.protocol, 'sftp')"
          type="button"
          :class="
            cn(
              'flex shrink-0 items-center gap-1 rounded-md px-2 py-0.5 text-[11px] transition-colors',
              filesShowing
                ? 'bg-accent text-foreground'
                : 'text-muted-foreground hover:bg-accent hover:text-foreground',
              'disabled:cursor-not-allowed disabled:opacity-40',
            )
          "
          :disabled="tab.panel === null && tab.status !== 'open'"
          :aria-pressed="filesShowing"
          :aria-label="t('terminal.panel.label')"
          :title="filesButtonTitle()"
          @click="showFiles()"
        >
          <Folder :size="11" />
          <span v-if="!split || width >= 340">{{ t("terminal.panel.label") }}</span>
        </button>
        <button
          type="button"
          :class="
            cn(
              'flex shrink-0 items-center gap-1 rounded-md px-2 py-0.5 text-[11px] transition-colors',
              panelTab === 'commands'
                ? 'bg-accent text-foreground'
                : 'text-muted-foreground hover:bg-accent hover:text-foreground',
            )
          "
          :aria-pressed="panelTab === 'commands'"
          :aria-label="t('commands.panel')"
          :title="t('commands.panel')"
          @click="showCommands()"
        >
          <ListChecks :size="11" />
          <span v-if="!split || width >= 340">{{ t("commands.panel") }}</span>
        </button>
      </footer>
      <p v-if="panelShowing && !panelFits" class="shrink-0 px-2 py-1 text-[11px] text-muted-foreground">
        {{ t("terminal.split.panelCompact") }}
      </p>

      <footer
        v-if="tab.status === 'ended' || tab.failure"
        :class="
          cn(
            'shrink-0 border-t px-3 py-2',
            tab.failure ? 'border-destructive/40 bg-destructive/10' : 'border-border bg-muted/40',
          )
        "
      >
        <div class="flex flex-wrap items-center justify-between gap-2">
          <div class="min-w-0">
            <p v-if="tab.status === 'ended'" class="text-xs">
              {{ tab.disconnected ? t("terminal.tab.disconnected") : t("terminal.tab.ended") }}
            </p>
            <p v-if="endedReason" class="text-[11px] text-muted-foreground">{{ endedReason }}</p>
            <p v-if="tab.failure" class="text-xs">{{ describeShellError(translate, tab.failure).message }}</p>
            <p v-if="tab.failure" class="font-mono text-[11px] text-muted-foreground">
              {{ describeShellError(translate, tab.failure).code }}
            </p>
          </div>
          <button
            v-if="tab.status !== 'open'"
            type="button"
            class="flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-xs hover:bg-accent"
            @click="terminals.reconnectPane(tab.id)"
          >
            <RotateCw :size="12" />
            {{ t("terminal.tab.reconnect") }}
          </button>
        </div>
      </footer>
    </div>

    <template v-if="panelShowing">
      <SplitDivider
        v-show="panelFits"
        :size="panelSize"
        :orientation="side ? 'vertical' : 'horizontal'"
        :pane="side === 'left' ? 'start' : 'end'"
        :min-size="split ? Math.min(panelMinimum, capacity) : panelMinimum"
        :keep-size="split ? (side ? 200 : keepHeight - SIDE_DIVIDER_PX) : undefined"
        class="order-2"
        @move="side ? preferences.setPanelWidth($event) : preferences.setPanelHeight($event)"
        @settle="side ? preferences.rememberPanelWidth() : preferences.rememberPanelHeight()"
        @reset="side ? preferences.resetPanelWidth() : preferences.resetPanelHeight()"
      />
      <div
        v-show="panelFits"
        data-terminal-panel
        :data-side="side ?? 'bottom'"
        :class="cn('flex flex-col overflow-hidden', side ? 'order-1 shrink-0' : 'order-3 shrink')"
        :style="panelStyle"
      >
        <CommandsPanel
          v-if="panelTab === 'commands'"
          :panel-placement="terminals.effectiveCommandPlacement(tab)"
          class="min-h-0 flex-1"
          @choose-panel-placement="chooseCommandPlacement($event)"
        />
        <FileBrowser
          v-else-if="tab.panel?.handle"
          :handle="tab.panel.handle"
          :protocol="tab.session.protocol"
          :panel-placement="terminals.effectivePlacement(tab)"
          class="min-h-0 flex-1"
          @choose-panel-placement="choosePlacement($event)"
        />
        <div v-else class="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-4 text-center">
          <CircleAlert v-if="tab.panel?.status === 'failed'" :size="18" class="text-destructive" />
          <LoaderCircle v-else :size="18" class="animate-spin text-muted-foreground" />
          <p v-if="tab.panel?.failure" class="text-xs">
            {{ describeShellError(translate, tab.panel.failure).message }}
          </p>
          <p v-if="tab.panel?.failure" class="font-mono text-[11px] text-muted-foreground">
            {{ describeShellError(translate, tab.panel.failure).code }}
          </p>
          <p v-else class="text-xs text-muted-foreground">{{ t("terminal.panel.opening") }}</p>
          <button
            v-if="tab.panel?.status === 'failed'"
            type="button"
            class="flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-xs hover:bg-accent"
            @click="terminals.openPanel(tab)"
          >
            <RotateCw :size="12" />
            {{ t("terminal.panel.retry") }}
          </button>
        </div>
        <footer
          v-if="tab.panel?.handle && tab.panel.failure"
          class="shrink-0 border-t border-destructive/40 bg-destructive/10 px-3 py-1.5"
        >
          <p class="text-[11px]">{{ describeShellError(translate, tab.panel.failure).message }}</p>
          <p class="font-mono text-[11px] text-muted-foreground">
            {{ describeShellError(translate, tab.panel.failure).code }}
          </p>
        </footer>
      </div>
    </template>
  </div>
</template>
