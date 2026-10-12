<script setup lang="ts">
/**
 * The transfers, wherever the person happens to be.
 *
 * It belongs to the shell rather than to a file panel because a transfer does: it holds the channel
 * it is moving bytes over for as long as it runs, so closing the tab that started it takes away the
 * browsing and nothing else. A transfer still moving with nowhere to be seen — and nowhere to be
 * stopped — is the one thing this panel exists to prevent.
 *
 * Two shapes, one surface. Open, it is the list: what is moving first, then what failed, then what
 * has stopped. Tucked away, it is a pill that says how much is going on, and the pill stays there
 * while there are rows — so "put away" never means lost, and a click anywhere else tucks it away
 * without anything being cancelled or forgotten.
 *
 * It refuses to be read as a document, like the listing and the monitor beside it: a drag in a panel
 * of rows means acting on a row, not taking a smear of its text with it — and what is worth taking
 * away is on the row's own menu. The one exception is an error, which is the thing somebody quotes
 * into a bug report: the failure line under a row, and the banner over the list, are selectable.
 *
 * The list is polled by the store rather than pushed: a transfer is started by a command and outlives
 * it, so there is no call left for the backend to answer into — and half a second of counters is
 * cheaper than a second event channel that would have to be kept in step with what is drawn here.
 */
import {
  ArrowDownToLine,
  ArrowUpFromLine,
  ChevronDown,
  ChevronUp,
  CircleAlert,
  CircleCheck,
  CirclePause,
  CircleStop,
  Eraser,
  FolderOpen,
  ListFilter,
  LoaderCircle,
  Pause,
  Play,
  RotateCcw,
  X,
} from "@lucide/vue";
import type { Component } from "vue";
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import { useContextMenu } from "@/composables/useContextMenu";
import { describeShellError } from "@/i18n/errors";
import { formatSize } from "@/lib/files/format";
import { downloadFolderMenu } from "@/lib/menu/downloads";
import { MENU_TITLES } from "@/lib/menu/titles";
import { transferMenu } from "@/lib/menu/transfers";
import { cn } from "@/lib/utils";
import { useFilesStore } from "@/stores/files";
import { useSessionsStore } from "@/stores/sessions";
import { sessionOf, useTerminalsStore } from "@/stores/terminals";
import type { TransferInfo, TransferState } from "@/types/session";

const { t } = useI18n();
const files = useFilesStore();
const terminals = useTerminalsStore();
const library = useSessionsStore();
const { openMenu } = useContextMenu();

/** One row of the panel. */
interface PanelRow {
  readonly key: string;
  readonly transfer: TransferInfo;
}

/** The session on screen, which is what the filter narrows to when it is on. */
const activeSessionId = computed(() => {
  const tab = terminals.activeTab;
  return tab ? (sessionOf(tab)?.id ?? null) : null;
});

/** Whether the list is narrowed to the session on screen. */
const onlyCurrent = ref(false);
const filtering = computed(() => onlyCurrent.value && activeSessionId.value !== null);

const shown = computed(() =>
  filtering.value ? files.transfers.filter((entry) => entry.sessionId === activeSessionId.value) : files.transfers,
);

const moving = computed(() => files.transfers.filter((entry) => isMoving(entry.state)));
const running = computed(() => files.transfers.filter((entry) => entry.state.kind === "running").length);
const settled = computed(() =>
  files.transfers.filter((entry) => !isMoving(entry.state) && entry.state.kind !== "failed"),
);
const failed = computed(() => files.transfers.some((entry) => entry.state.kind === "failed"));

/**
 * How far along everything that is running is, for the pill.
 *
 * Nothing at all when a size is still unknown — a packed transfer counts itself once the archive on
 * the far side exists — because a percentage over a total that is not final is a number that would
 * go backwards.
 */
const overall = computed(() => {
  let size = 0;
  let done = 0;
  for (const entry of files.transfers) {
    if (entry.state.kind !== "running") {
      continue;
    }
    if (entry.size === 0) {
      return null;
    }
    size += entry.size;
    done += Math.min(entry.transferred, entry.size);
  }
  return size > 0 ? Math.round((done / size) * 100) : null;
});

/**
 * What the list draws, in the order it is worth reading: what is moving, then what went wrong, then
 * what has stopped.
 *
 * One flat list, with no line folding a group of them away. There was one, for the batch that makes a
 * row per file — and it cost a line of height in every case, including the one row it could never
 * hide anything in, which is what it looked like: a card with a single finished download and a header
 * over it saying so. The order is what keeps a wall of finished rows from burying the two that are
 * moving, and the list scrolls within a height that does not grow with the transfers.
 */
const rows = computed<PanelRow[]>(() => {
  const drawn = shown.value;
  const moving = drawn.filter((entry) => isMoving(entry.state) || entry.state.kind === "failed");
  const rested = drawn.filter((entry) => !isMoving(entry.state) && entry.state.kind !== "failed");

  return [...moving, ...rested].map((transfer) => ({ key: transfer.id, transfer }));
});

/**
 * Whether the list holds transfers from more than one session.
 *
 * The session each row belongs to is written on the row only when there is more than one to tell
 * apart: on the usual list they are all from the session somebody is working in, and a label saying
 * so on every row is a column of noise.
 */
const severalSessions = computed(() => new Set(shown.value.map((entry) => entry.sessionId)).size > 1);

const problem = computed(() => (files.transferFailure ? describeShellError(translate, files.transferFailure) : null));

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

function isMoving(state: TransferState): boolean {
  return state.kind === "running" || state.kind === "paused";
}

function stateLabel(state: TransferState): string {
  switch (state.kind) {
    case "running":
      return t("files.transfer.state.running");
    case "paused":
      return t("files.transfer.state.paused");
    case "done":
      return t("files.transfer.state.done");
    case "failed":
      return t("files.transfer.state.failed");
    case "cancelled":
      return t("files.transfer.state.cancelled");
  }
}

/**
 * Which session a row is moving bytes for.
 *
 * The open tabs first, because that is where the name somebody chose is in use right now, and then
 * the library, because a transfer outlives the tab that started it: a row labelled with a session id
 * nobody can decode would be a row nobody can place.
 */
function sessionLabel(sessionId: string): string {
  const tab = terminals.tabs.find((entry) => sessionOf(entry)?.id === sessionId);
  const open = tab ? sessionOf(tab)?.name : undefined;
  return open ?? library.sessions.find((entry) => entry.id === sessionId)?.name ?? sessionId;
}

/**
 * What the one button on a row does, which is whatever that row needs next.
 *
 * A running transfer can be held; a paused one can be let go; one that stopped can be tried again;
 * one that has finished is taken off the list. Working it out in one place is what keeps a row from
 * offering "Hold" on a transfer that finished a minute ago — and the entries that open the file, or
 * show where it is, live in the row's menu, where a mis-click costs nothing.
 *
 * The button is an icon and its word is a tooltip: the word was read once and then re-read on every
 * row, and what it cost was the width the file's own name wanted. The glyphs are a family of their
 * own, apart from the states drawn on the left — those are circles, these are not — so "the thing on
 * the right of the row" is never mistaken for "what the row is doing".
 */
interface RowAction {
  readonly label: string;
  readonly icon: Component;
  readonly run: () => void;
}

function rowAction(info: TransferInfo): RowAction {
  switch (info.state.kind) {
    case "running":
      return {
        label: t("files.transfer.pause"),
        icon: Pause,
        run: () => void files.setTransferPaused(info.id, true),
      };
    case "paused":
      return {
        label: t("files.transfer.resume"),
        icon: Play,
        run: () => void files.setTransferPaused(info.id, false),
      };
    case "failed":
      return {
        label: t("files.transfer.retry"),
        icon: RotateCcw,
        run: () => void files.retryTransfer(info.id),
      };
    default:
      return {
        label: t("files.transfer.remove"),
        icon: X,
        run: () => void files.removeTransfer(info.id),
      };
  }
}

function directionLabel(info: TransferInfo): string {
  return info.direction === "download" ? t("files.transfer.direction.download") : t("files.transfer.direction.upload");
}

/** How full the bar is. An unknown size is drawn as an indeterminate bar instead. */
function percent(info: TransferInfo): number {
  if (info.state.kind === "done") {
    return 100;
  }
  if (info.size === 0) {
    return 0;
  }
  return Math.min(100, Math.round((info.transferred / info.size) * 100));
}

function fillClass(state: TransferState): string {
  switch (state.kind) {
    case "done":
      return "bg-success";
    case "failed":
      return "bg-destructive";
    case "cancelled":
      return "bg-muted-foreground/40";
    default:
      return "bg-primary";
  }
}

function sizeText(info: TransferInfo): string {
  return info.size === 0 ? t("files.transfer.unknownSize") : formatSize(info.size);
}

function indeterminate(info: TransferInfo): boolean {
  return info.state.kind === "running" && info.size === 0;
}

/** The row's menu, which offers everything the row's own button does and the file besides. */
function openTransferMenu(event: MouseEvent, info: TransferInfo): void {
  openMenu(
    event,
    { key: MENU_TITLES.transfer },
    transferMenu(info, {
      stop: (id) => void files.stopTransfer(id),
      remove: (id) => void files.removeTransfer(id),
      open: (id) => void files.openTransfer(id),
      reveal: (id) => void files.revealTransfer(id),
      setPaused: (id, paused) => void files.setTransferPaused(id, paused),
      retry: (id) => void files.retryTransfer(id),
    }),
  );
}

/**
 * The folder downloads land in, by its own name.
 *
 * `null` is "ask every time", which is a state rather than a missing value: it is what an
 * installation nobody has configured is in, and what somebody who wants the save dialog back
 * chooses.
 */
const downloadFolder = computed(() => files.downloadFolder?.current ?? null);

/**
 * The list behind that name: the folders remembered, and the two ways out.
 *
 * A name rather than a path, because that is all this side is told — the folder was chosen in a
 * native picker, and asking for the picker again is how a new one is named. Nothing here can point
 * the bytes at a place the person did not choose.
 */
function openDownloadFolderMenu(event: MouseEvent): void {
  openMenu(
    event,
    { key: MENU_TITLES.downloadFolder },
    downloadFolderMenu(files.downloadFolder ?? { current: null, history: [] }, {
      use: (index) => void files.useDownloadFolder(index),
      choose: () => void files.chooseDownloadFolder(t("files.transfer.folder.chooseTitle")),
      ask: () => void files.askForDownloadFolder(),
    }),
  );
}

/** Whether the question about stopping everything is on screen. */
const confirmingStopAll = ref(false);

/** Answers it: everything that is still going is asked to stop, and every row says so afterwards. */
function stopEverything(): void {
  confirmingStopAll.value = false;
  void files.stopAllTransfers();
}

/** A failure is worth the room even when the panel was put away: it is the only place it is drawn. */
const expanded = computed(() => files.panelExpanded || problem.value !== null);

const card = ref<HTMLElement | null>(null);

/**
 * A click that landed somewhere else puts the panel back to its pill.
 *
 * Tucking away and not closing: the rows are the store's, nothing is cancelled, and the pill is one
 * click from being the list again. A click inside a menu the panel opened is not somewhere else —
 * every entry in it is about one of these rows.
 */
function onPointerDown(event: PointerEvent): void {
  const target = event.target;
  if (!(target instanceof Node) || card.value?.contains(target) === true) {
    return;
  }
  if (target instanceof Element && target.closest("[data-menu-layer]") !== null) {
    return;
  }
  files.collapsePanel();
}

/**
 * Escape tucks it away, unless something the panel opened is on screen.
 *
 * A question and a menu both answer Escape themselves — the first one cancels, the second closes —
 * and a key press that also put the list away underneath would be two things happening from one
 * press. The menu's own layer is what says one is open, which is the same marker the pointerdown
 * above reads.
 *
 * Caught on the way *down*, and that is the whole trick: a menu takes itself off screen in its own
 * handler, so by the time a listener at the end of the bubble runs, the marker this checks for is
 * already gone and the panel closes underneath the key press it was meant to ignore. The same holds
 * for a question. Both are still on screen while the event is travelling down.
 */
function onKeyDown(event: KeyboardEvent): void {
  if (event.key !== "Escape") {
    return;
  }
  if (document.querySelector('[role="dialog"], [data-menu-layer]') !== null) {
    return;
  }
  files.collapsePanel();
}

watch(
  expanded,
  (open) => {
    if (open) {
      document.addEventListener("pointerdown", onPointerDown, true);
      window.addEventListener("keydown", onKeyDown, true);
    } else {
      document.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("keydown", onKeyDown, true);
    }
  },
  // Immediate, because a panel can be open before it is ever drawn: a transfer that was already
  // running when the shell mounted — a page reload with the core still moving bytes — expands the
  // panel in the store, and a listener armed only on a change would never be attached at all.
  { immediate: true },
);

onBeforeUnmount(() => {
  document.removeEventListener("pointerdown", onPointerDown, true);
  window.removeEventListener("keydown", onKeyDown, true);
});

// Read once when the panel arrives, which is when the vault opens: where downloads land is a fact
// about the installation rather than about a transfer, and one read answers it for every row.
onMounted(() => void files.loadDownloadFolder());
</script>

<template>
  <div
    v-if="files.transfers.length > 0 || problem || files.panelExpanded"
    class="pointer-events-none absolute right-3 bottom-3 z-30 flex max-w-[calc(100%-1.5rem)] flex-col items-end"
  >
    <button
      v-if="!expanded"
      type="button"
      data-transfer-pill
      class="pointer-events-auto flex items-center gap-1.5 rounded-full border border-border bg-card px-3 py-1.5 text-[11px] shadow-lg transition-colors hover:bg-accent"
      :title="t('files.transfer.expand')"
      :aria-label="t('files.transfer.expand')"
      @click="files.togglePanel()"
    >
      <LoaderCircle v-if="running > 0" :size="12" class="animate-spin text-primary" />
      <CircleAlert v-else-if="failed" :size="12" class="text-destructive" />
      <CircleCheck v-else :size="12" class="text-success" />
      <span>{{ t("files.transfer.pill", { count: files.transfers.length }) }}</span>
      <span v-if="overall !== null" class="font-mono text-muted-foreground">{{ overall }}%</span>
      <span v-if="failed" class="size-1.5 rounded-full bg-destructive"></span>
      <ChevronUp :size="12" class="text-muted-foreground" />
    </button>

    <section
      v-else
      ref="card"
      data-transfer-panel
      class="pointer-events-auto flex max-h-[70vh] w-80 cursor-default flex-col overflow-hidden rounded-lg border border-border bg-card select-none shadow-xl"
      :aria-label="t('files.transfer.title')"
    >
      <header class="flex shrink-0 items-center gap-1 border-b border-border px-2.5 py-1.5">
        <h3 class="min-w-0 flex-1 truncate text-[11px] font-medium tracking-wide uppercase">
          {{ t("files.transfer.title") }}
        </h3>

        <!--
          The three controls in this header are icons with their sentences as tooltips, for the reason
          the row's button is: a word read once is a word on screen for the rest of the session, and
          what it was taking was the room the file names want. Each still says what it does — a
          tooltip for whoever hovers, an aria-label for a reader, and the state of the filter as a
          colour rather than a word.
        -->
        <button
          v-if="activeSessionId !== null"
          type="button"
          data-transfer-filter
          :class="
            cn(
              'grid size-5 shrink-0 place-items-center rounded-sm transition-colors hover:bg-accent hover:text-foreground',
              filtering ? 'text-primary' : 'text-muted-foreground',
            )
          "
          :title="t('files.transfer.onlyCurrent')"
          :aria-label="t('files.transfer.currentSession')"
          :aria-pressed="onlyCurrent"
          @click="onlyCurrent = !onlyCurrent"
        >
          <ListFilter :size="12" />
        </button>

        <button
          v-if="moving.length > 0"
          type="button"
          data-transfer-stop-all
          class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
          :title="t('files.transfer.stopAll')"
          :aria-label="t('files.transfer.stopAll')"
          @click="confirmingStopAll = true"
        >
          <CircleStop :size="12" />
        </button>

        <button
          v-if="settled.length > 0"
          type="button"
          data-transfer-clear
          class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          :title="t('files.transfer.clearSettled', { count: settled.length })"
          :aria-label="t('files.transfer.clearSettled', { count: settled.length })"
          @click="files.clearSettledTransfers()"
        >
          <Eraser :size="12" />
        </button>

        <button
          type="button"
          data-transfer-collapse
          class="grid size-4 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          :title="t('files.transfer.collapse')"
          :aria-label="t('files.transfer.collapse')"
          @click="files.collapsePanel()"
        >
          <ChevronDown :size="12" />
        </button>
      </header>

      <!--
        Where downloads land. It belongs at the top of the list rather than on a row, because it is a
        decision about every download: a row that landed somewhere unexpected is a row somebody has
        to go and find. The name is a name and not a path — the folder was chosen in a native picker
        — and the button beside it opens it, which is how somebody answers "where is that, then".
      -->
      <div data-transfer-download-folder class="flex shrink-0 items-center gap-1 border-b border-border px-2.5 py-1">
        <span class="shrink-0 text-[10px] text-muted-foreground">{{ t("files.transfer.folder.label") }}</span>
        <button
          type="button"
          data-download-folder
          class="flex min-w-0 flex-1 items-center gap-1 rounded-sm px-1 py-0.5 text-left transition-colors hover:bg-accent"
          :aria-label="t('files.transfer.folder.choose')"
          :title="t('files.transfer.folder.choose')"
          @click="openDownloadFolderMenu($event)"
        >
          <span class="min-w-0 truncate text-[11px]" :class="downloadFolder === null ? 'text-muted-foreground' : ''">
            {{ downloadFolder ?? t("files.transfer.folder.ask") }}
          </span>
          <ChevronDown :size="11" class="shrink-0 text-muted-foreground" />
        </button>
        <button
          v-if="downloadFolder !== null"
          type="button"
          data-download-folder-open
          class="grid size-4 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          :title="t('files.transfer.folder.open')"
          :aria-label="t('files.transfer.folder.open')"
          @click="files.revealDownloadFolder()"
        >
          <FolderOpen :size="11" />
        </button>
      </div>

      <p
        v-if="problem"
        data-transfer-problem
        class="flex shrink-0 items-start gap-2 border-b border-border px-2.5 py-1.5 select-text"
      >
        <span class="min-w-0 flex-1">
          <span class="block text-[11px]">{{ problem.message }}</span>
          <span class="mt-0.5 block font-mono text-[10px] text-muted-foreground">{{ problem.code }}</span>
        </span>
        <button
          type="button"
          class="grid size-4 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          :title="t('files.transfer.dismiss')"
          :aria-label="t('files.transfer.dismiss')"
          @click="files.dismissTransferFailure()"
        >
          <X :size="11" />
        </button>
      </p>

      <!--
        Nothing to list, which is a state somebody can now ask for — the panel is reachable from the
        title bar whether or not a transfer exists. What stays is the strip above: "which folder do
        downloads land in" is the question this panel answers before anything is moving, and it is the
        reason opening it early is worth doing at all.
      -->
      <p v-if="rows.length === 0" data-transfer-empty class="px-2.5 py-3 text-[11px] text-muted-foreground">
        {{ t("files.transfer.empty") }}
      </p>

      <!--
        The list scrolls inside a height that does not grow with it.

        A cap rather than a fixed height: the card is meant to be as small as what is in it, and five
        rows is where "small" stops being true — beyond that the list scrolls rather than the panel
        covering whatever is behind it. A panel that grew with every file of a batch would move under
        the pointer as the batch went, which is the one thing a panel somebody is reading must not do.
      -->
      <ul v-else class="max-h-48 min-h-0 flex-1 space-y-1.5 overflow-y-auto p-2">
        <li v-for="entry in rows" :key="entry.key">
          <div
            data-transfer-row
            class="rounded-md border border-border bg-background px-2 py-1.5"
            @contextmenu="openTransferMenu($event, entry.transfer)"
          >
            <div class="flex items-center gap-2">
              <ArrowDownToLine
                v-if="entry.transfer.direction === 'download'"
                :size="12"
                class="shrink-0 text-muted-foreground"
                :title="directionLabel(entry.transfer)"
              />
              <ArrowUpFromLine
                v-else
                :size="12"
                class="shrink-0 text-muted-foreground"
                :title="directionLabel(entry.transfer)"
              />

              <span class="min-w-0 flex-1 truncate text-[11px]" :title="entry.transfer.remotePath">
                {{ entry.transfer.name }}
              </span>

              <!--
                Which session the bytes are for, when there is more than one on the list. The direction
                used to be spelled out here instead, next to the arrow that already says it — one of the
                two was always going to be the one that stopped being read.
              -->
              <span v-if="severalSessions" class="shrink-0 text-[10px] text-muted-foreground" data-transfer-session>
                {{ sessionLabel(entry.transfer.sessionId) }}
              </span>

              <!--
                The state as an icon with its word on hover, rather than a word of its own on a line of
                its own: a third line per row is height the card spends on every finished download in
                order to say "finished" a second time, under a green bar and a tick that already do.
                The one state that has to be readable rather than recognisable keeps its line below.
              -->
              <CircleCheck
                v-if="entry.transfer.state.kind === 'done'"
                :size="12"
                class="shrink-0 text-success"
                :title="stateLabel(entry.transfer.state)"
              />
              <CircleStop
                v-else-if="entry.transfer.state.kind === 'cancelled'"
                :size="12"
                class="shrink-0 text-muted-foreground"
                :title="stateLabel(entry.transfer.state)"
              />
              <CircleAlert
                v-else-if="entry.transfer.state.kind === 'failed'"
                :size="12"
                class="shrink-0 text-destructive"
                :title="stateLabel(entry.transfer.state)"
              />
              <CirclePause
                v-else-if="entry.transfer.state.kind === 'paused'"
                :size="12"
                class="shrink-0 text-muted-foreground"
                :title="stateLabel(entry.transfer.state)"
              />
              <span v-else class="shrink-0 text-[10px] text-muted-foreground">{{
                stateLabel(entry.transfer.state)
              }}</span>

              <button
                type="button"
                class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                :title="rowAction(entry.transfer).label"
                :aria-label="rowAction(entry.transfer).label"
                @click="rowAction(entry.transfer).run()"
              >
                <component :is="rowAction(entry.transfer).icon" :size="12" />
              </button>
            </div>

            <div class="mt-1 flex items-center gap-2">
              <div class="h-1.5 min-w-0 flex-1 overflow-hidden rounded-full bg-muted">
                <div
                  v-if="indeterminate(entry.transfer)"
                  :class="cn('h-full w-1/3 animate-pulse rounded-full', fillClass(entry.transfer.state))"
                ></div>
                <div
                  v-else
                  :class="cn('h-full rounded-full transition-[width] duration-300', fillClass(entry.transfer.state))"
                  :style="{ width: `${percent(entry.transfer)}%` }"
                ></div>
              </div>
              <span class="shrink-0 font-mono text-[10px] text-muted-foreground">
                {{
                  t("files.transfer.progress", {
                    transferred: formatSize(entry.transfer.transferred),
                    size: sizeText(entry.transfer),
                  })
                }}
              </span>
            </div>

            <!--
              What a failure said, and the only thing on the row a person has to read rather than
              recognise. Every other state is an icon, a bar and a button.
            -->
            <p
              v-if="entry.transfer.state.kind === 'failed'"
              data-transfer-detail
              class="mt-1 font-mono text-[10px] break-all text-muted-foreground select-text"
            >
              {{ entry.transfer.state.detail }}
            </p>
          </div>
        </li>
      </ul>
    </section>

    <!--
      The question about stopping everything. A transfer that is stopped part-way leaves a part
      file, so this asks first — and it says what it leaves behind rather than only what it does.
    -->
    <ConfirmDialog
      v-if="confirmingStopAll"
      :open="true"
      :title="t('files.transfer.stopAll')"
      :message="t('files.transfer.stopAllMessage', { count: moving.length })"
      :confirm-label="t('files.transfer.stopAll')"
      danger
      @confirm="stopEverything()"
      @update:open="confirmingStopAll = false"
    />
  </div>
</template>
