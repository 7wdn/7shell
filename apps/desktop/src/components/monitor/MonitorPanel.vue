<script setup lang="ts">
/**
 * The host monitor: what the machine on the other end of the terminal on screen is doing.
 *
 * It sits beside the workspace rather than inside a tab, because it describes a *connection*
 * rather than a view of one: it follows whichever session is on screen, and a person who switches
 * tabs expects the panel to switch with them. When nothing is connected it draws the same rows
 * with dashes in them, which is the honest picture — an empty column is not a reading of zero, and
 * a panel that disappeared would leave the workspace jumping whenever the last tab closed.
 *
 * It is also the way in to the host's pages: the system box, the process table and the network
 * card each open a tab that shows the same subject in full — every process with its command line,
 * every interface with its counters, the processor's own model and where its time goes. The panel
 * is the glance; those pages are the read. Each of them is opened for the session the panel is
 * about, and the page reads through whatever terminal that session has open.
 *
 * Three things are deliberately not here. It does not decide when to sample (the store does, and
 * stops entirely when this component is not mounted). It does not decide what a rate is (that is
 * two samples subtracted, in `lib/monitor/rates.ts`). And it does not turn a missing reading into
 * a zero: every section a host did not report says so, in words, because a dash next to a number
 * that means something is exactly the confusion a monitor must not create.
 */
import { Activity, ArrowDown, ArrowUp, Check, ChevronRight, CircleAlert, Copy, X } from "@lucide/vue";
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import InterfacePicker from "@/components/inspect/InterfacePicker.vue";
import MonitorSpark from "@/components/monitor/MonitorSpark.vue";
import { useContextMenu } from "@/composables/useContextMenu";
import { describeShellError } from "@/i18n/errors";
import { formatSize } from "@/lib/files/format";
import { diskMenu, monitorProcessMenu } from "@/lib/menu/monitor";
import { MENU_TITLES } from "@/lib/menu/titles";
import { perSecond, uptime } from "@/lib/monitor/format";
import { memoryUsed, share, swapUsed } from "@/lib/monitor/rates";
import { TABLE, rowClass } from "@/lib/table";
import { useMonitorStore } from "@/stores/monitor";
import { usePreferencesStore } from "@/stores/preferences";
import { sessionOf, useTerminalsStore, type InspectView } from "@/stores/terminals";
import type { DiskUsage, ProcessSample } from "@/types/monitor";

/** How long the copy button says it copied something. */
const COPIED_MS = 1500;

const { t } = useI18n();
const monitor = useMonitorStore();
const preferences = usePreferencesStore();
const terminals = useTerminalsStore();
const { openMenu } = useContextMenu();

// The panel is mounted exactly while the monitor is on, so being on screen is what starts the
// sampling and leaving the screen is what stops it. Nothing runs behind a hidden panel.
onMounted(() => monitor.start());
onBeforeUnmount(() => monitor.stop());

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const copied = ref(false);

const sample = computed(() => monitor.sample);
const memory = computed(() => sample.value?.memory ?? null);

/** The host, or a dash: the panel is drawn before anything is connected, and after. */
const host = computed(() => monitor.target?.host ?? "-");

/**
 * The session this panel is about. A page can only be opened for one that exists — and the tab on
 * screen is not always about a host, since the settings page is a tab too.
 */
const session = computed(() => {
  const active = terminals.activeTab;
  return monitor.target && active ? sessionOf(active) : null;
});

const failure = computed(() => (monitor.failure ? describeShellError(translate, monitor.failure) : null));

/** The load averages, which are run-queue lengths rather than percentages. */
const load = computed(() => {
  const load = sample.value?.load;
  if (!load) {
    return "-";
  }
  return [load.one, load.five, load.fifteen].map((value) => value.toFixed(2)).join(", ");
});

const upFor = computed(() => {
  const seconds = sample.value?.uptimeSeconds;
  if (seconds === null || seconds === undefined) {
    return "-";
  }
  const { days, hours, minutes } = uptime(seconds);
  const rest = { hours, minutes: String(minutes).padStart(2, "0") };
  return days > 0 ? t("monitor.uptime.days", { ...rest, days }) : t("monitor.uptime.hours", rest);
});

const memoryPercent = computed(() => (memory.value ? share(memoryUsed(memory.value), memory.value.totalBytes) : null));
const memoryText = computed(() =>
  memory.value ? `${formatSize(memoryUsed(memory.value))} / ${formatSize(memory.value.totalBytes)}` : "-",
);

const swapPercent = computed(() => (memory.value ? share(swapUsed(memory.value), memory.value.swapTotalBytes) : null));
const swapText = computed(() =>
  memory.value ? `${formatSize(swapUsed(memory.value))} / ${formatSize(memory.value.swapTotalBytes)}` : "-",
);

const rate = computed(() => monitor.currentRate);

const trafficSeries = computed(() => [
  { values: monitor.traffic.map((point) => point.received), class: "bg-primary/70" },
  { values: monitor.traffic.map((point) => point.transmitted), class: "bg-muted-foreground/40" },
]);
const roundTripSeries = computed(() => [{ values: monitor.roundTrips, class: "bg-primary/50" }]);

const lastRoundTrip = computed(() => {
  const value = monitor.roundTrips[monitor.roundTrips.length - 1];
  return value === undefined ? "-" : t("monitor.sample.milliseconds", { value });
});

const sampledAt = computed(() => {
  const at = sample.value?.sampledAt;
  return at === undefined ? null : t("monitor.sample.at", { time: new Date(at).toLocaleTimeString() });
});

/**
 * What each chart is, for a reader who cannot see it. A bar is not readable, so the label has to
 * carry both the subject and what the heights mean — which is the unit, and the fact that it was
 * measured between two samples rather than read off the host.
 */
const trafficLabel = computed(() => `${t("monitor.network.title")} — ${t("monitor.network.rateHint")}`);
const roundTripLabel = computed(() => `${t("monitor.sample.roundTrip")} — ${t("monitor.sample.hint")}`);

function percent(value: number | null): string {
  return value === null ? "-" : `${Math.round(value)}%`;
}

/** A bar for a percentage, or an empty track when there is nothing to draw. */
function barWidth(value: number | null): string {
  return `${Math.min(Math.max(value ?? 0, 0), 100)}%`;
}

/** Opens a host page for the session on screen. A panel with no connection has no page to open. */
function open(view: InspectView): void {
  const target = session.value;
  if (target) {
    terminals.openInspect(view, target);
  }
}

async function copyHost(): Promise<void> {
  const value = monitor.target?.host;
  if (!value) {
    return;
  }
  try {
    await navigator.clipboard.writeText(value);
    copied.value = true;
    window.setTimeout(() => {
      copied.value = false;
    }, COPIED_MS);
  } catch {
    // A webview that refuses the clipboard is not worth an error banner over a copy button: the
    // button stays as it was, and nothing claims to have been copied.
  }
}

/**
 * The small tables' menus.
 *
 * A row's way out is the page that shows the same subject in full, and `open` is the same call the
 * section heading makes — so a row and its heading lead to the same place, which is the only reason
 * both exist.
 */
function openProcessMenu(event: MouseEvent, process: ProcessSample): void {
  openMenu(event, { key: MENU_TITLES.process }, monitorProcessMenu(process, { openPage: () => open("processes") }));
}

function openDiskMenu(event: MouseEvent, disk: DiskUsage): void {
  openMenu(event, { key: MENU_TITLES.disk }, diskMenu(disk, { openPage: () => open("system") }));
}
</script>

<template>
  <!--
    The panel is an instrument rather than a document, and it says so once: left to itself the
    browser treats every reading and label as text — a drag across the panel smears a selection over
    the rows, and the pointer turns into an I-beam over the numbers. Both are its default for
    selectable content, and both are wrong here, where nothing is taken by hand: the host is copied
    by the button in the header, a process or a file system by its own menu. The failure block below
    is the one exception, because a code somebody has to quote is worth selecting.
  -->
  <aside
    class="flex w-80 shrink-0 flex-col overflow-hidden border-l border-border bg-card/20 cursor-default select-none"
    :aria-label="t('monitor.title')"
  >
    <header class="shrink-0 border-b border-border px-3 py-2">
      <div class="flex items-center gap-2">
        <Activity :size="13" :class="monitor.sampling ? 'animate-pulse text-primary' : 'text-muted-foreground'" />
        <h2 class="min-w-0 flex-1 truncate text-xs font-medium">{{ t("monitor.title") }}</h2>
        <button
          type="button"
          class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          :title="t('monitor.close')"
          :aria-label="t('monitor.close')"
          @click="preferences.setMonitorEnabled(false)"
        >
          <X :size="13" />
        </button>
      </div>

      <div class="mt-1.5 flex items-center gap-2">
        <span class="text-[10px] tracking-wide text-muted-foreground uppercase">{{ t("monitor.address") }}</span>
        <span class="min-w-0 flex-1 truncate font-mono text-xs">{{ host }}</span>
        <button
          type="button"
          class="flex shrink-0 items-center gap-1 rounded-md px-1.5 py-0.5 text-[10px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40 disabled:hover:bg-transparent"
          :disabled="monitor.target === null"
          :title="t('monitor.copy')"
          :aria-label="t('monitor.copy')"
          @click="copyHost()"
        >
          <Check v-if="copied" :size="11" class="text-success" />
          <Copy v-else :size="11" />
          {{ copied ? t("monitor.copied") : t("monitor.copy") }}
        </button>
      </div>
    </header>

    <div class="min-h-0 flex-1 space-y-3 overflow-y-auto px-3 py-2">
      <!--
        One line about why there is nothing to show, and never instead of the rows: the panel keeps
        its shape whether or not anything is connected, so the workspace beside it does not jump
        when the last terminal closes.
      -->
      <p v-if="!monitor.target" class="text-[11px] leading-relaxed text-muted-foreground">
        <span class="text-foreground">{{ t("monitor.idle.title") }}</span> — {{ t("monitor.idle.hint") }}
      </p>
      <div
        v-else-if="failure"
        data-monitor-failure
        class="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 select-text"
      >
        <p class="flex items-start gap-1.5 text-[11px]">
          <CircleAlert :size="12" class="mt-0.5 shrink-0 text-destructive" />
          <span>{{ failure.message }}</span>
        </p>
        <p class="mt-0.5 pl-[18px] font-mono text-[10px] text-muted-foreground">{{ failure.code }}</p>
      </div>
      <p v-else-if="!sample" class="text-[11px] text-muted-foreground">{{ t("monitor.waiting") }}</p>

      <!--
        The system box is a button, and everything inside it is a span: a button may only hold
        phrasing content, and a `dl` in a button is the kind of markup that works until the day it
        does not. The grid is a class on a span, which is a style and not a structure.
      -->
      <button
        type="button"
        class="group block w-full rounded-md border border-border bg-background/60 px-2 py-1.5 text-left transition-colors hover:border-primary/40 hover:bg-accent/40 disabled:cursor-default disabled:hover:border-border disabled:hover:bg-background/60"
        :disabled="session === null"
        :title="t('inspect.system.title')"
        :aria-label="t('inspect.system.title')"
        @click="open('system')"
      >
        <span class="flex items-center gap-1">
          <span class="text-[10px] tracking-wide text-muted-foreground uppercase">
            {{ t("monitor.system.title") }}
          </span>
          <span class="flex-1"></span>
          <ChevronRight :size="11" class="text-muted-foreground" />
        </span>
        <span class="mt-1 grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-[11px]">
          <span class="text-muted-foreground">{{ t("monitor.system.hostname") }}</span>
          <span class="truncate font-mono">{{ sample?.hostname ?? "-" }}</span>

          <span class="text-muted-foreground">{{ t("monitor.system.os") }}</span>
          <span class="truncate">{{ sample?.os ?? "-" }}</span>

          <span class="text-muted-foreground">{{ t("monitor.system.kernel") }}</span>
          <span class="truncate font-mono">
            {{ sample === null ? "-" : `${sample.kernel ?? "-"} ${sample.arch ?? ""}`.trim() }}
          </span>

          <span class="text-muted-foreground">{{ t("monitor.system.cores") }}</span>
          <span class="font-mono">{{ sample?.cores ?? "-" }}</span>
        </span>
      </button>

      <!--
        Uptime, load, CPU, memory and swap: one row each, drawn as label, bar, percentage, reading.
        The bar and the percentage are the same number, so a row without one has neither — a bar at
        zero beside a dash would read as an idle machine rather than an unread one.
      -->
      <section class="space-y-1 text-[11px]">
        <div class="flex items-center gap-2">
          <span class="w-10 shrink-0 text-muted-foreground">{{ t("monitor.uptime.label") }}</span>
          <span class="min-w-0 flex-1 truncate font-mono">{{ upFor }}</span>
        </div>

        <div class="flex items-center gap-2" :title="t('monitor.loadHint')">
          <span class="w-10 shrink-0 text-muted-foreground">{{ t("monitor.load") }}</span>
          <span class="min-w-0 flex-1 truncate font-mono">{{ load }}</span>
        </div>

        <div class="flex items-center gap-2" :title="t('monitor.cpuHint')">
          <span class="w-10 shrink-0 text-muted-foreground">{{ t("monitor.cpu") }}</span>
          <span class="h-2 min-w-0 flex-1 overflow-hidden rounded-sm bg-muted">
            <span class="block h-full rounded-sm bg-primary/70" :style="{ width: barWidth(monitor.cpuPercent) }" />
          </span>
          <span class="w-8 shrink-0 text-right font-mono">{{ percent(monitor.cpuPercent) }}</span>
        </div>

        <div class="flex items-center gap-2">
          <span class="w-10 shrink-0 text-muted-foreground">{{ t("monitor.memory") }}</span>
          <span class="h-2 min-w-0 flex-1 overflow-hidden rounded-sm bg-muted">
            <span class="block h-full rounded-sm bg-primary/70" :style="{ width: barWidth(memoryPercent) }" />
          </span>
          <span class="w-8 shrink-0 text-right font-mono">{{ percent(memoryPercent) }}</span>
          <span class="w-24 shrink-0 truncate text-right font-mono text-muted-foreground">{{ memoryText }}</span>
        </div>

        <div class="flex items-center gap-2">
          <span class="w-10 shrink-0 text-muted-foreground">{{ t("monitor.swap") }}</span>
          <span class="h-2 min-w-0 flex-1 overflow-hidden rounded-sm bg-muted">
            <span class="block h-full rounded-sm bg-primary/70" :style="{ width: barWidth(swapPercent) }" />
          </span>
          <span class="w-8 shrink-0 text-right font-mono">{{ percent(swapPercent) }}</span>
          <span class="w-24 shrink-0 truncate text-right font-mono text-muted-foreground">{{ swapText }}</span>
        </div>
      </section>

      <section>
        <button
          type="button"
          class="mb-1 flex w-full items-center gap-1 rounded-sm text-left transition-colors hover:text-foreground disabled:cursor-default"
          :disabled="session === null"
          :title="t('inspect.processes.title')"
          :aria-label="t('inspect.processes.title')"
          @click="open('processes')"
        >
          <span class="text-[10px] tracking-wide text-muted-foreground uppercase">
            {{ t("monitor.processes.title") }}
          </span>
          <span class="flex-1"></span>
          <ChevronRight :size="11" class="text-muted-foreground" />
        </button>
        <!--
          The panel's surface of the shared table vocabulary (`lib/table.ts`), and the same table the
          process page draws, at the size a 320px column can afford. What is said once for every table
          in this application — the heading rule, the band under the pointer, the message row that
          spans the columns — is said there; what is left here is which column is which.
        -->
        <table :class="TABLE.panel.table" :aria-label="t('inspect.processes.title')">
          <thead>
            <tr :class="TABLE.panel.headRow">
              <th class="w-16 text-right" :class="TABLE.panel.headCell">{{ t("monitor.processes.memory") }}</th>
              <th class="w-10 text-right" :class="TABLE.panel.headCell" :title="t('monitor.processes.cpuSinceStart')">
                {{ t("monitor.processes.cpu") }}
              </th>
              <th class="text-left" :class="TABLE.panel.headCell">{{ t("monitor.processes.command") }}</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="process in sample?.processes ?? []"
              :key="process.pid"
              :class="rowClass('panel')"
              @contextmenu="openProcessMenu($event, process)"
            >
              <td class="truncate text-right font-mono" :class="TABLE.panel.cell">
                {{ formatSize(process.residentBytes) }}
              </td>
              <td class="text-right font-mono" :class="TABLE.panel.cell">{{ process.cpuPercent.toFixed(1) }}</td>
              <td class="truncate font-mono" :class="TABLE.panel.cell">{{ process.name }}</td>
            </tr>
            <tr v-if="sample && sample.processes.length === 0">
              <td colspan="3" :class="TABLE.panel.emptyCell">{{ t("monitor.processes.empty") }}</td>
            </tr>
          </tbody>
        </table>
      </section>

      <section>
        <div class="mb-1 flex items-center gap-2">
          <button
            type="button"
            class="flex min-w-0 items-center gap-1 rounded-sm text-left transition-colors hover:text-foreground disabled:cursor-default"
            :disabled="session === null"
            :title="t('inspect.network.title')"
            :aria-label="t('inspect.network.title')"
            @click="open('network')"
          >
            <span class="text-[10px] tracking-wide text-muted-foreground uppercase">
              {{ t("monitor.network.title") }}
            </span>
            <ChevronRight :size="11" class="shrink-0 text-muted-foreground" />
          </button>
          <span class="flex-1"></span>
          <!-- The panel is always about the host it is drawn for, so its picker is always live. -->
          <InterfacePicker :live="monitor.target !== null" />
        </div>
        <div class="flex items-center gap-3 font-mono text-[11px]" :title="t('monitor.network.rateHint')">
          <span class="flex items-center gap-1 text-success">
            <ArrowUp :size="11" class="shrink-0" role="img" :aria-label="t('inspect.network.sent')" />
            {{ rate ? perSecond(rate.transmitted) : "-" }}
          </span>
          <span class="flex items-center gap-1 text-primary">
            <ArrowDown :size="11" class="shrink-0" role="img" :aria-label="t('inspect.network.received')" />
            {{ rate ? perSecond(rate.received) : "-" }}
          </span>
        </div>
        <MonitorSpark class="mt-1" :series="trafficSeries" :label="trafficLabel" />
        <p v-if="sample && sample.network.length === 0" class="mt-1 text-[10px] text-muted-foreground">
          {{ t("monitor.network.empty") }}
        </p>
      </section>

      <section>
        <div class="mb-1 flex items-baseline gap-2">
          <span class="font-mono text-[11px]">{{ lastRoundTrip }}</span>
          <span
            class="min-w-0 flex-1 truncate text-right text-[10px] text-muted-foreground"
            :title="t('monitor.sample.hint')"
          >
            {{ t("monitor.sample.roundTrip") }}
          </span>
        </div>
        <MonitorSpark :series="roundTripSeries" :label="roundTripLabel" />
      </section>

      <section>
        <h3 class="mb-1 text-[10px] tracking-wide text-muted-foreground uppercase">
          {{ t("monitor.disks.title") }}
        </h3>
        <table :class="TABLE.panel.table" :aria-label="t('monitor.disks.title')">
          <thead>
            <tr :class="TABLE.panel.headRow">
              <th class="text-left" :class="TABLE.panel.headCell">{{ t("monitor.disks.path") }}</th>
              <th class="w-28 text-right" :class="TABLE.panel.headCell">{{ t("monitor.disks.usage") }}</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="disk in sample?.disks ?? []"
              :key="`${disk.filesystem}:${disk.mountPoint}`"
              :class="rowClass('panel')"
              @contextmenu="openDiskMenu($event, disk)"
            >
              <td class="truncate font-mono" :class="TABLE.panel.cell" :title="disk.filesystem">
                {{ disk.mountPoint }}
              </td>
              <td class="truncate text-right font-mono" :class="TABLE.panel.cell">
                {{ formatSize(disk.availableBytes) }}/{{ formatSize(disk.sizeBytes) }}
              </td>
            </tr>
            <tr v-if="sample && sample.disks.length === 0">
              <td colspan="2" :class="TABLE.panel.emptyCell">{{ t("monitor.disks.empty") }}</td>
            </tr>
          </tbody>
        </table>
      </section>
    </div>

    <footer v-if="sampledAt" class="shrink-0 border-t border-border px-3 py-1 text-[10px] text-muted-foreground">
      {{ sampledAt }}
    </footer>
  </aside>
</template>
