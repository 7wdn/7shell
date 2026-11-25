<script setup lang="ts">
/**
 * The tab strip's plus: choose where a new tab connects.
 *
 * The list is the session library as the search leaves it. Nothing is opened until a row is
 * chosen; closing this, or answering nothing, creates no tab. Choosing a session goes through the
 * same open the session menu uses, so one that is already open asks its usual question instead of
 * silently opening a second connection. The browser that is already open for a host is a duplicate
 * on the tab's own menu, not a second meaning for this control.
 */
import { ChevronRight, Columns2, Folder, FolderOpen, MonitorCog, Plus, Rows2, Search, Terminal, X } from "@lucide/vue";
import { DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from "reka-ui";
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import SystemIcon from "@/components/ui/SystemIcon.vue";
import { sessionEndpoint } from "@/lib/menu/sessions";
import { NO_FILTER, filterLibrary } from "@/lib/sessions/filter";
import { effectiveParentId } from "@/lib/sessions/groups";
import { connectionMark } from "@/lib/sessions/marks";
import { sessionTabTitle } from "@/lib/sessions/reopen";
import { type TreeRow, visibleRows } from "@/lib/sessions/tree";
import { systemLabel, systemMark } from "@/lib/system/icons";
import { useAppStore } from "@/stores/app";
import { useSessionEditorStore } from "@/stores/sessionEditor";
import { useSessionsStore } from "@/stores/sessions";
import { type TerminalTab, useTerminalsStore } from "@/stores/terminals";
import { protocolSupports } from "@/types/generated/protocols";
import type { Session } from "@/types/session";

type SessionRow = Extract<TreeRow, { kind: "session" }>;
type PickerChoice = { kind: "session"; session: Session } | { kind: "terminal"; tab: TerminalTab };

const props = withDefaults(defineProps<{ mode?: "tab" | "split" }>(), { mode: "tab" });

const { t } = useI18n();
const app = useAppStore();
const terminals = useTerminalsStore();
const library = useSessionsStore();
const editor = useSessionEditorStore();
const splitting = computed(() => props.mode === "split");

const query = ref("");
const chosen = ref(0);
const field = ref<HTMLInputElement | null>(null);
const list = ref<HTMLElement | null>(null);
/** Folded groups in this opening of the picker; closed and reopened starts fully expanded. */
const collapsed = ref<ReadonlySet<string>>(new Set());
const source = ref<"new" | "existing">(
  splitting.value && terminals.terminalTabs.some((tab) => tab.id !== terminals.splitPicker?.paneId)
    ? "existing"
    : "new",
);
const existing = computed(() => {
  const search = query.value.trim().toLocaleLowerCase();
  return terminals.terminalTabs.filter(
    (tab) =>
      tab.id !== terminals.splitPicker?.paneId &&
      [tab.session.name, tab.session.host, tab.session.user].some((value) =>
        value.toLocaleLowerCase().includes(search),
      ),
  );
});

const filtered = computed(() =>
  filterLibrary(
    library.groups,
    splitting.value
      ? library.sessions.filter((session) => protocolSupports(session.protocol, "shell"))
      : library.sessions,
    { ...NO_FILTER, query: query.value },
    library.openSessionIds,
  ),
);
const searching = computed(() => query.value.trim() !== "");
const rows = computed(() =>
  visibleRows(
    filtered.value.groups,
    filtered.value.sessions,
    searching.value ? new Set() : collapsed.value,
    filtered.value.revealed,
  ),
);
/** The whole tree while search is empty: a collapsed group still has to be drawn as a group with sessions. */
const allRows = computed(() =>
  visibleRows(filtered.value.groups, filtered.value.sessions, new Set(), filtered.value.revealed),
);
const groupsWithSessions = computed(() => {
  const groups = new Set<string>();
  for (const row of allRows.value) {
    if (row.kind !== "session") {
      continue;
    }
    let parent = row.parentId;
    while (parent !== null) {
      groups.add(parent);
      parent = effectiveParentId(library.groups, parent);
    }
  }
  return groups;
});
/** Empty groups are folders with nothing to connect to; the picker leaves them out. */
const displayRows = computed(() =>
  rows.value.filter((row) => {
    if (row.kind === "session") {
      return true;
    }
    return groupsWithSessions.value.has(row.id);
  }),
);
const options = computed(() => displayRows.value.filter((row): row is SessionRow => row.kind === "session"));
const choices = computed<PickerChoice[]>(() =>
  splitting.value && source.value === "existing"
    ? existing.value.map((tab) => ({ kind: "terminal", tab }))
    : options.value.map((row) => ({ kind: "session", session: row.session })),
);
const chosenID = computed(() => {
  const choice = choices.value[chosen.value];
  return choice?.kind === "session" ? choice.session.id : (choice?.tab.id ?? null);
});

watch([query, source], () => {
  chosen.value = 0;
});

watch(choices, (entries) => {
  if (chosen.value >= entries.length) {
    chosen.value = Math.max(0, entries.length - 1);
  }
});

watch(chosenID, async () => {
  await nextTick();
  list.value?.querySelector('[data-chosen="true"]')?.scrollIntoView({ block: "nearest" });
});

function endpoint(session: Session): string {
  return sessionEndpoint(session, t);
}

/** The same mark rules the session list uses: the connection kind first, then the machine. */
function hasMark(session: Session): boolean {
  return systemMark(terminals.systemOf(session)) !== null;
}

function markTitle(session: Session): string | null {
  const name = systemLabel(terminals.systemOf(session));
  return name ? t("sessions.list.system", { name }) : null;
}

function isCollapsed(id: string): boolean {
  return collapsed.value.has(id);
}

function toggleGroup(id: string): void {
  const next = new Set(collapsed.value);
  if (!next.delete(id)) {
    next.add(id);
  }
  collapsed.value = next;
}

function sessionIndex(id: string): number {
  return options.value.findIndex((row) => row.session.id === id);
}

function move(step: number): void {
  const count = choices.value.length;
  if (count === 0) {
    return;
  }
  chosen.value = (chosen.value + step + count) % count;
}

/** Arrows and Enter pick a row; an input method's own Enter (picking a character) does not. */
function onKeydown(event: KeyboardEvent): void {
  if (event.isComposing) {
    return;
  }
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    move(event.key === "ArrowDown" ? 1 : -1);
    return;
  }
  if (event.key === "Enter") {
    event.preventDefault();
    chooseChosen();
  }
}

function choose(session: Session): void {
  if (splitting.value) {
    const target = terminals.splitPicker;
    if (target) {
      terminals.closeSplitPicker();
      void terminals.splitNewTerminal(target.paneId, target.direction, session);
    }
    return;
  }
  if (!terminals.newTabPickerOpen) {
    return;
  }
  terminals.closeNewTabPicker();
  void terminals.requestReopen(session);
}

function chooseChosen(): void {
  const choice = choices.value[chosen.value];
  if (choice?.kind === "session") {
    choose(choice.session);
  } else if (choice) {
    chooseExisting(choice.tab);
  }
}

function chooseExisting(tab: TerminalTab): void {
  const target = terminals.splitPicker;
  if (target) {
    terminals.closeSplitPicker();
    terminals.movePaneIntoSplit(tab.id, target.paneId, target.direction);
  }
}

function createSession(): void {
  terminals.closeNewTabPicker();
  editor.startNew();
}

function close(): void {
  if (splitting.value) {
    terminals.closeSplitPicker();
  } else {
    terminals.closeNewTabPicker();
  }
}

function setOpen(open: boolean): void {
  if (!open) {
    close();
  }
}

function focusField(): void {
  field.value?.focus();
}

onMounted(async () => {
  await nextTick();
  focusField();
});
</script>

<template>
  <DialogRoot :open="true" @update:open="setOpen">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-overlay" />
      <DialogContent
        :data-new-tab-picker="splitting ? undefined : ''"
        :data-split-picker="splitting ? '' : undefined"
        class="fixed top-1/2 left-1/2 z-50 flex h-[min(34rem,calc(100vh-4rem))] w-[min(42rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col rounded-lg border border-border bg-card shadow-xl select-none focus:outline-none"
        @open-auto-focus.prevent="focusField"
      >
        <div class="flex items-start justify-between gap-2 px-4 pt-4">
          <DialogTitle class="text-sm font-medium">{{
            t(splitting ? "terminal.split.pickerTitle" : "sessions.newTab.title")
          }}</DialogTitle>
          <button
            type="button"
            tabindex="-1"
            class="grid size-6 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :aria-label="t('common.action.close')"
            @click="close()"
          >
            <X :size="14" />
          </button>
        </div>
        <DialogDescription class="sr-only">{{
          t(splitting ? "terminal.split.pickerDescription" : "sessions.newTab.search")
        }}</DialogDescription>

        <div v-if="splitting" class="mx-4 mt-3 flex flex-wrap items-center justify-between gap-2">
          <div
            class="flex gap-1 rounded-md border border-border p-1"
            role="group"
            :aria-label="t('terminal.split.source')"
          >
            <button
              type="button"
              data-split-source="existing"
              :aria-pressed="source === 'existing'"
              class="rounded-sm px-2 py-1 text-xs"
              :class="source === 'existing' ? 'bg-accent text-foreground' : 'text-muted-foreground hover:bg-accent/50'"
              @click="source = 'existing'"
            >
              {{ t("terminal.split.existing") }}
            </button>
            <button
              type="button"
              data-split-source="new"
              :aria-pressed="source === 'new'"
              class="rounded-sm px-2 py-1 text-xs"
              :class="source === 'new' ? 'bg-accent text-foreground' : 'text-muted-foreground hover:bg-accent/50'"
              @click="source = 'new'"
            >
              {{ t("terminal.split.new") }}
            </button>
          </div>
          <div class="flex gap-1">
            <button
              type="button"
              data-split-direction="right"
              :aria-pressed="terminals.splitPicker?.direction === 'right'"
              class="grid size-7 place-items-center rounded-sm border border-border text-muted-foreground hover:bg-accent"
              :class="terminals.splitPicker?.direction === 'right' && 'bg-accent text-foreground'"
              :title="t('terminal.split.right')"
              :aria-label="t('terminal.split.right')"
              @click="terminals.setSplitDirection('right')"
            >
              <Columns2 :size="14" />
            </button>
            <button
              type="button"
              data-split-direction="down"
              :aria-pressed="terminals.splitPicker?.direction === 'down'"
              class="grid size-7 place-items-center rounded-sm border border-border text-muted-foreground hover:bg-accent"
              :class="terminals.splitPicker?.direction === 'down' && 'bg-accent text-foreground'"
              :title="t('terminal.split.down')"
              :aria-label="t('terminal.split.down')"
              @click="terminals.setSplitDirection('down')"
            >
              <Rows2 :size="14" />
            </button>
          </div>
        </div>

        <div class="relative mx-4 mt-3">
          <Search
            :size="14"
            class="pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-muted-foreground"
          />
          <input
            ref="field"
            v-model="query"
            type="text"
            autocapitalize="off"
            autocorrect="off"
            spellcheck="false"
            class="h-8 w-full rounded-md border border-border bg-background pr-3 pl-8 text-xs select-text focus:ring-1 focus:ring-ring focus:outline-none"
            :placeholder="t(splitting ? 'terminal.split.search' : 'sessions.newTab.search')"
            :aria-label="t(splitting ? 'terminal.split.search' : 'sessions.newTab.search')"
            @keydown="onKeydown"
          />
        </div>

        <div ref="list" class="mt-2 min-h-0 flex-1 overflow-y-auto px-2 pb-2">
          <template v-if="splitting && source === 'existing'">
            <p v-if="existing.length === 0" class="px-2 py-6 text-center text-xs text-muted-foreground">
              {{ t("terminal.split.emptyExisting") }}
            </p>
            <button
              v-for="(tab, index) in existing"
              :key="tab.id"
              type="button"
              data-split-existing-row
              :data-pane-id="tab.id"
              :data-chosen="tab.id === chosenID ? 'true' : undefined"
              class="flex w-full items-center gap-2 rounded-md px-2 py-2 text-left hover:bg-accent/60"
              :class="tab.id === chosenID ? 'bg-accent text-foreground' : 'text-muted-foreground'"
              @mouseenter="chosen = index"
              @click="chooseExisting(tab)"
            >
              <Terminal :size="14" class="shrink-0" />
              <span class="min-w-0 flex-1"
                ><span class="block truncate text-xs font-medium text-foreground">{{
                  sessionTabTitle(tab.session.name, terminals.copyOf(tab.id))
                }}</span
                ><span class="block truncate text-[11px] text-muted-foreground">{{ endpoint(tab.session) }}</span></span
              >
              <span class="shrink-0 text-[11px] text-muted-foreground">{{
                t(
                  tab.status === "open"
                    ? "terminal.split.running"
                    : tab.status === "ended"
                      ? "terminal.split.ended"
                      : "terminal.split.failed",
                )
              }}</span>
            </button>
          </template>
          <template v-else>
            <p v-if="displayRows.length === 0" class="px-2 py-6 text-center text-xs text-muted-foreground">
              {{ library.sessions.length === 0 ? t("sessions.list.empty") : t("sessions.newTab.empty") }}
            </p>
            <template v-for="row in displayRows" :key="row.key">
              <div
                v-if="row.kind === 'group' && searching"
                class="flex items-center gap-1 px-2 pt-2 pb-1 text-[10px] font-semibold tracking-wide text-muted-foreground"
                :style="{ paddingLeft: `${10 + row.depth * 12}px` }"
              >
                <Folder :size="12" class="shrink-0" />
                <span class="min-w-0 truncate">{{ row.group.name }}</span>
              </div>
              <button
                v-else-if="row.kind === 'group'"
                type="button"
                :data-group-id="row.id"
                :aria-expanded="!isCollapsed(row.id)"
                :title="
                  t(isCollapsed(row.id) ? 'sessions.list.groupExpand' : 'sessions.list.groupCollapse', {
                    name: row.group.name,
                  })
                "
                class="flex w-full items-center gap-1 rounded-md py-1 pr-2 text-left text-xs font-medium text-foreground transition-colors hover:bg-accent/40"
                :style="{ paddingLeft: `${8 + row.depth * 12}px` }"
                @mousedown.prevent
                @click="toggleGroup(row.id)"
              >
                <ChevronRight
                  :size="12"
                  class="shrink-0 text-muted-foreground"
                  :class="isCollapsed(row.id) ? '' : 'rotate-90'"
                />
                <FolderOpen v-if="!isCollapsed(row.id)" :size="13" class="shrink-0 text-primary" />
                <Folder v-else :size="13" class="shrink-0 text-primary" />
                <span class="min-w-0 truncate">{{ row.group.name }}</span>
              </button>
              <button
                v-else
                type="button"
                :data-new-tab-row="splitting ? undefined : ''"
                :data-split-new-row="splitting ? '' : undefined"
                :data-session-id="row.session.id"
                :data-chosen="row.session.id === chosenID ? 'true' : undefined"
                :aria-current="row.session.id === chosenID ? 'true' : undefined"
                class="flex w-full items-center gap-2 rounded-md py-1.5 pr-2 text-left transition-colors"
                :class="
                  row.session.id === chosenID
                    ? 'bg-accent text-foreground'
                    : 'text-muted-foreground hover:bg-accent/60 hover:text-foreground'
                "
                :style="{ paddingLeft: `${8 + row.depth * 12}px` }"
                @mouseenter="chosen = sessionIndex(row.session.id)"
                @click="choose(row.session)"
              >
                <SystemIcon
                  v-if="connectionMark(row.session, app.info?.os)"
                  :system="null"
                  :mark="connectionMark(row.session, app.info?.os)"
                />
                <SystemIcon
                  v-else-if="hasMark(row.session)"
                  :system="terminals.systemOf(row.session)"
                  :label="markTitle(row.session)"
                />
                <MonitorCog
                  v-else-if="row.session.protocol === 'vmware'"
                  :size="13"
                  class="shrink-0 text-muted-foreground"
                />
                <Terminal v-else :size="13" class="shrink-0 text-muted-foreground" />
                <span class="min-w-0 flex-1">
                  <span class="block truncate text-xs font-medium text-foreground">{{ row.session.name }}</span>
                  <span class="block truncate text-[10px] text-muted-foreground">{{ endpoint(row.session) }}</span>
                </span>
              </button>
            </template>
          </template>
        </div>

        <footer class="flex items-center justify-end border-t border-border px-4 py-2">
          <p v-if="splitting" class="text-xs text-muted-foreground">
            {{ t(source === "existing" ? "terminal.split.moveHint" : "terminal.split.newHint") }}
          </p>
          <button
            v-else
            type="button"
            data-new-tab-new-session
            class="flex items-center gap-1 rounded-md border border-border px-2 py-1 text-xs transition-colors hover:bg-accent"
            @click="createSession()"
          >
            <Plus :size="12" />
            {{ t("sessions.newTab.newSession") }}
          </button>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
