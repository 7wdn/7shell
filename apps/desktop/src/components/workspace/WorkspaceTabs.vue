<script setup lang="ts">
/**
 * The workspace: a tab per terminal and per file browser, and what state each one is in.
 *
 * A terminal tab is not dropped when its terminal ends. The reason the server gave, and a way to
 * connect again, are things only that tab can offer — closing it on someone's behalf would take
 * both away and leave a workspace that quietly lost a session.
 *
 * Panes are kept mounted and hidden rather than created on demand, so switching tabs does not throw
 * away a terminal's scrollback — and so the pane that owns a handle is always the one the terminal
 * bus has registered. A file browser keeps its selection and its typed path the same way.
 *
 * The tab itself is one union, so the bar draws every kind from one list: the icon and the label say
 * which kind it is, and only the tabs with a stream that can end carry a status dot.
 *
 * A terminal tab can also carry the file panel, which is the second half of the mode: the terminal
 * above, the browser below, and a line between them that can be dragged. Everything the terminal
 * owns — its status strip and its endings — stays above that line, so what is below it is only ever
 * the panel; the drag measures from the bottom of the tab, and that has to keep meaning "the
 * bottom of the panel".
 *
 * The third kind is a page about a host rather than a thing connected to one — the monitor's
 * system, process and network pages. It draws a host's own shape in the strip (its title and the
 * machine it is about) instead of a session name, and it has no status dot: whether it is being
 * read is a question about a terminal it does not own.
 *
 * The fourth is the settings page, which is about the application rather than about anything it is
 * connected to. It is drawn like the other panes — kept mounted behind its tab — so that a search
 * someone typed is still there when they come back to it.
 */
import { CircleAlert, Folder, ListChecks, LoaderCircle, RotateCw } from "@lucide/vue";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import { sessionEndpoint } from "@/lib/menu/sessions";
import BatchPanel from "@/components/batch/BatchPanel.vue";
import ContainerPane from "@/components/container/ContainerPane.vue";
import KubernetesPane from "@/components/k8s/KubernetesPane.vue";
import VmwarePane from "@/components/vmware/VmwarePane.vue";
import CommandInputBar from "@/components/commands/CommandInputBar.vue";
import CommandsPanel from "@/components/commands/CommandsPanel.vue";
import FileBrowser from "@/components/files/FileBrowser.vue";
import TabStrip from "@/components/workspace/TabStrip.vue";
import TerminalSchemeMenu from "@/components/workspace/TerminalSchemeMenu.vue";
import { protocolSupports } from "@/types/generated/protocols";
import HostPage from "@/components/inspect/HostPage.vue";
import NetworkReport from "@/components/inspect/NetworkReport.vue";
import ProcessTable from "@/components/inspect/ProcessTable.vue";
import SystemReport from "@/components/inspect/SystemReport.vue";
import SettingsPage from "@/components/settings/SettingsPage.vue";
import TerminalPane from "@/components/terminal/TerminalPane.vue";
import SplitDivider from "@/components/workspace/SplitDivider.vue";
import { describeShellError } from "@/i18n/errors";
import { MIN_FILE_PANEL_PX, MIN_PANE_PX, SIDE_DIVIDER_PX, SIDE_PANE_KEEP_PX } from "@/lib/workspace/split";
import { type DockedPanelPlacement, type TerminalPanelPlacement } from "@/types/app";
import type { BackspaceMode, TerminalEncoding } from "@/types/session";
import { cn } from "@/lib/utils";
import { usePreferencesStore } from "@/stores/preferences";
import { type FilesTab, type TerminalTab, type WorkspaceTab, useTerminalsStore } from "@/stores/terminals";

const { t } = useI18n();
const preferences = usePreferencesStore();
const terminals = useTerminalsStore();

/**
 * Which half of the panel under a terminal is showing: the files, or the commands.
 *
 * Kept here rather than in the store because it is the panel's own state and nothing else asks
 * about it — and because the two halves are not alike: a file browser owns a channel the backend
 * has to let go of, while the command library is a document that needs no connection at all, so
 * opening the commands must not open one.
 */
const panelTab = ref<Record<string, "files" | "commands">>({});

function panelShowing(tab: WorkspaceTab): boolean {
  if (tab.kind !== "terminal") {
    return false;
  }
  return tab.panel !== null || panelTab.value[tab.id] === "commands";
}

/**
 * Whether the files are the half on screen.
 *
 * The panel record alone is not the answer. Switching to the commands leaves the browser open —
 * the backend holds its channel either way, and only the half being drawn changes — so a panel
 * that exists while the commands are showing would light the files button too, and both buttons
 * would read as the one that is on screen.
 */
function filesShowing(tab: WorkspaceTab): boolean {
  return panelShowing(tab) && panelTab.value[tab.id] !== "commands";
}

/**
 * The commands half of the panel: show it, or put the panel away when it is already what is
 * showing.
 *
 * The files half closes through the store because a browser is a record with a channel to let go
 * of. The commands half has no record of its own — its visibility is this choice — so clicking it
 * again has to forget the choice, and a browser left underneath (files, then commands) is closed
 * with it so the panel does not stay up around nothing.
 */
async function showCommands(tab: TerminalTab): Promise<void> {
  if (panelShowing(tab) && panelTab.value[tab.id] === "commands") {
    if (tab.panel) {
      await terminals.togglePanel(tab);
    }
    delete panelTab.value[tab.id];
    return;
  }
  panelTab.value[tab.id] = "commands";
}

/**
 * The file half of the panel: show it, or close it when it is already what is showing.
 *
 * Both halves are set explicitly. Returning to the files from the commands has to say so, or the
 * panel would still be drawing the commands and the button would look broken — which is exactly
 * what the end-to-end run said before this was fixed.
 */
function showFiles(tab: TerminalTab): void {
  // A file panel is an SFTP channel. A local shell, a serial console and Telnet do not have one,
  // and asking anyway is what drew an unsupported-feature error under a perfectly good terminal.
  if (!protocolSupports(tab.session.protocol, "sftp")) {
    return;
  }
  // A terminal whose position is the standalone tab shows its files there; "Files" means the same
  // thing in all four positions, and the browser stays the position's while this tab is using it.
  if (terminals.effectivePlacement(tab) === "tab") {
    // The file manager is a tab of its own, so the commands half has nothing left to show here.
    delete panelTab.value[tab.id];
    void terminals.openFiles(tab.session, { origin: "placement" });
    return;
  }
  if (panelShowing(tab) && panelTab.value[tab.id] !== "commands") {
    // Closing is the store's: it owns the browser and the channel it has to let go of.
    void terminals.togglePanel(tab);
    return;
  }

  panelTab.value[tab.id] = "files";
  if (tab.panel === null && tab.status === "open") {
    void terminals.openPanel(tab);
  }
}

/**
 * Where the panel is drawn right now.
 *
 * The two halves keep their own position: the files can sit on the left while the commands stay at
 * the bottom, and showing one half moves the panel to that half's answer. Only one half is drawn at
 * a time, so there is never a second panel to squeeze the terminal from another side.
 */
function panelPlacement(tab: TerminalTab): TerminalPanelPlacement {
  if (panelTab.value[tab.id] === "commands") {
    return terminals.effectiveCommandPlacement(tab);
  }
  return terminals.effectivePlacement(tab);
}

/** The side the panel is docked to, or nothing when it is under the terminal. */
function panelSide(tab: TerminalTab): "left" | "right" | null {
  const placement = panelPlacement(tab);
  return placement === "left" || placement === "right" ? placement : null;
}

/** What the screen reads and the input is written in, from the session's own setting. */
function encodingOf(tab: TerminalTab): TerminalEncoding {
  if (tab.session.protocol === "telnet") {
    return tab.session.telnet?.encoding ?? "utf-8";
  }
  if (tab.session.protocol === "ssh") {
    return tab.session.terminal.encoding;
  }
  return "utf-8";
}

/**
 * Which byte a backspace leaves as. Telnet's key is decided in the core, after the line and its
 * editing; translating it here as well would send an edit the far side never asked for.
 */
function backspaceOf(tab: TerminalTab): BackspaceMode {
  return tab.session.protocol === "ssh" ? tab.session.terminal.backspace : "del";
}

function panelIsSide(tab: TerminalTab): boolean {
  return panelSide(tab) !== null;
}

/** How wide or tall the panel is drawn, with the terminal keeping its side's floor. */
function panelStyle(tab: TerminalTab): Record<string, string> | undefined {
  if (!panelIsSide(tab)) {
    return {
      height: `${preferences.resolvedPanelHeight}px`,
      minHeight: `${MIN_PANE_PX}px`,
    };
  }
  return {
    width: `${preferences.resolvedPanelWidth}px`,
    minWidth: `${MIN_FILE_PANEL_PX}px`,
    maxWidth: `calc(100% - ${SIDE_PANE_KEEP_PX + SIDE_DIVIDER_PX}px)`,
  };
}

/** What the Files button says it will do, which depends on where this tab's position puts it. */
function filesButtonTitle(tab: TerminalTab): string {
  if (terminals.effectivePlacement(tab) === "tab") {
    return t("terminal.panel.openTab");
  }
  return filesShowing(tab) ? t("terminal.panel.close") : t("terminal.panel.open");
}

/**
 * Takes this tab's files to one of their four positions.
 *
 * A tab means the standalone browser the session shares; the docked panel is closed first, because
 * two live browsers on one directory would disagree the moment either one moved. The other three
 * keep the panel open and simply let the workspace draw it somewhere else. The store owns the
 * move and the shared browser's lifespan; this only decides what the panel's own half does.
 */
async function choosePlacement(tab: TerminalTab, placement: TerminalPanelPlacement): Promise<void> {
  // A half already on screen moves rather than being replaced: only a panel that is away needs the
  // files opened for it, and choosing a tab closes the panel instead.
  const openDock = placement !== "tab" && !panelShowing(tab);
  const moved = await terminals.changeTabPanelPlacement(tab, placement, { remember: true, openDock });
  if (moved && placement === "tab") {
    delete panelTab.value[tab.id];
  }
}

/** Takes this tab's commands to one of their three sides, leaving the files where they are. */
function chooseCommandPlacement(tab: TerminalTab, placement: DockedPanelPlacement): void {
  void terminals.changeTabCommandPlacement(tab, placement, { remember: true });
}

/**
 * The terminal whose files the standalone browser is showing.
 *
 * The file tab is shared by the session, so the control inside it moves the first terminal that
 * chose the tab position; the store then decides whether the tab stays for the others.
 */
function filesTabOwner(tab: TerminalTab | FilesTab): TerminalTab | null {
  return (
    terminals.tabs.find(
      (candidate): candidate is TerminalTab =>
        candidate.kind === "terminal" &&
        candidate.session.id === tab.session.id &&
        terminals.effectivePlacement(candidate) === "tab",
    ) ?? null
  );
}

function filesTabPlacement(tab: TerminalTab | FilesTab): TerminalPanelPlacement | undefined {
  return filesTabOwner(tab) === null ? undefined : "tab";
}

function chooseFilesTabPlacement(tab: TerminalTab | FilesTab, placement: TerminalPanelPlacement): void {
  const owner = filesTabOwner(tab);
  if (owner !== null) {
    void choosePlacement(owner, placement);
  }
}

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

function translateStatus(tab: TerminalTab): string | null {
  // A connection the person dropped asked the server nothing, so there is no server answer to show.
  if (tab.status !== "ended" || tab.disconnected) {
    return null;
  }
  return tab.reason ? t("terminal.tab.endedReason", { reason: tab.reason }) : t("terminal.tab.endedNoReason");
}

/** Whether the bar is a column rather than a row, which follows from the edge it is drawn against. */
const vertical = computed(() => preferences.tabPlacement === "left" || preferences.tabPlacement === "right");

/** Whether the strip is drawn by the workspace at all: in the title bar it belongs to the window. */
const stripInWorkspace = computed(() => preferences.tabPlacement !== "titleBar");

/**
 * Whether the bar is drawn after the panes rather than before them.
 *
 * The bar is the first thing in the markup in every case — it is the way through the workspace, and
 * a keyboard reaches it first — and the two placements that put it at the far end say so with an
 * order rather than by being written twice.
 */
const atEnd = computed(() => preferences.tabPlacement === "bottom" || preferences.tabPlacement === "right");
</script>

<template>
  <section :class="cn('flex min-h-0 flex-1', vertical ? 'flex-row' : 'flex-col')">
    <TabStrip v-if="stripInWorkspace" />

    <div
      v-for="tab in terminals.tabs"
      v-show="tab.id === terminals.activeTabId"
      :key="tab.id"
      data-tab-pane
      :data-tab-id="tab.id"
      :data-active="tab.id === terminals.activeTabId"
      :class="cn('flex min-h-0 flex-1 flex-col', atEnd ? 'order-1' : 'order-2')"
    >
      <template v-if="tab.kind === 'terminal'">
        <!--
          The terminal and its panel are one region whose direction follows the remembered position:
          a row when the files sit beside the shell, a column when they sit under it. The terminal's
          own strip belongs to the terminal column, so where the panel goes never takes the way out
          of the panel with it.
        -->
        <div class="flex min-h-0 flex-1" :class="panelIsSide(tab) ? 'flex-row' : 'flex-col'">
          <div :class="cn('flex min-h-0 min-w-0 flex-1 flex-col', panelSide(tab) === 'left' ? 'order-3' : 'order-1')">
            <!--
          The terminal carries the same floor as the panel while a panel is under it. Without it,
          the height somebody chose in a large window would eat the whole terminal in a small one:
          the panel shrinks first, and it shrinks past the point where the terminal is still there.
          With no panel open the floor is left off, because then the terminal is the only thing
          asking for room and squeezing it is exactly what the window shrinking means.
        -->
            <TerminalPane
              v-if="tab.handle"
              :key="tab.id"
              :handle="tab.handle"
              :active="tab.id === terminals.activeTabId"
              :interactive="tab.status === 'open'"
              :encoding="encodingOf(tab)"
              :backspace="backspaceOf(tab)"
              class="min-h-0 min-w-0 flex-1"
              :style="tab.panel && !panelIsSide(tab) ? { minHeight: `${MIN_PANE_PX}px` } : undefined"
            />

            <!--
          A tab that never got a handle has no screen to draw: the open itself failed, and this is
          its place to say so. It is the exception, not the state of a terminal that ended — one
          that ended keeps its handle and keeps drawing what it last showed.
        -->
            <div v-else class="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-6 text-center">
              <CircleAlert :size="18" class="text-destructive" />
              <p class="text-xs text-muted-foreground">{{ t("terminal.tab.failedTitle") }}</p>
            </div>

            <!--
          The command bar, directly under the terminal and above its strip: a line typed here goes
          into the shell above it, which is what having it next to the prompt is about. It is drawn
          only while the terminal is open — a bar that could not deliver would be a lie.
        -->
            <CommandInputBar v-if="tab.status === 'open'" />

            <!--
          The terminal's own strip: where it is, and the way into the file panel. It stays when the
          panel is open — it is where the toggle that closes it lives — and it sits above the
          divider rather than below it, because what is under the line is the panel and nothing else.
        -->
            <footer
              data-terminal-strip
              class="flex shrink-0 items-center gap-2 border-t border-border bg-card/30 px-3 py-1 cursor-default select-none"
            >
              <span class="min-w-0 flex-1 truncate font-mono text-[10px] text-muted-foreground">
                {{ sessionEndpoint(tab.session, t) }}
              </span>
              <TerminalSchemeMenu />
              <button
                v-if="protocolSupports(tab.session.protocol, 'sftp')"
                type="button"
                :class="
                  cn(
                    'flex shrink-0 items-center gap-1 rounded-md px-2 py-0.5 text-[10px] transition-colors',
                    filesShowing(tab)
                      ? 'bg-accent text-foreground'
                      : 'text-muted-foreground hover:bg-accent hover:text-foreground',
                    'disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent',
                  )
                "
                :disabled="tab.panel === null && tab.status !== 'open'"
                :aria-pressed="filesShowing(tab)"
                :title="filesButtonTitle(tab)"
                @click="showFiles(tab)"
              >
                <Folder :size="11" />
                {{ t("terminal.panel.label") }}
              </button>
              <button
                type="button"
                :class="
                  cn(
                    'flex shrink-0 items-center gap-1 rounded-md px-2 py-0.5 text-[10px] transition-colors',
                    panelTab[tab.id] === 'commands'
                      ? 'bg-accent text-foreground'
                      : 'text-muted-foreground hover:bg-accent hover:text-foreground',
                  )
                "
                :aria-pressed="panelTab[tab.id] === 'commands'"
                @click="showCommands(tab)"
              >
                <ListChecks :size="11" />
                {{ t("commands.panel") }}
              </button>
            </footer>

            <footer
              v-if="tab.status === 'ended' || tab.failure"
              :class="
                cn(
                  'shrink-0 border-t px-4 py-2',
                  tab.failure ? 'border-destructive/40 bg-destructive/10' : 'border-border bg-muted/40',
                )
              "
            >
              <div class="flex flex-wrap items-center justify-between gap-2">
                <div class="min-w-0">
                  <p v-if="tab.status === 'ended'" class="text-xs">
                    {{ tab.disconnected ? t("terminal.tab.disconnected") : t("terminal.tab.ended") }}
                  </p>
                  <p v-if="translateStatus(tab)" class="text-[11px] text-muted-foreground">
                    {{ translateStatus(tab) }}
                  </p>
                  <p v-if="tab.failure" class="text-xs">{{ describeShellError(translate, tab.failure).message }}</p>
                  <p v-if="tab.failure" class="font-mono text-[10px] text-muted-foreground">
                    {{ describeShellError(translate, tab.failure).code }}
                  </p>
                </div>
                <button
                  v-if="tab.status !== 'open'"
                  type="button"
                  class="flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-xs transition-colors hover:bg-accent"
                  @click="terminals.reconnect(tab.id)"
                >
                  <RotateCw :size="12" />
                  {{ t("terminal.tab.reconnect") }}
                </button>
              </div>
            </footer>
          </div>

          <template v-if="panelShowing(tab)">
            <SplitDivider
              :size="panelIsSide(tab) ? preferences.resolvedPanelWidth : preferences.resolvedPanelHeight"
              :orientation="panelIsSide(tab) ? 'vertical' : 'horizontal'"
              :pane="panelSide(tab) === 'left' ? 'start' : 'end'"
              :min-size="panelIsSide(tab) ? MIN_FILE_PANEL_PX : MIN_PANE_PX"
              class="order-2"
              @move="panelIsSide(tab) ? preferences.setPanelWidth($event) : preferences.setPanelHeight($event)"
              @settle="panelIsSide(tab) ? preferences.rememberPanelWidth() : preferences.rememberPanelHeight()"
              @reset="panelIsSide(tab) ? preferences.resetPanelWidth() : preferences.resetPanelHeight()"
            />

            <!--
            The panel is the only thing on its side of the line, and the floor is a minimum size
            rather than a clamp computed here: a window too small for the size somebody chose
            shrinks the panel, because the half that has to stay usable is the terminal.
          -->
            <div
              data-terminal-panel
              :data-side="panelSide(tab) ?? 'bottom'"
              :class="cn('flex flex-col overflow-hidden', panelIsSide(tab) ? 'order-1 shrink-0' : 'order-3 shrink')"
              :style="panelStyle(tab)"
            >
              <CommandsPanel
                v-if="panelTab[tab.id] === 'commands'"
                :panel-placement="terminals.effectiveCommandPlacement(tab)"
                class="min-h-0 flex-1"
                @choose-panel-placement="chooseCommandPlacement(tab, $event)"
              />

              <FileBrowser
                v-else-if="tab.panel?.handle"
                :handle="tab.panel?.handle"
                :protocol="tab.session.protocol"
                :compact="panelIsSide(tab)"
                :panel-placement="terminals.effectivePlacement(tab)"
                class="min-h-0 flex-1"
                @choose-panel-placement="choosePlacement(tab, $event)"
              />

              <div v-else class="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-4 text-center">
                <CircleAlert v-if="tab.panel?.status === 'failed'" :size="18" class="text-destructive" />
                <LoaderCircle v-else :size="18" class="animate-spin text-muted-foreground" />
                <p v-if="tab.panel?.failure" class="text-xs">
                  {{ describeShellError(translate, tab.panel?.failure).message }}
                </p>
                <p v-if="tab.panel?.failure" class="font-mono text-[10px] text-muted-foreground">
                  {{ describeShellError(translate, tab.panel?.failure).code }}
                </p>
                <p v-else class="text-xs text-muted-foreground">{{ t("terminal.panel.opening") }}</p>
                <button
                  v-if="tab.panel?.status === 'failed'"
                  type="button"
                  class="flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-xs transition-colors hover:bg-accent"
                  @click="terminals.openPanel(tab)"
                >
                  <RotateCw :size="12" />
                  {{ t("terminal.panel.retry") }}
                </button>
              </div>

              <!--
              A browser that refused to close is still open, so it is still drawn — with the reason
              underneath it, because this is the one failure that leaves the panel working.
            -->
              <footer
                v-if="tab.panel?.handle && tab.panel?.failure"
                class="shrink-0 border-t border-destructive/40 bg-destructive/10 px-3 py-1.5"
              >
                <p class="text-[11px]">{{ describeShellError(translate, tab.panel?.failure).message }}</p>
                <p class="font-mono text-[10px] text-muted-foreground">
                  {{ describeShellError(translate, tab.panel?.failure).code }}
                </p>
              </footer>
            </div>
          </template>
        </div>
      </template>

      <template v-else-if="tab.kind === 'files'">
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
              <p v-if="tab.failure" class="font-mono text-[10px] text-muted-foreground">
                {{ describeShellError(translate, tab.failure).code }}
              </p>
            </div>
            <!-- A browser that already has a handle cannot be opened again: a retry would be a
                 second browser on the same session. Only a failed open is retryable. -->
            <button
              v-if="tab.handle === null"
              type="button"
              class="flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-xs transition-colors hover:bg-accent"
              @click="terminals.reconnect(tab.id)"
            >
              <RotateCw :size="12" />
              {{ t("files.tab.retry") }}
            </button>
          </div>
        </footer>
      </template>

      <!--
        A page about the host. It is handed the session rather than a handle, and the frame decides
        whether there is a connection to read through — which is why a page can be left open across
        a reconnect and why one whose host is not connected is a page waiting rather than a failure.
        The process page is handed whether it is the tab on screen as well, because it reads a list
        of its own and should do that only while somebody is looking at it.
      -->
      <template v-else-if="tab.kind === 'inspect'">
        <HostPage :session="tab.session" :active="tab.id === terminals.activeTabId">
          <template #default="{ live }">
            <SystemReport v-if="tab.view === 'system'" :live="live" />
            <ProcessTable v-else-if="tab.view === 'processes'" :active="tab.id === terminals.activeTabId" />
            <NetworkReport v-else :live="live" :active="tab.id === terminals.activeTabId" />
          </template>
        </HostPage>
      </template>

      <!--
        The settings page. It is kept mounted like every other pane, so a half-typed search is still
        there after a look at a terminal — and it is a tab rather than a dialog because a preference
        is changed while the thing it is about stays on screen next to it.
      -->
      <SettingsPage v-else-if="tab.kind === 'settings'" />

      <!--
        The batch panel. It is kept mounted like the other panes, and for a stronger reason than
        most: a run outlives a look at another tab, and a panel that unmounted would come back with
        rows that stopped being updated halfway through.
      -->
      <BatchPanel v-else-if="tab.kind === 'batch'" />

      <ContainerPane v-else-if="tab.kind === 'container'" :tab="tab" :active="tab.id === terminals.activeTabId" />

      <KubernetesPane v-else-if="tab.kind === 'kubernetes'" :tab="tab" :active="tab.id === terminals.activeTabId" />

      <VmwarePane v-else-if="tab.kind === 'vmware'" :tab="tab" :active="tab.id === terminals.activeTabId" />
    </div>
  </section>
</template>
