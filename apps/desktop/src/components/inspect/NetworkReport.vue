<script setup lang="ts">
/**
 * The host's network, as sockets rather than as interfaces.
 *
 * The page answers two questions, one under the other: which ports this host is waiting on, and —
 * for the one somebody picks — who is on it. Both come from one list the host produced, so the
 * count beside a port and the rows under it are the same reading rather than two that agree.
 *
 * Three things are deliberately shaped here:
 *
 * 1. **A row is the control.** Picking a port is what the table is for, so the whole row is
 *    clickable and focusable rather than a link buried in one cell — and it says it is selected,
 *    because the table underneath is about that row and nothing else on screen says which.
 * 2. **A count and its rows are computed from the same list.** "3 connections" and three rows are
 *    one fact; a count from the host and rows from somewhere else could disagree by the time
 *    somebody reads them.
 * 3. **Nothing is invented for a column the host did not answer.** A tool that reports no byte
 *    counters leaves both traffic columns as dashes, and `netstat` is that tool.
 */
import { X } from "@lucide/vue";
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import SortHeader from "@/components/inspect/SortHeader.vue";
import { describeShellError } from "@/i18n/errors";
import { formatSize } from "@/lib/files/format";
import { UNKNOWN } from "@/lib/monitor/format";
import { type SortDirection, startsDescending } from "@/lib/monitor/sort";
import { type ListenerSort, connectionsFor, listenerRows, sortListeners } from "@/lib/monitor/sockets";
import { useMonitorStore } from "@/stores/monitor";
import type { SocketRow } from "@/types/monitor";

const props = defineProps<{
  /** Whether the sockets on screen are about this page's host. */
  live: boolean;
  /** Whether this page is the tab on screen. A hidden page reads nothing. */
  active: boolean;
}>();

const { t } = useI18n();
const monitor = useMonitorStore();

/** Which socket the table underneath is about. Held here: it is a view of this page, not state. */
const picked = ref<SocketRow | null>(null);
const sort = ref<ListenerSort>("port");
const descending = ref(false);

const rows = computed(() =>
  sortListeners(props.live ? listenerRows(monitor.sockets) : [], sort.value, descending.value),
);

const connections = computed(() => (picked.value ? connectionsFor(monitor.sockets, picked.value) : []));

const failure = computed(() => (monitor.socketFailure ? describeShellError(translate, monitor.socketFailure) : null));

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

function bytes(value: number | null): string {
  return value === null ? UNKNOWN : formatSize(value);
}

function sameSocket(left: SocketRow | null, right: SocketRow): boolean {
  return (
    left !== null &&
    left.protocol === right.protocol &&
    left.address === right.address &&
    left.port === right.port &&
    left.pid === right.pid
  );
}

/** Picking a row again puts the table underneath away, which is what clicking a selected tab means. */
function pick(row: SocketRow): void {
  picked.value = sameSocket(picked.value, row) ? null : row;
}

/**
 * Orders by a column, or turns the order around when it is the column already in use.
 *
 * A column nobody has chosen yet starts at the end it is read from: an address reads from the
 * alphabet, a count from the top. That is the rule both tables in this application follow, and the
 * reason it is stated per column rather than fixed once is that "descending" and "ascending" mean
 * different things to a name and to a number.
 */
function orderBy(key: ListenerSort, first: SortDirection): void {
  const chosen = sort.value === key;
  sort.value = key;
  descending.value = chosen ? !descending.value : startsDescending(first);
}

const columns: readonly { key: ListenerSort; labelKey: string; first: SortDirection }[] = [
  { key: "address", labelKey: "inspect.network.address", first: "ascending" },
  { key: "port", labelKey: "inspect.network.port", first: "ascending" },
];

// The list is read while this page is the tab on screen, and not otherwise: it is the page that
// wants it, so it is the page that pays for it.
watch(
  () => props.active,
  (active) => (active ? monitor.startSockets() : monitor.stopSockets()),
  { immediate: true },
);

onBeforeUnmount(() => {
  if (props.active) {
    monitor.stopSockets();
  }
});

// A port that is no longer there cannot be the one the table underneath is about. Dropping the
// selection is better than drawing an empty table under a port that has gone.
watch(rows, (current) => {
  if (picked.value && !current.some((row) => sameSocket(picked.value, row.socket))) {
    picked.value = null;
  }
});
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col gap-2">
    <p v-if="failure" class="shrink-0 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2">
      <span class="text-xs">{{ failure.message }}</span>
      <span class="mt-0.5 block font-mono text-[10px] text-muted-foreground">{{ failure.code }}</span>
    </p>

    <div class="min-h-0 flex-1 overflow-auto">
      <table class="w-full border-collapse text-xs" :aria-label="t('inspect.network.listeners')">
        <thead class="sticky top-0 bg-background">
          <tr class="border-b border-border text-muted-foreground">
            <th class="w-20 py-1 pr-2 text-right font-medium">PID</th>
            <th class="w-36 py-1 pr-2 text-left font-medium">{{ t("inspect.process.name") }}</th>
            <th class="w-14 py-1 pr-2 text-left font-medium">{{ t("inspect.network.protocol") }}</th>
            <th
              v-for="column in columns"
              :key="column.key"
              class="py-1 pr-2 text-left font-medium"
              :aria-sort="sort === column.key ? (descending ? 'descending' : 'ascending') : 'none'"
            >
              <SortHeader
                :label="t(column.labelKey)"
                :sorted="sort === column.key"
                :descending="descending"
                :hint="t('inspect.network.sortBy', { column: t(column.labelKey) })"
                @sort="orderBy(column.key, column.first)"
              />
            </th>
            <th class="w-16 py-1 pr-2 text-right font-medium">{{ t("inspect.network.peers") }}</th>
            <th
              class="w-20 py-1 pr-2 text-right font-medium"
              :aria-sort="sort === 'connections' ? (descending ? 'descending' : 'ascending') : 'none'"
            >
              <SortHeader
                :label="t('inspect.network.count')"
                :sorted="sort === 'connections'"
                :descending="descending"
                :hint="t('inspect.network.sortBy', { column: t('inspect.network.count') })"
                @sort="orderBy('connections', 'descending')"
              />
            </th>
            <th class="w-24 py-1 pr-2 text-right font-medium">{{ t("inspect.network.sent") }}</th>
            <th class="w-24 py-1 text-right font-medium">{{ t("inspect.network.received") }}</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="row in rows"
            :key="`${row.socket.protocol}:${row.socket.address}:${row.socket.port}:${row.socket.pid}`"
            class="cursor-pointer border-b border-border/50 outline-none hover:bg-accent/40 focus-visible:bg-accent/60"
            :class="sameSocket(picked, row.socket) ? 'bg-accent/60' : ''"
            tabindex="0"
            :aria-selected="sameSocket(picked, row.socket)"
            @click="pick(row.socket)"
            @keydown.enter.prevent="pick(row.socket)"
            @keydown.space.prevent="pick(row.socket)"
          >
            <td class="py-1 pr-2 text-right font-mono">{{ row.socket.pid || UNKNOWN }}</td>
            <td class="truncate py-1 pr-2 font-mono" :title="row.socket.process">
              {{ row.socket.process || UNKNOWN }}
            </td>
            <td class="py-1 pr-2 font-mono text-muted-foreground">{{ row.socket.protocol }}</td>
            <td class="truncate py-1 pr-2 font-mono">{{ row.socket.address }}</td>
            <td class="py-1 pr-2 font-mono">{{ row.socket.port }}</td>
            <td class="py-1 pr-2 text-right font-mono">{{ row.summary.peers }}</td>
            <td class="py-1 pr-2 text-right font-mono">{{ row.summary.connections }}</td>
            <td class="py-1 pr-2 text-right font-mono">{{ bytes(row.summary.sentBytes) }}</td>
            <td class="py-1 text-right font-mono">{{ bytes(row.summary.receivedBytes) }}</td>
          </tr>
          <tr v-if="rows.length === 0">
            <td colspan="9" class="py-3 text-muted-foreground">
              {{ props.live ? t("inspect.network.empty") : UNKNOWN }}
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <!--
      The connections of the port above, in the shape the page uses for the sockets themselves:
      a strip naming what the table is about, then the table. The strip is what closes it, because
      a table that appeared under a row without saying which row would be a table nobody trusts.
    -->
    <section v-if="picked" class="flex min-h-0 flex-1 flex-col border-t border-border pt-1">
      <div class="flex shrink-0 items-center gap-2 pb-1">
        <span class="rounded-t-md border border-b-0 border-border bg-card/40 px-2 py-1 font-mono text-[11px]">
          {{ picked.address }}:{{ picked.port }}
        </span>
        <span class="min-w-0 flex-1 truncate text-[11px] text-muted-foreground">
          {{ picked.process || UNKNOWN }} · {{ t("inspect.network.connections") }}
        </span>
        <button
          type="button"
          class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          :title="t('inspect.network.close')"
          :aria-label="t('inspect.network.close')"
          @click="picked = null"
        >
          <X :size="11" />
        </button>
      </div>

      <div class="min-h-0 flex-1 overflow-auto">
        <table class="w-full border-collapse text-xs" :aria-label="t('inspect.network.connections')">
          <thead class="sticky top-0 bg-background">
            <tr class="border-b border-border text-muted-foreground">
              <th class="py-1 pr-2 text-left font-medium">{{ t("inspect.network.peer") }}</th>
              <th class="w-24 py-1 pr-2 text-right font-medium">{{ t("inspect.network.port") }}</th>
              <th class="w-28 py-1 pr-2 text-left font-medium">{{ t("inspect.network.state") }}</th>
              <th class="w-24 py-1 pr-2 text-right font-medium">{{ t("inspect.network.sent") }}</th>
              <th class="w-24 py-1 text-right font-medium">{{ t("inspect.network.received") }}</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="connection in connections"
              :key="`${connection.peerAddress}:${connection.peerPort}`"
              class="border-b border-border/50"
            >
              <td class="truncate py-1 pr-2 font-mono">{{ connection.peerAddress }}</td>
              <td class="py-1 pr-2 text-right font-mono">{{ connection.peerPort }}</td>
              <td class="truncate py-1 pr-2 font-mono text-muted-foreground">{{ connection.state || UNKNOWN }}</td>
              <td class="py-1 pr-2 text-right font-mono">{{ bytes(connection.sentBytes) }}</td>
              <td class="py-1 text-right font-mono">{{ bytes(connection.receivedBytes) }}</td>
            </tr>
            <tr v-if="connections.length === 0">
              <td colspan="5" class="py-2 text-muted-foreground">{{ t("inspect.network.connectionsEmpty") }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>
  </div>
</template>
