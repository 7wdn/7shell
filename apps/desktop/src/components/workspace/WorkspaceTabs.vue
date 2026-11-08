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
import { computed, ref } from "vue";
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
import { sessionTabTitle } from "@/lib/sessions/reopen";
import { MIN_PANE_PX } from "@/lib/workspace/split";
import { TAB_MAX_WIDTH_PX } from "@/types/app";
import { cn } from "@/lib/utils";
import { usePreferencesStore } from "@/stores/preferences";
import {
  type TerminalTab,
  type WorkspaceTab,
  type InspectView,
  sessionOf,
  useTerminalsStore,
} from "@/stores/terminals";

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
        total: drawnIds.value.length,
      },
      {
        close: () => void terminals.closeTab(tab.id),
        closeOthers: () => void terminals.closeOthers(tab.id),
        // The drawn order, because that is what "to the right" means to somebody looking at the bar:
        // under a sort or a grouping the array order and the drawn one are not the same list.
        closeToTheRight: () => void terminals.closeToTheRight(tab.id, drawnIds.value),
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
      // A session opened twice is drawn twice, and a strip of two identical names is a strip
      // somebody clicks wrong: the second one says which copy it is (see `lib/sessions/reopen`).
      return sessionTabTitle(tab.session.name, terminals.copyOf(tab.id));
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

/**
 * Whether the chrome is drawn in the classic layout: one tab bar, tighter headers.
 *
 * Asked here rather than expressed in the stylesheet because the tab's other classes are already a
 * function of its state — an active tab is a different shape, not a different colour — and two
 * systems deciding the same element's classes is how one of them ends up winning by accident. The
 * rest of the choice is a number, and that one is a token (see `globals.css`).
 */
const classic = computed(() => preferences.appLayout === "classic");

/**
 * Whether the bar wraps instead of scrolling.
 *
 * Read here rather than left to the stylesheet, because wrapping is not only a look: the bar stops
 * being one row tall, and the panel underneath gives up the difference — which is the whole trade the
 * setting makes, and it has to be decided where the bar is drawn. A bar down one side never wraps:
 * its rows are all the width it has, and the tabs scroll down it by definition.
 */
const wrapping = computed(() => preferences.tabLayout === "wrap" && !vertical.value);

/** Whether the bar is a column rather than a row, which follows from the edge it is drawn against. */
const vertical = computed(() => preferences.tabPlacement === "left" || preferences.tabPlacement === "right");

/**
 * Whether the bar is drawn after the panes rather than before them.
 *
 * The bar is the first thing in the markup in every case — it is the way through the workspace, and
 * a keyboard reaches it first — and the two placements that put it at the far end say so with an
 * order rather than by being written twice.
 */
const atEnd = computed(() => preferences.tabPlacement === "bottom" || preferences.tabPlacement === "right");

/**
 * The tabs in the order the bar draws them.
 *
 * The store's array is the manual order — what the tabs were opened in, until somebody drags one —
 * and the two other orders are read off it rather than written into it: a sorted copy each time, so
 * that going back to the manual order is the list that was always there. Names are compared in the
 * interface's own language, because that is the language they are written in.
 */
const ordered = computed(() => {
  const tabs = terminals.tabs;
  if (preferences.tabSortMode === "created-asc") {
    const opened = terminals.openedOrder;
    return [...tabs].sort((left, right) => opened.indexOf(left.id) - opened.indexOf(right.id));
  }
  if (preferences.tabSortMode === "title-asc") {
    return [...tabs].sort((left, right) => tabLabel(left).localeCompare(tabLabel(right), preferences.locale));
  }
  return tabs;
});

/** One line of the bar: a run's name, or a tab. */
type StripEntry =
  | { readonly kind: "group"; readonly id: string; readonly label: string }
  | {
      readonly kind: "tab";
      readonly id: string;
      readonly tab: WorkspaceTab;
    };

/**
 * The bar as it is drawn: the tabs in their order, cut into runs by session when the grouping says so.
 *
 * The group a tab belongs to is the session it is about. The tabs that are about no host at all — the
 * settings page and the batch panel — are drawn after every run rather than under a heading of their
 * own: they belong to nobody's session, and a heading for them would be a heading for nothing.
 */
const strip = computed<StripEntry[]>(() => {
  if (preferences.tabGroupMode !== "session") {
    return ordered.value.map((tab) => ({ kind: "tab" as const, id: tab.id, tab }));
  }
  const entries: StripEntry[] = [];
  const seen = new Set<string>();
  for (const tab of ordered.value) {
    const session = sessionOf(tab);
    if (!session) {
      continue;
    }
    if (!seen.has(session.id)) {
      seen.add(session.id);
      entries.push({ kind: "group", id: `group:${session.id}`, label: session.name });
    }
    entries.push({ kind: "tab", id: tab.id, tab });
  }
  for (const tab of ordered.value) {
    if (!sessionOf(tab)) {
      entries.push({ kind: "tab", id: tab.id, tab });
    }
  }
  return entries;
});

/** The ids in the order they are drawn, which is what the bar's own menu counts to the right. */
const drawnIds = computed(() => strip.value.flatMap((entry) => (entry.kind === "tab" ? [entry.id] : [])));

/**
 * How wide a horizontal tab may be drawn, or nothing where the name sets the width.
 *
 * The step is the setting and the pixels are what drawing it means, so the number is looked up here
 * rather than stored in the settings file: somebody choosing "medium" is choosing a look, and a build
 * that decided medium is a different number of pixels would be changing the drawing, not the choice.
 */
const maxTabWidth = computed(() => TAB_MAX_WIDTH_PX[preferences.tabMaxWidth]);

/**
 * Whether a tab can be dragged, which is only while the bar draws the manual order.
 *
 * Dragging under a sort would be a move into a list that is not the one on screen: the tab would land
 * somewhere in the array and the bar would redraw it wherever the sort says, which is a gesture that
 * appears to do nothing. So the handle is there only in the manual order — which is also the order a
 * drag writes to.
 */
const draggable = computed(() => preferences.tabSortMode === "manual");

/** The tab being dragged, and the one it is over, while a drag is in flight. */
const dragging = ref<string | null>(null);
const dragOver = ref<string | null>(null);

function onDragStart(event: DragEvent, tab: WorkspaceTab): void {
  if (!draggable.value) {
    return;
  }
  dragging.value = tab.id;
  event.dataTransfer?.setData("text/plain", tab.id);
  if (event.dataTransfer) {
    event.dataTransfer.effectAllowed = "move";
  }
}

function onDragOver(event: DragEvent, tab: WorkspaceTab): void {
  if (!dragging.value || dragging.value === tab.id) {
    return;
  }
  // A drop is only allowed where the drag over it said it would be, which is the whole of what the
  // browser wants to know before it will let one happen.
  event.preventDefault();
  dragOver.value = tab.id;
}

function onDrop(event: DragEvent, tab: WorkspaceTab): void {
  const moved = dragging.value;
  dragging.value = null;
  dragOver.value = null;
  if (!moved || moved === tab.id) {
    return;
  }
  event.preventDefault();
  terminals.moveTab(moved, tab.id);
}

function onDragEnd(): void {
  dragging.value = null;
  dragOver.value = null;
}

/**
 * What a tab wears: the layout's shape and the tab's own state together.
 *
 * One function rather than nested conditions in the template, because the two layouts disagree about
 * the shape of a tab and agree about what its state means — in both, the tab on screen is painted in
 * the panel's own surface and the ones beside it are not. The separated bar is a row of boxes with
 * air around each one, so the tab on screen is a box with a brighter edge; the classic bar is one run
 * of tabs joined to the top edge, so there the tab on screen is marked by a line along its bottom,
 * which is the edge the panel starts at.
 */
function tabClass(tab: WorkspaceTab): string {
  const active = tab.id === terminals.activeTabId;
  if (vertical.value) {
    // A column of names, in both chrome layouts: a box with a border inside a bar that is itself one
    // row wide would be a box drawn twice, and what tells the tab on screen apart is the rail down its
    // side and the surface under it.
    return cn(
      "flex w-full shrink-0 items-center gap-2 border-l-[3px] px-2 py-1.5",
      active
        ? "border-l-primary bg-accent text-foreground"
        : "border-l-transparent text-muted-foreground hover:bg-accent/60",
    );
  }
  const state = active
    ? classic.value
      ? "border-b-foreground/60 bg-background text-foreground"
      : "border-foreground/25 bg-background text-foreground"
    : classic.value
      ? "border-b-transparent text-muted-foreground hover:bg-accent/60"
      : "border-border/60 text-muted-foreground hover:bg-accent/60";
  const shape = classic.value
    ? "h-8 border-r border-b-2 border-border px-3"
    : "rounded-md border border-border/60 px-2 py-1";
  return cn("flex shrink-0 items-center gap-1", shape, state, dragging.value === tab.id && "opacity-50");
}

/** What the run a group of tabs belongs to is called, which is a name rather than a control. */
function groupClass(): string {
  return cn(
    "flex shrink-0 items-center gap-1.5 text-[11px] font-medium text-muted-foreground",
    vertical.value ? "w-full px-2 pt-2 pb-0.5" : "px-1.5",
  );
}

/** The size a tab is drawn at: the step's pixels across, or whatever the name asks for. */
function tabStyle(): Record<string, string> | undefined {
  if (vertical.value || maxTabWidth.value === null) {
    return undefined;
  }
  return { maxWidth: `${maxTabWidth.value}px` };
}
</script>

<template>
  <section :class="cn('flex min-h-0 flex-1', vertical ? 'flex-row' : 'flex-col')">
    <!--
      The strip of tabs is chrome, and so is the terminal's own strip below: a name on a tab is
      something to press rather than to copy, and a drag along it is somebody trying to scroll or
      re-order. `docs/ui-conventions.md` says which surfaces are text and which are not.
    -->
    <div
      data-workspace-tabs
      :data-tab-placement="preferences.tabPlacement"
      :class="
        cn(
          // The bar itself. Across the window it is one row that scrolls sideways, or as many rows as
          // the tabs need — and a bar that wraps has no fixed height and nothing to clip, because the
          // rows it grew are the point, and the panel below is what gets shorter. Down one side it is
          // a column, which scrolls the other way and never wraps.
          'flex shrink-0 bg-card/30 cursor-default select-none',
          vertical
            ? 'h-full w-60 flex-col overflow-y-auto py-1'
            : 'items-stretch ' + (wrapping ? 'flex-wrap content-start overflow-visible' : 'overflow-x-auto'),
          // The edge it is drawn against. A bar at the far end is drawn after the panes rather than
          // before them, which is the same markup read in the other order.
          vertical
            ? preferences.tabPlacement === 'left'
              ? 'order-1 border-r border-border'
              : 'order-2 border-l border-border'
            : preferences.tabPlacement === 'top'
              ? 'order-1 border-b border-border'
              : 'order-2 border-t border-border',
          // The tabs themselves: boxes with air around them, or one run joined to the edge. The
          // heights are close on purpose — the room this layout saves is in the headers, not here.
          vertical ? 'gap-0.5 px-1.5' : classic ? 'gap-0 px-0' : 'gap-1.5 px-2 py-1.5',
        )
      "
    >
      <template v-for="entry in strip" :key="entry.id">
        <!--
          A run of tabs that belong to one session, named once: the grouping is a heading rather than
          an indent, because the tabs inside it are the same tabs and every one of them still has to
          be readable and pressable.
        -->
        <div v-if="entry.kind === 'group'" data-tab-group :class="groupClass()">
          <span class="truncate">{{ entry.label }}</span>
        </div>

        <div
          v-else
          :class="tabClass(entry.tab)"
          :style="tabStyle()"
          :draggable="draggable"
          :data-tab="entry.tab.id"
          :data-drag-over="dragOver === entry.tab.id ? 'true' : undefined"
          @contextmenu="openTabMenu($event, entry.tab, drawnIds.indexOf(entry.tab.id))"
          @dragstart="onDragStart($event, entry.tab)"
          @dragover="onDragOver($event, entry.tab)"
          @drop="onDrop($event, entry.tab)"
          @dragend="onDragEnd"
        >
          <button
            type="button"
            :class="cn('flex min-w-0 items-center gap-2 text-xs', vertical ? 'flex-1 py-1' : 'py-1.5')"
            @click="terminals.activate(entry.tab.id)"
          >
            <span v-if="dotClass(entry.tab)" class="size-1.5 shrink-0 rounded-full" :class="dotClass(entry.tab)"></span>
            <Terminal v-if="entry.tab.kind === 'terminal'" :size="12" class="shrink-0" />
            <Folder v-else-if="entry.tab.kind === 'files'" :size="12" class="shrink-0" />
            <Settings v-else-if="entry.tab.kind === 'settings'" :size="12" class="shrink-0" />
            <ListChecks v-else-if="entry.tab.kind === 'batch'" :size="12" class="shrink-0" />
            <Cpu v-else-if="entry.tab.view === 'system'" :size="12" class="shrink-0" />
            <List v-else-if="entry.tab.view === 'processes'" :size="12" class="shrink-0" />
            <Network v-else :size="12" class="shrink-0" />
            <!-- The whole name on hover, which is what makes a width step a choice rather than a
                 loss: a truncated tab is read by its first word and its title. -->
            <span class="truncate" :title="tabLabel(entry.tab)">{{ tabLabel(entry.tab) }}</span>
          </button>
          <button
            type="button"
            class="grid size-4 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :title="closeLabel(entry.tab)"
            :aria-label="closeLabel(entry.tab)"
            @click="terminals.closeTab(entry.tab.id)"
          >
            <X :size="11" />
          </button>
        </div>
      </template>
    </div>

    <div
      v-for="tab in terminals.tabs"
      v-show="tab.id === terminals.activeTabId"
      :key="tab.id"
      :class="cn('flex min-h-0 flex-1 flex-col', atEnd ? 'order-1' : 'order-2')"
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
        <footer
          data-terminal-strip
          class="flex shrink-0 items-center gap-2 border-t border-border bg-card/30 px-3 py-1 cursor-default select-none"
        >
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
    </div>
  </section>
</template>
