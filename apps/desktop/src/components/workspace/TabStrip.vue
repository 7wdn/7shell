<script setup lang="ts">
/**
 * The tabs, drawn wherever the window keeps them: along an edge of the workspace, or inside the
 * title bar.
 *
 * One component rather than two strips, because everything a tab carries — its name, the copy
 * number, its status dot, the menu, the drag order, the `+` — is the same in both places. What
 * changes is only the direction and the chrome around it: a column down a side, a row joined to an
 * edge, or a row inside the window's own bar with no background of its own.
 *
 * The title-bar mode is deliberately one row. It is there to save a row, so a wrapping bar would
 * give the row back; tabs that do not fit scroll instead, and the empty room after them is the
 * window's drag region.
 */
import {
  Boxes,
  Cpu,
  Folder,
  List,
  ListChecks,
  MonitorCog,
  Network,
  Plus,
  Settings,
  ShipWheel,
  Terminal,
  X,
} from "@lucide/vue";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import SystemIcon from "@/components/ui/SystemIcon.vue";
import { useContextMenu } from "@/composables/useContextMenu";
import { MENU_TITLES } from "@/lib/menu/titles";
import { tabMenu } from "@/lib/menu/workspace";
import { connectionMark } from "@/lib/sessions/marks";
import { sessionTabTitle } from "@/lib/sessions/reopen";
import { cn } from "@/lib/utils";
import { TAB_MAX_WIDTH_PX } from "@/types/app";
import { useAppStore } from "@/stores/app";
import { usePreferencesStore } from "@/stores/preferences";
import {
  type InspectView,
  type TerminalTab,
  type WorkspaceTab,
  sessionOf,
  useTerminalsStore,
} from "@/stores/terminals";

const props = withDefaults(
  defineProps<{
    /** The workspace's edge, or the window's title bar. */
    mode?: "workspace" | "titlebar";
  }>(),
  { mode: "workspace" },
);

const { t } = useI18n();
const preferences = usePreferencesStore();
const terminals = useTerminalsStore();
const app = useAppStore();
const { openMenu } = useContextMenu();

/** Whether this strip is the one inside the title bar. */
const chrome = computed(() => props.mode === "titlebar");

/** Whether the bar is a column rather than a row. A title-bar strip is always a row. */
const vertical = computed(
  () => !chrome.value && (preferences.tabPlacement === "left" || preferences.tabPlacement === "right"),
);

/**
 * Whether the bar wraps instead of scrolling.
 *
 * A title-bar strip never wraps: it is there to take a row away, and wrapping would hand the row
 * back. A bar down one side never wraps either — its rows are all the width it has.
 */
const wrapping = computed(() => !chrome.value && preferences.tabLayout === "wrap" && !vertical.value);

/**
 * Whether the chrome is drawn in the classic layout.
 *
 * The title bar draws its own shape in both layouts: it is a strip of browser-like tabs inside a
 * 40px bar, and the workspace's two chrome shapes would be a box inside a box.
 */
const classic = computed(() => !chrome.value && preferences.appLayout === "classic");

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
 * The tabs that are about no host at all — the settings page and the batch panel — are drawn after
 * every run rather than under a heading of their own: they belong to nobody's session, and a heading
 * for them would be a heading for nothing.
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

/** How many terminal tabs are not open, which is what "connect all" would reconnect. */
const reconnectable = computed(
  () =>
    terminals.tabs.filter((entry): entry is TerminalTab => entry.kind === "terminal" && entry.status !== "open").length,
);

/** How wide a horizontal tab may be drawn, or nothing where the name sets the width. */
const maxTabWidth = computed(() => TAB_MAX_WIDTH_PX[preferences.tabMaxWidth]);

/** Whether a tab can be dragged, which is only while the bar draws the manual order. */
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
 * In the title bar the tab fills the bar's height and sits between hairline dividers, the way a
 * browser's tabs do. In the workspace it keeps the two shapes this build has always drawn: boxes
 * with air around them, or one run joined to the edge.
 */
function tabClass(tab: WorkspaceTab): string {
  const active = tab.id === terminals.activeTabId;
  if (chrome.value) {
    return cn(
      "my-1 flex shrink-0 items-center gap-1 rounded-md border px-3",
      active
        ? "border-foreground/25 bg-background text-foreground"
        : "border-border/60 bg-card/40 text-muted-foreground hover:bg-accent/60",
      dragging.value === tab.id && "opacity-50",
    );
  }
  if (vertical.value) {
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

/** The outer shape: the workspace's edge, or the free middle of the title bar. */
function stripClass(): string {
  if (chrome.value) {
    return cn("flex h-full min-w-0 flex-1 cursor-default items-stretch gap-1.5 overflow-x-auto select-none");
  }
  return cn(
    "flex shrink-0 bg-card/30 cursor-default select-none",
    vertical.value
      ? "h-full w-60 flex-col overflow-y-auto py-1"
      : "items-stretch " + (wrapping.value ? "flex-wrap content-start overflow-visible" : "overflow-x-auto"),
    vertical.value
      ? preferences.tabPlacement === "left"
        ? "order-1 border-r border-border"
        : "order-2 border-l border-border"
      : preferences.tabPlacement === "top"
        ? "order-1 border-b border-border"
        : "order-2 border-t border-border",
    vertical.value ? "gap-0.5 px-1.5" : classic.value ? "gap-0 px-0" : "gap-1.5 px-2 py-1.5",
  );
}

/** The connection kind's own mark, for the tabs whose row is about a kind rather than a host. */
function tabMark(tab: WorkspaceTab) {
  const session = sessionOf(tab);
  return session ? connectionMark(session, app.info?.os) : null;
}

/** The host's own mark, once a terminal has asked what the machine is. */
function tabSystem(tab: WorkspaceTab) {
  const session = sessionOf(tab);
  return session ? terminals.systemOf(session) : null;
}

/** What a tab says in the strip: which session, or which page. */
function tabLabel(tab: WorkspaceTab): string {
  switch (tab.kind) {
    case "inspect":
      return `${t(inspectTitleKey(tab.view))} · ${tab.session.host}`;
    case "settings":
      return t("settings.title");
    case "batch":
      return t("batch.title");
    default:
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
    case "container":
      return t("container.tab.close");
    case "kubernetes":
      return t("k8s.tab.close");
    default:
      return t("terminal.tab.close");
  }
}

/** A file browser has no stream to end, so among those only a failed one has a dot to show. */
function dotClass(tab: WorkspaceTab): string | null {
  if (tab.kind === "files") {
    return tab.status === "failed" ? "bg-destructive" : null;
  }
  if (
    tab.kind === "inspect" ||
    tab.kind === "settings" ||
    tab.kind === "batch" ||
    tab.kind === "container" ||
    tab.kind === "kubernetes" ||
    tab.kind === "vmware"
  ) {
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

/** Whether this tab is already drawing a browser: one with a handle must not be opened a second time. */
function hasHandle(tab: WorkspaceTab): boolean {
  if (
    tab.kind === "inspect" ||
    tab.kind === "settings" ||
    tab.kind === "batch" ||
    tab.kind === "container" ||
    tab.kind === "kubernetes" ||
    tab.kind === "vmware"
  ) {
    return false;
  }
  return tab.handle !== null;
}

/** The strip's menu, which is about the tab under the pointer. */
function openTabMenu(event: MouseEvent, tab: WorkspaceTab, index: number): void {
  openMenu(
    event,
    { key: MENU_TITLES.tab },
    tabMenu(
      {
        kind: tab.kind,
        status:
          tab.kind === "inspect" ||
          tab.kind === "settings" ||
          tab.kind === "batch" ||
          tab.kind === "container" ||
          tab.kind === "kubernetes" ||
          tab.kind === "vmware"
            ? null
            : tab.status,
        hasHandle: hasHandle(tab),
        index,
        total: drawnIds.value.length,
        reconnectable: reconnectable.value,
      },
      {
        duplicate: () => void terminals.duplicate(tab.id),
        close: () => void terminals.closeTab(tab.id),
        closeOthers: () => void terminals.closeOthers(tab.id),
        closeToTheLeft: () => void terminals.closeToTheLeft(tab.id, drawnIds.value),
        closeToTheRight: () => void terminals.closeToTheRight(tab.id, drawnIds.value),
        closeAll: () => void terminals.closeAll(),
        reconnect: () => void terminals.reconnect(tab.id),
        reconnectAll: () => void terminals.reconnectAll(),
        disconnect: () => void terminals.disconnect(tab.id),
      },
    ),
  );
}

/**
 * The plus: choose where the new tab connects. It used to duplicate the tab on screen, which made
 * the same control mean different things depending on what was selected; the tab's own menu still
 * offers that duplicate for anybody who wants one more shell on the same host.
 */
function openNewTab(): void {
  terminals.openNewTabPicker();
}
</script>

<template>
  <div data-workspace-tabs :data-tab-placement="chrome ? 'titleBar' : preferences.tabPlacement" :class="stripClass()">
    <template v-for="entry in strip" :key="entry.id">
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
          <SystemIcon v-if="tabMark(entry.tab)" :system="null" :mark="tabMark(entry.tab)" :size="14" />
          <SystemIcon
            v-else-if="entry.tab.kind === 'terminal' && tabSystem(entry.tab)"
            :system="tabSystem(entry.tab)"
            :size="14"
          />
          <Terminal v-else-if="entry.tab.kind === 'terminal'" :size="12" class="shrink-0" />
          <Folder v-else-if="entry.tab.kind === 'files'" :size="12" class="shrink-0" />
          <Settings v-else-if="entry.tab.kind === 'settings'" :size="12" class="shrink-0" />
          <ListChecks v-else-if="entry.tab.kind === 'batch'" :size="12" class="shrink-0" />
          <Boxes v-else-if="entry.tab.kind === 'container'" :size="12" class="shrink-0" />
          <ShipWheel v-else-if="entry.tab.kind === 'kubernetes'" :size="12" class="shrink-0" />
          <MonitorCog v-else-if="entry.tab.kind === 'vmware'" :size="12" class="shrink-0" />
          <Cpu v-else-if="entry.tab.view === 'system'" :size="12" class="shrink-0" />
          <List v-else-if="entry.tab.view === 'processes'" :size="12" class="shrink-0" />
          <Network v-else :size="12" class="shrink-0" />
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

    <button
      type="button"
      data-new-tab
      :class="
        cn(
          'grid size-6 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground',
          vertical ? 'mx-1.5 my-1' : chrome ? 'my-1 self-center' : 'my-1.5',
        )
      "
      :title="t('terminal.tab.newTab')"
      :aria-label="t('terminal.tab.newTab')"
      @click.stop="openNewTab()"
    >
      <Plus :size="13" />
    </button>

    <!-- The empty room after the tabs is what drags the window in title-bar mode. -->
    <div v-if="chrome" data-tauri-drag-region class="h-full min-w-4 flex-1"></div>
  </div>
</template>
