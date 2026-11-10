<script setup lang="ts">
/**
 * The containers on one engine, and what can be done to one.
 *
 * A table rather than cards, because the question this view answers is "which of these is not
 * running" — asked across rows, by comparing one column. Everything destructive is asked about
 * first: stop removes processes, remove removes the container, and restarting ends whatever was in
 * flight. Those three answers are also what tells the backend it may act (`acknowledged`), so the
 * dialog is not decoration — it is the gate.
 *
 * The row's low-frequency actions live under one menu, built from the shared context menu, so the
 * table keeps its width for the columns people read. What the menu offers depends on the runtime:
 * see the `supports*` predicates in the store.
 */
import { Camera, Ellipsis, Pencil, Play, RotateCw, ScrollText, Square, SquareTerminal, Trash2, X } from "@lucide/vue";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import Select from "@/components/ui/Select.vue";
import { useContextMenu } from "@/composables/useContextMenu";
import { describeShellError } from "@/i18n/errors";
import { asShellError } from "@/lib/backend/errors";
import type { MenuEntry } from "@/lib/menu/types";
import { TABLE, rowClass } from "@/lib/table";
import { cn } from "@/lib/utils";
import {
  CONTAINER_ACTIONS,
  type ContainerSession,
  filterContainers,
  shortId,
  supportsCommit,
  supportsRename,
  supportsRestart,
  useContainerStore,
} from "@/stores/container";
import { useTerminalsStore, type ContainerTab } from "@/stores/terminals";
import type { ContainerSummary } from "@/types/container";

import ContainerCommitDialog from "./ContainerCommitDialog.vue";
import ContainerCreateDialog from "./ContainerCreateDialog.vue";
import ContainerDetailDrawer from "./ContainerDetailDrawer.vue";
import ContainerDialog from "./ContainerDialog.vue";

const props = defineProps<{
  tab: ContainerTab;
  session: ContainerSession;
  imageFilter: string | null;
  stateFilter: string | null;
}>();

const emit = defineEmits<{
  "clear-image-filter": [];
  "clear-state-filter": [];
  "filter-state": [state: string | null];
}>();

const { t } = useI18n();
const containers = useContainerStore();
const terminals = useTerminalsStore();
const { openMenu } = useContextMenu();

const text = ref("");
/**
 * Running only, by default: an engine that has been up for a while accumulates exited containers
 * nobody is going to look at, and they are what buries the ones that are alive.
 */
const runningOnly = ref(true);

const rows = computed(() =>
  filterContainers(props.session.containers, {
    text: text.value,
    runningOnly: runningOnly.value,
    state: props.stateFilter,
    image: props.imageFilter,
  }),
);

const stateOptions = computed(() => {
  const states = new Set(props.session.containers.map((container) => container.state));
  return [
    { value: "", label: t("container.filters.allStates") },
    ...[...states].sort().map((state) => ({ value: state, label: state })),
  ];
});

function chooseState(state: string): void {
  emit("filter-state", state === "" ? null : state);
}

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

/** How a state is drawn: a dot with the colour of the thing it says. The word stays the runtime's. */
function stateDotClass(state: string): string {
  switch (state) {
    case "running":
      return "bg-success";
    case "paused":
      return "bg-warning";
    case "dead":
      return "bg-destructive";
    default:
      return "bg-muted-foreground/50";
  }
}

// -- drawers and dialogs -----------------------------------------------------

const drawerMode = ref<"detail" | "logs" | null>(null);
const drawerTarget = ref<ContainerSummary | null>(null);
const creating = ref(false);
const committing = ref<ContainerSummary | null>(null);
const renaming = ref<ContainerSummary | null>(null);
const renameText = ref("");
const renameProblem = ref<string | null>(null);
const renameBusy = ref(false);
const confirming = ref<{ kind: "stop" | "restart" | "remove"; container: ContainerSummary } | null>(null);

function openDetail(container: ContainerSummary): void {
  drawerTarget.value = container;
  drawerMode.value = "detail";
}

function openLogs(container: ContainerSummary): void {
  drawerTarget.value = container;
  drawerMode.value = "logs";
}

function closeDrawer(): void {
  drawerMode.value = null;
  drawerTarget.value = null;
}

function closeCommit(): void {
  committing.value = null;
}

function closeRename(): void {
  renaming.value = null;
}

// -- exec --------------------------------------------------------------------

async function openShell(container: ContainerSummary): Promise<void> {
  try {
    const handle = await containers.exec(props.tab.id, container.id);
    if (handle === null) {
      return;
    }
    // An ordinary terminal from here on: resize, write and close all go through its handle, so the
    // term pane needs no second code path for a shell that runs inside a container.
    terminals.openHandle({ ...props.tab.session, name: container.name }, handle);
  } catch (error) {
    containers.reportError(props.tab.id, error, "container_exec_open");
  }
}

// -- actions ------------------------------------------------------------------

/** Whether the row can be acted on at all; a fresh list is the only way to find out about the rest. */
function runAction(container: ContainerSummary, name: "start" | "stop" | "restart" | "remove"): void {
  if (name === "start") {
    void perform(CONTAINER_ACTIONS.start, container, false);
    return;
  }
  if (name === "stop") {
    confirming.value = { kind: "stop", container };
    return;
  }
  if (name === "restart") {
    confirming.value = { kind: "restart", container };
    return;
  }
  confirming.value = { kind: "remove", container };
}

/**
 * Carries out what the confirmation was about.
 *
 * Stopping and removing are the two the policy matrix calls destructive, so they travel with
 * `acknowledged` — the person answered the question. Restart is merely a change of state and does
 * not need the flag; it is still asked about, because it ends running processes.
 */
async function perform(
  name: (typeof CONTAINER_ACTIONS)[keyof typeof CONTAINER_ACTIONS],
  container: ContainerSummary,
  acknowledged: boolean,
): Promise<void> {
  try {
    await containers.action(props.tab.id, container.id, name, acknowledged);
  } catch {
    // The store kept the envelope and the pane's banner is showing it; nothing more to say here.
  }
}

const confirmTitle = computed(() => {
  const pending = confirming.value;
  if (!pending) {
    return "";
  }
  switch (pending.kind) {
    case "stop":
      return t("container.confirm.stop", { name: pending.container.name });
    case "restart":
      return t("container.confirm.restart", { name: pending.container.name });
    case "remove":
      return t("container.confirm.remove", { name: pending.container.name });
  }
});

const confirmMessage = computed(() => {
  const pending = confirming.value;
  if (!pending) {
    return "";
  }
  switch (pending.kind) {
    case "stop":
      return t("container.confirm.stopMessage");
    case "restart":
      return t("container.confirm.restartMessage");
    case "remove":
      return t("container.confirm.removeMessage");
  }
});

const confirmLabel = computed(() => {
  const pending = confirming.value;
  if (!pending) {
    return "";
  }
  switch (pending.kind) {
    case "stop":
      return t("container.action.stopVerb");
    case "restart":
      return t("container.action.restart");
    case "remove":
      return t("container.action.removeVerb");
  }
});

async function confirmAction(): Promise<void> {
  const pending = confirming.value;
  if (!pending) {
    return;
  }
  confirming.value = null;
  if (pending.kind === "stop") {
    await perform(CONTAINER_ACTIONS.stop, pending.container, true);
    return;
  }
  if (pending.kind === "restart") {
    await perform(CONTAINER_ACTIONS.restart, pending.container, false);
    return;
  }
  await perform(CONTAINER_ACTIONS.remove, pending.container, true);
}

// -- rename -------------------------------------------------------------------

function startRename(container: ContainerSummary): void {
  renaming.value = container;
  renameText.value = container.name;
  renameProblem.value = null;
}

async function submitRename(): Promise<void> {
  const target = renaming.value;
  const name = renameText.value.trim();
  if (!target || renameBusy.value) {
    return;
  }
  if (name === "") {
    renameProblem.value = t("container.rename.required");
    return;
  }
  if (name === target.name) {
    renaming.value = null;
    return;
  }
  renameBusy.value = true;
  try {
    await containers.rename(props.tab.id, target.id, name);
    renaming.value = null;
  } catch (error) {
    renameProblem.value = describeShellError(translate, asShellError(error, "container_rename")).message;
  } finally {
    renameBusy.value = false;
  }
}

// -- the row's menu -------------------------------------------------------------

function openRowMenu(event: MouseEvent, container: ContainerSummary): void {
  const runtime = props.session.runtime;
  const entries: MenuEntry[] = [];

  if (supportsRestart(runtime)) {
    entries.push({
      kind: "action",
      id: "restart",
      labelKey: "container.action.restart",
      icon: RotateCw,
      run: () => runAction(container, "restart"),
    });
  }
  if (supportsRename(runtime)) {
    entries.push({
      kind: "action",
      id: "rename",
      labelKey: "container.action.rename",
      icon: Pencil,
      run: () => startRename(container),
    });
  }
  if (supportsCommit(runtime)) {
    entries.push({
      kind: "action",
      id: "commit",
      labelKey: "container.action.commit",
      icon: Camera,
      run: () => {
        committing.value = container;
      },
    });
  }
  entries.push({
    kind: "action",
    id: "remove",
    labelKey: "container.action.remove",
    icon: Trash2,
    danger: true,
    run: () => runAction(container, "remove"),
  });

  openMenu(event, { key: "container.action.menu", params: { name: container.name } }, entries);
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <div class="flex shrink-0 flex-wrap items-center gap-1.5 border-b border-border px-2.5 py-1.5">
      <input
        v-model="text"
        type="text"
        spellcheck="false"
        class="h-6 w-52 rounded-md border border-border bg-background px-2 text-[11px] focus:outline-none"
        :placeholder="t('container.filters.text')"
        :aria-label="t('container.filters.text')"
      />

      <label class="flex items-center gap-1.5 text-[11px] text-muted-foreground">
        <input v-model="runningOnly" type="checkbox" class="size-3 accent-[var(--primary)]" />
        {{ t("container.filters.runningOnly") }}
      </label>

      <Select
        :model-value="props.stateFilter ?? ''"
        :options="stateOptions"
        size="compact"
        class="w-28"
        :aria-label="t('container.filters.state')"
        @update:model-value="chooseState"
      />

      <span
        v-if="props.imageFilter"
        class="flex max-w-56 items-center gap-1 rounded-sm border border-border px-1.5 py-0.5 text-[10px] text-muted-foreground"
      >
        <span class="truncate">{{ t("container.filters.image", { name: props.imageFilter }) }}</span>
        <button
          type="button"
          class="grid size-4 shrink-0 place-items-center rounded-sm hover:bg-accent hover:text-foreground"
          :title="t('container.filters.clearImage')"
          :aria-label="t('container.filters.clearImage')"
          @click="emit('clear-image-filter')"
        >
          <X :size="10" />
        </button>
      </span>

      <span
        v-if="props.stateFilter"
        class="flex items-center gap-1 rounded-sm border border-border px-1.5 py-0.5 text-[10px] text-muted-foreground"
      >
        <span class="truncate">{{ t("container.filters.stateValue", { name: props.stateFilter }) }}</span>
        <button
          type="button"
          class="grid size-4 shrink-0 place-items-center rounded-sm hover:bg-accent hover:text-foreground"
          :title="t('container.filters.clearState')"
          :aria-label="t('container.filters.clearState')"
          @click="emit('clear-state-filter')"
        >
          <X :size="10" />
        </button>
      </span>

      <div class="min-w-0 flex-1"></div>

      <button
        type="button"
        class="flex items-center gap-1 rounded-md border border-border px-2 py-1 text-[11px] transition-colors hover:bg-accent"
        @click="creating = true"
      >
        {{ t("container.create.title") }}
      </button>
      <button
        type="button"
        class="grid size-6 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        :title="t('container.refresh')"
        :aria-label="t('container.refresh')"
        @click="containers.refresh(props.tab.id)"
      >
        <RotateCw :size="12" />
      </button>
    </div>

    <div class="min-h-0 flex-1 overflow-auto">
      <table
        :class="cn(TABLE.page.table, 'table-fixed')"
        :style="{ minWidth: '760px' }"
        :aria-label="t('container.rail.containers')"
      >
        <thead :class="TABLE.page.head">
          <tr :class="TABLE.page.headRow">
            <th class="w-[20%]" :class="TABLE.page.headCell">{{ t("container.col.name") }}</th>
            <th class="w-[26%]" :class="TABLE.page.headCell">{{ t("container.col.image") }}</th>
            <th class="w-[9%]" :class="TABLE.page.headCell">{{ t("container.col.id") }}</th>
            <th class="w-[10%]" :class="TABLE.page.headCell">{{ t("container.col.state") }}</th>
            <th class="w-[15%]" :class="TABLE.page.headCell">{{ t("container.col.ports") }}</th>
            <th class="w-[12%]" :class="TABLE.page.headCell">{{ t("container.col.created") }}</th>
            <th class="w-[8rem]" :class="TABLE.page.headCell">{{ t("container.col.actions") }}</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="container in rows"
            :key="container.id"
            :class="cn(rowClass('page', { clickable: true }))"
            @click="openDetail(container)"
          >
            <td :class="TABLE.page.cell">
              <span class="block truncate" :title="container.name">{{ container.name }}</span>
            </td>
            <td :class="TABLE.page.cell">
              <span class="block truncate font-mono text-[11px]" :title="container.image">{{ container.image }}</span>
            </td>
            <td :class="TABLE.page.cell">
              <span class="block truncate font-mono text-[11px]" :title="container.id">{{
                shortId(container.id)
              }}</span>
            </td>
            <td :class="TABLE.page.cell">
              <span class="flex items-center gap-1.5">
                <span class="size-1.5 shrink-0 rounded-full" :class="stateDotClass(container.state)"></span>
                <span class="truncate">{{ container.state }}</span>
              </span>
            </td>
            <td :class="TABLE.page.cell">
              <span class="block truncate font-mono text-[11px]" :title="container.ports">{{ container.ports }}</span>
            </td>
            <td :class="TABLE.page.cell">
              <span class="block truncate text-[11px] text-muted-foreground" :title="container.createdAt">
                {{ container.createdAt }}
              </span>
            </td>
            <td :class="TABLE.page.cell" @click.stop>
              <span class="flex items-center gap-0.5">
                <button
                  type="button"
                  class="grid size-5 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:cursor-not-allowed disabled:opacity-30"
                  :title="t('container.action.exec', { name: container.name })"
                  :aria-label="t('container.action.exec', { name: container.name })"
                  :disabled="container.state !== 'running'"
                  @click="openShell(container)"
                >
                  <SquareTerminal :size="12" />
                </button>
                <button
                  type="button"
                  class="grid size-5 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                  :title="t('container.action.logs', { name: container.name })"
                  :aria-label="t('container.action.logs', { name: container.name })"
                  @click="openLogs(container)"
                >
                  <ScrollText :size="12" />
                </button>
                <button
                  v-if="container.state !== 'running'"
                  type="button"
                  class="grid size-5 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                  :title="t('container.action.start', { name: container.name })"
                  :aria-label="t('container.action.start', { name: container.name })"
                  @click="runAction(container, 'start')"
                >
                  <Play :size="12" />
                </button>
                <button
                  v-else
                  type="button"
                  class="grid size-5 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                  :title="t('container.action.stop', { name: container.name })"
                  :aria-label="t('container.action.stop', { name: container.name })"
                  @click="runAction(container, 'stop')"
                >
                  <Square :size="11" />
                </button>
                <button
                  type="button"
                  class="grid size-5 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                  :title="t('container.action.more', { name: container.name })"
                  :aria-label="t('container.action.more', { name: container.name })"
                  @click="openRowMenu($event, container)"
                >
                  <Ellipsis :size="12" />
                </button>
              </span>
            </td>
          </tr>
          <tr v-if="rows.length === 0">
            <td :colspan="7" :class="cn(TABLE.page.cell, TABLE.page.emptyCell)">
              {{ t("container.filters.none") }}
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <ContainerDetailDrawer :mode="drawerMode" :tab-id="props.tab.id" :target="drawerTarget" @close="closeDrawer()" />

    <ContainerCreateDialog
      :open="creating"
      :tab-id="props.tab.id"
      :runtime="props.session.runtime"
      @update:open="creating = $event"
    />

    <ContainerCommitDialog
      :open="committing !== null"
      :tab-id="props.tab.id"
      :target="committing"
      @update:open="closeCommit"
    />

    <ContainerDialog
      :open="renaming !== null"
      :title="t('container.rename.title')"
      :description="t('container.rename.message', { name: renaming?.name ?? '' })"
      :busy="renameBusy"
      @update:open="closeRename"
    >
      <label class="flex flex-col gap-1">
        <span class="text-[11px] text-muted-foreground">{{ t("container.rename.label") }}</span>
        <input
          v-model="renameText"
          type="text"
          spellcheck="false"
          class="rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:outline-none"
          @keydown.enter.prevent="submitRename()"
        />
      </label>
      <p v-if="renameProblem" class="mt-2 text-[11px] text-destructive">{{ renameProblem }}</p>
      <template #footer>
        <button
          type="button"
          class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent"
          @click="renaming = null"
        >
          {{ t("common.action.cancel") }}
        </button>
        <button
          type="button"
          class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
          :disabled="renameBusy"
          @click="submitRename()"
        >
          {{ t("container.rename.submit") }}
        </button>
      </template>
    </ContainerDialog>

    <ConfirmDialog
      :open="confirming !== null"
      :title="confirmTitle"
      :message="confirmMessage"
      :confirm-label="confirmLabel"
      danger
      @confirm="confirmAction()"
      @update:open="confirming = null"
    />
  </div>
</template>
