<script setup lang="ts">
/**
 * What one host is: the system, the processor, where its time goes, its memory, its interfaces and
 * its file systems.
 *
 * The panel answers "is anything wrong right now"; this page answers "what is this machine", which
 * is a different set of questions asked once rather than every two seconds. Everything on it comes
 * from the same sample the panel reads — there is no second command behind this page — so a number
 * here and a number there are the same reading, not two that agree by luck.
 */
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import InterfaceTable from "@/components/inspect/InterfaceTable.vue";
import { formatSize } from "@/lib/files/format";
import { UNKNOWN, orUnknown, percentOf, uptime } from "@/lib/monitor/format";
import { CPU_STATE_LABELS, memoryUsed, share, swapUsed } from "@/lib/monitor/rates";
import { TABLE, rowClass } from "@/lib/table";
import { useMonitorStore } from "@/stores/monitor";

const props = defineProps<{
  /** Whether there is a sample to draw. A page with none still draws every row, with dashes. */
  live: boolean;
}>();

const { t } = useI18n();
const monitor = useMonitorStore();

const sample = computed(() => (props.live ? monitor.sample : null));
const memory = computed(() => sample.value?.memory ?? null);

const upFor = computed(() => {
  const seconds = sample.value?.uptimeSeconds;
  if (seconds === null || seconds === undefined) {
    return UNKNOWN;
  }
  const { days, hours, minutes } = uptime(seconds);
  const rest = { hours, minutes: String(minutes).padStart(2, "0") };
  return days > 0 ? t("monitor.uptime.days", { ...rest, days }) : t("monitor.uptime.hours", rest);
});

const memoryRows = computed(() => {
  const usage = memory.value;
  const used = usage ? memoryUsed(usage) : null;
  return {
    total: usage ? formatSize(usage.totalBytes) : UNKNOWN,
    used: usage ? formatSize(used ?? 0) : UNKNOWN,
    percent: usage ? percentOf(share(used ?? 0, usage.totalBytes)) : UNKNOWN,
    available: usage ? formatSize(usage.availableBytes) : UNKNOWN,
  };
});

const swapRows = computed(() => {
  const usage = memory.value;
  const used = usage ? swapUsed(usage) : null;
  return {
    total: usage ? formatSize(usage.swapTotalBytes) : UNKNOWN,
    used: usage ? formatSize(used ?? 0) : UNKNOWN,
    percent: usage ? percentOf(share(used ?? 0, usage.swapTotalBytes)) : UNKNOWN,
    available: usage ? formatSize(usage.swapFreeBytes) : UNKNOWN,
  };
});

/** The share each CPU column was of the last interval, in the order the page lists them. */
const cpuShares = computed(() =>
  (monitor.cpuShares ?? []).map((share) => ({ state: share.state, labelKey: CPU_STATE_LABELS[share.state], share })),
);

/** Used space and how much of the file system it is, which is the column people read. */
function diskUsed(size: number, available: number): { used: string; percent: string } {
  const used = Math.max(0, size - available);
  return { used: formatSize(used), percent: percentOf(share(used, size)) };
}
</script>

<template>
  <div class="min-h-0 flex-1 space-y-5 overflow-auto">
    <section>
      <h3 class="mb-1.5 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
        {{ t("inspect.system.group") }}
      </h3>
      <dl class="grid grid-cols-[7rem_1fr] gap-x-4 gap-y-1 text-xs">
        <dt class="text-muted-foreground">{{ t("monitor.system.os") }}</dt>
        <dd>{{ orUnknown(sample?.os) }}</dd>

        <dt class="text-muted-foreground">{{ t("monitor.system.kernel") }}</dt>
        <dd class="font-mono">{{ orUnknown(sample?.kernel) }}</dd>

        <dt class="text-muted-foreground">{{ t("inspect.system.arch") }}</dt>
        <dd class="font-mono">{{ orUnknown(sample?.arch) }}</dd>

        <dt class="text-muted-foreground">{{ t("monitor.system.hostname") }}</dt>
        <dd class="font-mono">{{ orUnknown(sample?.hostname) }}</dd>

        <dt class="text-muted-foreground">{{ t("monitor.uptime.label") }}</dt>
        <dd class="font-mono">{{ upFor }}</dd>
      </dl>
    </section>

    <section>
      <h3 class="mb-1.5 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
        {{ t("monitor.cpu") }}
      </h3>
      <table :class="TABLE.page.table" :aria-label="t('monitor.cpu')">
        <thead :class="TABLE.page.head">
          <tr :class="TABLE.page.headRow">
            <th class="text-left" :class="TABLE.page.headCell">{{ t("inspect.cpu.name") }}</th>
            <th class="w-20 text-right" :class="TABLE.page.headCell">{{ t("inspect.cpu.cores") }}</th>
            <th class="w-28 text-right" :class="TABLE.page.headCell">{{ t("inspect.cpu.frequency") }}</th>
            <th class="w-24 text-right" :class="TABLE.page.headCell">{{ t("inspect.cpu.cache") }}</th>
            <th class="w-24 text-right" :class="TABLE.page.headCell">{{ t("inspect.cpu.bogomips") }}</th>
          </tr>
        </thead>
        <tbody>
          <tr :class="rowClass('page')">
            <td :class="TABLE.page.cell">{{ orUnknown(sample?.cpuModel?.name) }}</td>
            <td class="text-right font-mono" :class="TABLE.page.cell">{{ orUnknown(sample?.cores) }}</td>
            <td class="text-right font-mono" :class="TABLE.page.cell">
              {{
                sample?.cpuModel?.frequencyMhz === null || sample?.cpuModel?.frequencyMhz === undefined
                  ? UNKNOWN
                  : t("inspect.cpu.megahertz", { value: sample.cpuModel.frequencyMhz.toFixed(3) })
              }}
            </td>
            <td class="text-right font-mono" :class="TABLE.page.cell">
              {{
                sample?.cpuModel?.cacheKb === null || sample?.cpuModel?.cacheKb === undefined
                  ? UNKNOWN
                  : t("inspect.cpu.kilobytes", { value: sample.cpuModel.cacheKb })
              }}
            </td>
            <td class="text-right font-mono" :class="TABLE.page.cell">{{ orUnknown(sample?.cpuModel?.bogomips) }}</td>
          </tr>
        </tbody>
      </table>

      <p class="mt-2 mb-1 text-[11px] text-muted-foreground">{{ t("inspect.cpu.shareHint") }}</p>
      <table :class="TABLE.page.table" :aria-label="t('inspect.cpu.share')">
        <thead :class="TABLE.page.head">
          <tr :class="TABLE.page.headRow">
            <th v-for="entry in cpuShares" :key="entry.share.state" class="text-right" :class="TABLE.page.headCell">
              {{ t(entry.labelKey) }}
            </th>
            <th v-if="cpuShares.length === 0" class="text-left" :class="TABLE.page.headCell">
              {{ t("inspect.cpu.noShare") }}
            </th>
          </tr>
        </thead>
        <tbody>
          <tr :class="rowClass('page')">
            <td
              v-for="entry in cpuShares"
              :key="entry.share.state"
              class="text-right font-mono"
              :class="TABLE.page.cell"
            >
              {{ percentOf(entry.share.percent, 1) }}
            </td>
            <td v-if="cpuShares.length === 0" class="text-muted-foreground" :class="TABLE.page.cell">{{ UNKNOWN }}</td>
          </tr>
        </tbody>
      </table>
    </section>

    <section class="grid gap-5 lg:grid-cols-2">
      <div>
        <h3 class="mb-1.5 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
          {{ t("monitor.memory") }}
        </h3>
        <dl class="grid grid-cols-[4rem_1fr] gap-x-4 gap-y-1 text-xs">
          <dt class="text-muted-foreground">{{ t("inspect.memory.total") }}</dt>
          <dd class="font-mono">{{ memoryRows.total }}</dd>
          <dt class="text-muted-foreground">{{ t("inspect.memory.used") }}</dt>
          <dd class="font-mono">{{ memoryRows.used }}</dd>
          <dt class="text-muted-foreground">{{ t("inspect.memory.percent") }}</dt>
          <dd class="font-mono">{{ memoryRows.percent }}</dd>
          <dt class="text-muted-foreground">{{ t("inspect.memory.available") }}</dt>
          <dd class="font-mono">{{ memoryRows.available }}</dd>
        </dl>
      </div>

      <div>
        <h3 class="mb-1.5 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
          {{ t("monitor.swap") }}
        </h3>
        <dl class="grid grid-cols-[4rem_1fr] gap-x-4 gap-y-1 text-xs">
          <dt class="text-muted-foreground">{{ t("inspect.memory.total") }}</dt>
          <dd class="font-mono">{{ swapRows.total }}</dd>
          <dt class="text-muted-foreground">{{ t("inspect.memory.used") }}</dt>
          <dd class="font-mono">{{ swapRows.used }}</dd>
          <dt class="text-muted-foreground">{{ t("inspect.memory.percent") }}</dt>
          <dd class="font-mono">{{ swapRows.percent }}</dd>
          <dt class="text-muted-foreground">{{ t("inspect.memory.available") }}</dt>
          <dd class="font-mono">{{ swapRows.available }}</dd>
        </dl>
      </div>
    </section>

    <section>
      <h3 class="mb-1.5 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
        {{ t("inspect.network.group") }}
      </h3>
      <InterfaceTable :live="props.live" />
    </section>

    <section>
      <h3 class="mb-1.5 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
        {{ t("monitor.disks.title") }}
      </h3>
      <table :class="TABLE.page.table" :aria-label="t('monitor.disks.title')">
        <thead :class="TABLE.page.head">
          <tr :class="TABLE.page.headRow">
            <th class="text-left" :class="TABLE.page.headCell">{{ t("inspect.disk.name") }}</th>
            <th class="text-right" :class="TABLE.page.headCell">{{ t("inspect.disk.size") }}</th>
            <th class="text-right" :class="TABLE.page.headCell">{{ t("inspect.disk.used") }}</th>
            <th class="text-right" :class="TABLE.page.headCell">{{ t("inspect.disk.percent") }}</th>
            <th class="text-right" :class="TABLE.page.headCell">{{ t("inspect.disk.available") }}</th>
            <th class="text-right" :class="TABLE.page.headCell">{{ t("inspect.disk.mount") }}</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="disk in sample?.disks ?? []"
            :key="`${disk.filesystem}:${disk.mountPoint}`"
            :class="rowClass('page')"
          >
            <td class="max-w-0 truncate font-mono" :class="TABLE.page.cell" :title="disk.filesystem">
              {{ disk.filesystem }}
            </td>
            <td class="text-right font-mono" :class="TABLE.page.cell">{{ formatSize(disk.sizeBytes) }}</td>
            <td class="text-right font-mono" :class="TABLE.page.cell">
              {{ diskUsed(disk.sizeBytes, disk.availableBytes).used }}
            </td>
            <td class="text-right font-mono" :class="TABLE.page.cell">
              {{ diskUsed(disk.sizeBytes, disk.availableBytes).percent }}
            </td>
            <td class="text-right font-mono" :class="TABLE.page.cell">{{ formatSize(disk.availableBytes) }}</td>
            <td class="text-right font-mono" :class="TABLE.page.cell">{{ disk.mountPoint }}</td>
          </tr>
          <tr v-if="live && (sample?.disks.length ?? 0) === 0">
            <td colspan="6" :class="TABLE.page.emptyCell">{{ t("monitor.disks.empty") }}</td>
          </tr>
        </tbody>
      </table>
    </section>
  </div>
</template>
