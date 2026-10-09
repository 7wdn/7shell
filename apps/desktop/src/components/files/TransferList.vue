<script setup lang="ts">
/**
 * The transfers that are running or have run, as a strip inside the file browser.
 *
 * The list is polled by the store rather than pushed: a transfer is started by a command and
 * outlives it, so there is no call left for the backend to answer into — and half a second of
 * counters is cheaper than a second event channel that would have to be kept in step with the list
 * this strip already draws.
 *
 * A finished transfer is dismissed through the same call that stops a running one: the backend
 * removes the record, and the row goes with it. A second click on a transfer that is already gone
 * is deliberately not an error.
 */
import { ArrowDownToLine, ArrowUpFromLine, CircleAlert, CircleCheck, CirclePause, CircleSlash, X } from "@lucide/vue";
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import { describeShellError } from "@/i18n/errors";
import { useContextMenu } from "@/composables/useContextMenu";
import { formatSize } from "@/lib/files/format";
import { MENU_TITLES } from "@/lib/menu/titles";
import { transferMenu } from "@/lib/menu/transfers";
import { cn } from "@/lib/utils";
import { useFilesStore } from "@/stores/files";
import type { TransferInfo, TransferState } from "@/types/session";

const { t } = useI18n();
const files = useFilesStore();
const { openMenu } = useContextMenu();

const problem = computed(() => (files.transferFailure ? describeShellError(translate, files.transferFailure) : null));

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
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
 * What the one button on a row does, which is whatever that row needs next.
 *
 * A running transfer can be held; a paused one can be let go; one that stopped can be tried again
 * from where it stopped; and one that finished is dismissed — the same call as stopping, because the
 * backend drops the record either way. Working it out in one place is what keeps a row from offering
 * "Cancel" on a transfer that stopped a minute ago.
 */
interface RowAction {
  readonly label: string;
  readonly run: () => void;
}

function rowAction(info: TransferInfo): RowAction {
  switch (info.state.kind) {
    case "running":
      return { label: t("files.transfer.pause"), run: () => void files.setTransferPaused(info.id, true) };
    case "paused":
      return { label: t("files.transfer.resume"), run: () => void files.setTransferPaused(info.id, false) };
    case "failed":
      return { label: t("files.transfer.retry"), run: () => void files.retryTransfer(info.id) };
    default:
      return { label: t("files.transfer.dismiss"), run: () => void files.cancelTransfer(info.id) };
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

/** The row's menu, which offers the same things the row's own button does. */
function openTransferMenu(event: MouseEvent, info: TransferInfo): void {
  openMenu(
    event,
    { key: MENU_TITLES.transfer },
    transferMenu(info, {
      cancel: (id) => void files.cancelTransfer(id),
      setPaused: (id, paused) => void files.setTransferPaused(id, paused),
      retry: (id) => void files.retryTransfer(id),
    }),
  );
}
</script>

<template>
  <section v-if="files.transfers.length > 0 || problem" class="shrink-0 border-t border-border bg-card/30 px-3 py-2">
    <div class="mb-1.5 flex items-center justify-between">
      <h3 class="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
        {{ t("files.transfer.title") }}
      </h3>
      <button
        v-if="problem"
        type="button"
        class="grid size-4 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        :title="t('files.transfer.dismiss')"
        :aria-label="t('files.transfer.dismiss')"
        @click="files.dismissTransferFailure()"
      >
        <X :size="11" />
      </button>
    </div>

    <p v-if="problem" class="mb-1.5 rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5">
      <span class="block text-[11px]">{{ problem.message }}</span>
      <span class="mt-0.5 block font-mono text-[10px] text-muted-foreground">{{ problem.code }}</span>
    </p>

    <ul class="max-h-40 space-y-1.5 overflow-y-auto">
      <li
        v-for="info in files.transfers"
        :key="info.id"
        class="rounded-md border border-border bg-background px-2 py-1.5"
        @contextmenu="openTransferMenu($event, info)"
      >
        <div class="flex items-center gap-2">
          <ArrowDownToLine v-if="info.direction === 'download'" :size="12" class="shrink-0 text-muted-foreground" />
          <ArrowUpFromLine v-else :size="12" class="shrink-0 text-muted-foreground" />

          <span class="min-w-0 flex-1 truncate text-[11px]" :title="info.remotePath">{{ info.name }}</span>

          <span class="shrink-0 text-[10px] text-muted-foreground">{{ directionLabel(info) }}</span>

          <CircleCheck v-if="info.state.kind === 'done'" :size="12" class="shrink-0 text-success" />
          <CircleSlash v-else-if="info.state.kind === 'cancelled'" :size="12" class="shrink-0 text-muted-foreground" />
          <CircleAlert v-else-if="info.state.kind === 'failed'" :size="12" class="shrink-0 text-destructive" />
          <CirclePause v-else-if="info.state.kind === 'paused'" :size="12" class="shrink-0 text-muted-foreground" />
          <span v-else class="shrink-0 text-[10px] text-muted-foreground">{{ stateLabel(info.state) }}</span>

          <button
            type="button"
            class="shrink-0 rounded-sm px-1.5 py-0.5 text-[10px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            @click="rowAction(info).run()"
          >
            {{ rowAction(info).label }}
          </button>
        </div>

        <div class="mt-1 flex items-center gap-2">
          <div class="h-1.5 min-w-0 flex-1 overflow-hidden rounded-full bg-muted">
            <div
              v-if="indeterminate(info)"
              :class="cn('h-full w-1/3 animate-pulse rounded-full', fillClass(info.state))"
            ></div>
            <div
              v-else
              :class="cn('h-full rounded-full transition-[width] duration-300', fillClass(info.state))"
              :style="{ width: `${percent(info)}%` }"
            ></div>
          </div>
          <span class="shrink-0 font-mono text-[10px] text-muted-foreground">
            {{ t("files.transfer.progress", { transferred: formatSize(info.transferred), size: sizeText(info) }) }}
          </span>
        </div>

        <div v-if="info.state.kind !== 'running'" class="mt-1 flex items-baseline gap-2">
          <span
            :class="
              cn('shrink-0 text-[10px]', info.state.kind === 'failed' ? 'text-destructive' : 'text-muted-foreground')
            "
          >
            {{ stateLabel(info.state) }}
          </span>
          <span
            v-if="info.state.kind === 'failed'"
            class="min-w-0 font-mono text-[10px] break-all text-muted-foreground"
          >
            {{ info.state.detail }}
          </span>
        </div>
      </li>
    </ul>
  </section>
</template>
