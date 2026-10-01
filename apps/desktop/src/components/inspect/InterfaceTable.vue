<script setup lang="ts">
/**
 * The interfaces a host reported, and what each has carried.
 *
 * Drawn on two pages — the system report lists them among everything else, and the network page is
 * about them — so it lives in one place. Both read the same sample, which is what keeps the totals
 * on the two pages identical rather than merely consistent.
 *
 * The rates are dashes on the first sample after a connection opens, and that is not a gap to be
 * filled in: a rate is a difference between two readings, and there is only one.
 */
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import { formatSize } from "@/lib/files/format";
import { UNKNOWN, perSecond } from "@/lib/monitor/format";
import { useMonitorStore } from "@/stores/monitor";

const props = defineProps<{
  /** Whether there is a sample about this page's host. Without one the table draws dashes. */
  live: boolean;
}>();

const { t } = useI18n();
const monitor = useMonitorStore();

/**
 * The interfaces to list, and nothing at all when the sample is about another host: a page that is
 * hidden behind a tab is still mounted, and it must not read the store's numbers as its own.
 */
const entries = computed(() => (props.live ? (monitor.sample?.network ?? []) : []));

function rate(name: string, direction: "received" | "transmitted"): string {
  const entry = props.live ? monitor.rates.find((rate) => rate.name === name) : undefined;
  return entry ? perSecond(entry[direction]) : UNKNOWN;
}
</script>

<template>
  <table class="w-full border-collapse text-xs">
    <thead>
      <tr class="border-b border-border text-muted-foreground">
        <th class="py-1 pr-2 text-left font-medium">{{ t("inspect.network.name") }}</th>
        <th class="py-1 pr-2 text-right font-medium">{{ t("inspect.network.sent") }}</th>
        <th class="py-1 pr-2 text-right font-medium">{{ t("inspect.network.received") }}</th>
        <th class="py-1 pr-2 text-right font-medium">{{ t("inspect.network.sentRate") }}</th>
        <th class="py-1 text-right font-medium">{{ t("inspect.network.receivedRate") }}</th>
      </tr>
    </thead>
    <tbody>
      <tr v-for="entry in entries" :key="entry.name" class="border-b border-border/50">
        <td class="truncate py-1 pr-2 font-mono">{{ entry.name }}</td>
        <td class="py-1 pr-2 text-right font-mono">{{ formatSize(entry.transmittedBytes) }}</td>
        <td class="py-1 pr-2 text-right font-mono">{{ formatSize(entry.receivedBytes) }}</td>
        <td class="py-1 pr-2 text-right font-mono">{{ rate(entry.name, "transmitted") }}</td>
        <td class="py-1 text-right font-mono">{{ rate(entry.name, "received") }}</td>
      </tr>
      <tr v-if="props.live && entries.length === 0">
        <td colspan="5" class="py-2 text-muted-foreground">{{ t("monitor.network.empty") }}</td>
      </tr>
    </tbody>
  </table>
</template>
