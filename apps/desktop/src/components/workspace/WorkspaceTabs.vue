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
import {
  CircleAlert,
  Cpu,
  Folder,
  List,
  ListChecks,
  LoaderCircle,
  Network,
  PanelBottom,
  RotateCw,
  Settings,
  Terminal,
  X,
} from "@lucide/vue";
import { ref } from "vue";
import { useI18n } from "vue-i18n";

import BatchPanel from "@/components/batch/BatchPanel.vue";
import CommandInputBar from "@/components/commands/CommandInputBar.vue";
import CommandsPanel from "@/components/commands/CommandsPanel.vue";
import FileBrowser from "@/components/files/FileBrowser.vue";
import HostPage from "@/components/inspect/HostPage.vue";
import NetworkReport from "@/components/inspect/NetworkReport.vue";
import ProcessTable from "@/components/inspect/ProcessTable.vue";
import SystemReport from "@/components/inspect/SystemReport.vue";
import SettingsPage from "@/components/settings/SettingsPage.vue";
import TerminalPane from "@/components/terminal/TerminalPane.vue";
import SplitDivider from "@/components/workspace/SplitDivider.vue";
import { useContextMenu } from "@/composables/useContextMenu";
import { describeShellError } from "@/i18n/errors";
import { MENU_TITLES } from "@/lib/menu/titles";
import { tabMenu } from "@/lib/menu/workspace";
import { MIN_PANE_PX } from "@/lib/workspace/split";
import { cn } from "@/lib/utils";
import { usePreferencesStore } from "@/stores/preferences";
import { type TerminalTab, type WorkspaceTab, type InspectView, useTerminalsStore } from "@/stores/terminals";

const { t } = useI18n();
const preferences = usePreferencesStore();
const terminals = useTerminalsStore();
const { openMenu } = useContextMenu();

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

function showCommands(tab: TerminalTab): void {
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

/** Whether this tab is already drawing a browser: one with a handle must not be opened a second time. */
function hasHandle(tab: WorkspaceTab): boolean {
  if (tab.kind === "inspect" || tab.kind === "settings" || tab.kind === "batch") {
    return false;
  }
  return tab.handle !== null;
}

/**
 * The strip's menu, which is about the tab under the pointer.
 *
 * Closing the others and the ones to the right go through the store rather than through this
 * component, because a tab being closed can take a browser and a channel with it and only the store
 * knows how that conversation is had — and how a refusal leaves things.
 */
function openTabMenu(event: MouseEvent, tab: WorkspaceTab, index: number): void {
  openMenu(
    event,
    { key: MENU_TITLES.tab },
    tabMenu(
      {
        kind: tab.kind,
        // The settings page has no stream of its own either: it writes a file rather than holding
        // a connection, so there is nothing for the menu to offer a retry for.
        status: tab.kind === "inspect" || tab.kind === "settings" || tab.kind === "batch" ? null : tab.status,
        hasHandle: hasHandle(tab),
        index,
        total: terminals.tabs.length,
      },
      {
        close: () => void terminals.closeTab(tab.id),
        closeOthers: () => void terminals.closeOthers(tab.id),
        closeToTheRight: () => void terminals.closeToTheRight(tab.id),
        reconnect: () => void terminals.reconnect(tab.id),
      },
    ),
  );
}

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

function translateStatus(tab: TerminalTab): string | null {
  if (tab.status !== "ended") {
    return null;
  }
  return tab.reason ? t("terminal.tab.endedReason", { reason: tab.reason }) : t("terminal.tab.endedNoReason");
}

/** What a tab says in the strip: which session, or which page. */
function tabLabel(tab: WorkspaceTab): string {
  switch (tab.kind) {
    case "inspect":
      // A page names the machine it is about rather than only its kind: two hosts' system pages
      // look identical in a tab strip otherwise, and which machine a tab is showing is exactly
      // what somebody reads a strip for.
      return `${t(inspectTitleKey(tab.view))} · ${tab.session.host}`;
    case "settings":
      return t("settings.title");
    case "batch":
      return t("batch.title");
    default:
      return tab.session.name;
  }
}

function inspectTitleKey(view: InspectView): string {
  switch (view) {
    case "system":
      return "inspect.system.title";
    case "processes":
      return "inspect.processes.title";
    default:
      return "inspect.network.title";
  }
}

/** What the close button says. Each kind of tab closes something of its own. */
function closeLabel(tab: WorkspaceTab): string {
  switch (tab.kind) {
    case "files":
      return t("files.tab.close");
    case "inspect":
      return t("inspect.tab.close");
    case "settings":
      return t("settings.tab.close");
    case "batch":
      return t("batch.tab.close");
    default:
      return t("terminal.tab.close");
  }
}

/** A file browser has no stream to end, so among those only a failed one has a dot to show. */
function dotClass(tab: WorkspaceTab): string | null {
  if (tab.kind === "files") {
    return tab.status === "failed" ? "bg-destructive" : null;
  }
  // Neither page has a status of its own: an inspect page's connection belongs to a terminal
  // somewhere else, and the settings page has none to report.
  if (tab.kind === "inspect" || tab.kind === "settings" || tab.kind === "batch") {
    return null;
  }
  switch (tab.status) {
    case "failed":
      return "bg-destructive";
    case "ended":
      return "bg-warning";
    default:
      return "bg-success";
  }
}
</script>

<template>
  <section class="flex min-h-0 flex-1 flex-col">
    <div class="flex shrink-0 items-stretch gap-1 overflow-x-auto border-b border-border bg-card/30 px-2 pt-1.5">
      <div
        v-for="(tab, index) in terminals.tabs"
        :key="tab.id"
        :class="
          cn(
            'flex max-w-60 shrink-0 items-center gap-1 rounded-t-md border border-b-0 px-2',
            tab.id === terminals.activeTabId
              ? 'border-border bg-background'
              : 'border-transparent text-muted-foreground hover:bg-accent/60',
          )
        "
        @contextmenu="openTabMenu($event, tab, index)"
      >
        <button
          type="button"
          class="flex min-w-0 items-center gap-2 py-1.5 text-xs"
          @click="terminals.activate(tab.id)"
        >
          <span v-if="dotClass(tab)" class="size-1.5 shrink-0 rounded-full" :class="dotClass(tab)"></span>
          <Terminal v-if="tab.kind === 'terminal'" :size="12" class="shrink-0" />
          <Folder v-else-if="tab.kind === 'files'" :size="12" class="shrink-0" />
          <Settings v-else-if="tab.kind === 'settings'" :size="12" class="shrink-0" />
          <ListChecks v-else-if="tab.kind === 'batch'" :size="12" class="shrink-0" />
          <Cpu v-else-if="tab.view === 'system'" :size="12" class="shrink-0" />
          <List v-else-if="tab.view === 'processes'" :size="12" class="shrink-0" />
          <Network v-else :size="12" class="shrink-0" />
          <span class="truncate">{{ tabLabel(tab) }}</span>
        </button>
        <button
          type="button"
          class="grid size-4 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          :title="closeLabel(tab)"
          :aria-label="closeLabel(tab)"
          @click="terminals.closeTab(tab.id)"
        >
          <X :size="11" />
        </button>
      </div>
    </div>

    <div
      v-for="tab in terminals.tabs"
      v-show="tab.id === terminals.activeTabId"
      :key="tab.id"
      class="flex min-h-0 flex-1 flex-col"
    >
      <template v-if="tab.kind === 'terminal'">
        <!--
          The terminal carries the same floor as the panel while a panel is under it. Without it,
          the height somebody chose in a large window would eat the whole terminal in a small one:
          the panel shrinks first, and it shrinks past the point where the terminal is still there.
          With no panel open the floor is left off, because then the terminal is the only thing
          asking for room and squeezing it is exactly what the window shrinking means.
        -->
        <TerminalPane
          v-if="tab.handle"
          :key="`${tab.id}:${tab.handle}`"
          :handle="tab.handle"
          :active="tab.id === terminals.activeTabId"
          :interactive="tab.status === 'open'"
          class="min-h-0 flex-1"
          :style="tab.panel ? { minHeight: `${MIN_PANE_PX}px` } : undefined"
        />

        <!--
          The command bar, directly under the terminal and above its strip: a line typed here goes
          into the shell above it, which is what having it next to the prompt is about. It is drawn
          only while the terminal is open — a bar that could not deliver would be a lie.
        -->
        <CommandInputBar v-if="tab.status === 'open'" />

        <div v-else class="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-6 text-center">
          <CircleAlert :size="18" class="text-destructive" />
          <p class="text-xs text-muted-foreground">{{ t("terminal.tab.failedTitle") }}</p>
        </div>

        <!--
          The terminal's own strip: where it is, and the way into the file panel. It stays when the
          panel is open — it is where the toggle that closes it lives — and it sits above the
          divider rather than below it, because what is under the line is the panel and nothing else.
        -->
        <footer class="flex shrink-0 items-center gap-2 border-t border-border bg-card/30 px-3 py-1">
          <span class="min-w-0 flex-1 truncate font-mono text-[10px] text-muted-foreground">
            {{ tab.session.user }}@{{ tab.session.host }}:{{ tab.session.port }}
          </span>
          <button
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
            :title="filesShowing(tab) ? t('terminal.panel.close') : t('terminal.panel.open')"
            @click="showFiles(tab)"
          >
            <PanelBottom :size="11" />
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
              <p v-if="tab.status === 'ended'" class="text-xs">{{ t("terminal.tab.ended") }}</p>
              <p v-if="translateStatus(tab)" class="text-[11px] text-muted-foreground">{{ translateStatus(tab) }}</p>
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

        <template v-if="panelShowing(tab)">
          <SplitDivider
            :height="preferences.resolvedPanelHeight"
            @move="preferences.setPanelHeight($event)"
            @settle="preferences.rememberPanelHeight()"
            @reset="preferences.resetPanelHeight()"
          />

          <!--
            The panel is the only thing below the line, and the floor is a minimum height rather
            than a clamp computed here: a window too short for the height somebody chose shrinks
            the panel, because the half that has to stay usable is the terminal.
          -->
          <div
            class="flex shrink flex-col overflow-hidden"
            :style="{ height: `${preferences.resolvedPanelHeight}px`, minHeight: `${MIN_PANE_PX}px` }"
          >
            <CommandsPanel v-if="panelTab[tab.id] === 'commands'" class="min-h-0 flex-1" />

            <FileBrowser v-else-if="tab.panel?.handle" :handle="tab.panel?.handle" class="min-h-0 flex-1" />

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
      </template>

      <template v-else-if="tab.kind === 'files'">
        <FileBrowser v-if="tab.handle" :handle="tab.handle" class="min-h-0 flex-1" />

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
        <HostPage :session="tab.session" :view="tab.view" :active="tab.id === terminals.activeTabId">
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
    </div>
  </section>
</template>
