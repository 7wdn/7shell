<script setup lang="ts">
/**
 * Every process the host would tell us about: what it is, what it is using, and — for the ones
 * somebody opens — how it was started and with what.
 *
 * Two things here are quietly different from the panel beside it, and both are deliberate:
 *
 * 1. **The CPU column is an interval.** The panel shows the host's own column, which is an average
 *    over each process's whole life; this page shows what the process used since the last read,
 *    because a table that refreshes and never changes is a table nobody believes. Both readings are
 *    true, and each says which it is.
 * 2. **The table is ordered here, not by the host.** The host cuts its answer before sending it — a
 *    full table with command lines is not something to ask for every few seconds — and what arrives
 *    is ordered by the host's own measure. Sorting that is a redraw rather than another command on
 *    somebody's server.
 *
 * Opening a process adds a tab under the table rather than replacing what is there: comparing two
 * processes is the reason to open this page at all, and a panel that replaced its own contents
 * would make that two visits.
 */
import { LoaderCircle, RefreshCw, X } from "@lucide/vue";
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import SortHeader from "@/components/inspect/SortHeader.vue";
import { describeShellError } from "@/i18n/errors";
import { formatSize } from "@/lib/files/format";
import { UNKNOWN } from "@/lib/monitor/format";
import type { ProcessSortKey } from "@/lib/monitor/processes";
import { type SortDirection, startsDescending } from "@/lib/monitor/sort";
import { useMonitorStore } from "@/stores/monitor";

const props = defineProps<{ active: boolean }>();

const { t } = useI18n();
const monitor = useMonitorStore();

/** One process somebody opened, with the name it had when they opened it. */
interface OpenProcess {
  readonly pid: number;
  readonly name: string;
}

const opened = ref<OpenProcess[]>([]);
const selected = ref<number | null>(null);

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const rows = computed(() => monitor.processes);

/** Pressing a heading orders by it, and pressing the one in use turns the table around. */
function orderBy(key: ProcessSortKey, first: SortDirection): void {
  const chosen = monitor.processSort === key;
  monitor.sortProcesses(key, chosen ? !monitor.processDescending : startsDescending(first));
}

/** Opens a process under the table, or brings the tab that is already there forward. */
function open(pid: number, name: string): void {
  if (!opened.value.some((entry) => entry.pid === pid)) {
    opened.value = [...opened.value, { pid, name }];
    // Read once, when somebody asks about this process — never as part of the poll.
    void monitor.readProcessDetail(pid);
  }
  selected.value = pid;
}

function close(pid: number): void {
  opened.value = opened.value.filter((entry) => entry.pid !== pid);
  monitor.forgetProcess(pid);
  if (selected.value === pid) {
    selected.value = opened.value[opened.value.length - 1]?.pid ?? null;
  }
}

const detail = computed(() => (selected.value === null ? null : (monitor.details[selected.value] ?? null)));
const detailFailure = computed(() =>
  selected.value === null ? null : (monitor.detailFailures[selected.value] ?? null),
);
const opening = computed(() => selected.value !== null && monitor.readingDetail === selected.value);

/** A percentage, or a dash for a process that was not in the previous read to compare with. */
function cpu(row: (typeof rows.value)[number]): string {
  return row.cpuPercent === null ? UNKNOWN : row.cpuPercent.toFixed(1);
}

// The list is read while this page is the tab on screen, and not otherwise.
watch(
  () => props.active,
  (active) => (active ? monitor.startProcesses() : monitor.stopProcesses()),
  { immediate: true },
);

onBeforeUnmount(() => {
  if (props.active) {
    monitor.stopProcesses();
  }
});

// A process that has gone cannot be the one the panel below is about: its tab is dropped rather
// than left showing the last thing it said under a name that is no longer running.
watch(rows, (current) => {
  const alive = new Set(current.map((row) => row.sample.pid));
  // Walking the list while closing tabs in it is safe here because closing assigns a new array
  // rather than removing from this one: the loop is over the array it started with.
  for (const entry of opened.value) {
    if (!alive.has(entry.pid)) {
      close(entry.pid);
    }
  }
});
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col gap-2">
    <p
      v-if="monitor.processFailure"
      class="shrink-0 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2"
    >
      <span class="text-xs">{{ describeShellError(translate, monitor.processFailure).message }}</span>
      <span class="mt-0.5 block font-mono text-[10px] text-muted-foreground">
        {{ describeShellError(translate, monitor.processFailure).code }}
      </span>
    </p>

    <div class="min-h-0 flex-1 overflow-auto">
      <table class="w-full border-collapse text-xs" :aria-label="t('inspect.processes.title')">
        <thead class="sticky top-0 bg-background">
          <tr class="border-b border-border text-muted-foreground">
            <th class="w-20 py-1 pr-2 text-right font-medium">
              <span class="flex items-center justify-end gap-1">
                PID
                <LoaderCircle v-if="monitor.readingProcesses" :size="10" class="animate-spin" />
              </span>
            </th>
            <th class="w-20 py-1 pr-2 text-left font-medium">{{ t("inspect.process.user") }}</th>
            <th class="w-24 py-1 pr-2 text-right font-medium">
              <SortHeader
                :label="t('monitor.processes.memory')"
                :sorted="monitor.processSort === 'memory'"
                :descending="monitor.processDescending"
                :hint="t('inspect.process.sortBy', { column: t('monitor.processes.memory') })"
                @sort="orderBy('memory', 'descending')"
              />
            </th>
            <th
              class="w-20 py-1 pr-2 text-right font-medium"
              :title="t('inspect.process.cpuSinceRead')"
              :aria-sort="
                monitor.processSort === 'cpu' ? (monitor.processDescending ? 'descending' : 'ascending') : 'none'
              "
            >
              <SortHeader
                :label="t('monitor.processes.cpu')"
                :sorted="monitor.processSort === 'cpu'"
                :descending="monitor.processDescending"
                :hint="t('inspect.process.sortBy', { column: t('monitor.processes.cpu') })"
                @sort="orderBy('cpu', 'descending')"
              />
            </th>
            <th
              class="w-40 py-1 pr-2 text-left font-medium"
              :aria-sort="
                monitor.processSort === 'name' ? (monitor.processDescending ? 'descending' : 'ascending') : 'none'
              "
            >
              <SortHeader
                :label="t('inspect.process.name')"
                :sorted="monitor.processSort === 'name'"
                :descending="monitor.processDescending"
                :hint="t('inspect.process.sortBy', { column: t('inspect.process.name') })"
                @sort="orderBy('name', 'ascending')"
              />
            </th>
            <th class="py-1 pr-2 text-left font-medium">{{ t("inspect.process.command") }}</th>
            <th class="w-56 py-1 text-left font-medium">{{ t("inspect.process.executable") }}</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="row in rows"
            :key="row.sample.pid"
            class="cursor-pointer border-b border-border/50 outline-none hover:bg-accent/40 focus-visible:bg-accent/60"
            :class="selected === row.sample.pid ? 'bg-accent/60' : ''"
            tabindex="0"
            :aria-selected="selected === row.sample.pid"
            @click="open(row.sample.pid, row.sample.name)"
            @keydown.enter.prevent="open(row.sample.pid, row.sample.name)"
            @keydown.space.prevent="open(row.sample.pid, row.sample.name)"
          >
            <td class="py-1 pr-2 text-right font-mono">{{ row.sample.pid }}</td>
            <td class="truncate py-1 pr-2 font-mono">{{ row.sample.user }}</td>
            <td class="py-1 pr-2 text-right font-mono">{{ formatSize(row.sample.residentBytes) }}</td>
            <td class="py-1 pr-2 text-right font-mono">{{ cpu(row) }}</td>
            <td class="truncate py-1 pr-2 font-mono" :title="row.sample.name">{{ row.sample.name }}</td>
            <td class="max-w-0 truncate py-1 pr-2 font-mono text-muted-foreground" :title="row.sample.command">
              {{ row.sample.command }}
            </td>
            <td class="max-w-0 truncate py-1 font-mono text-muted-foreground" :title="row.sample.executable ?? ''">
              {{ row.sample.executable ?? UNKNOWN }}
            </td>
          </tr>
          <tr v-if="rows.length === 0">
            <td colspan="7" class="py-3 text-muted-foreground">{{ t("inspect.process.empty") }}</td>
          </tr>
        </tbody>
      </table>
    </div>

    <!--
      The processes somebody opened, one tab each. The strip says which of them the panel is about,
      because a panel that appeared under a table without saying which row would be a panel nobody
      could trust — and it is also what closes one.
    -->
    <section v-if="opened.length > 0" class="flex min-h-0 flex-1 flex-col border-t border-border pt-1">
      <div class="flex shrink-0 items-center gap-1 overflow-x-auto">
        <div
          v-for="entry in opened"
          :key="entry.pid"
          class="flex shrink-0 items-center gap-1 rounded-t-md border border-b-0 px-2 py-1 text-[11px]"
          :class="selected === entry.pid ? 'border-border bg-background' : 'border-transparent text-muted-foreground'"
        >
          <button type="button" class="max-w-48 truncate font-mono" @click="selected = entry.pid">
            {{ entry.pid }}-{{ entry.name }}
          </button>
          <button
            type="button"
            class="grid size-4 shrink-0 place-items-center rounded-sm transition-colors hover:bg-accent"
            :title="t('inspect.process.close')"
            :aria-label="t('inspect.process.close')"
            @click="close(entry.pid)"
          >
            <X :size="10" />
          </button>
        </div>

        <span class="flex-1"></span>
        <button
          v-if="selected !== null"
          type="button"
          class="flex shrink-0 items-center gap-1 rounded-md px-2 py-0.5 text-[10px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          :title="t('inspect.process.refresh')"
          :aria-label="t('inspect.process.refresh')"
          @click="selected !== null && monitor.readProcessDetail(selected)"
        >
          <RefreshCw :size="11" :class="opening ? 'animate-spin' : ''" />
          {{ t("inspect.process.refresh") }}
        </button>
      </div>

      <div class="min-h-0 flex-1 overflow-auto pt-2">
        <p v-if="detailFailure" class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2">
          <span class="text-xs">{{ describeShellError(translate, detailFailure).message }}</span>
          <span class="mt-0.5 block font-mono text-[10px] text-muted-foreground">
            {{ describeShellError(translate, detailFailure).code }}
          </span>
        </p>

        <p v-else-if="opening || !detail" class="text-xs text-muted-foreground">{{ t("common.status.loading") }}</p>

        <div v-else class="flex flex-col gap-4 lg:flex-row">
          <dl class="grid min-w-0 flex-1 grid-cols-[7rem_1fr] gap-x-4 gap-y-1 text-xs">
            <dt class="text-muted-foreground">PID</dt>
            <dd class="font-mono">{{ detail.pid }}</dd>

            <dt class="text-muted-foreground">{{ t("inspect.process.name") }}</dt>
            <dd class="font-mono">{{ detail.name || UNKNOWN }}</dd>

            <dt class="text-muted-foreground">{{ t("inspect.process.executable") }}</dt>
            <dd class="min-w-0 truncate font-mono" :title="detail.executable">
              {{ detail.executable || UNKNOWN }}
            </dd>

            <dt class="text-muted-foreground">{{ t("inspect.process.workingDirectory") }}</dt>
            <dd class="min-w-0 truncate font-mono" :title="detail.workingDirectory">
              {{ detail.workingDirectory || UNKNOWN }}
            </dd>

            <dt class="text-muted-foreground">{{ t("inspect.process.command") }}</dt>
            <dd class="min-w-0 font-mono break-all">{{ detail.commandLine || UNKNOWN }}</dd>
          </dl>

          <div class="min-w-0 flex-1">
            <h3 class="mb-1 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
              {{ t("inspect.process.environment") }}
            </h3>
            <table v-if="detail.environment.length > 0" class="w-full border-collapse text-[11px]">
              <tbody>
                <tr v-for="entry in detail.environment" :key="entry.name" class="border-b border-border/50">
                  <td
                    class="max-w-48 truncate py-0.5 pr-3 align-top font-mono text-muted-foreground"
                    :title="entry.name"
                  >
                    {{ entry.name }}
                  </td>
                  <td class="py-0.5 font-mono break-all">{{ entry.value }}</td>
                </tr>
              </tbody>
            </table>
            <p v-else class="text-xs text-muted-foreground">{{ t("inspect.process.environmentEmpty") }}</p>
          </div>
        </div>
      </div>
    </section>
  </div>
</template>
